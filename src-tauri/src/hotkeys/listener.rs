use crate::types::{HotkeyBinding, RecordingMode};
use rdev::{listen, Event, EventType, Key};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

const MAX_HOTKEY_KEYS: usize = 2;

/// What a key event asks the frontend to do.
#[derive(Debug, PartialEq)]
enum HotkeyAction {
    Start,
    Stop,
    Toggle,
}

/// Which combo keys are down, and what the combo already did.
struct ComboState {
    held: Vec<&'static str>,
    /// Prevents keyboard auto-repeat from firing the action multiple times per physical press.
    triggered: bool,
    recording_active: bool,
    /// Asks the OS whether a key is really down (None: can't tell). Tests use
    /// their own, since their key events are not real keyboard state.
    is_down: fn(&str) -> Option<bool>,
}

impl Default for ComboState {
    fn default() -> Self {
        Self {
            held: Vec::new(),
            triggered: false,
            recording_active: false,
            is_down: if cfg!(test) { |_| None } else { key_physically_down },
        }
    }
}

/// Whether the key is physically down right now, asked of the OS. None where
/// the OS can't tell (Linux, CapsLock whose state is a toggle).
///
/// A key release the listener never sees (screen lock, a password field's
/// Secure Input, focus moving to a privileged window) would otherwise leave the
/// key "held" forever, and pressing the combo's other key alone would fire it.
#[cfg(target_os = "macos")]
fn key_physically_down(code: &str) -> Option<bool> {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
    }
    const HID_SYSTEM_STATE: i32 = 1;
    let keycode: u16 = match code {
        "ControlLeft" => 0x3B,
        "ControlRight" => 0x3E,
        "MetaLeft" => 0x37,
        "MetaRight" => 0x36,
        "ShiftLeft" => 0x38,
        "ShiftRight" => 0x3C,
        "AltLeft" => 0x3A,
        "AltRight" => 0x3D,
        "Tab" => 0x30,
        "Escape" => 0x35,
        "F1" => 0x7A,
        "F2" => 0x78,
        "F3" => 0x63,
        "F4" => 0x76,
        "F5" => 0x60,
        "F6" => 0x61,
        "F7" => 0x62,
        "F8" => 0x64,
        "F9" => 0x65,
        "F10" => 0x6D,
        "F11" => 0x67,
        "F12" => 0x6F,
        _ => return None,
    };
    Some(unsafe { CGEventSourceKeyState(HID_SYSTEM_STATE, keycode) })
}

#[cfg(target_os = "windows")]
fn key_physically_down(code: &str) -> Option<bool> {
    extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }
    let vk: i32 = match code {
        "ControlLeft" => 0xA2,
        "ControlRight" => 0xA3,
        "MetaLeft" => 0x5B,
        "MetaRight" => 0x5C,
        "ShiftLeft" => 0xA0,
        "ShiftRight" => 0xA1,
        "AltLeft" => 0xA4,
        "AltRight" => 0xA5,
        "Tab" => 0x09,
        "Escape" => 0x1B,
        f if f.starts_with('F') => match f[1..].parse::<i32>() {
            Ok(n @ 1..=12) => 0x6F + n,
            _ => return None,
        },
        _ => return None,
    };
    Some(unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn key_physically_down(_code: &str) -> Option<bool> {
    None
}


impl ComboState {
    fn on_event(&mut self, event: &EventType, config: &HotkeyBinding, suppressed: bool) -> Option<HotkeyAction> {
        if config.keys.len() != MAX_HOTKEY_KEYS {
            return None;
        }
        match event {
            // Settings suppresses the hotkey while it is open. Releases still
            // count: the hotkey opens Settings when no model is installed, so its
            // keys come up while suppressed, and a key left "held" would let the
            // other key alone fire the combo later.
            EventType::KeyPress(_) if suppressed => None,
            EventType::KeyPress(key) => {
                let code = key_to_code(key)?;
                if config.keys.iter().any(|k| k == code) && !self.held.contains(&code) {
                    self.held.push(code);
                }
                // Drop keys whose release was missed (screen lock, a password
                // field's Secure Input). The key of this event is trusted as is;
                // only earlier ones are re-checked.
                let is_down = self.is_down;
                self.held.retain(|h| *h == code || is_down(h) != Some(false));
                let all_held = config.keys.iter().all(|k| self.held.iter().any(|h| k == h));
                if !all_held || self.triggered {
                    return None;
                }
                self.triggered = true;
                match config.mode {
                    RecordingMode::Hold if !self.recording_active => {
                        self.recording_active = true;
                        Some(HotkeyAction::Start)
                    }
                    RecordingMode::Hold => None,
                    // React resolves this against the actual recording lifecycle.
                    // A speculative native flag diverges after a failed start or
                    // when recording is stopped from the UI.
                    RecordingMode::Toggle => Some(HotkeyAction::Toggle),
                }
            }
            EventType::KeyRelease(key) => {
                let code = key_to_code(key)?;
                self.held.retain(|k| *k != code);
                if !config.keys.iter().any(|k| k == code) {
                    return None;
                }
                // Reset so the next physical key press can trigger the combo again.
                self.triggered = false;
                // Hold mode: releasing any combo key stops recording.
                // Toggle mode: key releases have no effect on recording state.
                if config.mode == RecordingMode::Hold && self.recording_active {
                    self.recording_active = false;
                    return Some(HotkeyAction::Stop);
                }
                None
            }
            _ => None,
        }
    }
}

/// Map an rdev Key to a stable string code matching browser KeyboardEvent.code names.
fn key_to_code(key: &Key) -> Option<&'static str> {
    match key {
        Key::ControlLeft => Some("ControlLeft"),
        Key::ControlRight => Some("ControlRight"),
        Key::MetaLeft => Some("MetaLeft"),
        Key::MetaRight => Some("MetaRight"),
        Key::ShiftLeft => Some("ShiftLeft"),
        Key::ShiftRight => Some("ShiftRight"),
        Key::Alt => Some("AltLeft"),
        Key::AltGr => Some("AltRight"),
        Key::CapsLock => Some("CapsLock"),
        Key::Tab => Some("Tab"),
        Key::Escape => Some("Escape"),
        Key::F1 => Some("F1"),
        Key::F2 => Some("F2"),
        Key::F3 => Some("F3"),
        Key::F4 => Some("F4"),
        Key::F5 => Some("F5"),
        Key::F6 => Some("F6"),
        Key::F7 => Some("F7"),
        Key::F8 => Some("F8"),
        Key::F9 => Some("F9"),
        Key::F10 => Some("F10"),
        Key::F11 => Some("F11"),
        Key::F12 => Some("F12"),
        _ => None,
    }
}

/// Start the global keyboard listener. Reads hotkey_config on every event so
/// changes take effect immediately without restarting the thread.
pub fn start_hotkey_listener(
    app_handle: tauri::AppHandle,
    hotkey_config: Arc<RwLock<HotkeyBinding>>,
    hotkey_suppressed: Arc<AtomicBool>,
) {
    use tauri::Emitter;

    // macOS fix: Check Accessibility (Input Monitoring) permission on launch.
    // rdev uses CGEventTap under the hood, which silently receives zero events
    // if the app is not trusted. Without this check the hotkey listener starts
    // but never fires, with no error message. If permission is missing we emit
    // an "accessibility-missing" event so the React frontend can show a banner
    // telling the user to grant permission in System Settings.
    #[cfg(target_os = "macos")]
    {
        // Silent check — do NOT prompt every launch. The setup UI handles
        // prompting and deep-links to System Settings when needed.
        let accessibility_trusted = macos_accessibility_trusted(false);
        let input_monitoring_trusted = macos_input_monitoring_trusted();
        if !accessibility_trusted || !input_monitoring_trusted {
            eprintln!(
                "[WARN] Hotkey permissions missing — accessibility: {}, input monitoring: {}",
                accessibility_trusted, input_monitoring_trusted
            );
            eprintln!(
                "[WARN] Grant Taurscribe access in System Settings → Privacy & Security → Accessibility and Input Monitoring."
            );
            let _ = app_handle.emit("accessibility-missing", ());
        } else {
            println!("[INFO] Hotkey permissions granted");
        }
    }

    let mut combo = ComboState::default();
    let app_c = app_handle.clone();
    let config_c = hotkey_config.clone();
    let suppressed_c = hotkey_suppressed.clone();

    let callback = move |event: Event| {
        let action = {
            let config = config_c.read().unwrap();
            combo.on_event(&event.event_type, &config, suppressed_c.load(Ordering::SeqCst))
        };
        match action {
            Some(HotkeyAction::Start) => {
                println!("[HOTKEY] Hold — starting recording");
                let _ = app_c.emit("hotkey-start-recording", ());
            }
            Some(HotkeyAction::Stop) => {
                println!("[HOTKEY] Hold — stopping recording");
                let _ = app_c.emit("hotkey-stop-recording", ());
            }
            Some(HotkeyAction::Toggle) => {
                let _ = app_c.emit("hotkey-toggle-recording", ());
            }
            None => {}
        }
    };

    if let Err(e) = listen(callback) {
        eprintln!("[ERROR] Hotkey listener error: {:?}", e);
    }
}

/// macOS fix: Check whether this process has Accessibility / Input Monitoring trust.
/// Uses the private AXIsProcessTrustedWithOptions API. When `prompt` is true,
/// macOS shows the system permission dialog if the user hasn't decided yet.
/// This is required because rdev's CGEventTap silently fails without it.
#[cfg(target_os = "macos")]
fn macos_accessibility_trusted(prompt: bool) -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: core_foundation::base::CFTypeRef) -> bool;
    }

    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = if prompt {
        CFBoolean::true_value()
    } else {
        CFBoolean::false_value()
    };
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);

    unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef()) }
}

#[cfg(target_os = "macos")]
fn macos_input_monitoring_trusted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGPreflightListenEventAccess() -> bool;
    }

    unsafe { CGPreflightListenEventAccess() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(mode: RecordingMode) -> HotkeyBinding {
        HotkeyBinding { keys: vec!["ControlLeft".into(), "AltLeft".into()], mode }
    }
    fn press(s: &mut ComboState, c: &HotkeyBinding, key: Key, suppressed: bool) -> Option<HotkeyAction> {
        s.on_event(&EventType::KeyPress(key), c, suppressed)
    }
    fn release(s: &mut ComboState, c: &HotkeyBinding, key: Key, suppressed: bool) -> Option<HotkeyAction> {
        s.on_event(&EventType::KeyRelease(key), c, suppressed)
    }

    #[test]
    fn combo_fires_once_per_press_and_hold_stops_on_release() {
        let c = binding(RecordingMode::Hold);
        let mut s = ComboState::default();
        assert_eq!(press(&mut s, &c, Key::ControlLeft, false), None);
        assert_eq!(press(&mut s, &c, Key::Alt, false), Some(HotkeyAction::Start));
        // Auto-repeat.
        assert_eq!(press(&mut s, &c, Key::Alt, false), None);
        assert_eq!(release(&mut s, &c, Key::Alt, false), Some(HotkeyAction::Stop));
        assert_eq!(release(&mut s, &c, Key::ControlLeft, false), None);
        // Settings is open: the hotkey does nothing.
        assert_eq!(press(&mut s, &c, Key::ControlLeft, true), None);
        assert_eq!(press(&mut s, &c, Key::Alt, true), None);
    }

    #[test]
    fn a_key_whose_release_was_missed_does_not_complete_the_combo() {
        let c = binding(RecordingMode::Toggle);
        // The OS says Control is up: its release was never delivered.
        let mut s = ComboState { is_down: |code| Some(code != "ControlLeft"), ..ComboState::default() };
        press(&mut s, &c, Key::ControlLeft, false);
        // Screen locked; Control came up without an event. Option alone now:
        assert_eq!(press(&mut s, &c, Key::Alt, false), None);
        release(&mut s, &c, Key::Alt, false);
        // Pressing both for real still works.
        s.is_down = |_| Some(true);
        press(&mut s, &c, Key::ControlLeft, false);
        assert_eq!(press(&mut s, &c, Key::Alt, false), Some(HotkeyAction::Toggle));
    }

    #[test]
    fn keys_released_while_settings_is_open_are_not_left_held() {
        for mode in [RecordingMode::Toggle, RecordingMode::Hold] {
            let c = binding(mode);
            let mut s = ComboState::default();
            press(&mut s, &c, Key::ControlLeft, false);
            assert!(press(&mut s, &c, Key::Alt, false).is_some());
            // With no model installed the hotkey opens Settings, which suppresses
            // the hotkey before the keys come up.
            release(&mut s, &c, Key::ControlLeft, true);
            release(&mut s, &c, Key::Alt, true);
            // Settings closed. Ctrl on its own (Ctrl+C, Ctrl+V) must not record.
            for _ in 0..3 {
                assert_eq!(press(&mut s, &c, Key::ControlLeft, false), None, "{mode:?}");
                release(&mut s, &c, Key::ControlLeft, false);
            }
            press(&mut s, &c, Key::ControlLeft, false);
            assert!(press(&mut s, &c, Key::Alt, false).is_some(), "{mode:?}: the combo still works");
        }
    }
}
