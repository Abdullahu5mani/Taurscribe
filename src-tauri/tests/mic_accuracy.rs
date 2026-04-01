//! Accuracy test: live microphone recording path.
//!
//! Simulates the recording thread from `commands/recording.rs` without cpal or Tauri.
//! Each audio file is decoded at its native sample rate and fed through the exact same
//! chunk-accumulation and preprocessing loop the app uses during a live recording:
//!
//! Whisper / Cohere (6s chunks + VAD gate):
//!   native-rate mono → 6s chunks → preprocess_live_transcribe_chunk
//!     → vad.is_speech() > 0.35 gate → transcribe_chunk
//!
//! Parakeet (4s chunks, no VAD gate):
//!   native-rate mono → 4s chunks → preprocess_live_transcribe_chunk
//!     → pad to 64000 samples → transcribe_chunk
//!
//! Usage:
//!   TAURSCRIBE_EVAL_MANIFEST=src-tauri/manifest.jsonl \
//!     cargo test mic_accuracy -- --ignored --nocapture
//!
//! Skip (CI / no models): TAURSCRIBE_ASR_SMOKE_SKIP=1
//!
//! If manifest FLAC paths are stale, set `TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT` to your
//! LibriSpeech `test-clean` directory (same as `librispeech_eval --audio-root`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::cohere::CohereManager;
use taurscribe_lib::librispeech_wer;
use taurscribe_lib::parakeet::ParakeetManager;
use taurscribe_lib::vad::VADManager;
use taurscribe_lib::whisper::WhisperManager;

// ── Manifest ──────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ManifestRow {
    utt_id: String,
    flac_path: String,
    ref_text: String,
}

fn load_manifest(path: &str) -> Vec<ManifestRow> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Cannot read manifest {path}: {e}"));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("Bad manifest line: {e}\n{l}")))
        .collect()
}

// ── Audio loading (native rate, like the cpal callback) ───────────────────────

/// Decode file to mono at its native sample rate — simulates cpal mic input.
/// The recording thread receives native-rate stereo and converts to mono;
/// `preprocess_live_transcribe_chunk` then handles the resample to 16 kHz.
fn load_native_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let (raw, sample_rate, channels) = audio_decode::decode_audio_interleaved_f32(path)?;
    let mono: Vec<f32> = if channels > 1 {
        let ch = channels as usize;
        raw.chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    } else {
        raw
    };
    Ok((mono, sample_rate))
}

// ── Mic simulation helpers ────────────────────────────────────────────────────

/// Whisper / Cohere live path: 6s chunks, VAD-gated, no clean_transcript.
/// Mirrors `vad_gated_transcribe` inside `start_recording_blocking`.
fn mic_sim_whisper_granite<F>(
    samples: &[f32],
    sample_rate: u32,
    vad: &Arc<Mutex<VADManager>>,
    mut transcribe: F,
) -> String
where
    F: FnMut(&[f32]) -> Result<String, String>,
{
    const VAD_THRESHOLD: f32 = 0.25;
    // Short-tail threshold in 16kHz samples (3 seconds)
    const SHORT_TAIL_SAMPLES_16K: usize = 16000 * 3;

    let chunk_size = (sample_rate as usize) * 6;
    let mut parts: Vec<String> = Vec::new();

    let chunks: Vec<&[f32]> = samples.chunks(chunk_size).collect();
    let total = chunks.len();

    for (i, chunk) in chunks.iter().enumerate() {
        let pcm16 =
            audio_preprocess::preprocess_live_transcribe_chunk(chunk, sample_rate, false, None);
        if pcm16.is_empty() {
            continue;
        }

        let is_tail = i == total - 1;
        let bypass_vad = is_tail && pcm16.len() < SHORT_TAIL_SAMPLES_16K;

        let should_transcribe = if bypass_vad {
            true
        } else {
            // Mirror recording.rs: scan full chunk, use peak prob.
            let prob = vad
                .lock()
                .ok()
                .map(|mut v| v.max_speech_prob(&pcm16, usize::MAX))
                .unwrap_or(0.0);
            prob > VAD_THRESHOLD
        };

        if should_transcribe {
            if let Ok(t) = transcribe(&pcm16) {
                if !t.trim().is_empty() {
                    parts.push(t.trim().to_string());
                }
            }
        }
    }

    parts.join(" ")
}

/// Parakeet live path: 4s chunks, no VAD gate, pad to 64000 samples.
/// Mirrors `parakeet_preprocess_for_transcribe` + accumulation loop.
fn mic_sim_parakeet(samples: &[f32], sample_rate: u32, p: &mut ParakeetManager) -> String {
    const MIN_PARAKEET_SAMPLES: usize = 16000 * 4; // 64000

    let chunk_size = (sample_rate as usize) * 4;
    let mut parts: Vec<String> = Vec::new();

    for chunk in samples.chunks(chunk_size) {
        let mut pcm16 =
            audio_preprocess::preprocess_live_transcribe_chunk(chunk, sample_rate, false, None);
        if pcm16.is_empty() {
            continue;
        }
        if pcm16.len() < MIN_PARAKEET_SAMPLES {
            pcm16.resize(MIN_PARAKEET_SAMPLES, 0.0);
        }
        if let Ok(t) = p.transcribe_chunk(&pcm16, 16000) {
            if !t.trim().is_empty() {
                parts.push(t.trim().to_string());
            }
        }
    }

    parts.join(" ")
}

// ── Stats helpers ─────────────────────────────────────────────────────────────

fn median(mut xs: Vec<f64>) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = xs.len() / 2;
    if xs.len() % 2 == 1 {
        xs[mid]
    } else {
        (xs[mid - 1] + xs[mid]) / 2.0
    }
}

fn wer(ref_text: &str, hyp: &str) -> f64 {
    let r = librispeech_wer::normalize_for_wer(ref_text);
    let h = librispeech_wer::normalize_for_wer(hyp);
    librispeech_wer::word_error_rate(&r, &h)
}

// ── Test ──────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "Needs TAURSCRIBE_EVAL_MANIFEST + at least one engine installed. Run with --ignored --nocapture."]
fn mic_accuracy() {
    if std::env::var("TAURSCRIBE_ASR_SMOKE_SKIP").as_deref() == Ok("1") {
        eprintln!("SKIP mic_accuracy (TAURSCRIBE_ASR_SMOKE_SKIP=1)");
        return;
    }

    let manifest_path = std::env::var("TAURSCRIBE_EVAL_MANIFEST")
        .expect("Set TAURSCRIBE_EVAL_MANIFEST to a .jsonl manifest path");
    let rows = load_manifest(&manifest_path);
    assert!(!rows.is_empty(), "manifest is empty");

    let audio_root: Option<std::path::PathBuf> = std::env::var("TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT")
        .ok()
        .map(Into::into);

    // One VAD per engine run; reset_state() called before each utterance
    let vad = Arc::new(Mutex::new(
        VADManager::new().expect("VADManager::new failed"),
    ));

    let mut results: HashMap<&str, Vec<f64>> = HashMap::new();

    // ── Whisper ───────────────────────────────────────────────────────────────
    match WhisperManager::list_available_models() {
        Ok(models) if !models.is_empty() => {
            let mut w = WhisperManager::new();
            match w.initialize(Some(&models[0].id), true) {
                Ok(_) => {
                    let wers = results.entry("whisper").or_default();
                    for row in &rows {
                        let flac = librispeech_wer::resolve_librispeech_flac(
                            &row.flac_path,
                            &row.utt_id,
                            audio_root.as_deref(),
                        );
                        let (samples, rate) = match load_native_mono(&flac) {
                            Ok(v) => v,
                            Err(e) => {
                                eprintln!("[whisper] {} audio error: {e}", row.utt_id);
                                continue;
                            }
                        };
                        // Reset LSTM state — simulates start of a fresh recording
                        if let Ok(mut v) = vad.lock() {
                            v.reset_state();
                        }

                        let vad_ref = Arc::clone(&vad);
                        let hyp = mic_sim_whisper_granite(&samples, rate, &vad_ref, |pcm| {
                            w.transcribe_chunk(pcm, 16000)
                        });
                        let w_val = wer(&row.ref_text, &hyp);
                        let snippet: String = hyp.chars().take(80).collect();
                        eprintln!(
                            "[whisper] {} | wer={:.3} | ref: {} | hyp: {}",
                            row.utt_id, w_val, &row.ref_text, snippet
                        );
                        wers.push(w_val);
                    }
                }
                Err(e) => eprintln!("[SKIP] Whisper init: {e}"),
            }
            w.unload();
        }
        Ok(_) => eprintln!("[SKIP] Whisper: no models installed"),
        Err(e) => eprintln!("[SKIP] Whisper list_models: {e}"),
    }

    // ── Parakeet ──────────────────────────────────────────────────────────────
    match ParakeetManager::list_available_models() {
        Ok(models) if !models.is_empty() => {
            let mut p = ParakeetManager::new();
            match p.initialize(None, true) {
                Ok(_) => {
                    let wers = results.entry("parakeet").or_default();
                    for row in &rows {
                        let flac = librispeech_wer::resolve_librispeech_flac(
                            &row.flac_path,
                            &row.utt_id,
                            audio_root.as_deref(),
                        );
                        let (samples, rate) = match load_native_mono(&flac) {
                            Ok(v) => v,
                            Err(e) => {
                                eprintln!("[parakeet] {} audio error: {e}", row.utt_id);
                                continue;
                            }
                        };
                        let hyp = mic_sim_parakeet(&samples, rate, &mut p);
                        let w_val = wer(&row.ref_text, &hyp);
                        let snippet: String = hyp.chars().take(80).collect();
                        eprintln!(
                            "[parakeet] {} | wer={:.3} | ref: {} | hyp: {}",
                            row.utt_id, w_val, &row.ref_text, snippet
                        );
                        wers.push(w_val);
                    }
                }
                Err(e) => eprintln!("[SKIP] Parakeet init: {e}"),
            }
            p.unload();
        }
        Ok(_) => eprintln!("[SKIP] Parakeet: no models installed"),
        Err(e) => eprintln!("[SKIP] Parakeet list_models: {e}"),
    }

    // ── Cohere ───────────────────────────────────────────────────────────────
    let mut g = CohereManager::new();
    match g.initialize(None, true) {
        Ok(_) => {
            let wers = results.entry("granite").or_default();
            for row in &rows {
                let flac = librispeech_wer::resolve_librispeech_flac(
                    &row.flac_path,
                    &row.utt_id,
                    audio_root.as_deref(),
                );
                let (samples, rate) = match load_native_mono(&flac) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[granite] {} audio error: {e}", row.utt_id);
                        continue;
                    }
                };
                if let Ok(mut v) = vad.lock() {
                    v.reset_state();
                }

                let vad_ref = Arc::clone(&vad);
                let hyp = mic_sim_whisper_granite(&samples, rate, &vad_ref, |pcm| {
                    g.transcribe_chunk(pcm, 16000)
                });
                let w_val = wer(&row.ref_text, &hyp);
                let snippet: String = hyp.chars().take(80).collect();
                eprintln!(
                    "[granite] {} | wer={:.3} | ref: {} | hyp: {}",
                    row.utt_id, w_val, &row.ref_text, snippet
                );
                wers.push(w_val);
            }
        }
        Err(e) => eprintln!("[SKIP] Cohere init: {e} (need q4f16 bundle in granite-speech-1b)"),
    }
    g.unload();

    // ── Summary ───────────────────────────────────────────────────────────────
    eprintln!("\n=== mic_accuracy summary ===");
    if results.is_empty() {
        eprintln!("No engines produced results — check model installation.");
        return;
    }
    for (engine, wers) in &results {
        let mean = wers.iter().sum::<f64>() / wers.len() as f64;
        let med = median(wers.clone());
        eprintln!(
            "[SUMMARY] {engine}: mean_wer={mean:.4} median_wer={med:.4} n={}",
            wers.len()
        );
    }
}
