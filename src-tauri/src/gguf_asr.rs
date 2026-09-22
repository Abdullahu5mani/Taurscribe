//! GGUF speech models through transcribe.cpp (ggml): Granite Speech 5 and
//! Qwen3-ASR, the same unquantized F16 files on every platform.
//!
//! transcribe.cpp picks the best compiled-in device (Metal on Apple Silicon,
//! CUDA / Vulkan / ROCm where built in) and falls back to its CPU path.

use crate::utils::strip_whitelisted_sound_captions;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One loaded GGUF model with its inference session.
pub struct GgufAsr {
    session: transcribe_cpp::Session,
    backend: String,
}

impl GgufAsr {
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::load_on(path, transcribe_cpp::Backend::Auto)
    }

    /// Load on a specific backend (`Auto` picks the best compiled-in device).
    pub fn load_on(path: &Path, backend: transcribe_cpp::Backend) -> Result<Self, String> {
        Self::load_with(path, &transcribe_cpp::ModelOptions { backend, device: None })
    }

    /// Load with explicit transcribe.cpp options (e.g. a specific GPU device).
    pub fn load_with(path: &Path, options: &transcribe_cpp::ModelOptions) -> Result<Self, String> {
        let model = transcribe_cpp::Model::load_with(path, options)
            .map_err(|e| format!("load {}: {e}", path.display()))?;
        let backend = model.backend();
        let session = model.session().map_err(|e| format!("session: {e}"))?;
        Ok(Self { session, backend })
    }

    /// Transcribe 16 kHz mono PCM.
    pub fn transcribe(&mut self, pcm: &[f32]) -> Result<String, String> {
        let out = self
            .session
            .run(pcm, &transcribe_cpp::RunOptions::default())
            .map_err(|e| format!("transcribe: {e}"))?;
        Ok(out.text.trim().to_string())
    }

    /// Transcribe, aborting mid-run once `cancel` is set. transcribe.cpp polls its
    /// own token between decode steps; a watcher thread mirrors `cancel` into it.
    /// Qwen3-ASR checks it only before a run starts, Granite also between its
    /// internal audio windows, so a short run always completes.
    pub fn transcribe_cancellable(&mut self, pcm: &[f32], cancel: &Arc<AtomicBool>) -> Result<String, String> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Transcription cancelled".to_string());
        }
        let token = transcribe_cpp::CancelToken::new();
        self.session.set_cancel_token(&token);
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let (token, done, cancel) = (token.clone(), done.clone(), cancel.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        token.cancel();
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
        };
        let result = self.session.run(pcm, &transcribe_cpp::RunOptions::default());
        done.store(true, Ordering::Relaxed);
        let _ = watcher.join();
        self.session.clear_cancel_token();
        match result {
            Ok(out) => Ok(out.text.trim().to_string()),
            Err(transcribe_cpp::Error::Aborted { .. }) => Err("Transcription cancelled".to_string()),
            Err(e) => Err(format!("transcribe: {e}")),
        }
    }

    /// Device transcribe.cpp chose (e.g. "MTL0", "CUDA0", "Vulkan0", "CPU").
    pub fn backend(&self) -> &str {
        &self.backend
    }
}

/// A downloadable model of a GGUF family. `id` is also its folder under the
/// models directory, which holds exactly one `.gguf` file.
pub struct GgufModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub size_mb: f32,
}

pub const GRANITE_MODELS: &[GgufModelSpec] = &[GgufModelSpec {
    id: "granite-speech-5-nc",
    display_name: "Granite Speech 5 (470M)",
    size_mb: 949.0,
}];

pub const QWEN3_MODELS: &[GgufModelSpec] = &[
    GgufModelSpec {
        id: "qwen3-asr-1.7b",
        display_name: "Qwen3-ASR 1.7B",
        size_mb: 4_091.0,
    },
    GgufModelSpec {
        id: "qwen3-asr-0.6b",
        display_name: "Qwen3-ASR 0.6B",
        size_mb: 1_580.0,
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct GgufStatus {
    pub loaded: bool,
    pub model_id: Option<String>,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GgufModelInfo {
    pub id: String,
    pub display_name: String,
    pub size_mb: f32,
}

/// Loads and runs one model of a GGUF family (Granite or Qwen3).
pub struct GgufAsrManager {
    family: &'static str,
    models: &'static [GgufModelSpec],
    /// Granite emits lowercase, unpunctuated text; give it sentence casing.
    sentence_case: bool,
    engine: Option<GgufAsr>,
    model_id: Option<String>,
}

pub type GraniteManager = GgufAsrManager;
pub type Qwen3Manager = GgufAsrManager;

impl GgufAsrManager {
    pub fn granite() -> Self {
        Self::new("Granite", GRANITE_MODELS, true)
    }

    pub fn qwen3() -> Self {
        Self::new("Qwen3", QWEN3_MODELS, false)
    }

    fn new(family: &'static str, models: &'static [GgufModelSpec], sentence_case: bool) -> Self {
        Self {
            family,
            models,
            sentence_case,
            engine: None,
            model_id: None,
        }
    }

    pub fn get_status(&self) -> GgufStatus {
        GgufStatus {
            loaded: self.engine.is_some(),
            model_id: self.model_id.clone(),
            backend: self
                .engine
                .as_ref()
                .map(|e| e.backend().to_string())
                .unwrap_or_else(|| "none".to_string()),
        }
    }

    pub fn unload(&mut self) {
        if self.engine.take().is_some() {
            println!("[{}] Unloaded", self.family.to_uppercase());
        }
        self.model_id = None;
        crate::memory::trim_process_memory();
    }

    /// Stateless per-chunk engine: nothing to reset between recordings.
    pub fn clear_context(&mut self) {}

    /// Models of this family that are downloaded.
    pub fn list_available_models(&self) -> Result<Vec<GgufModelInfo>, String> {
        list_available(self.models)
    }

    /// Load `model_id`, or the first downloaded model of the family.
    /// transcribe.cpp chooses the device; `force_cpu` pins it to the CPU.
    pub fn initialize(&mut self, model_id: Option<&str>, force_cpu: bool) -> Result<String, String> {
        self.unload();
        let models_dir = crate::utils::get_models_dir()?;
        let spec = match model_id {
            Some(id) => self
                .models
                .iter()
                .find(|m| m.id == id)
                .ok_or_else(|| format!("Unknown {} model: {id}", self.family))?,
            None => self
                .models
                .iter()
                .find(|m| gguf_in(&models_dir.join(m.id)).is_some())
                .ok_or_else(|| format!("No {} model downloaded", self.family))?,
        };
        let path = gguf_in(&models_dir.join(spec.id)).ok_or_else(|| {
            format!("{} is not downloaded. Download it from Settings > Models.", spec.display_name)
        })?;

        let started = std::time::Instant::now();
        let backend = if force_cpu { transcribe_cpp::Backend::Cpu } else { transcribe_cpp::Backend::Auto };
        let mut engine = GgufAsr::load_on(&path, backend)?;
        // Compile GPU kernels now rather than on the first dictation.
        let _ = engine.transcribe(&vec![0.0; 16_000]);
        println!(
            "[{}] {} ready on {} in {:.2}s",
            self.family.to_uppercase(),
            spec.display_name,
            engine.backend(),
            started.elapsed().as_secs_f32()
        );
        let message = format!("{} loaded ({})", spec.display_name, engine.backend());
        self.engine = Some(engine);
        self.model_id = Some(spec.id.to_string());
        Ok(message)
    }

    pub fn transcribe_audio_data(&mut self, samples: &[f32], prompt: Option<&str>) -> Result<String, String> {
        self.transcribe_chunk(samples, 16_000, prompt)
    }

    /// Transcribe mono PCM at `sample_rate`. `prompt` (custom vocabulary) is not
    /// used by these models.
    pub fn transcribe_chunk(&mut self, samples: &[f32], sample_rate: u32, _prompt: Option<&str>) -> Result<String, String> {
        self.transcribe_inner(samples, sample_rate, None)
    }

    /// Like `transcribe_chunk`, but setting `cancel` aborts the run mid-inference
    /// (a single long chunk can take seconds). Returns "Transcription cancelled".
    pub fn transcribe_chunk_cancellable(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String, String> {
        self.transcribe_inner(samples, sample_rate, Some(cancel))
    }

    fn transcribe_inner(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<String, String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let engine = self
            .engine
            .as_mut()
            .ok_or_else(|| format!("{} model not loaded", self.family))?;
        let resampled;
        let audio = if sample_rate == 16_000 {
            samples
        } else {
            resampled = crate::audio_preprocess::resample_mono_to_16k(samples, sample_rate)?;
            &resampled
        };
        let text = match cancel {
            None => engine.transcribe(audio)?,
            Some(cancel) => engine.transcribe_cancellable(audio, cancel)?,
        };
        let text = strip_whitelisted_sound_captions(&text).trim().to_string();
        Ok(if self.sentence_case { sentence_case(&text) } else { text })
    }
}

/// Downloaded models among `models`.
pub fn list_available(models: &[GgufModelSpec]) -> Result<Vec<GgufModelInfo>, String> {
    let models_dir = crate::utils::get_models_dir()?;
    Ok(models
        .iter()
        .filter(|m| gguf_in(&models_dir.join(m.id)).is_some())
        .map(|m| GgufModelInfo {
            id: m.id.to_string(),
            display_name: m.display_name.to_string(),
            size_mb: m.size_mb,
        })
        .collect())
}

/// Capitalise the first letter and the pronoun "i" of lowercase CTC output.
pub fn sentence_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if i == 0 || word == "i" || word.starts_with("i'") {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        } else {
            out.push_str(word);
        }
    }
    out
}

/// The single `.gguf` file in a model folder, if there is exactly one.
pub fn gguf_in(dir: &Path) -> Option<PathBuf> {
    let mut found = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gguf")));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gguf_in_requires_exactly_one_file() {
        let dir = std::env::temp_dir().join(format!("taurscribe_gguf_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(gguf_in(&dir).is_none());
        std::fs::write(dir.join("a.gguf"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert_eq!(gguf_in(&dir).unwrap().file_name().unwrap(), "a.gguf");
        std::fs::write(dir.join("b.gguf"), b"x").unwrap();
        assert!(gguf_in(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sentence_case_capitalises_start_and_pronoun() {
        assert_eq!(sentence_case("i think i am here"), "I think I am here");
        assert_eq!(sentence_case("and so my fellow americans"), "And so my fellow americans");
        assert_eq!(sentence_case(""), "");
    }

    #[test]
    fn managers_start_unloaded() {
        for mut m in [GgufAsrManager::granite(), GgufAsrManager::qwen3()] {
            assert!(!m.get_status().loaded);
            assert_eq!(m.transcribe_chunk(&[], 16_000, None).unwrap(), "");
            assert!(m.transcribe_chunk(&[0.0; 10], 16_000, None).is_err());
            m.unload();
        }
    }

    #[test]
    fn model_ids_are_unique_folders() {
        let ids: std::collections::HashSet<_> = GRANITE_MODELS.iter().chain(QWEN3_MODELS).map(|m| m.id).collect();
        assert_eq!(ids.len(), GRANITE_MODELS.len() + QWEN3_MODELS.len());
    }
}
