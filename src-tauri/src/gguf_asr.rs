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
    minimum_samples: usize,
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
        // Granite CTC repeatedly halves the encoder sequence. Very short file
        // tails otherwise produce zero frames and report a misleading status 4.
        let minimum_samples = if model.arch() == "granite_speech5_ctc" { 16_000 } else { 0 };
        let session = model.session().map_err(|e| format!("session: {e}"))?;
        Ok(Self { session, backend, minimum_samples })
    }

    /// Transcribe 16 kHz mono PCM.
    pub fn transcribe(&mut self, pcm: &[f32]) -> Result<String, String> {
        let max_samples = self.session.limits().ok().and_then(|limits| {
            (limits.effective_max_audio_ms > 0).then_some(limits.effective_max_audio_ms as usize * 16)
        });
        recover_decode_limits(pcm, max_samples, &mut |audio| {
            run_with_minimum_audio(audio, self.minimum_samples, &mut |input| {
                self.session.run(input, &transcribe_cpp::RunOptions::default()).map(|out| out.text.trim().to_string())
            })
        }).map_err(|e| format!("transcribe: {e}"))
    }

    pub fn transcribe_timed_cancellable(&mut self, pcm: &[f32], cancel: &Arc<AtomicBool>) -> Result<crate::meeting_transcript::TimedTranscript,String> {
        crate::meeting_job::check(cancel)?;
        let watch=crate::meeting_job::NativeCancelWatch::new(cancel.clone());
        self.session.set_cancel_token(&watch.token);
        let result=self.transcribe_timed(pcm);
        self.session.clear_cancel_token();
        crate::meeting_job::check(cancel)?;
        result
    }

    pub fn transcribe_timed(&mut self, pcm: &[f32]) -> Result<crate::meeting_transcript::TimedTranscript, String> {
        use crate::meeting_transcript::{TimedText, TimedTranscript};
        let max_samples = self.session.limits().ok().and_then(|l| (l.effective_max_audio_ms > 0).then_some(l.effective_max_audio_ms as usize * 16));
        let mut units = Vec::new();
        let text = recover_decode_limits(pcm, max_samples, &mut |audio| {
            let offset = (audio.as_ptr() as usize - pcm.as_ptr() as usize) / std::mem::size_of::<f32>() / 16;
            let duration = audio.len() as u64 / 16;
            run_with_minimum_audio(audio, self.minimum_samples, &mut |input| {
                let out = self.session.run(input, &transcribe_cpp::RunOptions::default())?;
                let rows: Vec<_> = if out.timestamp_kind == transcribe_cpp::TimestampKind::None {
                    Vec::new()
                } else if !out.words.is_empty() {
                    out.words.into_iter().map(|w|(w.t0_ms,w.t1_ms,w.text)).collect()
                } else { out.segments.into_iter().map(|w|(w.t0_ms,w.t1_ms,w.text)).collect() };
                for (start,end,text) in rows {
                    if start >= 0 && end > start && end as u64 <= duration {
                        units.push(TimedText { start_ms:offset as u64 + start as u64, end_ms:offset as u64 + end as u64, text });
                    }
                }
                Ok(out.text.trim().to_string())
            })
        }).map_err(|e| format!("transcribe: {e}"))?;
        let mut result=TimedTranscript{text,units};
        if !result.covers_text() { result.units.clear(); }
        Ok(result)
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
        let max_samples = self.session.limits().ok().and_then(|limits| {
            (limits.effective_max_audio_ms > 0).then_some(limits.effective_max_audio_ms as usize * 16)
        });
        let result = recover_decode_limits(pcm, max_samples, &mut |audio| {
            if cancel.load(Ordering::Relaxed) {
                return Err(transcribe_cpp::Error::Aborted { message: "Cancelled".into(), partial: None });
            }
            run_with_minimum_audio(audio, self.minimum_samples, &mut |input| {
                self.session.run(input, &transcribe_cpp::RunOptions::default()).map(|out| out.text.trim().to_string())
            })
        });
        done.store(true, Ordering::Relaxed);
        let _ = watcher.join();
        self.session.clear_cancel_token();
        match result {
            Ok(text) => Ok(text),
            Err(transcribe_cpp::Error::Aborted { .. }) => Err("Transcription cancelled".to_string()),
            Err(e) => Err(format!("transcribe: {e}")),
        }
    }

    /// Device transcribe.cpp chose (e.g. "MTL0", "CUDA0", "Vulkan0", "CPU").
    pub fn backend(&self) -> &str {
        &self.backend
    }
}

/// Add silence only when an encoder cannot represent a short nonempty clip.
/// All original speech samples stay unchanged; empty input remains empty.
fn run_with_minimum_audio(
    pcm: &[f32], minimum: usize,
    run: &mut impl FnMut(&[f32]) -> Result<String, transcribe_cpp::Error>,
) -> Result<String, transcribe_cpp::Error> {
    if pcm.is_empty() { return Ok(String::new()); }
    if pcm.len() >= minimum { return run(pcm); }
    let mut padded = pcm.to_vec();
    padded.resize(minimum, 0.0);
    run(&padded)
}

/// A failed decode is incomplete by contract. Retry all its audio, never its
/// partial text. This shared path covers files, live speech, and meeting turns.
fn recover_decode_limits(
    pcm: &[f32],
    max_samples: Option<usize>,
    run: &mut impl FnMut(&[f32]) -> Result<String, transcribe_cpp::Error>,
) -> Result<String, transcribe_cpp::Error> {
    const MIN_SECTION: usize = 16_000 * 2;
    let over_input_limit = max_samples.is_some_and(|max| max > 0 && pcm.len() > max);
    if !over_input_limit {
        match run(pcm) {
            Ok(text) => return Ok(text),
            Err(error @ (transcribe_cpp::Error::OutputTruncated { .. } | transcribe_cpp::Error::InputTooLong(_))) => {
                if pcm.len() < MIN_SECTION * 2 {
                    return Err(transcribe_cpp::Error::InputTooLong(format!(
                        "The speech model still reached its decode limit on a short section after retrying. The recording/file has not been marked complete; try another speech model. ({error})"
                    )));
                }
                eprintln!("[ASR] Decode limit on {:.1}s; retrying shorter sections", pcm.len() as f32 / 16_000.0);
            }
            Err(transcribe_cpp::Error::ModelLoad(message))
                if message.starts_with("run:") && message.contains("status 4") => {
                if pcm.len() < MIN_SECTION * 2 {
                    return Err(transcribe_cpp::Error::ModelLoad(format!(
                        "The loaded speech model could not process this audio, even in a short section. Try reloading the model or another compute device. The file has not been saved as complete. ({message})"
                    )));
                }
                eprintln!("[ASR] Inference graph failed on {:.1}s; retrying all audio in shorter sections: {message}", pcm.len() as f32 / 16_000.0);
            }
            Err(other) => return Err(other),
        }
    } else if pcm.len() < MIN_SECTION * 2 {
        // Still let the engine return its precise error for an unusually tiny limit.
        return run(pcm);
    }
    // Search around the midpoint for quiet audio, reducing cuts through words.
    let midpoint = pcm.len() / 2;
    let start = (midpoint - pcm.len() / 8).max(MIN_SECTION);
    let end = (midpoint + pcm.len() / 8).min(pcm.len() - MIN_SECTION);
    let mut cut = midpoint;
    let mut quietest = f32::INFINITY;
    for at in (start..end).step_by(320) {
        let energy: f32 = pcm[at..(at + 320).min(pcm.len())].iter().map(|sample| sample * sample).sum();
        if energy < quietest { quietest = energy; cut = at; }
    }
    let first = recover_decode_limits(&pcm[..cut], max_samples, run)?;
    let second = recover_decode_limits(&pcm[cut..], max_samples, run)?;
    Ok([first, second].into_iter().filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" "))
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
    /// `gpu::Target` key the loaded model was loaded for.
    device_key: String,
    /// Copy of `get_status()` kept up to date by `initialize` / `unload`, so the
    /// UI can read it without waiting on this manager's lock (held for a whole
    /// transcription).
    snapshot: Arc<std::sync::Mutex<GgufStatus>>,
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
            device_key: String::new(),
            snapshot: Arc::new(std::sync::Mutex::new(GgufStatus {
                loaded: false,
                model_id: None,
                backend: "none".to_string(),
            })),
        }
    }

    /// Shared, always-current status; readable while the manager is locked.
    pub fn status_handle(&self) -> Arc<std::sync::Mutex<GgufStatus>> {
        self.snapshot.clone()
    }

    fn publish_status(&self) {
        let status = self.get_status();
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) = status;
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
        self.publish_status();
        crate::memory::trim_process_memory();
    }

    /// The `gpu::Target` key the loaded model was loaded for.
    pub fn device_key(&self) -> &str {
        &self.device_key
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
        let target = crate::gpu::Target::resolve(force_cpu);
        let options = transcribe_cpp::ModelOptions {
            backend: if target == crate::gpu::Target::Cpu { transcribe_cpp::Backend::Cpu } else { transcribe_cpp::Backend::Auto },
            device: target.transcribe_device(),
        };
        let mut engine = GgufAsr::load_with(&path, &options)?;
        // Compile GPU kernels now rather than on the first dictation.
        engine.transcribe(&vec![0.0; 16_000]).map_err(|error| format!("Model warm-up failed; the model is not ready: {error}"))?;
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
        self.device_key = target.key();
        self.publish_status();
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

    pub fn transcribe_timed(&mut self, pcm: &[f32]) -> Result<crate::meeting_transcript::TimedTranscript, String> {
        let mut result = self.engine.as_mut().ok_or("Speech model not loaded")?.transcribe_timed(pcm)?;
        result.text = strip_whitelisted_sound_captions(&result.text).trim().to_string();
        if self.sentence_case { result.text = sentence_case(&result.text); }
        fill_word_times(&mut result, pcm);
        Ok(result)
    }

    pub fn transcribe_timed_cancellable(&mut self, pcm: &[f32], cancel: &Arc<AtomicBool>) -> Result<crate::meeting_transcript::TimedTranscript,String> {
        let mut result=self.engine.as_mut().ok_or("Speech model not loaded")?.transcribe_timed_cancellable(pcm,cancel)?;
        result.text=strip_whitelisted_sound_captions(&result.text).trim().to_string();
        if self.sentence_case {result.text=sentence_case(&result.text);}
        fill_word_times(&mut result,pcm); Ok(result)
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


/// These decoders return one span for a whole clip, not word times. Align the
/// transcript to the audio with the CTC word aligner so speakers can be assigned
/// word by word; without the model (or when the audio and text do not line up)
/// the units stay empty and the caller keeps its per-speaker decode path.
fn fill_word_times(result: &mut crate::meeting_transcript::TimedTranscript, pcm: &[f32]) {
    if result.covers_text() {
        return;
    }
    result.units.clear();
    // Opt-in: measured on Granite, alignment costs ~1.4 s per 30 s chunk (several
    // times the decoder) and does not improve the transcript, because re-decoding
    // a speaker's turn is already cheap. Kept for decoders that are slow per turn.
    if result.text.trim().is_empty() || std::env::var("TAURSCRIBE_WORD_ALIGN").as_deref() != Ok("1") {
        return;
    }
    if let Some(units) = crate::word_align::align_transcript(pcm, &result.text) {
        result.units = units;
        if !result.covers_text() {
            result.units.clear();
        }
    }
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

#[cfg(test)]
mod decode_limit_tests {
    use super::*;
    /// Uses a real model but injects the first status-18 result, making the
    /// recovery reproducible without depending on nondeterministic hallucination.
    #[test]
    #[ignore = "requires installed Granite model and a GPU"]
    fn native_decode_limit_recovery_preserves_fixture_transcript() {
        let model_path = gguf_in(&crate::utils::get_models_dir().unwrap().join("granite-speech-5-nc")).expect("Granite model required");
        let mut engine = GgufAsr::load(&model_path).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../taurscribe-runtime/samples/jfk.wav");
        let (audio, rate) = crate::audio_decode::decode_audio_mono_f32(&fixture).unwrap();
        let audio = crate::audio_preprocess::resample_mono_to_16k(&audio, rate).unwrap();
        let baseline = engine.transcribe(&audio).unwrap();
        let mut first = true;
        let recovered = recover_decode_limits(&audio, None, &mut |part| {
            if first {
                first = false;
                return Err(transcribe_cpp::Error::OutputTruncated { message: "injected status 18".into(), partial: None });
            }
            engine.session.run(part, &transcribe_cpp::RunOptions::default()).map(|out| out.text.trim().to_string())
        }).unwrap();
        let normalize = |text: &str| text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>();
        assert_eq!(normalize(&recovered), normalize(&baseline), "shorter retries changed the fixed-fixture transcript");
        println!("NATIVE_RECOVERY backend={} baseline={} recovered={}", engine.backend(), baseline, recovered);
    }

    #[test]
    #[ignore = "requires installed Granite model; reproduces status 4 then tests the repair"]
    fn native_short_tail_no_longer_fails_after_model_load() {
        let path=gguf_in(&crate::utils::get_models_dir().unwrap().join("granite-speech-5-nc")).expect("Granite model required");
        let mut engine=GgufAsr::load(&path).unwrap();
        let clip:Vec<f32>=(0..800).map(|i|(i as f32*0.1).sin()*0.01).collect();
        let failure=engine.session.run(&clip,&Default::default()).unwrap_err();
        assert!(failure.to_string().contains("status 4"),"{failure}");
        for length in [160,800,1600,3200] {
            let clip:Vec<f32>=(0..length).map(|i|(i as f32*0.1).sin()*0.01).collect();
            engine.transcribe(&clip).unwrap();
            engine.transcribe_cancellable(&clip,&Arc::new(AtomicBool::new(false))).unwrap();
        }
        println!("NATIVE_SHORT_TAIL backend={} repaired all lengths",engine.backend());
    }

    #[test]
    fn tiny_input_preserves_speech_and_only_pads_silence() {
        let original = vec![0.25; 800];
        let text = run_with_minimum_audio(&original, 16_000, &mut |part| {
            assert_eq!(&part[..800], original.as_slice());
            assert_eq!(part.len(), 16_000);
            assert!(part[800..].iter().all(|sample| *sample == 0.0));
            Ok("short speech".into())
        }).unwrap();
        assert_eq!(text, "short speech");
        assert_eq!(run_with_minimum_audio(&[], 16_000, &mut |_| panic!("empty audio must not run")).unwrap(), "");
    }

    #[test]
    fn inference_status_four_recovers_every_sample_but_load_errors_do_not_retry() {
        let audio: Vec<f32> = (0..160_000).map(|i| i as f32).collect();
        let mut recovered = Vec::new();
        recover_decode_limits(&audio, None, &mut |part| {
            if part.len() > 64_000 {
                Err(transcribe_cpp::Error::ModelLoad("run: gguf load error (status 4)".into()))
            } else { recovered.extend_from_slice(part); Ok("speech".into()) }
        }).unwrap();
        assert_eq!(recovered, audio);
        let mut calls = 0;
        let result = recover_decode_limits(&audio, None, &mut |_| {
            calls += 1;
            Err(transcribe_cpp::Error::ModelLoad("load model: gguf load error (status 4)".into()))
        });
        assert!(result.is_err()); assert_eq!(calls, 1);
        let result = recover_decode_limits(&audio, None, &mut |_| {
            Err(transcribe_cpp::Error::ModelLoad("run: gguf load error (status 4)".into()))
        });
        assert!(result.unwrap_err().to_string().contains("not been saved as complete"));
    }

    #[test]
    fn truncated_file_or_meeting_turn_retries_every_sample_in_order() {
        let audio = (0..16_000 * 25).map(|i| i as f32).collect::<Vec<_>>();
        let mut successful = Vec::new(); let mut failures = 0; let mut sections = 0;
        let text = recover_decode_limits(&audio, None, &mut |part| {
            if part.len() > 16_000 * 5 {
                failures += 1;
                Err(transcribe_cpp::Error::OutputTruncated { message: "status 18".into(), partial: Some(Box::new(transcribe_cpp::Transcript { text: "incomplete partial must be discarded".into(), ..Default::default() })) })
            } else {
                sections += 1; successful.extend_from_slice(part);
                Ok(format!("{}", part[0]))
            }
        }).unwrap();
        assert!(failures > 1); assert_eq!(successful, audio);
        assert_eq!(text.split_whitespace().count(), sections);
        assert!(text.starts_with('0')); assert!(!text.contains("incomplete"));
    }
    #[test]
    fn declared_input_limit_avoids_an_oversized_run() {
        let mut samples = 0;
        recover_decode_limits(&vec![0.0; 160_000], Some(64_000), &mut |part| {
            assert!(part.len() <= 64_000); samples += part.len(); Ok("speech".into())
        }).unwrap();
        assert_eq!(samples, 160_000);
    }
    #[test]
    fn cancellation_and_non_limit_failures_do_not_retry_or_return_partial_success() {
        for cancelled in [false, true] {
            let mut calls = 0;
            let result = recover_decode_limits(&vec![0.0; 160_000], None, &mut |_| {
                calls += 1;
                if calls == 1 { return Err(transcribe_cpp::Error::InputTooLong("cap".into())); }
                if calls == 2 { return Ok("first half".into()); }
                if cancelled { Err(transcribe_cpp::Error::Aborted { message: "cancel".into(), partial: None }) }
                else { Err(transcribe_cpp::Error::Backend("device failed".into())) }
            });
            assert!(result.is_err()); assert_eq!(calls, 3);
        }
    }
    #[test]
    fn pathological_output_is_bounded_and_never_saved_as_complete() {
        let mut calls = 0;
        let result = recover_decode_limits(&vec![0.0; 160_000], None, &mut |_| {
            calls += 1; Err(transcribe_cpp::Error::OutputTruncated { message: "cap".into(), partial: None })
        });
        assert!(result.unwrap_err().to_string().contains("not been marked complete"));
        assert!(calls <= 4);
    }
}
