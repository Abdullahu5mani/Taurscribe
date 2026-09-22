use crate::audio::RecordingHandle;
use crate::denoise::Denoiser;
use crate::gguf_asr::{GgufAsrManager, GRANITE_MODELS, QWEN3_MODELS};
use crate::types::{ASREngine, HotkeyBinding};
use crate::vad::VADManager;
use crate::whisper::WhisperManager;
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc, Mutex, RwLock,
};

#[derive(Default)]
struct ModelActivity {
    active_operations: usize,
    exclusive: bool,
}

/// Keeps model weights resident while a recording or transcription uses them.
pub struct ModelOperationGuard {
    activity: Arc<Mutex<ModelActivity>>,
    exclusive: bool,
}

impl Drop for ModelOperationGuard {
    fn drop(&mut self) {
        if let Ok(mut activity) = self.activity.lock() {
            if self.exclusive {
                activity.exclusive = false;
            } else {
                activity.active_operations -= 1;
            }
        }
    }
}

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

    // Granite Speech 5 (GGUF through transcribe.cpp). Also shared across threads.
    pub granite: Arc<Mutex<GgufAsrManager>>,

    // The Voice Activity Detector. Also shared.
    pub vad: Arc<Mutex<VADManager>>,

    // macOS fix: Arc-wrapped so async commands can clone it into spawn_blocking.
    pub last_recording_path: Arc<Mutex<Option<String>>>,

    // macOS fix: Arc-wrapped for async command access.
    pub active_engine: Arc<Mutex<ASREngine>>,

    // Accumulates the live transcript during a recording session
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

    // Qwen3-ASR (GGUF through transcribe.cpp), loaded on demand.
    pub qwen3: Arc<Mutex<GgufAsrManager>>,

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
    model_activity: Arc<Mutex<ModelActivity>>,

    // Inactivity timeout in seconds before loaded ASR models are automatically unloaded.
    // 0 = never / disabled
    // 1 = immediate (unload after every transcription)
    // >1 = seconds of inactivity (e.g. 300 = 5m, 1800 = 30m)
    pub auto_unload_seconds: Arc<AtomicU64>,

    // UNIX epoch seconds of the last user transcription or model load activity.
    pub last_activity_timestamp: Arc<AtomicU64>,

    // Meeting Detection manager
    pub meeting_detector: Arc<crate::meeting_detector::MeetingDetectorManager>,

    // Active audio recording mode: "mic" or "dual_channel"
    pub audio_source_mode: Arc<Mutex<String>>,

    // Auto-record calls when a meeting is detected
    pub auto_record_meetings: Arc<AtomicBool>,

    // Tracks if the most recent recording was in dual-channel mode
    pub last_recording_is_dual_channel: Arc<AtomicBool>,
}

impl AudioState {
    pub fn begin_model_operation(&self) -> Result<ModelOperationGuard, String> {
        let mut activity = self.model_activity.lock().map_err(|e| e.to_string())?;
        if activity.exclusive {
            return Err("A model switch is in progress".into());
        }
        activity.active_operations += 1;
        Ok(ModelOperationGuard { activity: self.model_activity.clone(), exclusive: false })
    }

    pub fn begin_exclusive_model_operation(&self) -> Result<ModelOperationGuard, String> {
        let mut activity = self.model_activity.lock().map_err(|e| e.to_string())?;
        if activity.exclusive || activity.active_operations > 0
            || self.recording_handle.lock().map_err(|e| e.to_string())?.is_some()
        {
            return Err("An audio or model operation is in progress".into());
        }
        activity.exclusive = true;
        Ok(ModelOperationGuard { activity: self.model_activity.clone(), exclusive: true })
    }

    pub fn new(
        whisper: WhisperManager,
        granite: GgufAsrManager,
        vad: VADManager,
        qwen3: GgufAsrManager,
    ) -> Self {
        Self {
            recording_handle: Arc::new(Mutex::new(None)),
            whisper: Arc::new(Mutex::new(whisper)),
            granite: Arc::new(Mutex::new(granite)),
            vad: Arc::new(Mutex::new(vad)),
            last_recording_path: Arc::new(Mutex::new(None)),
            active_engine: Arc::new(Mutex::new(ASREngine::Whisper)),
            session_transcript: Arc::new(Mutex::new(String::new())),
            llm: Arc::new(Mutex::new(None)),
            hotkey_config: Arc::new(RwLock::new(HotkeyBinding::default())),
            selected_input_device: Arc::new(Mutex::new(None)),
            denoiser: Arc::new(Mutex::new(None)),
            close_behavior: Arc::new(Mutex::new("tray".to_string())),
            qwen3: Arc::new(Mutex::new(qwen3)),
            hotkey_suppressed: Arc::new(AtomicBool::new(false)),
            recording_paused: Arc::new(AtomicBool::new(false)),
            model_loaded: Arc::new(AtomicBool::new(false)),
            engine_loading: Arc::new(AtomicBool::new(false)),
            model_activity: Arc::new(Mutex::new(ModelActivity::default())),
            auto_unload_seconds: Arc::new(AtomicU64::new(1800)),
            last_activity_timestamp: Arc::new(AtomicU64::new(0)),
            meeting_detector: Arc::new(crate::meeting_detector::MeetingDetectorManager::new()),
            audio_source_mode: Arc::new(Mutex::new("mic".to_string())),
            auto_record_meetings: Arc::new(AtomicBool::new(false)),
            last_recording_is_dual_channel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Record activity timestamp (called whenever audio is transcribed or a model is loaded)
    pub fn touch_activity(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.last_activity_timestamp
            .store(now, std::sync::atomic::Ordering::Relaxed);
    }

    /// True when at least one ASR bundle exists on disk for the currently selected engine.
    /// Used by the tray menu to distinguish "Load Model" from "No model found".
    pub fn active_engine_has_downloaded_model(&self) -> bool {
        let engine = match self.active_engine.lock() {
            Ok(g) => *g,
            Err(_) => ASREngine::Whisper,
        };
        match engine {
            ASREngine::Whisper => crate::whisper::WhisperManager::list_available_models()
                .map(|v| !v.is_empty())
                .unwrap_or(false),
            ASREngine::Granite => crate::gguf_asr::list_available(GRANITE_MODELS)
                .map(|v| !v.is_empty())
                .unwrap_or(false),
            ASREngine::Qwen3 => crate::gguf_asr::list_available(QWEN3_MODELS)
                .map(|v| !v.is_empty())
                .unwrap_or(false),
        }
    }

    /// The GGUF manager behind `engine` (Granite or Qwen3); None for Whisper.
    pub fn gguf_manager(&self, engine: ASREngine) -> Option<Arc<Mutex<GgufAsrManager>>> {
        match engine {
            ASREngine::Granite => Some(self.granite.clone()),
            ASREngine::Qwen3 => Some(self.qwen3.clone()),
            ASREngine::Whisper => None,
        }
    }

    /// Drops weights for every ASR engine that still has a model in memory.
    /// Used by unload UI / tray so we never rely on `active_engine` alone (it can desync).
    pub fn unload_all_loaded_asr(&self) -> Result<Vec<&'static str>, String> {
        // Keep this gate locked through the entire unload. New operations cannot
        // start between the idle check and dropping the weights.
        let activity = self.model_activity.lock().map_err(|e| e.to_string())?;
        if activity.active_operations > 0 || activity.exclusive
            || self.recording_handle.lock().map_err(|e| e.to_string())?.is_some()
            || self.engine_loading.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("An audio or model operation is in progress".into());
        }
        let mut unloaded = Vec::new();

        {
            let mut w = self.whisper.lock().map_err(|e| e.to_string())?;
            if w.get_current_model().is_some() {
                w.unload();
                unloaded.push("whisper");
            }
        }
        {
            let mut g = self.granite.lock().map_err(|e| e.to_string())?;
            if g.get_status().loaded {
                g.unload();
                unloaded.push("granite");
            }
        }
        {
            let mut q = self.qwen3.lock().map_err(|e| e.to_string())?;
            if q.get_status().loaded {
                q.unload();
                unloaded.push("qwen3");
            }
        }

        Ok(unloaded)
    }
}

#[cfg(test)]
mod model_activity_tests {
    use super::*;

    #[test]
    fn unload_waits_until_active_transcription_finishes() {
        let state = AudioState::new(
            WhisperManager::new(),
            GgufAsrManager::granite(),
            VADManager::new().unwrap(),
            GgufAsrManager::qwen3(),
        );
        let operation = state.begin_model_operation().unwrap();
        assert!(state.unload_all_loaded_asr().is_err());
        assert!(state.begin_exclusive_model_operation().is_err());
        drop(operation);
        let switch = state.begin_exclusive_model_operation().unwrap();
        assert!(state.begin_model_operation().is_err());
        drop(switch);
        assert!(state.unload_all_loaded_asr().is_ok());
    }
}
