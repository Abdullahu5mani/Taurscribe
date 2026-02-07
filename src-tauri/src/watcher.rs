use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc;
use tauri::{AppHandle, Emitter};

/// Starts watching the models directory for changes
/// Emits "models-changed" event to frontend when files are added/removed
pub fn start_models_watcher(app_handle: AppHandle) -> Result<(), String> {
    // Get the models directory path
    let models_dir = crate::utils::get_models_dir()?;

    println!("[WATCHER] Starting file watcher for: {:?}", models_dir);

    // Create a channel to receive events
    let (tx, rx) = mpsc::channel();

    // Create a watcher
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                // Only send for create/remove events
                match event.kind {
                    notify::EventKind::Create(_) | notify::EventKind::Remove(_) => {
                        let _ = tx.send(event);
                    }
                    _ => {}
                }
            }
        },
        Config::default(),
    )
    .map_err(|e| format!("Failed to create watcher: {}", e))?;

    // Start watching the models directory (recursive to catch Parakeet subdirs)
    watcher
        .watch(Path::new(&models_dir), RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch directory: {}", e))?;

    // Spawn a thread to handle events and emit to frontend
    std::thread::spawn(move || {
        // Keep the watcher alive
        let _watcher = watcher;

        // Debounce timer to avoid spamming events
        let mut last_emit = std::time::Instant::now();
        let debounce_duration = std::time::Duration::from_millis(500);

        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(event) => {
                    // Check if enough time has passed since last emit
                    if last_emit.elapsed() >= debounce_duration {
                        println!("[WATCHER] Model files changed: {:?}", event.paths);

                        // Emit event to frontend
                        if let Err(e) = app_handle.emit("models-changed", ()) {
                            eprintln!("[WATCHER] Failed to emit event: {}", e);
                        }

                        last_emit = std::time::Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // No events, continue watching
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    println!("[WATCHER] Channel disconnected, stopping watcher");
                    break;
                }
            }
        }
    });

    Ok(())
}
