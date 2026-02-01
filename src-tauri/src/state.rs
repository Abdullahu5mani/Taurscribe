use crate::audio::RecordingHandle;
use crate::parakeet::ParakeetManager;
use crate::types::{ASREngine, AppState};
use crate::vad::VADManager;
use crate::whisper::WhisperManager;
use std::sync::{Arc, Mutex};

/// The Global "Brain" of the application.
/// This struct holds all the data that needs to live as long as the app runs.
pub struct AudioState {
    // Holds the active recording stream. If None, we are not recording.
    // Use Mutex because we need to change it (start/stop) safely.
    pub recording_handle: Mutex<Option<RecordingHandle>>,

    // The Whisper AI engine. Wrapped in Arc<Mutex<>> so it can be shared and used by multiple threads.
    pub whisper: Arc<Mutex<WhisperManager>>,

    // The Parakeet AI engine (alternative to Whisper). Also shared across threads.
    pub parakeet: Arc<Mutex<ParakeetManager>>,

    // The Voice Activity Detector. Also shared.
    pub vad: Arc<Mutex<VADManager>>,

    // Remembers where we saved the last WAV file so we can process it when recording stops.
    pub last_recording_path: Mutex<Option<String>>,

    // Keeps track of whether we are Ready, Recording, or Processing.
    pub current_app_state: Mutex<AppState>,

    // Which ASR engine is currently active?
    pub active_engine: Mutex<ASREngine>,

    // Accumulates the full transcript during a recording session (for Parakeet streaming reuse)
    pub session_transcript: Arc<Mutex<String>>,

    // The Gemma LLM engine (optional, loaded on demand)
    pub llm: Arc<Mutex<Option<crate::llm::LLMEngine>>>,
}

impl AudioState {
    pub fn new(whisper: WhisperManager, parakeet: ParakeetManager, vad: VADManager) -> Self {
        Self {
            recording_handle: Mutex::new(None),
            whisper: Arc::new(Mutex::new(whisper)),
            parakeet: Arc::new(Mutex::new(parakeet)),
            vad: Arc::new(Mutex::new(vad)),
            last_recording_path: Mutex::new(None),
            current_app_state: Mutex::new(AppState::Ready),
            active_engine: Mutex::new(ASREngine::Whisper),
            session_transcript: Arc::new(Mutex::new(String::new())),
            llm: Arc::new(Mutex::new(None)),
        }
    }
}
