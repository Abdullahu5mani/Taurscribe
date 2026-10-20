//! Accuracy test: file drag-and-drop transcription path.
//!
//! Exercises file preprocessing and installed engines over a JSONL manifest.
//! This diagnostic calls engine managers directly; the native UI harness covers
//! `commands/file_transcription.rs` scheduling, progress and persistence.
//!
//!   decode → mono mix → resample 16kHz → trim edges
//!     → assemble_speech_audio (energy VAD) → preprocess_assembled_speech_16k
//!     → chunked engine call → clean_transcript → WER
//!
//! Usage:
//!   TAURSCRIBE_EVAL_MANIFEST=src-tauri/manifest.jsonl \
//!     cargo test file_drop_accuracy -- --ignored --nocapture
//!
//! Skip (CI / no models): TAURSCRIBE_ASR_SMOKE_SKIP=1
//!
//! Stale manifest paths: set `TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT` to LibriSpeech `test-clean`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::librispeech_wer;
use taurscribe_lib::gguf_asr::GgufAsrManager;
use taurscribe_lib::utils::clean_transcript;
use taurscribe_lib::vad::assemble_speech_audio;
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

// ── File preprocessing ────────────────────────────────────────────────────────

fn prepare_file_audio(path: &Path) -> Result<Vec<f32>, String> {
    let (mut mono, sample_rate) = audio_decode::decode_audio_mono_f32(path)?;

    // Resample to 16 kHz
    if sample_rate != 16000 {
        let resampled = audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
        drop(mono);
        mono = resampled;
    }

    // Trim long edge silence
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);
    if mono.is_empty() {
        return Err("buffer empty after edge trim".into());
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let mut speech = assemble_speech_audio(&mono, Some(&cancel))?;

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
const STREAM_CHUNK_SAMPLES: usize = 16000 * 15; // 15 seconds

fn transcribe_whisper(w: &mut WhisperManager, pcm: &[f32]) -> Result<String, String> {
    let parts: Vec<String> = pcm
        .chunks(WHISPER_CHUNK_SAMPLES)
        .map(|chunk| w.transcribe_audio_data(chunk, None))
        .collect::<Result<_, _>>()?;
    Ok(clean_transcript(&parts.join(" ")))
}

fn transcribe_granite(p: &mut GgufAsrManager, pcm: &[f32]) -> Result<String, String> {
    let parts: Vec<String> = pcm
        .chunks(STREAM_CHUNK_SAMPLES)
        .map(|chunk| p.transcribe_chunk(chunk, 16000, None))
        .collect::<Result<_, _>>()?;
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
    assert!(rows.iter().all(|row| !row.ref_text.trim().is_empty()), "reference text is empty");

    let audio_root: Option<std::path::PathBuf> = std::env::var("TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT")
        .ok()
        .map(Into::into);

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
                        let pcm = match prepare_file_audio(&flac) {
                            Ok(p) => p,
                            Err(e) => {
                                panic!("[whisper] {} audio error: {e}", row.utt_id);
                            }
                        };
                        let hyp = match transcribe_whisper(&mut w, &pcm) {
                            Ok(t) => t,
                            Err(e) => {
                                panic!("[whisper] {} transcribe error: {e}", row.utt_id);
                            }
                        };
                        assert!(!hyp.trim().is_empty(), "[whisper] {} produced no transcript", row.utt_id);
                        let w_val = wer(&row.ref_text, &hyp);
                        let snippet: String = hyp.chars().take(80).collect();
                        eprintln!(
                            "[whisper] {} | wer={:.3} | ref: {} | hyp: {}",
                            row.utt_id, w_val, &row.ref_text, snippet
                        );
                        wers.push(w_val);
                    }
                }
                Err(e) => panic!("Installed Whisper model failed to initialize: {e}"),
            }
            w.unload();
        }
        Ok(_) => eprintln!("[SKIP] Whisper: no models installed"),
        Err(e) => panic!("Whisper list_models: {e}"),
    }

    // ── Granite ──────────────────────────────────────────────────────────────
    match GgufAsrManager::granite().list_available_models() {
        Ok(models) if !models.is_empty() => {
            let mut p = GgufAsrManager::granite();
            match p.initialize(None, true) {
                Ok(_) => {
                    let wers = results.entry("granite").or_default();
                    for row in &rows {
                        let flac = librispeech_wer::resolve_librispeech_flac(
                            &row.flac_path,
                            &row.utt_id,
                            audio_root.as_deref(),
                        );
                        let pcm = match prepare_file_audio(&flac) {
                            Ok(p) => p,
                            Err(e) => {
                                panic!("[granite] {} audio error: {e}", row.utt_id);
                            }
                        };
                        let hyp = match transcribe_granite(&mut p, &pcm) {
                            Ok(t) => t,
                            Err(e) => {
                                panic!("[granite] {} transcribe error: {e}", row.utt_id);
                            }
                        };
                        assert!(!hyp.trim().is_empty(), "[granite] {} produced no transcript", row.utt_id);
                        let w_val = wer(&row.ref_text, &hyp);
                        let snippet: String = hyp.chars().take(80).collect();
                        eprintln!(
                            "[granite] {} | wer={:.3} | ref: {} | hyp: {}",
                            row.utt_id, w_val, &row.ref_text, snippet
                        );
                        wers.push(w_val);
                    }
                }
                Err(e) => panic!("Installed Granite model failed to initialize: {e}"),
            }
            p.unload();
        }
        Ok(_) => eprintln!("[SKIP] Granite: no models installed"),
        Err(e) => panic!("Granite list_models: {e}"),
    }

    // ── Summary ───────────────────────────────────────────────────────────────
    eprintln!("\n=== file_drop_accuracy summary ===");
    if results.is_empty() {
        panic!("No engines produced results. Install at least one model or set TAURSCRIBE_ASR_SMOKE_SKIP=1 for an intentional skip.");
    }
    for (engine, wers) in &results {
        assert_eq!(wers.len(), rows.len(), "{engine}: every fixture must be evaluated");
        assert!(wers.iter().all(|wer| wer.is_finite()), "{engine}: invalid WER");
        let mean = wers.iter().sum::<f64>() / wers.len() as f64;
        let med = median(wers.clone());
        eprintln!(
            "[SUMMARY] {engine}: mean_wer={mean:.4} median_wer={med:.4} n={}",
            wers.len()
        );
    }
}
