use tauri::{AppHandle, Emitter};
use rdev::{listen, Event, EventType, Key};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

/// BACKGROUND: Listen for Ctrl+Win global hotkeys
pub fn start_hotkey_listener(app_handle: AppHandle) {
    // Shared "flags" to remember if keys are pressed
    let ctrl_held = Arc::new(AtomicBool::new(false));
    let meta_held = Arc::new(AtomicBool::new(false)); // Meta = Windows Key
    let recording_active = Arc::new(AtomicBool::new(false));

    // Clones for the closure
    let ctrl_held_clone = ctrl_held.clone();
    let meta_held_clone = meta_held.clone();
    let recording_active_clone = recording_active.clone();
    let app_handle_clone = app_handle.clone();

    // The callback runs for EVERY key press on the system
    let callback = move |event: Event| {
        match event.event_type {
            EventType::KeyPress(key) => {
                match key {
                    Key::ControlLeft | Key::ControlRight => {
                        ctrl_held_clone.store(true, Ordering::SeqCst);
                    }
                    Key::MetaLeft | Key::MetaRight => {
                        meta_held_clone.store(true, Ordering::SeqCst);
                    }
                    _ => {}
                }

                // CHECK: Are BOTH keys pressed? And are we NOT recording?
                if ctrl_held_clone.load(Ordering::SeqCst)
                    && meta_held_clone.load(Ordering::SeqCst)
                    && !recording_active_clone.load(Ordering::SeqCst)
                {
                    recording_active_clone.store(true, Ordering::SeqCst);
                    println!("[HOTKEY] Ctrl+Win pressed - Starting recording");

                    // Send signal to frontend to simulate button click
                    let _ = app_handle_clone.emit("hotkey-start-recording", ());
                }
            }
            EventType::KeyRelease(key) => {
                match key {
                    Key::ControlLeft | Key::ControlRight => {
                        ctrl_held_clone.store(false, Ordering::SeqCst);
                    }
                    Key::MetaLeft | Key::MetaRight => {
                        meta_held_clone.store(false, Ordering::SeqCst);
                    }
                    _ => {}
                }

                // If keys released, STOP recording
                if recording_active_clone.load(Ordering::SeqCst)
                    && (!ctrl_held_clone.load(Ordering::SeqCst)
                        || !meta_held_clone.load(Ordering::SeqCst))
                {
                    recording_active_clone.store(false, Ordering::SeqCst);
                    println!("[HOTKEY] Ctrl+Win released - Stopping recording");

                    let _ = app_handle_clone.emit("hotkey-stop-recording", ());
                }
            }
            _ => {}
        }
    };

    // Start the listener (this blocks the thread, so it must be in a spawn)
    if let Err(error) = listen(callback) {
        eprintln!("[ERROR] Hotkey listener error: {:?}", error);
    }
}
