use crate::state::AudioState;
use crate::tray;
use crate::types::{ASREngine, AppState, EngineSelectionState, HotkeyBinding};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, State};

// ── Non-blocking engine status ──────────────────────────────────────────────
//
// The engine mutexes are held for the whole of an inference (seconds to
// minutes). These commands are synchronous, so Tauri runs them on the macOS
// main thread, and the UI polls them every few seconds: waiting on those locks
// froze the app during transcription. They read the status snapshots the
// managers publish on every load and unload instead (`AudioState::*_snapshot`).

/// Ask the backend what hardware is running the AI (CPU vs GPU)
/// Returns the backend of whichever engine is currently active
#[tauri::command]
pub fn get_backend_info(state: State<AudioState>) -> Result<String, String> {
    Ok(backend_info(&state))
}

fn backend_info(state: &AudioState) -> String {
    let active = *state.active_engine.lock().unwrap();
    match active {
        ASREngine::Whisper => state.whisper_snapshot().backend,
        engine => state.gguf_snapshot(engine).map(|s| s.backend).unwrap_or_default(),
    }
}

#[tauri::command]
pub fn get_engine_selection_state(
    state: State<AudioState>,
) -> Result<EngineSelectionState, String> {
    Ok(engine_selection_state(&state))
}

fn engine_selection_state(state: &AudioState) -> EngineSelectionState {
    let active = *state.active_engine.lock().unwrap();
    let active_engine = match active {
        ASREngine::Whisper => "whisper",
        ASREngine::Granite => "granite",
        ASREngine::Qwen3 => "qwen3",
    }
    .to_string();

    let (selected_model_id, loaded_engine, loaded_model_id, backend) = match active {
        ASREngine::Whisper => {
            let whisper = state.whisper_snapshot();
            let loaded = whisper.model.clone();
            (
                whisper.model,
                loaded.as_ref().map(|_| "whisper".to_string()),
                loaded,
                whisper.backend,
            )
        }
        ASREngine::Granite | ASREngine::Qwen3 => {
            let status = state.gguf_snapshot(active).expect("GGUF engine");
            let family = if active == ASREngine::Granite { "granite" } else { "qwen3" };
            let loaded = status.loaded.then(|| status.model_id.clone()).flatten();
            (
                status.model_id.clone(),
                loaded.as_ref().map(|_| family.to_string()),
                loaded,
                status.backend,
            )
        }
    };

    EngineSelectionState {
        active_engine,
        selected_model_id,
        loaded_engine,
        loaded_model_id,
        backend,
        engine_loading: state.engine_loading.load(Ordering::Relaxed),
    }
}



/// Return the current hotkey binding
#[tauri::command]
pub fn get_hotkey(state: State<AudioState>) -> HotkeyBinding {
    state.hotkey_config.read().unwrap().clone()
}

/// Update the hotkey binding — takes effect immediately (no restart needed).
/// Rejects bindings that don't have exactly 2 keys.
#[tauri::command]
pub fn set_hotkey(state: State<AudioState>, binding: HotkeyBinding) -> Result<(), String> {
    if binding.keys.len() != 2 {
        return Err(format!(
            "Hotkey must be exactly 2 keys, got {}",
            binding.keys.len()
        ));
    }
    *state.hotkey_config.write().unwrap() = binding;
    Ok(())
}

/// Suppress or unsuppress the global hotkey listener.
/// Called by the frontend when the Settings modal opens (suppress) and closes (unsuppress)
/// so accidental key combos don't trigger recording while the user is rebinding.
#[tauri::command]
pub fn set_hotkey_suppressed(state: State<AudioState>, suppressed: bool) {
    state.hotkey_suppressed.store(suppressed, Ordering::SeqCst);
}

/// Set the preferred input device. Pass None to revert to the system default.
#[tauri::command]
pub fn set_input_device(state: State<AudioState>, name: Option<String>) {
    *state.selected_input_device.lock().unwrap() = name;
}


/// Set the close-button behavior. "tray" hides to tray; "quit" exits the process.
#[tauri::command]
pub fn set_close_behavior(state: State<AudioState>, behavior: String) -> Result<(), String> {
    match behavior.as_str() {
        "tray" | "quit" => {
            *state.close_behavior.lock().unwrap() = behavior;
            Ok(())
        }
        _ => Err(format!("Unknown close behavior: {}", behavior)),
    }
}

/// Shows or hides the menu-bar / system-tray icon. The choice is saved by the
/// frontend as `show_tray_icon` and re-applied at startup.
#[tauri::command]
pub fn set_tray_icon_visible(app: tauri::AppHandle, visible: bool) -> Result<(), String> {
    match app.tray_by_id("main-tray") {
        Some(tray) => tray.set_visible(visible).map_err(|e| e.to_string()),
        None => Err("Tray icon not found".to_string()),
    }
}

/// Update the system tray icon manually from the frontend with optional meeting metadata
#[tauri::command]
pub fn set_tray_state(
    app: AppHandle,
    state: State<AudioState>,
    new_state: String,
    meeting_platform: Option<String>,
    meeting_process: Option<String>,
    meeting_pid: Option<u32>,
    detail: Option<String>,
) -> Result<(), String> {
    let app_state = match new_state.as_str() {
        "ready" => AppState::Ready,
        "recording" => AppState::Recording,
        "processing" => AppState::Processing,
        "processing_speech" => AppState::ProcessingSpeech,
        "processing_meeting" => AppState::ProcessingMeeting,
        "processing_file" => AppState::ProcessingFile,
        "loading_model" => AppState::LoadingModel,
        "paused" => AppState::Paused,
        "downloading" => AppState::Downloading,
        "grammar" => AppState::Grammar,
        "done" => AppState::Done,
        "nothing_heard" => AppState::NothingHeard,
        "paste_failed" => AppState::PasteFailed,
        "error" => AppState::Error,
        "mic_blocked" => AppState::MicBlocked,
        "cancelled" => AppState::Cancelled,
        _ => return Err(format!("Unknown state: {}", new_state)),
    };

    tray::set_status(
        &app,
        app_state,
        meeting_platform.as_deref(),
        meeting_process.as_deref(),
        meeting_pid,
        detail,
    )?;

    let loaded = state.model_loaded.load(std::sync::atomic::Ordering::Relaxed);
    let meeting_info = meeting_platform
        .as_deref()
        .and_then(|plat| meeting_pid.map(|pid| (plat, pid)));
    let is_recording = matches!(app_state, AppState::Recording | AppState::Paused);
    tray::update_tray_menu(&app, loaded, meeting_info, is_recording);

    Ok(())
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use crate::gguf_asr::GgufAsrManager;
    use std::sync::mpsc;
    use std::time::Duration;

    fn test_state() -> AudioState {
        AudioState::new(
            crate::whisper::WhisperManager::new(),
            GgufAsrManager::granite(),
            crate::vad::VADManager::new().unwrap(),
            GgufAsrManager::qwen3(),
        )
    }

    /// Runs `f` on another thread and fails if it does not return promptly
    /// (the old code blocked until the engine lock was released).
    fn returns_promptly<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(2)).expect("status command blocked on a busy engine")
    }

    #[test]
    fn status_commands_do_not_wait_for_a_transcribing_engine() {
        for engine in [ASREngine::Whisper, ASREngine::Granite, ASREngine::Qwen3] {
            let state = test_state();
            *state.active_engine.lock().unwrap() = engine;
            // Read once while idle, like the UI's first poll.
            let idle = engine_selection_state(&state);
            // Hold the engine the way a long transcription does.
            let (locked_tx, locked_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel::<()>();
            let holder = {
                let state = state.clone();
                std::thread::spawn(move || {
                    let _whisper;
                    let _gguf;
                    match engine {
                        ASREngine::Whisper => _whisper = Some(state.whisper.lock().unwrap()),
                        ASREngine::Granite => _gguf = Some(state.granite.lock().unwrap()),
                        ASREngine::Qwen3 => _gguf = Some(state.qwen3.lock().unwrap()),
                    }
                    locked_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
            };
            locked_rx.recv().unwrap();
            let busy = {
                let state = state.clone();
                returns_promptly(move || engine_selection_state(&state))
            };
            assert_eq!(busy.active_engine, idle.active_engine, "{engine:?}");
            assert_eq!(busy.loaded_model_id, idle.loaded_model_id, "{engine:?}");
            let state2 = state.clone();
            returns_promptly(move || backend_info(&state2));
            // What get_granite_status / get_qwen3_status / get_current_model return.
            let state3 = state.clone();
            returns_promptly(move || (state3.gguf_snapshot(engine), state3.whisper_snapshot()));
            let state4 = state.clone();
            returns_promptly(move || state4.any_asr_loaded());
            release_tx.send(()).unwrap();
            holder.join().unwrap();
        }
    }

    #[test]
    fn snapshots_follow_the_managers_while_locked() {
        let state = test_state();
        assert_eq!(state.whisper_snapshot().model, None);
        assert!(!state.gguf_snapshot(ASREngine::Granite).unwrap().loaded);
        assert!(state.gguf_snapshot(ASREngine::Whisper).is_none());
        assert!(!state.any_asr_loaded());
        // Unload publishes too, and the snapshot stays readable while the
        // engine lock is held.
        let mut granite = state.granite.lock().unwrap();
        granite.unload();
        let status = state.gguf_snapshot(ASREngine::Granite).unwrap();
        assert_eq!((status.loaded, status.backend.as_str()), (false, "none"));
        drop(granite);
        let whisper = state.whisper.lock().unwrap();
        assert_eq!(state.whisper_snapshot().backend, "CPU");
        drop(whisper);
    }
}
