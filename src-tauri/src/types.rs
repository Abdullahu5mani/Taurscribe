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
}

/// Hotkey binding — up to 2 keyboard keys held simultaneously.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HotkeyBinding {
    pub keys: Vec<String>,
}

impl Default for HotkeyBinding {
    fn default() -> Self {
        HotkeyBinding { keys: vec!["ControlLeft".to_string(), "MetaLeft".to_string()] }
    }
}

/// Structured payload for live transcription chunks
#[derive(Debug, Clone, serde::Serialize)]
pub struct TranscriptionChunk {
    pub text: String,
    pub processing_time_ms: u32,
    pub method: String,
}
