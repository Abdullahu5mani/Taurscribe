//! Cohere backend eval smoke test.
//!
//! Runs the same JFK fixture through Cohere in three backend modes:
//! - CPU: encoder CPU + decoder CPU
//! - Hybrid: encoder CUDA + decoder CPU
//! - CUDA: encoder CUDA + decoder CUDA
//!
//! The test is ignored because it requires the Cohere ONNX bundle and, for
//! hybrid/CUDA modes, a working CUDA stack.
//!
//! Run:
//!   cargo test cohere_backend_eval -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use taurscribe_lib::cohere::CohereManager;
use taurscribe_lib::librispeech_wer;
use taurscribe_lib::utils::clean_transcript;

const JFK_REF: &str =
    "and so my fellow americans ask not what your country can do for you ask what you can do for your country";

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn resolve_jfk_wav() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("JFK_WAV") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "tests/fixtures/jfk.wav",
        "../jfk.wav",
        "../taurscribe-runtime/samples/jfk.wav",
    ] {
        let path = manifest_dir.join(rel);
        if path.is_file() {
            return Some(path);
        }
    }

    None
}

fn load_wav_mono_f32(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels.max(1) as usize;

    let mono = match spec.sample_format {
        hound::SampleFormat::Float => {
            let interleaved: Vec<f32> = reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if channels == 1 {
                interleaved
            } else {
                interleaved
                    .chunks(channels)
                    .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
                    .collect()
            }
        }
        hound::SampleFormat::Int => {
            if spec.bits_per_sample != 16 {
                return Err(format!(
                    "only 16-bit int WAV fixtures are supported, got {} bits",
                    spec.bits_per_sample
                ));
            }
            let interleaved: Vec<i16> = reader
                .samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let scale = 1.0 / 32768.0;
            if channels == 1 {
                interleaved.iter().map(|&x| x as f32 * scale).collect()
            } else {
                interleaved
                    .chunks(channels)
                    .map(|frame| {
                        frame.iter().map(|&x| x as f32).sum::<f32>() / channels as f32 * scale
                    })
                    .collect()
            }
        }
    };

    Ok((mono, sample_rate))
}

fn jfk_pcm16_preprocessed_for_asr() -> Result<Vec<f32>, String> {
    let path = resolve_jfk_wav()
        .ok_or("jfk.wav not found; set JFK_WAV or place it at repo root/tests/fixtures")?;
    let (mono, sample_rate) = load_wav_mono_f32(&path)?;
    let mut pcm = taurscribe_lib::audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
    taurscribe_lib::audio_preprocess::trim_file_buffer_edges_16k(&mut pcm);
    if pcm.is_empty() {
        return Err("edge trim emptied JFK fixture".to_string());
    }
    taurscribe_lib::audio_preprocess::preprocess_assembled_speech_16k(&mut pcm);
    if pcm.is_empty() {
        return Err("preprocess emptied JFK fixture".to_string());
    }
    Ok(pcm)
}

fn wer(ref_text: &str, hyp: &str) -> f64 {
    let ref_words = librispeech_wer::normalize_for_wer(ref_text);
    let hyp_words = librispeech_wer::normalize_for_wer(hyp);
    librispeech_wer::word_error_rate(&ref_words, &hyp_words)
}

#[test]
#[ignore = "Needs JFK fixture, Cohere model, and CUDA for hybrid/CUDA modes."]
fn cohere_backend_eval() {
    let _guard = env_lock().lock().expect("env lock poisoned");
    std::env::set_var("TAURSCRIBE_COHERE_DEBUG", "0");
    std::env::remove_var("TAURSCRIBE_COHERE_DECODER_ONNX");

    let pcm = jfk_pcm16_preprocessed_for_asr().unwrap_or_else(|e| panic!("{e}"));
    assert!(pcm.len() > 8000, "JFK fixture too short: {}", pcm.len());

    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("cohere_backend_eval.csv");
    let mut csv = String::from("mode,backend,seconds,speed_x,wer,hyp_snippet\n");
    let mut failures = Vec::new();

    for (mode, expected_backend, force_cpu) in [
        ("cpu", "CPU", true),
        ("hybrid", "Hybrid", false),
        ("cuda", "CUDA", false),
    ] {
        std::env::set_var("TAURSCRIBE_COHERE_BACKEND", mode);

        let mut manager = CohereManager::new();
        let load_msg = match manager.initialize(None, force_cpu) {
            Ok(msg) => msg,
            Err(e) => {
                failures.push(format!("{mode}: initialize failed: {e}"));
                continue;
            }
        };

        let status = manager.get_status();
        let started = Instant::now();
        let hyp_raw = match manager.transcribe_chunk(&pcm, 16000) {
            Ok(text) => text,
            Err(e) => {
                manager.unload();
                failures.push(format!("{mode}: transcribe failed: {e}"));
                continue;
            }
        };
        let elapsed = started.elapsed().as_secs_f64();
        let audio_seconds = pcm.len() as f64 / 16000.0;
        let speed_x = audio_seconds / elapsed.max(0.001);
        let hyp = clean_transcript(&hyp_raw);
        let score = wer(JFK_REF, &hyp);
        let snippet: String = hyp.chars().take(120).collect();

        eprintln!(
            "[cohere-backend-eval] mode={mode} backend={} seconds={elapsed:.3} speed={speed_x:.1}x wer={score:.3} hyp={snippet:?}",
            status.backend
        );
        csv.push_str(&format!(
            "{mode},{},{elapsed:.3},{speed_x:.3},{score:.6},\"{}\"\n",
            status.backend,
            snippet.replace('"', "\"\"")
        ));

        if status.backend != expected_backend {
            failures.push(format!(
                "{mode}: expected backend {expected_backend}, got {} ({load_msg})",
                status.backend
            ));
        }
        if score > 0.15 {
            failures.push(format!("{mode}: WER too high: {score:.3}; hyp={hyp:?}"));
        }

        manager.unload();
    }

    std::fs::create_dir_all(out_path.parent().expect("target parent")).expect("create target dir");
    std::fs::write(&out_path, csv).expect("write cohere_backend_eval.csv");
    eprintln!("[cohere-backend-eval] wrote {}", out_path.display());

    std::env::remove_var("TAURSCRIBE_COHERE_BACKEND");

    assert!(
        failures.is_empty(),
        "cohere backend eval failed:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "Needs JFK fixture and installed Granite bundle."]
fn granite_cpu_jfk_eval_once() {
    let _guard = env_lock().lock().expect("env lock poisoned");
    std::env::set_var("TAURSCRIBE_COHERE_BACKEND", "cpu");

    let pcm = jfk_pcm16_preprocessed_for_asr().unwrap_or_else(|e| panic!("{e}"));
    let mut manager = CohereManager::new();
    manager
        .initialize(Some("granite-speech-4.1-2b-nar-portable"), true)
        .unwrap_or_else(|e| panic!("granite initialize failed: {e}"));
    let started = Instant::now();
    let hyp_raw = manager
        .transcribe_chunk(&pcm, 16000)
        .unwrap_or_else(|e| panic!("granite transcribe failed: {e}"));
    let elapsed = started.elapsed().as_secs_f64();
    let hyp = clean_transcript(&hyp_raw);
    let score = wer(JFK_REF, &hyp);
    eprintln!("[granite-cpu-jfk] seconds={elapsed:.3} wer={score:.3} hyp={hyp:?}");
    manager.unload();
    std::env::remove_var("TAURSCRIBE_COHERE_BACKEND");
    assert!(
        score <= 0.15,
        "Granite WER too high: {score:.3}; hyp={hyp:?}"
    );
}

#[test]
#[ignore = "Needs JFK fixture, installed Granite bundle, and CUDA."]
fn granite_cuda_jfk_eval_once() {
    let _guard = env_lock().lock().expect("env lock poisoned");
    std::env::set_var("TAURSCRIBE_COHERE_BACKEND", "cuda");

    let pcm = jfk_pcm16_preprocessed_for_asr().unwrap_or_else(|e| panic!("{e}"));
    let mut manager = CohereManager::new();
    manager
        .initialize(Some("granite-speech-4.1-2b-nar-cuda"), false)
        .unwrap_or_else(|e| panic!("granite initialize failed: {e}"));
    let status = manager.get_status();
    assert_eq!(status.backend, "CUDA", "expected CUDA backend");
    let started = Instant::now();
    let hyp_raw = manager
        .transcribe_chunk(&pcm, 16000)
        .unwrap_or_else(|e| panic!("granite transcribe failed: {e}"));
    let elapsed = started.elapsed().as_secs_f64();
    let hyp = clean_transcript(&hyp_raw);
    let score = wer(JFK_REF, &hyp);
    eprintln!("[granite-cuda-jfk] seconds={elapsed:.3} wer={score:.3} hyp={hyp:?}");
    manager.unload();
    std::env::remove_var("TAURSCRIBE_COHERE_BACKEND");
    assert!(
        score <= 0.15,
        "Granite WER too high: {score:.3}; hyp={hyp:?}"
    );
}

#[test]
#[ignore = "Needs JFK fixture, installed Granite bundle, and Windows DirectML."]
fn granite_directml_jfk_eval_once() {
    // With a portable bundle whose manifest sets `encoder_dml_safe` (built by
    // scripts/make_granite_portable_dml.py), this runs all four graphs on
    // DirectML; older bundles fall back to the CPU-encoder hybrid.
    let _guard = env_lock().lock().expect("env lock poisoned");
    std::env::set_var("TAURSCRIBE_COHERE_BACKEND", "directml");

    let pcm = jfk_pcm16_preprocessed_for_asr().unwrap_or_else(|e| panic!("{e}"));
    let mut manager = CohereManager::new();
    manager
        .initialize(Some("granite-speech-4.1-2b-nar-portable"), false)
        .unwrap_or_else(|e| panic!("granite initialize failed: {e}"));
    let status = manager.get_status();
    assert_eq!(
        status.backend, "DirectML",
        "expected initial DirectML backend"
    );
    let started = Instant::now();
    let hyp_raw = manager
        .transcribe_chunk(&pcm, 16000)
        .unwrap_or_else(|e| panic!("granite transcribe failed: {e}"));
    let elapsed = started.elapsed().as_secs_f64();
    let hyp = clean_transcript(&hyp_raw);
    let final_status = manager.get_status();
    let score = wer(JFK_REF, &hyp);
    eprintln!(
        "[granite-directml-jfk] initial_backend={} final_backend={} seconds={elapsed:.3} wer={score:.3} hyp={hyp:?}",
        status.backend,
        final_status.backend
    );
    manager.unload();
    std::env::remove_var("TAURSCRIBE_COHERE_BACKEND");
    assert!(
        score <= 0.15,
        "Granite WER too high: {score:.3}; hyp={hyp:?}"
    );
}
