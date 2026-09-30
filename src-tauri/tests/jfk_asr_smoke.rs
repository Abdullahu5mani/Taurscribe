//! Fixed-fixture accuracy tests: preprocessed `jfk.wav` through installed engines.
//!
//! Requires:
//! - The checked-in `taurscribe-runtime/samples/jfk.wav` (or `JFK_WAV` override)
//! - **Whisper**: at least one installed ggml model in Taurscribe's models dir
//! - **Granite**: an installed Granite GGUF bundle in the same models dir
//! - **Qwen3**: an installed Qwen3-ASR GGUF bundle for its separate test
//!
//! The smoke test is `#[ignore]` by default. Run with:
//!   cargo test jfk_audio_through_whisper_and_granite -- --ignored --nocapture
//!
//! An explicitly requested run fails if a requested engine or the fixture is missing.

use std::path::{Path, PathBuf};
use taurscribe_lib::gguf_asr::GgufAsrManager;
use taurscribe_lib::whisper::WhisperManager;

#[path = "support/asr_reference.rs"]
mod asr_reference;
use asr_reference::{check_transcript, word_error_rate, JFK_REFERENCE};

#[test]
fn transcript_metric_ignores_case_and_punctuation_but_penalizes_missing_words() {
    assert_eq!(word_error_rate("Ask not, Americans!", "ASK NOT AMERICANS."), 0.0);
    assert_eq!(word_error_rate("a b c d", "a c d"), 0.25);
    assert_eq!(word_error_rate("a b c d", "a b changed d extra"), 0.5);
}

#[test]
fn unrelated_nonempty_output_cannot_pass_the_asr_smoke_test() {
    assert!(check_transcript("test", "hello world").is_err());
    assert!(check_transcript("test", "").is_err());
    assert!(check_transcript("test", JFK_REFERENCE).is_ok());
}

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// Resolves `jfk.wav`: checks `JFK_WAV` env, then `tests/fixtures/jfk.wav`, then `../jfk.wav`.
fn resolve_jfk_wav() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("JFK_WAV") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let m = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for rel in ["../taurscribe-runtime/samples/jfk.wav", "tests/fixtures/jfk.wav", "../jfk.wav"] {
        let p = m.join(rel);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn load_wav_mono_f32(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let rate = spec.sample_rate;
    let ch = spec.channels.max(1) as usize;

    let mono: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            let interleaved: Vec<f32> = reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if ch <= 1 {
                interleaved
            } else {
                interleaved
                    .chunks(ch)
                    .map(|fr| fr.iter().sum::<f32>() / ch as f32)
                    .collect()
            }
        }
        hound::SampleFormat::Int => {
            if spec.bits_per_sample != 16 {
                return Err(format!(
                    "test helper supports only 16-bit int WAV, got {} bits",
                    spec.bits_per_sample
                ));
            }
            let interleaved: Vec<i16> = reader
                .samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let g = 1.0 / 32768.0;
            if ch <= 1 {
                interleaved.iter().map(|&x| x as f32 * g).collect()
            } else {
                interleaved
                    .chunks(ch)
                    .map(|fr| fr.iter().map(|&x| x as f32).sum::<f32>() / ch as f32 * g)
                    .collect()
            }
        }
    };

    Ok((mono, rate))
}

/// Loads jfk.wav and runs the full pre-processing chain (resample → trim → preprocess).
fn jfk_pcm16_preprocessed_for_asr() -> Result<Vec<f32>, String> {
    let path = resolve_jfk_wav()
        .ok_or("jfk.wav not found (tests/fixtures/jfk.wav, ../jfk.wav, or JFK_WAV)")?;
    let (mono, rate) = load_wav_mono_f32(&path)?;
    if mono.is_empty() {
        return Err("jfk.wav has no samples".into());
    }
    let mut pcm16 = taurscribe_lib::audio_preprocess::resample_mono_to_16k(&mono, rate)?;
    taurscribe_lib::audio_preprocess::trim_file_buffer_edges_16k(&mut pcm16);
    if pcm16.is_empty() {
        return Err("edge trim emptied jfk buffer".into());
    }
    taurscribe_lib::audio_preprocess::preprocess_assembled_speech_16k(&mut pcm16);
    if pcm16.is_empty() {
        return Err("preprocess emptied jfk buffer".into());
    }
    Ok(pcm16)
}

// ── Smoke test ────────────────────────────────────────────────────────────────

#[test]
#[ignore = "Needs an installed Qwen3-ASR model. Run with --ignored --nocapture."]
fn jfk_audio_through_qwen3() {
    let pcm = jfk_pcm16_preprocessed_for_asr().unwrap_or_else(|e| panic!("{e}"));
    let mut manager = GgufAsrManager::qwen3();
    let models = manager.list_available_models().expect("list installed Qwen3 models");
    let id = &models.first().expect("install a Qwen3-ASR model before explicitly running this test").id;
    manager.initialize(Some(id), false).expect("initialize Qwen3");
    eprintln!("Qwen3 model: {id}; backend: {}", manager.get_status().backend);
    let result = manager.transcribe_chunk(&pcm, 16000, None);
    manager.unload();
    assert!(!manager.get_status().loaded);
    check_transcript("Qwen3", &result.expect("transcribe Qwen3 fixture")).unwrap();
}

#[test]
#[ignore = "Needs installed Whisper and Granite models. Run with --ignored --nocapture."]
fn jfk_audio_through_whisper_and_granite() {
    let pcm = jfk_pcm16_preprocessed_for_asr().unwrap_or_else(|e| panic!("{e}"));
    assert!(
        pcm.len() > 8000,
        "jfk buffer too short: {} samples",
        pcm.len()
    );

    let mut failures: Vec<String> = Vec::new();

    // ── Whisper ───────────────────────────────────────────────────────────────
    match WhisperManager::list_available_models() {
        Ok(models) if !models.is_empty() => {
            let id = models[0].id.clone();
            eprintln!("Whisper model: {id}");
            let mut w = WhisperManager::new();
            match w.initialize(Some(id.as_str()), true) {
                Ok(_) => match w.transcribe_audio_data(&pcm, None) {
                    Ok(text) => if let Err(e) = check_transcript("Whisper", &text) { failures.push(e); },
                    Err(e) => failures.push(format!("Whisper transcribe: {e}")),
                },
                Err(e) => failures.push(format!("Whisper init ({id}): {e}")),
            }
            w.unload();
        }
        Ok(_) => {
            failures.push("Whisper: no ggml-*.bin in models dir (download a Whisper model)".into())
        }
        Err(e) => failures.push(format!("Whisper list_models: {e}")),
    }

    // ── Granite ──────────────────────────────────────────────────────────────
    match GgufAsrManager::granite().list_available_models() {
        Ok(models) if !models.is_empty() => {
            let id = models[0].id.clone();
            eprintln!("Granite model: {id}");
            let mut p = GgufAsrManager::granite();
            match p.initialize(Some(id.as_str()), true) {
                Ok(_) => match p.transcribe_chunk(&pcm, 16000, None) {
                    Ok(text) => if let Err(e) = check_transcript("Granite", &text) { failures.push(e); },
                    Err(e) => failures.push(format!("Granite transcribe: {e}")),
                },
                Err(e) => failures.push(format!("Granite init: {e}")),
            }
            p.unload();
        }
        Ok(_) => failures
            .push("Granite: no GGUF bundle in models dir (download Granite Speech 5)".into()),
        Err(e) => failures.push(format!("Granite list_models: {e}")),
    }

    assert!(
        failures.is_empty(),
        "jfk two-engine smoke failed:\n{}",
        failures.join("\n")
    );
}
