//! Integration smoke test: runs preprocessed `jfk.wav` through Whisper, Parakeet, and Granite.
//!
//! Requires:
//! - `jfk.wav` at `tests/fixtures/jfk.wav`, repo root, or `JFK_WAV` env var
//! - **Whisper**: at least one `ggml-*.bin` in `%LOCALAPPDATA%\Taurscribe\models`
//! - **Parakeet**: a detected ONNX bundle under the same models dir
//! - **Granite**: INT4 (or compatible) Granite Speech bundle — FP16-only + `force_cpu` will fail by design
//!
//! The smoke test is `#[ignore]` by default. Run with:
//!   cargo test jfk_audio_through_whisper_parakeet_and_granite -- --ignored --nocapture
//!
//! Set `TAURSCRIBE_ASR_SMOKE_SKIP=1` to no-op pass when models are absent.

use std::path::{Path, PathBuf};
use taurscribe_lib::granite_speech::GraniteSpeechManager;
use taurscribe_lib::parakeet::ParakeetManager;
use taurscribe_lib::whisper::WhisperManager;

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
    for rel in ["tests/fixtures/jfk.wav", "../jfk.wav"] {
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
#[ignore = "Needs jfk.wav + Whisper, Parakeet, Granite in %LOCALAPPDATA%/Taurscribe/models. Run with --ignored."]
fn jfk_audio_through_whisper_parakeet_and_granite() {
    if std::env::var("TAURSCRIBE_ASR_SMOKE_SKIP").as_deref() == Ok("1") {
        eprintln!("SKIP jfk ASR smoke (TAURSCRIBE_ASR_SMOKE_SKIP=1)");
        return;
    }

    let pcm = jfk_pcm16_preprocessed_for_asr().unwrap_or_else(|e| panic!("{e}"));
    assert!(pcm.len() > 8000, "jfk buffer too short: {} samples", pcm.len());

    let mut failures: Vec<String> = Vec::new();

    // ── Whisper ───────────────────────────────────────────────────────────────
    match WhisperManager::list_available_models() {
        Ok(models) if !models.is_empty() => {
            let id = models[0].id.clone();
            let mut w = WhisperManager::new();
            match w.initialize(Some(id.as_str()), true) {
                Ok(_) => match w.transcribe_audio_data(&pcm, None) {
                    Ok(text) if text.trim().is_empty() => {
                        failures.push("Whisper: empty transcript".into())
                    }
                    Ok(_) => {}
                    Err(e) => failures.push(format!("Whisper transcribe: {e}")),
                },
                Err(e) => failures.push(format!("Whisper init ({id}): {e}")),
            }
            w.unload();
        }
        Ok(_) => failures.push("Whisper: no ggml-*.bin in models dir (download a Whisper model)".into()),
        Err(e) => failures.push(format!("Whisper list_models: {e}")),
    }

    // ── Parakeet ──────────────────────────────────────────────────────────────
    match ParakeetManager::list_available_models() {
        Ok(models) if !models.is_empty() => {
            let mut p = ParakeetManager::new();
            match p.initialize(None, true) {
                Ok(_) => match p.transcribe_chunk(&pcm, 16000) {
                    Ok(text) if text.trim().is_empty() => {
                        failures.push("Parakeet: empty transcript".into())
                    }
                    Ok(_) => {}
                    Err(e) => failures.push(format!("Parakeet transcribe: {e}")),
                },
                Err(e) => failures.push(format!("Parakeet init: {e}")),
            }
            p.unload();
        }
        Ok(_) => failures.push("Parakeet: no ONNX bundle in models dir (download Parakeet/Nemotron)".into()),
        Err(e) => failures.push(format!("Parakeet list_models: {e}")),
    }

    // ── Granite Speech ────────────────────────────────────────────────────────
    let mut g = GraniteSpeechManager::new();
    match g.initialize(None, true) {
        Ok(_) => match g.transcribe_chunk(&pcm, 16000) {
            Ok(text) if text.trim().is_empty() => failures.push("Granite: empty transcript".into()),
            Ok(_) => {}
            Err(e) => failures.push(format!("Granite transcribe: {e}")),
        },
        Err(e) => failures.push(format!(
            "Granite init: {e} (need INT4 Granite bundle; FP16 is GPU-only on Windows)"
        )),
    }
    g.unload();

    assert!(failures.is_empty(), "jfk three-engine smoke failed:\n{}", failures.join("\n"));
}
