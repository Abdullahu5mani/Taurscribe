// Tauri commands for the Granite Speech ONNX engine.

use crate::state::AudioState;
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(serde::Serialize)]
pub struct GraniteSpeechModelInfo {
    pub id: String,
    pub display_name: String,
    pub size_mb: f32,
}

/// List available (downloaded) Granite Speech models.
/// Returns a single-item array if the model directory exists, or empty if not downloaded.
#[tauri::command]
pub fn list_granite_models() -> Vec<GraniteSpeechModelInfo> {
    let models_dir = match crate::utils::get_models_dir() {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    if models_dir.join("granite-speech-1b").exists() {
        vec![GraniteSpeechModelInfo {
            id: "granite-speech-1b".to_string(),
            display_name: "Granite 4.0 1B Speech".to_string(),
            size_mb: 2048.0,
        }]
    } else {
        vec![]
    }
}

/// Initialize the Granite Speech engine (load ONNX models + tokenizer).
#[tauri::command]
pub async fn init_granite_speech(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    model_path: Option<String>,
    force_cpu: Option<bool>,
) -> Result<String, String> {
    use crate::types::ASREngine;

    // 1. Atomically claim the loading slot — bail if another load is already in flight.
    if state.engine_loading.compare_exchange(
        false, true, Ordering::Acquire, Ordering::Relaxed,
    ).is_err() {
        return Err("A model is already loading — please wait".to_string());
    }

    let whisper_arc       = state.whisper.clone();
    let parakeet_arc      = state.parakeet.clone();
    let granite_arc       = state.granite_speech.clone();
    let active_engine_arc = state.active_engine.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        // 2. Check what is currently loaded.
        let granite_status  = granite_arc.lock().map_err(|e| format!("Lock error: {}", e))?.get_status();
        let whisper_loaded  = whisper_arc.lock().unwrap().get_current_model().is_some();
        let parakeet_loaded = parakeet_arc.lock().unwrap().get_status().loaded;
        let active          = *active_engine_arc.lock().unwrap();

        // 3. Skip if Granite is already the active engine with a model loaded.
        if granite_status.loaded
            && active == ASREngine::GraniteSpeech
            && !whisper_loaded
            && !parakeet_loaded
        {
            println!("[GRANITE] Model is already loaded — skipping reload");
            return Ok::<String, String>("Already loaded".to_string());
        }

        // 4. Unload any competing engines before loading.
        if whisper_loaded {
            println!("[GRANITE] Unloading Whisper before switching to Granite Speech");
            whisper_arc.lock().unwrap().unload();
        }
        if parakeet_loaded {
            println!("[GRANITE] Unloading Parakeet before switching to Granite Speech");
            parakeet_arc.lock().unwrap().unload();
        }

        // 5. Load Granite Speech.
        let mut gs = granite_arc.lock().map_err(|e| format!("Lock error: {}", e))?;
        let msg = gs.initialize(model_path.as_deref(), force_cpu.unwrap_or(false))?;
        *active_engine_arc.lock().unwrap() = ASREngine::GraniteSpeech;
        Ok(msg)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e));
    state.engine_loading.store(false, Ordering::Relaxed);

    let msg = result??;
    state.model_loaded.store(true, Ordering::Relaxed);
    crate::tray::update_tray_model_item(&app, true);
    Ok(msg)
}

/// Get the current status of the Granite Speech engine.
#[tauri::command]
pub fn get_granite_speech_status(
    state: State<'_, AudioState>,
) -> Result<crate::granite_speech::GraniteSpeechStatus, String> {
    let gs = state
        .granite_speech
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    Ok(gs.get_status())
}
