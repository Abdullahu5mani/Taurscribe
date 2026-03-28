use crate::parakeet;
use crate::state::AudioState;
use crate::tray;
use crate::types::ASREngine;
use crate::whisper;
use std::sync::atomic::Ordering;
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
    app: tauri::AppHandle,
    model_id: String,
    use_gpu: Option<bool>,
) -> Result<String, String> {
    let force_cpu = !use_gpu.unwrap_or(true);

    // 1. Safety check: don't switch models while recording.
    {
        let handle = state.recording_handle.lock().unwrap();
        if handle.is_some() {
            return Err("Cannot switch models while recording".to_string());
        }
    }

    // 2. Atomically claim the loading slot — bail if another load is already in flight.
    if state.engine_loading.compare_exchange(
        false, true, Ordering::Acquire, Ordering::Relaxed,
    ).is_err() {
        return Err("A model is already loading — please wait".to_string());
    }

    println!(
        "[INFO] Switching to Whisper model: {}{}",
        model_id,
        if force_cpu { " [CPU-only]" } else { "" }
    );

    let parakeet_arc   = state.parakeet.clone();
    let granite_arc    = state.granite_speech.clone();
    let whisper_arc    = state.whisper.clone();
    let active_engine_arc = state.active_engine.clone();
    let mid = model_id.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        // 3. Check what is currently loaded.
        let whisper_current  = whisper_arc.lock().unwrap().get_current_model().cloned();
        let parakeet_loaded  = parakeet_arc.lock().unwrap().get_status().loaded;
        let granite_loaded   = granite_arc.lock().unwrap().get_status().loaded;
        let active           = *active_engine_arc.lock().unwrap();

        let whisper_on_cpu = {
            let w = whisper_arc.lock().unwrap();
            matches!(*w.get_backend(), whisper::GpuBackend::Cpu)
        };

        // 4. Skip only if same model, same engine, and CPU/GPU preference already matches (toggle must reload).
        if whisper_current.as_deref() == Some(mid.as_str())
            && active == ASREngine::Whisper
            && !parakeet_loaded
            && !granite_loaded
            && whisper_on_cpu == force_cpu
        {
            println!("[INFO] Whisper model '{}' is already loaded — skipping reload", mid);
            return Ok("Already loaded".to_string());
        }

        // 5. Unload any competing engines before loading.
        if parakeet_loaded {
            println!("[INFO] Unloading Parakeet before switching to Whisper");
            parakeet_arc.lock().unwrap().unload();
        }
        if granite_loaded {
            println!("[INFO] Unloading Granite Speech before switching to Whisper");
            granite_arc.lock().unwrap().unload();
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
    app: tauri::AppHandle,
    model_id: Option<String>,
    use_gpu: Option<bool>,
) -> Result<String, String> {
    let force_cpu = !use_gpu.unwrap_or(true);

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
        let parakeet_status  = parakeet_arc.lock().unwrap().get_status();
        let whisper_loaded   = whisper_arc.lock().unwrap().get_current_model().is_some();
        let granite_loaded   = granite_arc.lock().unwrap().get_status().loaded;
        let active           = *active_engine_arc.lock().unwrap();

        // 3. Skip if the same Parakeet model is already active on the same CPU/GPU preference.
        let target_id = model_id.as_deref();
        let parakeet_on_cpu = parakeet_status.backend == "CPU";
        let already_loaded = parakeet_status.loaded
            && active == ASREngine::Parakeet
            && !whisper_loaded
            && !granite_loaded
            && (target_id.is_none() || parakeet_status.model_id.as_deref() == target_id)
            && parakeet_on_cpu == force_cpu;
        if already_loaded {
            println!("[INFO] Parakeet model is already loaded — skipping reload");
            return Ok::<String, String>("Already loaded".to_string());
        }

        // 4. Unload any competing engines before loading.
        if whisper_loaded {
            println!("[INFO] Unloading Whisper before switching to Parakeet");
            whisper_arc.lock().unwrap().unload();
        }
        if granite_loaded {
            println!("[INFO] Unloading Granite Speech before switching to Parakeet");
            granite_arc.lock().unwrap().unload();
        }

        // Free any existing Parakeet sessions before acquiring the lock for a fresh load
        // (initialize() also unloads if needed; this covers edge cases and makes logs explicit).
        if parakeet_status.loaded {
            println!("[INFO] Unloading existing Parakeet model before re-initializing");
            parakeet_arc.lock().unwrap().unload();
        }

        // 5. Load Parakeet.
        let mut parakeet = parakeet_arc.lock().unwrap();
        let result = parakeet.initialize(model_id.as_deref(), force_cpu)?;
        *active_engine_arc.lock().unwrap() = ASREngine::Parakeet;
        Ok::<String, String>(result)
    })
    .await
    .map_err(|e| format!("init_parakeet task failed: {}", e));
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

/// Ask for Parakeet status (Model, Type, Backend)
#[tauri::command]
pub fn get_parakeet_status(state: State<AudioState>) -> Result<parakeet::ParakeetStatus, String> {
    let parakeet = state.parakeet.lock().unwrap();
    Ok(parakeet.get_status())
}
