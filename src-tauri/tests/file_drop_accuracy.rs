//! Accuracy test: file drag-and-drop transcription path.
//!
//! Runs each audio file in a JSONL manifest through the **exact same pipeline**
//! as `commands/file_transcription.rs: transcribe_file_blocking`:
//!
//!   decode → mono mix → resample 16kHz → trim edges
//!     → assemble_speech_audio (Silero VAD) → preprocess_assembled_speech_16k
//!     → chunked engine call → clean_transcript → WER
//!
//! Usage:
//!   TAURSCRIBE_EVAL_MANIFEST=src-tauri/manifest.jsonl \
//!     cargo test file_drop_accuracy -- --ignored --nocapture
//!
//! Skip (CI / no models): TAURSCRIBE_ASR_SMOKE_SKIP=1

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::granite_speech::GraniteSpeechManager;
use taurscribe_lib::librispeech_wer;
use taurscribe_lib::parakeet::ParakeetManager;
use taurscribe_lib::utils::clean_transcript;
use taurscribe_lib::vad::{assemble_speech_audio, VADManager};
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

// ── Audio pipeline (mirrors transcribe_file_blocking exactly) ─────────────────

fn prepare_file_audio(path: &Path) -> Result<Vec<f32>, String> {
    let (raw, sample_rate, channels) = audio_decode::decode_audio_interleaved_f32(path)?;

    // Merge to mono
    let mut mono: Vec<f32> = if channels > 1 {
        let ch = channels as usize;
        raw.chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    } else {
        raw
    };

    // Resample to 16 kHz
    if sample_rate != 16000 {
        mono = audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
    }

    // Trim long edge silence
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);
    if mono.is_empty() {
        return Err("buffer empty after edge trim".into());
    }

    // VAD assembly — fresh instance per utterance, no cancel
    let vad = Arc::new(Mutex::new(
        VADManager::new().map_err(|e| format!("VADManager::new: {e}"))?,
    ));
    let cancel = Arc::new(AtomicBool::new(false));
    let mut speech = assemble_speech_audio(&mono, &vad, Some(&cancel))?;

    if speech.is_empty() {
        return Err("VAD found no speech".into());
    }

    // Universal preprocess on speech-only buffer
    audio_preprocess::preprocess_assembled_speech_16k(&mut speech);
    if speech.is_empty() {
        return Err("buffer empty after preprocess".into());
    }

    Ok(speech)
}

// ── Engine helpers (same chunk sizes as transcribe_file_blocking) ─────────────

const WHISPER_CHUNK_SAMPLES: usize = 16000 * 180; // 3 minutes
const STREAM_CHUNK_SAMPLES: usize = 16000 * 15;   // 15 seconds

fn transcribe_whisper(w: &mut WhisperManager, pcm: &[f32]) -> Result<String, String> {
    let parts: Vec<String> = pcm
        .chunks(WHISPER_CHUNK_SAMPLES)
        .filter_map(|chunk| {
            w.transcribe_audio_data(chunk, None)
                .ok()
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.trim().to_string())
        })
        .collect();
    Ok(clean_transcript(&parts.join(" ")))
}

fn transcribe_parakeet(p: &mut ParakeetManager, pcm: &[f32]) -> Result<String, String> {
    let parts: Vec<String> = pcm
        .chunks(STREAM_CHUNK_SAMPLES)
        .filter_map(|chunk| {
            p.transcribe_chunk(chunk, 16000)
                .ok()
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.trim().to_string())
        })
        .collect();
    Ok(clean_transcript(&parts.join(" ")))
}

fn transcribe_granite(g: &mut GraniteSpeechManager, pcm: &[f32]) -> Result<String, String> {
    let parts: Vec<String> = pcm
        .chunks(STREAM_CHUNK_SAMPLES)
        .filter_map(|chunk| {
            g.transcribe_chunk(chunk, 16000)
                .ok()
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.trim().to_string())
        })
        .collect();
    Ok(clean_transcript(&parts.join(" ")))
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
fn file_drop_accuracy() {
    if std::env::var("TAURSCRIBE_ASR_SMOKE_SKIP").as_deref() == Ok("1") {
        eprintln!("SKIP file_drop_accuracy (TAURSCRIBE_ASR_SMOKE_SKIP=1)");
        return;
    }

    let manifest_path = std::env::var("TAURSCRIBE_EVAL_MANIFEST")
        .expect("Set TAURSCRIBE_EVAL_MANIFEST to a .jsonl manifest path");
    let rows = load_manifest(&manifest_path);
    assert!(!rows.is_empty(), "manifest is empty");

    let mut results: HashMap<&str, Vec<f64>> = HashMap::new();

    // ── Whisper ───────────────────────────────────────────────────────────────
    match WhisperManager::list_available_models() {
        Ok(models) if !models.is_empty() => {
            let mut w = WhisperManager::new();
            match w.initialize(Some(&models[0].id), true) {
                Ok(_) => {
                    let wers = results.entry("whisper").or_default();
                    for row in &rows {
                        let pcm = match prepare_file_audio(Path::new(&row.flac_path)) {
                            Ok(p) => p,
                            Err(e) => { eprintln!("[whisper] {} audio error: {e}", row.utt_id); continue; }
                        };
                        let hyp = match transcribe_whisper(&mut w, &pcm) {
                            Ok(t) => t,
                            Err(e) => { eprintln!("[whisper] {} transcribe error: {e}", row.utt_id); continue; }
                        };
                        let w_val = wer(&row.ref_text, &hyp);
                        let snippet: String = hyp.chars().take(80).collect();
                        eprintln!("[whisper] {} | wer={:.3} | ref: {} | hyp: {}", row.utt_id, w_val, &row.ref_text, snippet);
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
                        let pcm = match prepare_file_audio(Path::new(&row.flac_path)) {
                            Ok(p) => p,
                            Err(e) => { eprintln!("[parakeet] {} audio error: {e}", row.utt_id); continue; }
                        };
                        let hyp = match transcribe_parakeet(&mut p, &pcm) {
                            Ok(t) => t,
                            Err(e) => { eprintln!("[parakeet] {} transcribe error: {e}", row.utt_id); continue; }
                        };
                        let w_val = wer(&row.ref_text, &hyp);
                        let snippet: String = hyp.chars().take(80).collect();
                        eprintln!("[parakeet] {} | wer={:.3} | ref: {} | hyp: {}", row.utt_id, w_val, &row.ref_text, snippet);
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

    // ── Granite Speech ────────────────────────────────────────────────────────
    let mut g = GraniteSpeechManager::new();
    match g.initialize(None, true) {
        Ok(_) => {
            let wers = results.entry("granite").or_default();
            for row in &rows {
                let pcm = match prepare_file_audio(Path::new(&row.flac_path)) {
                    Ok(p) => p,
                    Err(e) => { eprintln!("[granite] {} audio error: {e}", row.utt_id); continue; }
                };
                let hyp = match transcribe_granite(&mut g, &pcm) {
                    Ok(t) => t,
                    Err(e) => { eprintln!("[granite] {} transcribe error: {e}", row.utt_id); continue; }
                };
                let w_val = wer(&row.ref_text, &hyp);
                let snippet: String = hyp.chars().take(80).collect();
                eprintln!("[granite] {} | wer={:.3} | ref: {} | hyp: {}", row.utt_id, w_val, &row.ref_text, snippet);
                wers.push(w_val);
            }
        }
        Err(e) => eprintln!("[SKIP] Granite init: {e} (need INT4 bundle; FP16 is GPU-only on Windows)"),
    }
    g.unload();

    // ── Summary ───────────────────────────────────────────────────────────────
    eprintln!("\n=== file_drop_accuracy summary ===");
    if results.is_empty() {
        eprintln!("No engines produced results — check model installation.");
        return;
    }
    for (engine, wers) in &results {
        let mean = wers.iter().sum::<f64>() / wers.len() as f64;
        let med = median(wers.clone());
        eprintln!("[SUMMARY] {engine}: mean_wer={mean:.4} median_wer={med:.4} n={}", wers.len());
    }
}
