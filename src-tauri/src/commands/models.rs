use tauri::State;
use crate::parakeet;
use crate::state::AudioState;
use crate::types::ASREngine;
use crate::whisper;

/// List all available AI models found in the models folder
#[tauri::command]
pub fn list_models() -> Result<Vec<whisper::ModelInfo>, String> {
    whisper::WhisperManager::list_available_models()
}

/// Ask which model is currently loaded
#[tauri::command]
pub fn get_current_model(state: State<AudioState>) -> Result<Option<String>, String> {
    let whisper = state.whisper.lock().unwrap();
    Ok(whisper.get_current_model().cloned())
}

/// Command to swap the AI model (e.g. from Tiny to Large)
#[tauri::command]
pub fn switch_model(state: State<AudioState>, model_id: String) -> Result<String, String> {
    // 1. Safety Check: Don't switch models while recording!
    let handle = state.recording_handle.lock().unwrap();
    if handle.is_some() {
        return Err("Cannot switch models while recording".to_string());
    }
    drop(handle);

    println!("[INFO] Switching to model: {}", model_id);

    // 2. Initialize the new model
    let mut whisper = state.whisper.lock().unwrap();
    whisper.initialize(Some(&model_id))
}

/// List Parakeet models
#[tauri::command]
pub fn list_parakeet_models() -> Result<Vec<parakeet::ParakeetModelInfo>, String> {
    parakeet::ParakeetManager::list_available_models()
}

/// Initialize Parakeet
#[tauri::command]
pub fn init_parakeet(state: State<AudioState>, model_id: Option<String>) -> Result<String, String> {
    let mut parakeet = state.parakeet.lock().unwrap();
    let result = parakeet.initialize(model_id.as_deref())?;

    // Auto-switch to parakeet if initialized
    *state.active_engine.lock().unwrap() = ASREngine::Parakeet;

    Ok(result)
}

/// Ask for Parakeet status (Model, Type, Backend)
#[tauri::command]
pub fn get_parakeet_status(state: State<AudioState>) -> Result<parakeet::ParakeetStatus, String> {
    let parakeet = state.parakeet.lock().unwrap();
    Ok(parakeet.get_status())
}
