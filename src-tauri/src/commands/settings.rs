use crate::state::AudioState;
use crate::tray;
use crate::types::{ASREngine, AppState, EngineSelectionState, HotkeyBinding};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, State};

/// Ask the backend what hardware is running the AI (CPU vs GPU)
/// Returns the backend of whichever engine is currently active
#[tauri::command]
pub fn get_backend_info(state: State<AudioState>) -> Result<String, String> {
    let active = *state.active_engine.lock().unwrap();
    match active {
        ASREngine::Granite => {
            let granite = state.granite.lock().unwrap();
            let status = granite.get_status();
            Ok(status.backend)
        }
        ASREngine::Whisper => {
            let whisper = state.whisper.lock().unwrap();
            Ok(format!("{}", whisper.get_backend()))
        }
        ASREngine::Qwen3 => Ok(state.qwen3.lock().unwrap().get_status().backend),
    }
}

#[tauri::command]
pub fn get_engine_selection_state(
    state: State<AudioState>,
) -> Result<EngineSelectionState, String> {
    let active = *state.active_engine.lock().unwrap();
    let active_engine = match active {
        ASREngine::Whisper => "whisper",
        ASREngine::Granite => "granite",
        ASREngine::Qwen3 => "qwen3",
    }
    .to_string();

    let whisper_model = state.whisper.lock().unwrap().get_current_model().cloned();
    let granite_status = state.granite.lock().unwrap().get_status();
    let qwen3_status = state.qwen3.lock().unwrap().get_status();

    let (selected_model_id, loaded_engine, loaded_model_id, backend) = match active {
        ASREngine::Whisper => {
            let loaded = whisper_model.clone();
            let backend = {
                let whisper = state.whisper.lock().unwrap();
                format!("{}", whisper.get_backend())
            };
            (
                whisper_model.clone(),
                loaded.as_ref().map(|_| "whisper".to_string()),
                loaded,
                backend,
            )
        }
        ASREngine::Granite => {
            let loaded = if granite_status.loaded {
                granite_status.model_id.clone()
            } else {
                None
            };
            (
                granite_status.model_id.clone(),
                loaded.as_ref().map(|_| "granite".to_string()),
                loaded,
                granite_status.backend,
            )
        }
        ASREngine::Qwen3 => {
            let loaded = qwen3_status
                .loaded
                .then(|| qwen3_status.model_id.clone())
                .flatten();
            (
                qwen3_status.model_id.clone(),
                loaded.as_ref().map(|_| "qwen3".to_string()),
                loaded,
                qwen3_status.backend,
            )
        }
    };

    Ok(EngineSelectionState {
        active_engine,
        selected_model_id,
        loaded_engine,
        loaded_model_id,
        backend,
        engine_loading: state.engine_loading.load(Ordering::Relaxed),
    })
}

/// Change the active ASR engine
#[tauri::command]
pub fn set_active_engine(
    app: AppHandle,
    state: State<AudioState>,
    engine: String,
) -> Result<String, String> {
    let new_engine = match engine.to_lowercase().as_str() {
        "whisper" => ASREngine::Whisper,
        "granite" | "granitespeech" | "granite_speech" | "granite-speech" | "parakeet" => ASREngine::Granite,
        "qwen3" | "qwen3-asr" | "qwen3_asr" => ASREngine::Qwen3,
        _ => return Err(format!("Unknown engine: {}", engine)),
    };

    *state.active_engine.lock().unwrap() = new_engine;
    println!("[ENGINE] Active engine switched to: {:?}", new_engine);
    let loaded = state.model_loaded.load(Ordering::Relaxed);
    tray::update_tray_model_item(&app, loaded);
    Ok(format!("Engine switched to {:?}", new_engine))
}

/// Ask which engine is active
#[tauri::command]
pub fn get_active_engine(state: State<AudioState>) -> Result<ASREngine, String> {
    Ok(*state.active_engine.lock().unwrap())
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

/// Return the current close-button behavior ("tray" or "quit")
#[tauri::command]
pub fn get_close_behavior(state: State<AudioState>) -> String {
    state.close_behavior.lock().unwrap().clone()
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
