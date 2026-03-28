// Tauri commands for the Granite Speech ONNX engine.

use crate::granite_speech::{granite_logical_model_id_for_dir, resolve_granite_model_dir};
use crate::state::AudioState;
use crate::tray;
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(serde::Serialize)]
pub struct GraniteSpeechModelInfo {
    pub id: String,
    pub display_name: String,
    pub size_mb: f32,
    /// True for the FP16 package — needs GPU on Windows/Linux; use INT4 bundle for CPU.
    pub requires_gpu: bool,
}

/// List available (downloaded) Granite Speech models (INT4 and/or FP16 bundles).
#[tauri::command]
pub fn list_granite_models() -> Vec<GraniteSpeechModelInfo> {
    let models_dir = match crate::utils::get_models_dir() {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    let d_int4 = models_dir.join("granite-speech-1b");
    if crate::granite_speech::granite_int4_bundle_ready(&d_int4) {
        out.push(GraniteSpeechModelInfo {
            id: "granite-speech-1b".to_string(),
            display_name: "Granite 4.0 1B Speech (INT4)".to_string(),
            size_mb: 1843.0,
            requires_gpu: false,
        });
    }
    let d_fp16 = models_dir.join("granite-speech-1b-fp16");
    if crate::granite_speech::granite_fp16_bundle_ready(&d_fp16) {
        out.push(GraniteSpeechModelInfo {
            id: "granite-speech-1b-fp16".to_string(),
            display_name: "Granite 4.0 1B Speech (FP16 · CUDA)".to_string(),
            size_mb: 4700.0,
            requires_gpu: true,
        });
    }
    out
}

/// Initialize the Granite Speech engine (load ONNX models + tokenizer).
#[tauri::command]
pub async fn init_granite_speech(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    model_id: Option<String>,
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

        // 3. Skip only if the same on-disk bundle + CPU/GPU mode is already active (model id matters for INT4 vs FP16).
        let want_cpu = force_cpu.unwrap_or(false);
        let granite_on_cpu = granite_status.backend == "CPU";
        let target_logical = crate::utils::get_models_dir()
            .ok()
            .and_then(|d| resolve_granite_model_dir(&d, model_id.as_deref()).ok())
            .map(|dir| granite_logical_model_id_for_dir(&dir));
        if granite_status.loaded
            && active == ASREngine::GraniteSpeech
            && !whisper_loaded
            && !parakeet_loaded
            && granite_on_cpu == want_cpu
            && target_logical.is_some()
            && granite_status.model_id.as_deref() == target_logical.as_deref()
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
        let msg = gs.initialize(model_id.as_deref(), force_cpu.unwrap_or(false))?;
        *active_engine_arc.lock().unwrap() = ASREngine::GraniteSpeech;
        Ok(msg)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e));
    state.engine_loading.store(false, Ordering::Relaxed);

    match result {
        Ok(Ok(msg)) => {
            state.model_loaded.store(true, Ordering::Relaxed);
            tray::update_tray_model_item(&app, true);
            Ok(msg)
        }
        Ok(Err(e)) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            Err(e)
        }
        Err(join_err) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            Err(join_err)
        }
    }
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
