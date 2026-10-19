//! Mute / unmute system audio output during recording.
//!
//! - **Windows**: WASAPI — mutes every active render device.
//! - **macOS**: AppleScript `osascript` — `set volume output muted true/false`.
//! - **Linux**: `pactl` (PulseAudio/PipeWire) with `amixer` (ALSA) fallback.
//!
//! Only what Taurscribe itself muted is ever restored. What was muted is written
//! to a marker file in the app data folder, so `unmute()` after a crash (it runs
//! at startup) restores exactly that and nothing else: a device the user had
//! muted stays muted, and an app launch never unmutes a muted computer.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// Serialises mute/unmute so a slow unmute cannot interleave with the next mute.
static LOCK: Mutex<()> = Mutex::new(());

/// What Taurscribe changed, and how to put it back.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
struct MutedByUs {
    /// macOS / Linux: the output was muted by us.
    #[serde(default)]
    system: bool,
    /// Linux: which tool muted it ("pactl" or "amixer").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    /// Windows: devices we changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    devices: Vec<MutedDevice>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct MutedDevice {
    id: String,
    /// Set when muting failed and the volume was lowered to 0 instead; the
    /// volume to restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_volume: Option<f32>,
}

impl MutedByUs {
    fn is_empty(&self) -> bool {
        !self.system && self.devices.is_empty()
    }
}

fn marker_path() -> Option<std::path::PathBuf> {
    crate::storage::app_data_root().ok().map(|d| d.join("audio-muted-by-taurscribe.json"))
}

fn load_marker() -> Option<MutedByUs> {
    let text = std::fs::read_to_string(marker_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_marker(state: &MutedByUs) {
    let Some(path) = marker_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(state) {
        if let Err(e) = std::fs::write(&path, json) {
            eprintln!("[AUDIO] Could not record mute state: {}", e);
        }
    }
}

fn clear_marker() {
    if let Some(path) = marker_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Mutes system output for a recording. Outputs that are already muted are left
/// alone (and are not unmuted later).
pub fn mute() -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // A previous mute that was never restored (e.g. two mutes in a row) is
    // still ours: keep its record so the later unmute restores it.
    let mut state = load_marker().unwrap_or_default();
    platform::mute(&mut state)?;
    if state.is_empty() {
        clear_marker();
    } else {
        save_marker(&state);
    }
    Ok(())
}

/// Restores whatever `mute()` changed. Does nothing when Taurscribe muted
/// nothing, so it is safe to call at startup, on exit and after every recording.
pub fn unmute() -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(state) = load_marker() else {
        return Ok(());
    };
    let result = platform::unmute(&state);
    clear_marker();
    if result.is_ok() && !state.is_empty() {
        println!("[AUDIO] System audio restored after recording");
    }
    result
}

// ── Windows implementation ──────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod platform {
    use super::{MutedByUs, MutedDevice};
    use windows::core::{Error as WinError, PCWSTR};
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    };

    /// Initialises COM for this call and uninitialises it on drop (only when this
    /// call was the one that initialised it).
    struct ComScope(bool);

    impl ComScope {
        fn enter() -> Result<Self, String> {
            const RPC_E_CHANGED_MODE: i32 = 0x80010106u32 as i32;
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_ok() {
                Ok(ComScope(true))
            } else if hr.0 == RPC_E_CHANGED_MODE {
                Ok(ComScope(false))
            } else {
                Err(format!("COM init failed: {}", WinError::from(hr)))
            }
        }
    }

    impl Drop for ComScope {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn enumerator() -> Result<IMMDeviceEnumerator, String> {
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| format!("Failed to create device enumerator: {}", e))
    }

    fn device_id(device: &IMMDevice) -> Option<String> {
        unsafe {
            let raw = device.GetId().ok()?;
            let id = raw.to_string().ok();
            CoTaskMemFree(Some(raw.0 as *const _));
            id
        }
    }

    pub fn mute(state: &mut MutedByUs) -> Result<(), String> {
        let _com = ComScope::enter()?;
        let enumerator = enumerator()?;
        unsafe {
            let collection = enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .map_err(|e| format!("Failed to list audio outputs: {}", e))?;
            let count = collection.GetCount().unwrap_or(0);
            for i in 0..count {
                let Ok(device) = collection.Item(i) else { continue };
                let Some(id) = device_id(&device) else { continue };
                if state.devices.iter().any(|d| d.id == id) {
                    continue; // already ours from an earlier mute
                }
                let Ok(volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) else {
                    continue;
                };
                match volume.GetMute() {
                    Ok(muted) if muted.as_bool() => continue, // the user's mute: leave it
                    Ok(_) => {}
                    Err(_) => continue,
                }
                if volume.SetMute(true, std::ptr::null()).is_ok() {
                    state.devices.push(MutedDevice { id, previous_volume: None });
                } else if let Ok(previous) = volume.GetMasterVolumeLevelScalar() {
                    if volume.SetMasterVolumeLevelScalar(0.0, std::ptr::null()).is_ok() {
                        state.devices.push(MutedDevice { id, previous_volume: Some(previous) });
                    }
                }
            }
        }
        if !state.devices.is_empty() {
            println!("[AUDIO] System audio muted for recording ({} device(s))", state.devices.len());
        } else {
            println!("[AUDIO] System audio was already muted, skipping");
        }
        Ok(())
    }

    pub fn unmute(state: &MutedByUs) -> Result<(), String> {
        if state.devices.is_empty() {
            return Ok(());
        }
        let _com = ComScope::enter()?;
        let enumerator = enumerator()?;
        for muted in &state.devices {
            let wide: Vec<u16> = muted.id.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                // A device unplugged since the recording simply isn't found.
                let Ok(device) = enumerator.GetDevice(PCWSTR(wide.as_ptr())) else { continue };
                let Ok(volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) else {
                    continue;
                };
                match muted.previous_volume {
                    Some(level) => {
                        let _ = volume.SetMasterVolumeLevelScalar(level, std::ptr::null());
                    }
                    None => {
                        let _ = volume.SetMute(false, std::ptr::null());
                    }
                }
            }
        }
        Ok(())
    }
}

// ── macOS implementation (AppleScript) ────────────────────────────────────────

#[cfg(target_os = "macos")]
mod platform {
    use super::MutedByUs;

    pub fn mute(state: &mut MutedByUs) -> Result<(), String> {
        if state.system {
            return Ok(());
        }
        let check = std::process::Command::new("osascript")
            .args(["-e", "output muted of (get volume settings)"])
            .output()
            .map_err(|e| format!("osascript check failed: {}", e))?;
        let already = String::from_utf8_lossy(&check.stdout)
            .trim()
            .eq_ignore_ascii_case("true");
        if already {
            println!("[AUDIO] System audio was already muted, skipping");
            return Ok(());
        }

        let status = std::process::Command::new("osascript")
            .args(["-e", "set volume output muted true"])
            .status()
            .map_err(|e| format!("osascript mute failed: {}", e))?;
        if status.success() {
            state.system = true;
            println!("[AUDIO] System audio muted for recording (macOS)");
        }
        Ok(())
    }

    pub fn unmute(state: &MutedByUs) -> Result<(), String> {
        if !state.system {
            return Ok(());
        }
        std::process::Command::new("osascript")
            .args(["-e", "set volume output muted false"])
            .status()
            .map_err(|e| format!("osascript unmute failed: {}", e))?;
        Ok(())
    }
}

// ── Linux implementation (PulseAudio/PipeWire → ALSA fallback) ─────────────────

#[cfg(target_os = "linux")]
mod platform {
    use super::MutedByUs;

    pub fn mute(state: &mut MutedByUs) -> Result<(), String> {
        if state.system {
            return Ok(());
        }
        // Try pactl first (PulseAudio / PipeWire)
        if let Ok(out) = std::process::Command::new("pactl")
            .args(["get-sink-mute", "@DEFAULT_SINK@"])
            .output()
        {
            if out.status.success() {
                let out = String::from_utf8_lossy(&out.stdout).to_lowercase();
                if out.contains("yes") || out.contains("true") {
                    println!("[AUDIO] System audio was already muted, skipping");
                    return Ok(());
                }
                let status = std::process::Command::new("pactl")
                    .args(["set-sink-mute", "@DEFAULT_SINK@", "1"])
                    .status()
                    .map_err(|e| format!("pactl set-sink-mute failed: {}", e))?;
                if status.success() {
                    state.system = true;
                    state.tool = Some("pactl".into());
                    println!("[AUDIO] System audio muted for recording (PulseAudio/PipeWire)");
                }
                return Ok(());
            }
        }

        // Fallback: amixer (ALSA)
        let check = std::process::Command::new("amixer")
            .args(["-D", "default", "get", "Master"])
            .output()
            .map_err(|e| format!("amixer get failed: {}", e))?;
        let out = String::from_utf8_lossy(&check.stdout);
        if out.contains("[off]") {
            println!("[AUDIO] System audio was already muted, skipping");
            return Ok(());
        }
        let status = std::process::Command::new("amixer")
            .args(["-D", "default", "set", "Master", "mute"])
            .status()
            .map_err(|e| format!("amixer mute failed: {}", e))?;
        if status.success() {
            state.system = true;
            state.tool = Some("amixer".into());
            println!("[AUDIO] System audio muted for recording (ALSA)");
        }
        Ok(())
    }

    pub fn unmute(state: &MutedByUs) -> Result<(), String> {
        if !state.system {
            return Ok(());
        }
        if state.tool.as_deref() != Some("amixer") {
            if let Ok(status) = std::process::Command::new("pactl")
                .args(["set-sink-mute", "@DEFAULT_SINK@", "0"])
                .status()
            {
                if status.success() {
                    return Ok(());
                }
            }
        }
        std::process::Command::new("amixer")
            .args(["-D", "default", "set", "Master", "unmute"])
            .status()
            .map_err(|e| format!("amixer unmute failed: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_round_trips_device_volumes() {
        let state = MutedByUs {
            system: false,
            tool: None,
            devices: vec![
                MutedDevice { id: "{0.0.0}.{abc}".into(), previous_volume: None },
                MutedDevice { id: "{0.0.0}.{def}".into(), previous_volume: Some(0.4) },
            ],
        };
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<MutedByUs>(&json).unwrap(), state);
        assert!(MutedByUs::default().is_empty());
    }
}
