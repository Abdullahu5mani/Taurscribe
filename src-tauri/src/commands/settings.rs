use tauri::{AppHandle, State};
use crate::state::AudioState;
use crate::tray;
use crate::types::{AppState, ASREngine, HotkeyBinding};

/// Ask the backend what hardware is running the AI (CPU vs GPU)
/// Returns the backend of whichever engine is currently active
#[tauri::command]
pub fn get_backend_info(state: State<AudioState>) -> Result<String, String> {
    let active = *state.active_engine.lock().unwrap();
    match active {
        ASREngine::Parakeet => {
            let parakeet = state.parakeet.lock().unwrap();
            let status = parakeet.get_status();
            Ok(status.backend)
        }
        ASREngine::Whisper => {
            let whisper = state.whisper.lock().unwrap();
            Ok(format!("{}", whisper.get_backend()))
        }
    }
}

/// Change the active ASR engine
#[tauri::command]
pub fn set_active_engine(state: State<AudioState>, engine: String) -> Result<String, String> {
    let new_engine = match engine.to_lowercase().as_str() {
        "whisper" => ASREngine::Whisper,
        "parakeet" => ASREngine::Parakeet,
        _ => return Err(format!("Unknown engine: {}", engine)),
    };

    *state.active_engine.lock().unwrap() = new_engine;
    println!("[ENGINE] Active engine switched to: {:?}", new_engine);
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
    state.hotkey_config.lock().unwrap().clone()
}

/// Update the hotkey binding — takes effect immediately (no restart needed)
#[tauri::command]
pub fn set_hotkey(state: State<AudioState>, binding: HotkeyBinding) -> Result<(), String> {
    *state.hotkey_config.lock().unwrap() = binding;
    Ok(())
}

/// Return the currently preferred input device name (None = system default).
#[tauri::command]
pub fn get_input_device(state: State<AudioState>) -> Option<String> {
    state.selected_input_device.lock().unwrap().clone()
}

/// Set the preferred input device. Pass None to revert to the system default.
#[tauri::command]
pub fn set_input_device(state: State<AudioState>, name: Option<String>) {
    *state.selected_input_device.lock().unwrap() = name;
}

/// Update the system tray icon manually from the frontend
#[tauri::command]
pub fn set_tray_state(
    app: AppHandle,
    state: State<AudioState>,
    new_state: String,
) -> Result<(), String> {
    // Convert string command ("ready") to Enum (AppState::Ready)
    let app_state = match new_state.as_str() {
        "ready" => AppState::Ready,
        "recording" => AppState::Recording,
        "processing" => AppState::Processing,
        _ => return Err(format!("Unknown state: {}", new_state)),
    };

    // Update our internal memory of the state
    *state.current_app_state.lock().unwrap() = app_state;

    // Actually change the visual icon
    tray::update_tray_icon(&app, app_state)?;

    Ok(())
}
