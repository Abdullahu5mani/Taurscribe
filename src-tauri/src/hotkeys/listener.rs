use crate::types::{HotkeyBinding, RecordingMode};
use rdev::{listen, Event, EventType, Key};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

const MAX_HOTKEY_KEYS: usize = 2;

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
    hotkey_config: Arc<Mutex<HotkeyBinding>>,
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

    let recording_active = Arc::new(AtomicBool::new(false));
    let held_keys: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    // Prevents keyboard auto-repeat from firing the action multiple times per physical press.
    let combo_triggered = Arc::new(AtomicBool::new(false));

    let recording_active_c = recording_active.clone();
    let held_keys_c = held_keys.clone();
    let combo_triggered_c = combo_triggered.clone();
    let app_c = app_handle.clone();
    let config_c = hotkey_config.clone();

    let suppressed_c = hotkey_suppressed.clone();

    let callback = move |event: Event| {
        if suppressed_c.load(Ordering::SeqCst) {
            return;
        }

        let config = config_c.lock().unwrap().clone();

        if config.keys.len() != MAX_HOTKEY_KEYS {
            return;
        }

        match event.event_type {
            EventType::KeyPress(key) => {
                if let Some(code) = key_to_code(&key) {
                    let mut held = held_keys_c.lock().unwrap();
                    if config.keys.contains(&code.to_string()) && !held.contains(&code.to_string()) {
                        held.push(code.to_string());
                    }
                    let all_held = config.keys.iter().all(|k| held.contains(k));
                    if all_held && !config.keys.is_empty() && !combo_triggered_c.load(Ordering::SeqCst) {
                        combo_triggered_c.store(true, Ordering::SeqCst);
                        drop(held);
                        match config.mode {
                            RecordingMode::Hold => {
                                if !recording_active_c.load(Ordering::SeqCst) {
                                    recording_active_c.store(true, Ordering::SeqCst);
                                    println!("[HOTKEY] Hold — starting recording");
                                    let _ = app_c.emit("hotkey-start-recording", ());
                                }
                            }
                            RecordingMode::Toggle => {
                                if recording_active_c.load(Ordering::SeqCst) {
                                    recording_active_c.store(false, Ordering::SeqCst);
                                    println!("[HOTKEY] Toggle — stopping recording");
                                    let _ = app_c.emit("hotkey-stop-recording", ());
                                } else {
                                    recording_active_c.store(true, Ordering::SeqCst);
                                    println!("[HOTKEY] Toggle — starting recording");
                                    let _ = app_c.emit("hotkey-start-recording", ());
                                }
                            }
                        }
                    }
                }
            }

            EventType::KeyRelease(key) => {
                if let Some(code) = key_to_code(&key) {
                    held_keys_c.lock().unwrap().retain(|k| k != code);
                    if config.keys.contains(&code.to_string()) {
                        // Reset so the next physical key press can trigger the combo again.
                        combo_triggered_c.store(false, Ordering::SeqCst);
                        // Hold mode: releasing any combo key stops recording.
                        // Toggle mode: key releases have no effect on recording state.
                        if config.mode == RecordingMode::Hold
                            && recording_active_c.load(Ordering::SeqCst)
                        {
                            recording_active_c.store(false, Ordering::SeqCst);
                            println!("[HOTKEY] Hold — stopping recording");
                            let _ = app_c.emit("hotkey-stop-recording", ());
                        }
                    }
                }
            }

            _ => {}
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
        fn AXIsProcessTrustedWithOptions(
            options: core_foundation::base::CFTypeRef,
        ) -> bool;
    }

    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = if prompt { CFBoolean::true_value() } else { CFBoolean::false_value() };
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
