/// Defines the possible states of our application
/// This helps us decide which icon to show in the tray
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    Ready,      // Green: Waiting for user input
    Recording,  // Red: Mic is active, recording audio
    Processing, // Yellow: Computing/Transcribing
}

/// The possible ASR engines we support
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum ASREngine {
    Whisper,
    Parakeet,
    Cohere,
}

/// Recording mode: hold keys down the whole time, or press once to start / again to stop.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RecordingMode {
    Hold,
    Toggle,
}

impl Default for RecordingMode {
    fn default() -> Self {
        RecordingMode::Hold
    }
}

/// Hotkey binding — up to 2 keyboard keys held simultaneously.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HotkeyBinding {
    pub keys: Vec<String>,
    #[serde(default)]
    pub mode: RecordingMode,
}

impl Default for HotkeyBinding {
    fn default() -> Self {
        // macOS: Ctrl+Option (Cmd is intercepted by the OS at the kernel level)
        // Windows / Linux: Ctrl+Win / Ctrl+Super
        #[cfg(target_os = "macos")]
        let keys = vec!["ControlLeft".to_string(), "AltLeft".to_string()];
        #[cfg(not(target_os = "macos"))]
        let keys = vec!["ControlLeft".to_string(), "MetaLeft".to_string()];
        HotkeyBinding {
            keys,
            mode: RecordingMode::default(),
        }
    }
}

/// Structured payload for live transcription chunks
#[derive(Debug, Clone, serde::Serialize)]
pub struct TranscriptionChunk {
    pub text: String,
    pub processing_time_ms: u32,
    pub method: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommandResult<T>
where
    T: serde::Serialize,
{
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<CommandError>,
}

impl<T> CommandResult<T>
where
    T: serde::Serialize,
{
    pub fn ok(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(CommandError::new(code, message)),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineSelectionState {
    pub active_engine: String,
    pub selected_model_id: Option<String>,
    pub loaded_engine: Option<String>,
    pub loaded_model_id: Option<String>,
    pub backend: String,
    pub engine_loading: bool,
}
