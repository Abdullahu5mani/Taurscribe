use crate::gguf_asr::{self, GgufModelInfo, GgufStatus, GRANITE_MODELS, QWEN3_MODELS};
use crate::state::AudioState;
use crate::tray;
use crate::types::{ASREngine, CommandResult};
use crate::whisper;
use std::sync::atomic::Ordering;
use tauri::State;

/// List all available AI models found in the models folder
#[tauri::command]
pub async fn list_models() -> Result<Vec<whisper::ModelInfo>, String> {
    tauri::async_runtime::spawn_blocking(whisper::WhisperManager::list_available_models)
        .await
        .map_err(|e| format!("list_models task failed: {e}"))?
}

/// Ask which model is currently loaded
#[tauri::command]
pub async fn get_current_model(state: State<'_, AudioState>) -> Result<Option<String>, String> {
    let whisper = state.whisper.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(whisper.lock().map_err(|e| e.to_string())?.get_current_model().cloned())
    })
    .await
    .map_err(|e| format!("get_current_model task failed: {e}"))?
}

/// Command to swap the AI model (e.g. from Tiny to Large)
///
/// macOS fix: Made async with spawn_blocking because loading/unloading heavy
/// ML models blocks for seconds. Tauri 2 runs sync commands on the macOS
/// AppKit main thread, which would freeze the entire window.
#[tauri::command]
pub async fn switch_model(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    model_id: String,
    use_gpu: Option<bool>,
) -> Result<CommandResult<String>, String> {
    let force_cpu = !use_gpu.unwrap_or(true);

    // 1. Safety check: don't switch models while recording.
    {
        let handle = state.recording_handle.lock().unwrap();
        if handle.is_some() {
            return Ok(CommandResult::err(
                "already_recording",
                "Cannot switch models while recording",
            ));
        }
    }

    let _model_operation = state.begin_exclusive_model_operation()?;

    // 2. Atomically claim the loading slot — bail if another load is already in flight.
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

    println!(
        "[INFO] Switching to Whisper model: {}{}",
        model_id,
        if force_cpu { " [CPU-only]" } else { "" }
    );
    crate::memory::log_process_memory("switch_model command start");

    let granite_arc = state.granite.clone();
    let qwen3_arc = state.qwen3.clone();
    let whisper_arc = state.whisper.clone();
    let active_engine_arc = state.active_engine.clone();
    let mid = model_id.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        // 3. Check what is currently loaded.
        let whisper_current = whisper_arc.lock().unwrap().get_current_model().cloned();
        let granite_loaded = granite_arc.lock().unwrap().get_status().loaded;
        let qwen3_loaded = qwen3_arc.lock().unwrap().get_status().loaded;
        let active = *active_engine_arc.lock().unwrap();

        let whisper_on_cpu = {
            let w = whisper_arc.lock().unwrap();
            matches!(*w.get_backend(), whisper::GpuBackend::Cpu)
        };

        // 4. Skip only if same model, same engine, and CPU/GPU preference already matches (toggle must reload).
        if whisper_current.as_deref() == Some(mid.as_str())
            && active == ASREngine::Whisper
            && !granite_loaded
            && !qwen3_loaded
            && whisper_on_cpu == force_cpu
        {
            println!(
                "[INFO] Whisper model '{}' is already loaded — skipping reload",
                mid
            );
            return Ok("Already loaded".to_string());
        }

        // 5. Unload any competing engines before loading.
        if granite_loaded {
            println!("[INFO] Unloading Granite before switching to Whisper");
            granite_arc.lock().unwrap().unload();
        }
        if qwen3_loaded {
            println!("[INFO] Unloading Qwen3 before switching to Whisper");
            qwen3_arc.lock().unwrap().unload();
        }

        // 6. Load the requested Whisper model.
        let mut whisper = whisper_arc.lock().unwrap();
        let res = whisper.initialize(Some(&mid), force_cpu);
        if res.is_ok() {
            *active_engine_arc.lock().unwrap() = ASREngine::Whisper;
        }
        res
    })
    .await
    .map_err(|e| format!("switch_model task failed: {}", e));
    state.engine_loading.store(false, Ordering::Relaxed);

    match result {
        Ok(Ok(msg)) => {
            state.model_loaded.store(true, Ordering::Relaxed);
            tray::update_tray_model_item(&app, true);
            crate::memory::log_process_memory("switch_model command success");
            Ok(CommandResult::ok(msg))
        }
        Ok(Err(e)) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            let code = if e.to_lowercase().contains("no models")
                || e.to_lowercase().contains("not found")
                || e.to_lowercase().contains("missing")
            {
                "model_missing"
            } else {
                "model_load_failed"
            };
            crate::memory::log_process_memory("switch_model command error");
            Ok(CommandResult::err(code, e))
        }
        Err(join_err) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            crate::memory::log_process_memory("switch_model command join_error");
            Ok(CommandResult::err("model_load_failed", join_err))
        }
    }
}

#[tauri::command]
pub fn list_granite_models() -> Result<Vec<GgufModelInfo>, String> {
    gguf_asr::list_available(GRANITE_MODELS)
}

#[tauri::command]
pub fn get_granite_status(state: State<AudioState>) -> Result<GgufStatus, String> {
    Ok(state.granite.lock().map_err(|e| e.to_string())?.get_status())
}

#[tauri::command]
pub async fn init_granite(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    model_id: Option<String>,
    use_gpu: Option<bool>,
) -> Result<CommandResult<String>, String> {
    init_gguf(state, app, ASREngine::Granite, model_id, use_gpu).await
}

#[tauri::command]
pub fn list_qwen3_models() -> Result<Vec<GgufModelInfo>, String> {
    gguf_asr::list_available(QWEN3_MODELS)
}

#[tauri::command]
pub fn get_qwen3_status(state: State<AudioState>) -> Result<GgufStatus, String> {
    Ok(state.qwen3.lock().map_err(|e| e.to_string())?.get_status())
}

#[tauri::command]
pub async fn init_qwen3(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    model_id: Option<String>,
    use_gpu: Option<bool>,
) -> Result<CommandResult<String>, String> {
    init_gguf(state, app, ASREngine::Qwen3, model_id, use_gpu).await
}

/// Loads a Granite or Qwen3 model (unloading every other engine first) and
/// makes that engine active. Runs on a blocking thread: loading takes seconds
/// and would otherwise freeze the macOS main thread.
async fn init_gguf(
    state: State<'_, AudioState>,
    app: tauri::AppHandle,
    engine: ASREngine,
    model_id: Option<String>,
    use_gpu: Option<bool>,
) -> Result<CommandResult<String>, String> {
    if state.recording_handle.lock().unwrap().is_some() {
        return Ok(CommandResult::err(
            "already_recording",
            "Cannot switch models while recording",
        ));
    }
    let _model_operation = state.begin_exclusive_model_operation()?;
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
    let target = state.gguf_manager(engine).expect("GGUF engine");
    let others: Vec<_> = [ASREngine::Granite, ASREngine::Qwen3]
        .into_iter()
        .filter(|e| *e != engine)
        .filter_map(|e| state.gguf_manager(e))
        .collect();
    let whisper = state.whisper.clone();
    let active = state.active_engine.clone();
    let force_cpu = !use_gpu.unwrap_or(true);
    let result = tauri::async_runtime::spawn_blocking(move || {
        {
            let status = target.lock().map_err(|e| e.to_string())?.get_status();
            let same_model = model_id.is_none() || status.model_id.as_deref() == model_id.as_deref();
            let on_cpu = status.backend.eq_ignore_ascii_case("cpu");
            if status.loaded && same_model && on_cpu == force_cpu && *active.lock().unwrap() == engine {
                return Ok("Already loaded".to_string());
            }
        }
        whisper.lock().map_err(|e| e.to_string())?.unload();
        for other in &others {
            other.lock().map_err(|e| e.to_string())?.unload();
        }
        let message = target
            .lock()
            .map_err(|e| e.to_string())?
            .initialize(model_id.as_deref(), force_cpu)?;
        *active.lock().map_err(|e| e.to_string())? = engine;
        Ok::<_, String>(message)
    })
    .await
    .map_err(|e| e.to_string());
    state.engine_loading.store(false, Ordering::Relaxed);
    match result {
        Ok(Ok(message)) => {
            state.model_loaded.store(true, Ordering::Relaxed);
            state.touch_activity();
            tray::update_tray_model_item(&app, true);
            Ok(CommandResult::ok(message))
        }
        Ok(Err(error)) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            let code = if error.contains("not downloaded") || error.contains("No ") {
                "model_missing"
            } else {
                "model_load_failed"
            };
            Ok(CommandResult::err(code, error))
        }
        Err(error) => {
            tray::reconcile_model_loaded_tray(&app, &state);
            Ok(CommandResult::err("model_load_failed", error))
        }
    }
}
