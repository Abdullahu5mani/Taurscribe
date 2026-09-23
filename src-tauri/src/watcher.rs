use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bumped on every (re)start so a watcher on an old models folder stops itself.
static GENERATION: AtomicU64 = AtomicU64::new(0);
use std::sync::mpsc;
use tauri::{AppHandle, Emitter, Manager};

/// Starts watching the models directory for changes
/// Emits "models-changed" after writes, renames, or removals settle.
/// Calling it again (after the models folder moves) replaces the old watcher.
pub fn start_models_watcher(app_handle: AppHandle) -> Result<(), String> {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    // Get the models directory path
    let models_dir = crate::utils::get_models_dir()?;

    println!("[WATCHER] Starting file watcher for: {:?}", models_dir);

    // Create a channel to receive events
    let (tx, rx) = mpsc::channel();

    // Create a watcher
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                // macOS can report a rename on an external volume as a generic
                // modify event. Refresh for any filesystem change, but ignore
                // reads so status checks do not trigger another refresh.
                if !matches!(event.kind, notify::EventKind::Access(_)) {
                    let _ = tx.send(event);
                }
            }
        },
        Config::default(),
    )
    .map_err(|e| format!("Failed to create watcher: {}", e))?;

    // Start watching the models directory (recursive to catch per-model subfolders)
    // Watch the real folder: a symlinked models dir (e.g. moved to another
    // disk) otherwise reports no events at all.
    let watch_dir = std::fs::canonicalize(&models_dir).unwrap_or_else(|_| models_dir.clone());
    watcher
        .watch(Path::new(&watch_dir), RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch directory: {}", e))?;

    // Spawn a thread to handle events and emit to frontend
    std::thread::spawn(move || {
        // Keep the watcher alive
        let _watcher = watcher;

        // Emit after a burst settles. A leading-edge throttle drops the second
        // half of a quick remove/restore and leaves Settings showing stale state.
        let mut pending = false;
        let mut last_change = std::time::Instant::now();
        let debounce_duration = std::time::Duration::from_millis(500);

        loop {
            if GENERATION.load(Ordering::SeqCst) != generation {
                break;
            }
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(event) => {
                    println!("[WATCHER] Model files changed: {:?}", event.paths);
                    pending = true;
                    last_change = std::time::Instant::now();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    println!("[WATCHER] Channel disconnected, stopping watcher");
                    break;
                }
            }

            if pending && last_change.elapsed() >= debounce_duration {
                if let Err(e) = app_handle.emit("models-changed", ()) {
                    eprintln!("[WATCHER] Failed to emit event: {}", e);
                }
                if let Some(st) = app_handle.try_state::<crate::state::AudioState>() {
                    let loaded = st.model_loaded.load(Ordering::Relaxed);
                    crate::tray::update_tray_model_item(&app_handle, loaded);
                }
                pending = false;
            }
        }
    });

    Ok(())
}
