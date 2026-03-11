// Tauri commands for the Granite Speech ONNX engine.

use crate::state::AudioState;
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
    model_path: Option<String>,
    force_cpu: Option<bool>,
) -> Result<String, String> {
    let granite = state.granite_speech.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut gs = granite.lock().map_err(|e| format!("Lock error: {}", e))?;
        gs.initialize(
            model_path.as_deref(),
            force_cpu.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
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
