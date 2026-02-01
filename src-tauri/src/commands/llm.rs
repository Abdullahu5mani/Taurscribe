use crate::llm::LLMEngine;
use crate::state::AudioState;
use tauri::State;

#[tauri::command]
pub async fn init_llm(state: State<'_, AudioState>) -> Result<String, String> {
    println!("[COMMAND] init_llm requested");

    // Check if already loaded
    {
        let llm_guard = state.llm.lock().unwrap();
        if llm_guard.is_some() {
            return Ok("LLM already initialized".to_string());
        }
    }

    // Load in a blocking task since it's heavy
    let result = tauri::async_runtime::spawn_blocking(move || LLMEngine::new())
        .await
        .map_err(|e| format!("JoinError: {}", e))?;

    match result {
        Ok(engine) => {
            let mut llm_guard = state.llm.lock().unwrap();
            *llm_guard = Some(engine);
            println!("[SUCCESS] Gemma LLM initialized!");
            Ok("Gemma LLM initialized successfully".to_string())
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to load LLM: {}", e);
            Err(format!("Failed to load LLM: {}", e))
        }
    }
}

#[tauri::command]
pub async fn run_llm_inference(
    state: State<'_, AudioState>,
    prompt: String,
) -> Result<String, String> {
    // We need to lock the LLM, but generating text is slow, so we shouldn't hold the lock
    // for the entire generation if we can help it, BUT LLMEngine is not Clone.
    // So we must hold the lock or wrap it in another mutex.
    // Since inference is sequential single-user, holding the lock is fine for now.

    // However, LLMEngine::run function is synchronous. We should run it in blocking task.
    // But we can't pass the MutexGuard to another thread easily if it's not 'static scope.
    // We will use a slightly different pattern for async wrapping.

    let llm_handle = state.llm.clone();
    let prompt = prompt.clone();

    let output = tauri::async_runtime::spawn_blocking(move || {
        let mut llm_guard = llm_handle.lock().unwrap();
        if let Some(engine) = llm_guard.as_mut() {
            engine.run(&prompt).map_err(|e| e.to_string())
        } else {
            Err("LLM not initialized. Call init_llm first.".to_string())
        }
    })
    .await
    .map_err(|e| format!("Join Erorr: {}", e))??;

    Ok(output)
}

#[tauri::command]
pub fn check_llm_status(state: State<'_, AudioState>) -> bool {
    let llm_guard = state.llm.lock().unwrap();
    llm_guard.is_some()
}

#[tauri::command]
pub async fn correct_text(state: State<'_, AudioState>, text: String) -> Result<String, String> {
    println!(
        "[LLM] correct_text request received. Input length: {}",
        text.len()
    );
    let llm_handle = state.llm.clone();
    let prompt = text.clone();

    let output = tauri::async_runtime::spawn_blocking(move || {
        let mut llm_guard = llm_handle.lock().unwrap();
        if let Some(engine) = llm_guard.as_mut() {
            println!("[LLM] Running inference on text: '{}'", prompt.trim());
            engine.run(&prompt).map_err(|e| e.to_string())
        } else {
            eprintln!("[LLM] Error: Engine not initialized");
            Err("LLM not initialized. Please load Gemma first.".to_string())
        }
    })
    .await
    .map_err(|e| format!("Join Error: {}", e))??;

    println!("[LLM] Inference finished. Output length: {}", output.len());
    Ok(output)
}
