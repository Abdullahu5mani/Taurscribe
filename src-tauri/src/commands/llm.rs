use crate::llm::LLMEngine;
use crate::state::AudioState;
use tauri::State;

/// Returns true if the FlowScribe model file exists and can be loaded.
#[tauri::command]
pub fn check_grammar_llm_available() -> bool {
    crate::llm::v3_model_path().is_some()
}

#[tauri::command]
pub async fn init_llm(state: State<'_, AudioState>, use_gpu: bool) -> Result<String, String> {
    println!("[COMMAND] init_llm requested. use_gpu: {}", use_gpu);

    // One load at a time: a second request waits for the first, then finds it
    // loaded, instead of loading another ~1.5 GB copy and dropping one.
    static LOADING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _loading = LOADING.lock().await;
    // Atomically participate in the same lifecycle gate as model relocation.
    // Checking models_moving alone has a check-then-load race.
    let _model_operation = state.begin_model_operation()?;
    // FlowScribe isn't under the ASR model gate; don't open its weights in a
    // models folder that is being moved.
    if crate::commands::models_moving() {
        return Err("The models folder is being moved; FlowScribe loads once the move finishes".into());
    }

    // Check if already loaded
    {
        let llm_guard = state.llm.lock().unwrap();
        if llm_guard.is_some() {
            return Ok("LLM already initialized".to_string());
        }
    }

    // Load in a blocking task since it's heavy
    let result = tauri::async_runtime::spawn_blocking(move || LLMEngine::new(use_gpu))
        .await
        .map_err(|e| format!("JoinError: {}", e))?;

    match result {
        Ok(engine) => {
            let mut llm_guard = state.llm.lock().unwrap();
            *llm_guard = Some(engine);
            println!("[SUCCESS] Grammar LLM initialized!");
            Ok("Grammar LLM initialized successfully".to_string())
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to load LLM: {}", e);
            Err(format!("Failed to load LLM: {}", e))
        }
    }
}



/// Clean up a transcript with FlowScribe.
#[tauri::command]
pub async fn correct_text(
    app: tauri::AppHandle,
    state: State<'_, AudioState>,
    text: String,
    style: Option<String>,
) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(String::new());
    }
    println!(
        "[LLM] correct_text request received. Input length: {}",
        text.len()
    );
    let llm_handle = state.llm.clone();
    let _model_operation = state.begin_model_operation()?;
    // v3 context: which recognizer produced the text, the clean-up level, the
    // app being typed into (still frontmost at this point) and the dictionary.
    let engine_tag = match *state.active_engine.lock().unwrap() {
        crate::types::ASREngine::Whisper => "whisper",
        crate::types::ASREngine::Granite => "granite",
        crate::types::ASREngine::Qwen3 => "qwen3",
    };
    // The Accessibility lookup waits for the main thread (see
    // context::on_main_thread), so do it on the blocking pool, not a Tokio worker.
    let app_category = tauri::async_runtime::spawn_blocking(move || crate::context::active_app_category(&app))
        .await
        .unwrap_or("generic");
    let request = crate::llm::FlowRequest {
        engine: engine_tag.to_string(),
        level: crate::llm::level_for_style(style.as_deref()).to_string(),
        app: app_category.to_string(),
        vocab: crate::context::load_custom_vocabulary_from_settings().0,
        prev: None,
    };

    let output = tauri::async_runtime::spawn_blocking(move || {
        let mut llm_guard = llm_handle.lock().unwrap();
        if let Some(engine) = llm_guard.as_mut() {
            println!(
                "[LLM] Cleaning transcript (engine={}, level={}, app={}, vocab={})",
                request.engine, request.level, request.app, request.vocab.len()
            );
            match engine.clean_transcript(&text, &request) {
                Ok(formatted) => {
                    println!("[LLM] Correction finished. Output length: {}", formatted.len());
                    Ok(formatted)
                }
                Err(e) => {
                    eprintln!("[LLM] Correction failed: {}", e);
                    Ok(text)
                }
            }
        } else {
            Err("LLM not initialized. Download FlowScribe V3 from the Models tab.".to_string())
        }
    })
    .await
    .map_err(|e| format!("Join Error: {}", e))??;

    Ok(output)
}

#[tauri::command]
pub async fn unload_llm(state: State<'_, AudioState>) -> Result<String, String> {
    let operation = state.begin_model_operation()?;
    let handle = state.llm.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let engine = handle.lock().map_err(|e| e.to_string())?.take();
        let loaded = engine.is_some();
        drop(engine);
        crate::memory::trim_process_memory();
        Ok(if loaded { "LLM unloaded successfully" } else { "LLM was not loaded" }.to_string())
    }).await.map_err(|e| format!("LLM unload task failed: {e}"))?
}
