// Tauri commands for the Cohere Transcribe ONNX engine.

use crate::cohere::{granite_logical_model_id_for_dir, resolve_granite_model_dir};
use crate::state::AudioState;
use crate::tray;
use crate::types::CommandResult;
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(serde::Serialize)]
pub struct CohereModelInfo {
    pub id: String,
    pub display_name: String,
    pub size_mb: f32,
    /// Cohere is CUDA-only in the current implementation.
    pub requires_gpu: bool,
}

/// List available (downloaded) Cohere engine bundles.
#[tauri::command]
pub fn list_cohere_models() -> Vec<CohereModelInfo> {
    let models_dir = match crate::utils::get_models_dir() {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    let d_int4 = models_dir.join("granite-speech-1b");
    if crate::cohere::granite_int4_bundle_ready(&d_int4) {
        out.push(CohereModelInfo {
            id: "granite-speech-1b-cpu".to_string(),
            display_name: "Cohere Transcribe 03-2026 (q4f16)".to_string(),
            size_mb: 1600.0,
            requires_gpu: true,
        });
    }
    out
}

/// Initialize the Cohere Transcribe engine (load ONNX models + tokenizer).
#[tauri::command]
pub async fn init_cohere(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    model_id: Option<String>,
    force_cpu: Option<bool>,
) -> Result<CommandResult<String>, String> {
    use crate::types::ASREngine;
    crate::memory::log_process_memory("init_cohere command start");
    if force_cpu.unwrap_or(false) {
        return Ok(CommandResult::err(
            "model_load_failed",
            "Cohere is CUDA-only in this build. Disable CPU mode and retry.",
        ));
    }

    // 1. Atomically claim the loading slot — bail if another load is already in flight.
    if state
        .engine_loading
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Ok(CommandResult::err(
            "engine_loading",
            "A model is already loading — please wait",
        ));
    }

    let whisper_arc = state.whisper.clone();
    let parakeet_arc = state.parakeet.clone();
    let cohere_arc = state.cohere.clone();
    let active_engine_arc = state.active_engine.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        // 2. Check what is currently loaded.
        let cohere_status = cohere_arc
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .get_status();
        let whisper_loaded = whisper_arc.lock().unwrap().get_current_model().is_some();
        let parakeet_loaded = parakeet_arc.lock().unwrap().get_status().loaded;
        let active = *active_engine_arc.lock().unwrap();

        // 3. Skip only if the same on-disk bundle + CPU/GPU mode is already active.
        let want_cpu = force_cpu.unwrap_or(false);
        let cohere_on_cpu = cohere_status.backend == "CPU";
        let target_logical = crate::utils::get_models_dir()
            .ok()
            .and_then(|d| resolve_granite_model_dir(&d, model_id.as_deref()).ok())
            .map(|dir| granite_logical_model_id_for_dir(&dir));
        if cohere_status.loaded
            && active == ASREngine::Cohere
            && !whisper_loaded
            && !parakeet_loaded
            && cohere_on_cpu == want_cpu
            && target_logical.is_some()
            && cohere_status.model_id.as_deref() == target_logical.as_deref()
        {
            println!("[COHERE] Model is already loaded — skipping reload");
            return Ok::<String, String>("Already loaded".to_string());
        }

        // 4. Unload any competing engines before loading.
        if whisper_loaded {
            println!("[COHERE] Unloading Whisper before switching to Cohere");
            whisper_arc.lock().unwrap().unload();
        }
        if parakeet_loaded {
            println!("[COHERE] Unloading Parakeet before switching to Cohere");
            parakeet_arc.lock().unwrap().unload();
        }

        // 5. Load Cohere Transcribe.
        let mut gs = cohere_arc
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let msg = gs.initialize(model_id.as_deref(), force_cpu.unwrap_or(false))?;
        *active_engine_arc.lock().unwrap() = ASREngine::Cohere;
        Ok(msg)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e));
    state.engine_loading.store(false, Ordering::Relaxed);

    match result {
        Ok(Ok(msg)) => {
            state.model_loaded.store(true, Ordering::Relaxed);
            tray::update_tray_model_item(&app, true);
            crate::memory::log_process_memory("init_cohere command success");
            Ok(CommandResult::ok(msg))
        }
        Ok(Err(e)) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            let code = if e.to_lowercase().contains("not found")
                || e.to_lowercase().contains("missing")
                || e.to_lowercase().contains("bundle")
            {
                "model_missing"
            } else {
                "model_load_failed"
            };
            crate::memory::log_process_memory("init_cohere command error");
            Ok(CommandResult::err(code, e))
        }
        Err(join_err) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            crate::memory::log_process_memory("init_cohere command join_error");
            Ok(CommandResult::err("model_load_failed", join_err))
        }
    }
}

/// Get the current status of the Cohere Transcribe engine.
#[tauri::command]
pub fn get_cohere_status(
    state: State<'_, AudioState>,
) -> Result<crate::cohere::CohereStatus, String> {
    let gs = state
        .cohere
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    Ok(gs.get_status())
}
