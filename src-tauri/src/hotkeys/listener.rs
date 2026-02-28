use crate::types::HotkeyBinding;
use rdev::{listen, Event, EventType, Key};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

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
) {
    use tauri::Emitter;

    let recording_active = Arc::new(AtomicBool::new(false));
    let held_keys: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let recording_active_c = recording_active.clone();
    let held_keys_c = held_keys.clone();
    let app_c = app_handle.clone();
    let config_c = hotkey_config.clone();

    let callback = move |event: Event| {
        let config = config_c.lock().unwrap().clone();

        match event.event_type {
            EventType::KeyPress(key) => {
                if let Some(code) = key_to_code(&key) {
                    let mut held = held_keys_c.lock().unwrap();
                    if config.keys.contains(&code.to_string()) && !held.contains(&code.to_string()) {
                        held.push(code.to_string());
                    }
                    let all_held = config.keys.iter().all(|k| held.contains(k));
                    if all_held && !config.keys.is_empty() && !recording_active_c.load(Ordering::SeqCst) {
                        drop(held);
                        recording_active_c.store(true, Ordering::SeqCst);
                        println!("[HOTKEY] Hotkey pressed — starting recording");
                        let _ = app_c.emit("hotkey-start-recording", ());
                    }
                }
            }

            EventType::KeyRelease(key) => {
                if let Some(code) = key_to_code(&key) {
                    held_keys_c.lock().unwrap().retain(|k| k != code);
                    if recording_active_c.load(Ordering::SeqCst)
                        && config.keys.contains(&code.to_string())
                    {
                        recording_active_c.store(false, Ordering::SeqCst);
                        println!("[HOTKEY] Hotkey released — stopping recording");
                        let _ = app_c.emit("hotkey-stop-recording", ());
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
