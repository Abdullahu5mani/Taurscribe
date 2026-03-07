use crate::parakeet;
use crate::state::AudioState;
use crate::types::ASREngine;
use crate::whisper;
use tauri::State;

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
///
/// macOS fix: Made async with spawn_blocking because loading/unloading heavy
/// ML models blocks for seconds. Tauri 2 runs sync commands on the macOS
/// AppKit main thread, which would freeze the entire window.
#[tauri::command]
pub async fn switch_model(
    state: State<'_, AudioState>,
    model_id: String,
    use_gpu: Option<bool>,
) -> Result<String, String> {
    let force_cpu = !use_gpu.unwrap_or(true);

    // 1. Safety Check: Don't switch models while recording!
    {
        let handle = state.recording_handle.lock().unwrap();
        if handle.is_some() {
            return Err("Cannot switch models while recording".to_string());
        }
    }

    println!(
        "[INFO] Switching to model: {}{}",
        model_id,
        if force_cpu { " [CPU-only]" } else { "" }
    );

    let parakeet_arc = state.parakeet.clone();
    let whisper_arc = state.whisper.clone();
    let active_engine_arc = state.active_engine.clone();
    let mid = model_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        // 2. Unload Parakeet if loaded (Exclusive Mode)
        parakeet_arc.lock().unwrap().unload();

        // 3. Initialize the new model
        let mut whisper = whisper_arc.lock().unwrap();
        let res = whisper.initialize(Some(&mid), force_cpu);

        // Update active engine
        *active_engine_arc.lock().unwrap() = ASREngine::Whisper;

        res
    })
    .await
    .map_err(|e| format!("switch_model task failed: {}", e))?
}

/// List Parakeet models
#[tauri::command]
pub fn list_parakeet_models() -> Result<Vec<parakeet::ParakeetModelInfo>, String> {
    parakeet::ParakeetManager::list_available_models()
}

/// Initialize Parakeet
///
/// macOS fix: Made async with spawn_blocking because loading Parakeet's ONNX
/// models blocks for seconds. Without this, the macOS AppKit main thread
/// freezes and the window becomes unresponsive.
#[tauri::command]
pub async fn init_parakeet(
    state: State<'_, AudioState>,
    model_id: Option<String>,
    use_gpu: Option<bool>,
) -> Result<String, String> {
    let force_cpu = !use_gpu.unwrap_or(true);

    let whisper_arc = state.whisper.clone();
    let parakeet_arc = state.parakeet.clone();
    let active_engine_arc = state.active_engine.clone();

    tauri::async_runtime::spawn_blocking(move || {
        // 1. Unload Whisper if loaded (Exclusive Mode)
        whisper_arc.lock().unwrap().unload();

        // 2. Load Parakeet
        let mut parakeet = parakeet_arc.lock().unwrap();
        let result = parakeet.initialize(model_id.as_deref(), force_cpu)?;

        // Auto-switch to parakeet if initialized
        *active_engine_arc.lock().unwrap() = ASREngine::Parakeet;

        Ok(result)
    })
    .await
    .map_err(|e| format!("init_parakeet task failed: {}", e))?
}

/// Ask for Parakeet status (Model, Type, Backend)
#[tauri::command]
pub fn get_parakeet_status(state: State<AudioState>) -> Result<parakeet::ParakeetStatus, String> {
    let parakeet = state.parakeet.lock().unwrap();
    Ok(parakeet.get_status())
}
