//! Mute / unmute system audio output during recording.
//!
//! - **Windows**: WASAPI — mutes all default roles (Multimedia, Console, Communications)
//!   and enumerates all active render devices.
//! - **macOS**: AppleScript `osascript` — `set volume output muted true/false`.
//! - **Linux**: `pactl` (PulseAudio/PipeWire) with `amixer` (ALSA) fallback.

use std::sync::atomic::{AtomicBool, Ordering};

/// Tracks whether the system was already muted before we touched it.
/// When true, `unmute()` will skip restoring so we don't override a manual mute.
static WAS_ALREADY_MUTED: AtomicBool = AtomicBool::new(false);

/// Tracks which endpoints we muted (so we restore only those).
/// Bit 0 = multimedia, bit 1 = console, bit 2 = communications.
static WE_MUTED_ROLES: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

// ── Windows implementation ──────────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub fn mute() -> Result<(), String> {
    use windows::core::Error as WinError;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eCommunications, eConsole, eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
        DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    unsafe {
        let mut did_init_com = false;
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if hr.is_ok() {
            did_init_com = true;
        } else {
            const RPC_E_CHANGED_MODE: i32 = 0x80010106u32 as i32;
            if hr.0 != RPC_E_CHANGED_MODE {
                return Err(format!("COM init failed: {}", WinError::from(hr)));
            }
        }

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| format!("Failed to create device enumerator: {}", e))?;

        let mut we_muted = 0u8;
        let mut any_already_muted = false;

        // 1. Mute all three default roles (Multimedia, Console, Communications)
        let roles: [(u8, windows::Win32::Media::Audio::ERole); 3] = [
            (1 << 0, eMultimedia),
            (1 << 1, eConsole),
            (1 << 2, eCommunications),
        ];

        for (bit, role) in roles {
            if let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, role) {
                if let Ok(volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    if let Ok(muted) = volume.GetMute() {
                        if muted.as_bool() {
                            any_already_muted = true;
                        } else {
                            if volume.SetMute(true, std::ptr::null()).is_ok() {
                                we_muted |= bit;
                            } else if volume
                                .SetMasterVolumeLevelScalar(0.0, std::ptr::null())
                                .is_ok()
                            {
                                we_muted |= bit;
                            }
                        }
                    }
                }
            }
        }

        // 2. Enumerate ALL active render devices and mute each
        if let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
            let count = collection.GetCount().unwrap_or(0);
            for i in 0..count {
                if let Ok(device) = collection.Item(i) {
                    if let Ok(volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                        let _ = volume.SetMute(true, std::ptr::null());
                    }
                }
            }
            if count > 0 {
                we_muted |= 0x80; // Mark that we enumerated and muted
            }
        }

        WE_MUTED_ROLES.store(we_muted, Ordering::SeqCst);
        WAS_ALREADY_MUTED.store(any_already_muted && we_muted == 0, Ordering::SeqCst);

        if we_muted != 0 {
            println!(
                "[AUDIO] System audio muted for recording (roles={:#x})",
                we_muted
            );
        } else if any_already_muted {
            println!("[AUDIO] System audio was already muted, skipping");
        }

        if did_init_com {
            CoUninitialize();
        }
    }

    Ok(())
}

#[cfg(target_os = "windows")]
pub fn unmute() -> Result<(), String> {
    use windows::core::Error as WinError;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eCommunications, eConsole, eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
        DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    if WAS_ALREADY_MUTED.load(Ordering::SeqCst) {
        println!("[AUDIO] System was already muted before recording, leaving muted");
        return Ok(());
    }

    let we_muted = WE_MUTED_ROLES.swap(0, Ordering::SeqCst);
    if we_muted == 0 {
        return Ok(());
    }

    unsafe {
        let mut did_init_com = false;
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if hr.is_ok() {
            did_init_com = true;
        } else {
            const RPC_E_CHANGED_MODE: i32 = 0x80010106u32 as i32;
            if hr.0 != RPC_E_CHANGED_MODE {
                return Err(format!("COM init failed: {}", WinError::from(hr)));
            }
        }

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| format!("Failed to create device enumerator: {}", e))?;

        // Unmute all three roles we touched
        let roles: [(u8, windows::Win32::Media::Audio::ERole); 3] = [
            (1 << 0, eMultimedia),
            (1 << 1, eConsole),
            (1 << 2, eCommunications),
        ];

        for (bit, role) in roles {
            if (we_muted & bit) != 0 {
                if let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, role) {
                    if let Ok(volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                        let _ = volume.SetMute(false, std::ptr::null());
                    }
                }
            }
        }

        // Unmute all enumerated devices
        if (we_muted & 0x80) != 0 {
            if let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
                let count = collection.GetCount().unwrap_or(0);
                for i in 0..count {
                    if let Ok(device) = collection.Item(i) {
                        if let Ok(volume) =
                            device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                        {
                            let _ = volume.SetMute(false, std::ptr::null());
                        }
                    }
                }
            }
        }

        println!("[AUDIO] System audio unmuted after recording");

        if did_init_com {
            CoUninitialize();
        }
    }

    Ok(())
}

/// Unconditionally unmute all audio endpoints. Used at startup to recover
/// from a crash that left the system muted.
#[cfg(target_os = "windows")]
pub fn force_unmute() -> Result<(), String> {
    use windows::core::Error as WinError;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eCommunications, eConsole, eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
        DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    unsafe {
        let mut did_init_com = false;
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if hr.is_ok() {
            did_init_com = true;
        } else {
            const RPC_E_CHANGED_MODE: i32 = 0x80010106u32 as i32;
            if hr.0 != RPC_E_CHANGED_MODE {
                return Err(format!("COM init failed: {}", WinError::from(hr)));
            }
        }

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| format!("Failed to create device enumerator: {}", e))?;

        // Unmute all three default roles
        for role in [eMultimedia, eConsole, eCommunications] {
            if let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, role) {
                if let Ok(volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    let _ = volume.SetMute(false, std::ptr::null());
                }
            }
        }

        // Unmute all enumerated active render devices
        if let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
            let count = collection.GetCount().unwrap_or(0);
            for i in 0..count {
                if let Ok(device) = collection.Item(i) {
                    if let Ok(volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                        let _ = volume.SetMute(false, std::ptr::null());
                    }
                }
            }
        }

        // Reset stale state
        WAS_ALREADY_MUTED.store(false, Ordering::SeqCst);
        WE_MUTED_ROLES.store(0, Ordering::SeqCst);

        if did_init_com {
            CoUninitialize();
        }
    }

    Ok(())
}

// ── macOS implementation (AppleScript) ────────────────────────────────────────

#[cfg(target_os = "macos")]
pub fn mute() -> Result<(), String> {
    let check = std::process::Command::new("osascript")
        .args(["-e", "output muted of (get volume settings)"])
        .output()
        .map_err(|e| format!("osascript check failed: {}", e))?;

    let already = String::from_utf8_lossy(&check.stdout)
        .trim()
        .eq_ignore_ascii_case("true");
    WAS_ALREADY_MUTED.store(already, Ordering::SeqCst);

    if already {
        println!("[AUDIO] System audio was already muted, skipping");
        return Ok(());
    }

    std::process::Command::new("osascript")
        .args(["-e", "set volume output muted true"])
        .status()
        .map_err(|e| format!("osascript mute failed: {}", e))?;

    println!("[AUDIO] System audio muted for recording (macOS)");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn unmute() -> Result<(), String> {
    if WAS_ALREADY_MUTED.load(Ordering::SeqCst) {
        println!("[AUDIO] System was already muted before recording, leaving muted");
        return Ok(());
    }

    std::process::Command::new("osascript")
        .args(["-e", "set volume output muted false"])
        .status()
        .map_err(|e| format!("osascript unmute failed: {}", e))?;

    println!("[AUDIO] System audio unmuted after recording");
    Ok(())
}

/// Unconditionally unmute. Used at startup to recover from a crash.
#[cfg(target_os = "macos")]
pub fn force_unmute() -> Result<(), String> {
    std::process::Command::new("osascript")
        .args(["-e", "set volume output muted false"])
        .status()
        .map_err(|e| format!("osascript force unmute failed: {}", e))?;
    WAS_ALREADY_MUTED.store(false, Ordering::SeqCst);
    Ok(())
}

// ── Linux implementation (PulseAudio/PipeWire → ALSA fallback) ─────────────────

#[cfg(target_os = "linux")]
pub fn mute() -> Result<(), String> {
    // Try pactl first (PulseAudio / PipeWire)
    let pactl_check = std::process::Command::new("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output();

    if let Ok(out) = pactl_check {
        if out.status.success() {
            let out = String::from_utf8_lossy(&out.stdout);
            let already = out.contains("yes") || out.to_lowercase().contains("true");
            WAS_ALREADY_MUTED.store(already, Ordering::SeqCst);

            if already {
                println!("[AUDIO] System audio was already muted, skipping");
                return Ok(());
            }

            std::process::Command::new("pactl")
                .args(["set-sink-mute", "@DEFAULT_SINK@", "1"])
                .status()
                .map_err(|e| format!("pactl set-sink-mute failed: {}", e))?;

            println!("[AUDIO] System audio muted for recording (PulseAudio/PipeWire)");
            return Ok(());
        }
    }

    // Fallback: amixer (ALSA)
    let check = std::process::Command::new("amixer")
        .args(["-D", "default", "get", "Master"])
        .output()
        .map_err(|e| format!("amixer get failed: {}", e))?;

    let out = String::from_utf8_lossy(&check.stdout);
    let already = out.contains("[off]") || out.to_lowercase().contains("mute");
    WAS_ALREADY_MUTED.store(already, Ordering::SeqCst);

    if already {
        println!("[AUDIO] System audio was already muted, skipping");
        return Ok(());
    }

    std::process::Command::new("amixer")
        .args(["-D", "default", "set", "Master", "mute"])
        .status()
        .map_err(|e| format!("amixer mute failed: {}", e))?;

    println!("[AUDIO] System audio muted for recording (ALSA)");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn unmute() -> Result<(), String> {
    if WAS_ALREADY_MUTED.load(Ordering::SeqCst) {
        println!("[AUDIO] System was already muted before recording, leaving muted");
        return Ok(());
    }

    // Try pactl first (PulseAudio / PipeWire)
    if let Ok(status) = std::process::Command::new("pactl")
        .args(["set-sink-mute", "@DEFAULT_SINK@", "0"])
        .status()
    {
        if status.success() {
            println!("[AUDIO] System audio unmuted after recording");
            return Ok(());
        }
    }

    // Fallback: amixer (ALSA)
    std::process::Command::new("amixer")
        .args(["-D", "default", "set", "Master", "unmute"])
        .status()
        .map_err(|e| format!("amixer unmute failed: {}", e))?;

    println!("[AUDIO] System audio unmuted after recording");
    Ok(())
}

/// Unconditionally unmute. Used at startup to recover from a crash.
#[cfg(target_os = "linux")]
pub fn force_unmute() -> Result<(), String> {
    // Try pactl first
    if let Ok(status) = std::process::Command::new("pactl")
        .args(["set-sink-mute", "@DEFAULT_SINK@", "0"])
        .status()
    {
        if status.success() {
            WAS_ALREADY_MUTED.store(false, Ordering::SeqCst);
            return Ok(());
        }
    }
    // Fallback: amixer
    std::process::Command::new("amixer")
        .args(["-D", "default", "set", "Master", "unmute"])
        .status()
        .map_err(|e| format!("amixer force unmute failed: {}", e))?;
    WAS_ALREADY_MUTED.store(false, Ordering::SeqCst);
    Ok(())
}
