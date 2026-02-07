use crate::spellcheck::SpellChecker;
use crate::state::AudioState;
use tauri::State;

#[tauri::command]
pub async fn init_spellcheck(state: State<'_, AudioState>) -> Result<String, String> {
    println!("[COMMAND] init_spellcheck requested");

    // Check if already loaded
    {
        let sc_guard = state.spellcheck.lock().unwrap();
        if sc_guard.is_some() {
            return Ok("SymSpell already initialized".to_string());
        }
    }

    // Load in a blocking task
    let result = tauri::async_runtime::spawn_blocking(move || SpellChecker::new())
        .await
        .map_err(|e| format!("JoinError: {}", e))?;

    match result {
        Ok(checker) => {
            let mut sc_guard = state.spellcheck.lock().unwrap();
            *sc_guard = Some(checker);
            println!("[SUCCESS] SymSpell initialized!");
            Ok("SymSpell spell checker initialized successfully".to_string())
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to load SymSpell: {}", e);
            Err(format!("Failed to load SymSpell: {}", e))
        }
    }
}

#[tauri::command]
pub fn check_spellcheck_status(state: State<'_, AudioState>) -> bool {
    let sc_guard = state.spellcheck.lock().unwrap();
    sc_guard.is_some()
}

#[tauri::command]
pub async fn correct_spelling(state: State<'_, AudioState>, text: String) -> Result<String, String> {
    println!(
        "[SPELL] correct_spelling request received. Input length: {}",
        text.len()
    );

    let sc_handle = state.spellcheck.clone();
    let input_text = text.clone();

    let output = tauri::async_runtime::spawn_blocking(move || {
        let sc_guard = sc_handle.lock().unwrap();
        if let Some(checker) = sc_guard.as_ref() {
            Ok(checker.correct(&input_text))
        } else {
            Err("SymSpell not initialized. Call init_spellcheck first.".to_string())
        }
    })
    .await
    .map_err(|e| format!("Join Error: {}", e))??;

    println!("[SPELL] Correction finished. Output length: {}", output.len());
    Ok(output)
}

#[tauri::command]
pub fn unload_spellcheck(state: State<'_, AudioState>) -> Result<String, String> {
    let mut sc_guard = state.spellcheck.lock().unwrap();
    if sc_guard.is_none() {
        return Ok("SymSpell was not loaded".to_string());
    }
    *sc_guard = None;
    println!("[INFO] SymSpell unloaded.");
    Ok("SymSpell unloaded successfully".to_string())
}
