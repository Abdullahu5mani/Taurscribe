use crate::audio::RecordingHandle;
use crate::denoise::Denoiser;
use crate::granite_speech::GraniteSpeechManager;
use crate::parakeet::ParakeetManager;
use crate::types::{ASREngine, AppState, HotkeyBinding};
use crate::vad::VADManager;
use crate::whisper::WhisperManager;
use std::sync::{
    atomic::AtomicBool,
    Arc, Mutex, RwLock,
};

/// The Global "Brain" of the application.
/// This struct holds all the data that needs to live as long as the app runs.
///
/// Every field is an `Arc<…>`, so `Clone` is derived and is free (just bumps
/// ref-counts). This lets `start_recording` (and similar async commands) pass
/// a single `state.clone()` into `spawn_blocking` instead of cloning every
/// field individually before the closure.
#[derive(Clone)]
pub struct AudioState {
    // macOS fix: Arc-wrapped so it can be cloned into spawn_blocking closures
    // in start_recording / stop_recording async commands.
    pub recording_handle: Arc<Mutex<Option<RecordingHandle>>>,

    // The Whisper AI engine. Wrapped in Arc<Mutex<>> so it can be shared and used by multiple threads.
    pub whisper: Arc<Mutex<WhisperManager>>,

    // The Parakeet AI engine (alternative to Whisper). Also shared across threads.
    pub parakeet: Arc<Mutex<ParakeetManager>>,

    // The Voice Activity Detector. Also shared.
    pub vad: Arc<Mutex<VADManager>>,

    // macOS fix: Arc-wrapped so async commands can clone it into spawn_blocking.
    pub last_recording_path: Arc<Mutex<Option<String>>>,

    // macOS fix: Arc-wrapped for async command access.
    pub current_app_state: Arc<Mutex<AppState>>,

    // macOS fix: Arc-wrapped for async command access.
    pub active_engine: Arc<Mutex<ASREngine>>,

    // Accumulates the full transcript during a recording session (for Parakeet streaming reuse)
    pub session_transcript: Arc<Mutex<String>>,

    // The Gemma LLM engine (optional, loaded on demand)
    pub llm: Arc<Mutex<Option<crate::llm::LLMEngine>>>,

    // The user-configured global hotkey binding (keyboard combo or mouse button).
    // Shared with the hotkey listener thread so changes take effect immediately.
    // RwLock: the listener reads on every key event; writes are rare (user reconfigures hotkey).
    pub hotkey_config: Arc<RwLock<HotkeyBinding>>,

    // macOS fix: Arc-wrapped for async command access.
    pub selected_input_device: Arc<Mutex<Option<String>>>,

    // RNNoise denoiser (created fresh per recording session, None when idle)
    pub denoiser: Arc<Mutex<Option<Denoiser>>>,

    // What happens when the user clicks the window close button.
    // "tray"  → hide to system tray (default)
    // "quit"  → exit the process
    pub close_behavior: Arc<Mutex<String>>,

    // The Granite Speech ONNX engine (alternative to Whisper/Parakeet)
    pub granite_speech: Arc<Mutex<GraniteSpeechManager>>,

    // When true the global hotkey listener ignores all key events.
    // Used to prevent accidental recording while the user is re-binding
    // the hotkey inside the Settings modal.
    pub hotkey_suppressed: Arc<AtomicBool>,

    // Tracks whether the current recording stream is temporarily paused.
    pub recording_paused: Arc<AtomicBool>,

    // True when an ASR model is fully loaded and ready.
    // Used by the tray menu to show "Load Model" vs "Unload Model".
    pub model_loaded: Arc<AtomicBool>,

    // True while an ASR engine is actively loading (blocks unload attempts).
    pub engine_loading: Arc<AtomicBool>,
}

impl AudioState {
    pub fn new(
        whisper: WhisperManager,
        parakeet: ParakeetManager,
        vad: VADManager,
        granite_speech: GraniteSpeechManager,
    ) -> Self {
        Self {
            recording_handle: Arc::new(Mutex::new(None)),
            whisper: Arc::new(Mutex::new(whisper)),
            parakeet: Arc::new(Mutex::new(parakeet)),
            vad: Arc::new(Mutex::new(vad)),
            last_recording_path: Arc::new(Mutex::new(None)),
            current_app_state: Arc::new(Mutex::new(AppState::Ready)),
            active_engine: Arc::new(Mutex::new(ASREngine::Whisper)),
            session_transcript: Arc::new(Mutex::new(String::new())),
            llm: Arc::new(Mutex::new(None)),
            hotkey_config: Arc::new(RwLock::new(HotkeyBinding::default())),
            selected_input_device: Arc::new(Mutex::new(None)),
            denoiser: Arc::new(Mutex::new(None)),
            close_behavior: Arc::new(Mutex::new("tray".to_string())),
            granite_speech: Arc::new(Mutex::new(granite_speech)),
            hotkey_suppressed: Arc::new(AtomicBool::new(false)),
            recording_paused: Arc::new(AtomicBool::new(false)),
            model_loaded: Arc::new(AtomicBool::new(false)),
            engine_loading: Arc::new(AtomicBool::new(false)),
        }
    }
}
