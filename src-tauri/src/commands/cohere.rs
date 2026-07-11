// Tauri commands for the Granite Speech ONNX engine.

use crate::cohere::{cohere_logical_model_id_for_dir, resolve_cohere_model_dir};
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
    /// Granite can fall back to CPU, but prefers GPU when available.
    pub requires_gpu: bool,
}

/// List available (downloaded) Granite engine bundles.
#[tauri::command]
pub fn list_granite_models() -> Vec<CohereModelInfo> {
    let models_dir = match crate::utils::get_models_dir() {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    let granite_cuda_dir = models_dir.join("granite-speech-4.1-2b-nar-cuda");
    if crate::cohere::cohere_onnx_bundle_ready(&granite_cuda_dir) {
        out.push(CohereModelInfo {
            id: "granite-speech-4.1-2b-nar-cuda".to_string(),
            display_name: "CUDA".to_string(),
            size_mb: 2280.0,
            requires_gpu: true,
        });
    }
    let granite_portable_dir = models_dir.join("granite-speech-4.1-2b-nar-portable");
    if crate::cohere::cohere_onnx_bundle_ready(&granite_portable_dir) {
        out.push(CohereModelInfo {
            id: "granite-speech-4.1-2b-nar-portable".to_string(),
            display_name: "AMD / Intel / CPU".to_string(),
            size_mb: 2280.0,
            requires_gpu: false,
        });
    }
    out
}

/// Legacy IPC alias retained for older frontend builds.
#[tauri::command]
pub fn list_cohere_models() -> Vec<CohereModelInfo> {
    list_granite_models()
}

/// Initialize the Granite Speech engine (load ONNX models + tokenizer).
#[tauri::command]
pub async fn init_granite(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    model_id: Option<String>,
    force_cpu: Option<bool>,
) -> Result<CommandResult<String>, String> {
    use crate::types::ASREngine;
    crate::memory::log_process_memory("init_granite command start");

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
            .and_then(|d| resolve_cohere_model_dir(&d, model_id.as_deref()).ok())
            .map(|dir| cohere_logical_model_id_for_dir(&dir));
        if cohere_status.loaded
            && active == ASREngine::Granite
            && !whisper_loaded
            && !parakeet_loaded
            && cohere_on_cpu == want_cpu
            && target_logical.is_some()
            && cohere_status.model_id.as_deref() == target_logical.as_deref()
        {
            println!("[GRANITE] Model is already loaded — skipping reload");
            return Ok::<String, String>("Already loaded".to_string());
        }

        // 4. Unload any competing engines before loading.
        if whisper_loaded {
            println!("[GRANITE] Unloading Whisper before switching to Granite");
            whisper_arc.lock().unwrap().unload();
        }
        if parakeet_loaded {
            println!("[GRANITE] Unloading Parakeet before switching to Granite");
            parakeet_arc.lock().unwrap().unload();
        }

        // 5. Load Granite Speech.
        let mut gs = cohere_arc
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let msg = gs.initialize(model_id.as_deref(), force_cpu.unwrap_or(false))?;
        *active_engine_arc.lock().unwrap() = ASREngine::Granite;
        Ok(msg)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e));
    state.engine_loading.store(false, Ordering::Relaxed);

    match result {
        Ok(Ok(msg)) => {
            state.model_loaded.store(true, Ordering::Relaxed);
            tray::update_tray_model_item(&app, true);
            crate::memory::log_process_memory("init_granite command success");
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
            crate::memory::log_process_memory("init_granite command error");
            Ok(CommandResult::err(code, e))
        }
        Err(join_err) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            crate::memory::log_process_memory("init_granite command join_error");
            Ok(CommandResult::err("model_load_failed", join_err))
        }
    }
}

/// Legacy IPC alias retained for older frontend builds.
#[tauri::command]
pub async fn init_cohere(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    model_id: Option<String>,
    force_cpu: Option<bool>,
) -> Result<CommandResult<String>, String> {
    init_granite(state, app, model_id, force_cpu).await
}

/// Get the current status of the Granite Speech engine.
#[tauri::command]
pub fn get_granite_status(
    state: State<'_, AudioState>,
) -> Result<crate::cohere::CohereStatus, String> {
    let gs = state
        .cohere
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    Ok(gs.get_status())
}

/// Legacy IPC alias retained for older frontend builds.
#[tauri::command]
pub fn get_cohere_status(
    state: State<'_, AudioState>,
) -> Result<crate::cohere::CohereStatus, String> {
    get_granite_status(state)
}
