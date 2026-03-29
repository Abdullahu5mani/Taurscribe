//! Offline LibriSpeech eval: same audio chain as `test_fixtures::jfk_pcm16_preprocessed_for_asr`
//! (decode → mono 16 kHz → `trim_file_buffer_edges_16k` → `preprocess_assembled_speech_16k`).
//! Does **not** use file-drop VAD assembly (short read utterances = one contiguous clip).
//!
//! Models: `%LOCALAPPDATA%\\Taurscribe\\models` (or platform equivalent). Optional env:
//! `TAURSCRIBE_WHISPER_MODEL_ID`, `TAURSCRIBE_PARAKEET_MODEL_ID`, `TAURSCRIBE_GRANITE_MODEL_ID`.
//!
//! If manifest `flac_path` entries point at another machine (or a moved corpus), set
//! `--audio-root` or `TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT` to the **`test-clean` directory**
//! (the folder that contains per-reader subdirs like `908/`). Paths are then rebuilt from `utt_id`.
//!
//! Usage:
//!   cargo run --release --bin librispeech_eval -- --manifest eval_manifest.jsonl --out results.csv
//!   cargo run --release --bin librispeech_eval -- --manifest m.jsonl --audio-root ../taurscribe-runtime/librispeech/LibriSpeech/test-clean --engines whisper,granite --limit 50 --force-cpu

use serde::Deserialize;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::granite_speech::GraniteSpeechManager;
use taurscribe_lib::librispeech_wer;
use taurscribe_lib::parakeet::ParakeetManager;
use taurscribe_lib::utils::clean_transcript;
use taurscribe_lib::whisper::WhisperManager;

#[derive(Debug, Deserialize)]
struct ManifestRow {
    utt_id: String,
    flac_path: String,
    ref_text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Engine {
    Whisper,
    Parakeet,
    Granite,
}

impl Engine {
    fn as_str(self) -> &'static str {
        match self {
            Engine::Whisper => "whisper",
            Engine::Parakeet => "parakeet",
            Engine::Granite => "granite",
        }
    }
}

struct Args {
    manifest: String,
    out_csv: String,
    engines: Vec<Engine>,
    limit: Option<usize>,
    force_cpu: bool,
    audio_root: Option<PathBuf>,
}

fn usage() -> ! {
    eprintln!(
        "librispeech_eval --manifest <path.jsonl> [--out results.csv] [--engines whisper,parakeet,granite] [--limit N] [--audio-root <test-clean-dir>] [--force-cpu]"
    );
    eprintln!("Env: TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT (same as --audio-root if flag omitted)");
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut manifest: Option<String> = None;
    let mut out_csv = "librispeech_results.csv".to_string();
    let mut engines_str: Option<String> = None;
    let mut limit: Option<usize> = None;
    let mut force_cpu = false;
    let mut audio_root: Option<PathBuf> = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--manifest" => manifest = Some(it.next().unwrap_or_else(|| usage())),
            "--out" => out_csv = it.next().unwrap_or_else(|| usage()),
            "--engines" => engines_str = Some(it.next().unwrap_or_else(|| usage())),
            "--limit" => {
                limit = Some(
                    it.next()
                        .unwrap_or_else(|| usage())
                        .parse()
                        .unwrap_or_else(|_| usage()),
                );
            }
            "--audio-root" => {
                audio_root = Some(PathBuf::from(it.next().unwrap_or_else(|| usage())));
            }
            "--force-cpu" => force_cpu = true,
            "-h" | "--help" => usage(),
            _ => usage(),
        }
    }
    let engines = if let Some(s) = engines_str {
        let mut v = Vec::new();
        let mut seen = HashSet::new();
        for part in s.split(',') {
            let p = part.trim().to_lowercase();
            let e = match p.as_str() {
                "whisper" => Engine::Whisper,
                "parakeet" => Engine::Parakeet,
                "granite" => Engine::Granite,
                _ => usage(),
            };
            if seen.insert(e) {
                v.push(e);
            }
        }
        if v.is_empty() {
            usage();
        }
        v
    } else {
        vec![Engine::Whisper, Engine::Parakeet, Engine::Granite]
    };
    Args {
        manifest: manifest.unwrap_or_else(|| usage()),
        out_csv,
        engines,
        limit,
        force_cpu,
        audio_root,
    }
}

fn csv_cell(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Eval contract: matches `jfk_pcm16_preprocessed_for_asr` (no VAD).
fn pcm_for_eval(flac_path: &Path) -> Result<Vec<f32>, String> {
    let (raw, sample_rate, channels) = audio_decode::decode_audio_interleaved_f32(flac_path)?;
    let mut mono = if channels > 1 {
        let ch = channels as usize;
        raw.chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect::<Vec<f32>>()
    } else {
        raw
    };
    if sample_rate != 16000 {
        mono = audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
    }
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);
    if mono.is_empty() {
        return Err("edge trim emptied buffer".to_string());
    }
    audio_preprocess::preprocess_assembled_speech_16k(&mut mono);
    if mono.is_empty() {
        return Err("preprocess emptied buffer".to_string());
    }
    Ok(mono)
}

const WHISPER_CHUNK_SAMPLES: usize = 16000 * 180;
const STREAM_CHUNK_SAMPLES: usize = 16000 * 15;

fn transcribe_whisper(w: &mut WhisperManager, pcm: &[f32]) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    for chunk in pcm.chunks(WHISPER_CHUNK_SAMPLES) {
        let t = w.transcribe_audio_data(chunk, None)?;
        if !t.trim().is_empty() {
            parts.push(t.trim().to_string());
        }
    }
    Ok(parts.join(" "))
}

fn transcribe_parakeet(p: &mut ParakeetManager, pcm: &[f32]) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    for chunk in pcm.chunks(STREAM_CHUNK_SAMPLES) {
        let t = p.transcribe_chunk(chunk, 16000)?;
        if !t.trim().is_empty() {
            parts.push(t.trim().to_string());
        }
    }
    Ok(parts.join(" "))
}

fn transcribe_granite(g: &mut GraniteSpeechManager, pcm: &[f32]) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    for chunk in pcm.chunks(STREAM_CHUNK_SAMPLES) {
        let t = g.transcribe_chunk(chunk, 16000)?;
        if !t.trim().is_empty() {
            parts.push(t.trim().to_string());
        }
    }
    Ok(parts.join(" "))
}

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();
    let audio_root = args
        .audio_root
        .clone()
        .or_else(|| {
            std::env::var("TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT")
                .ok()
                .map(PathBuf::from)
        });
    let manifest_path = Path::new(&args.manifest);
    let text = std::fs::read_to_string(manifest_path)?;
    let mut rows: Vec<ManifestRow> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(line)?);
    }
    if let Some(n) = args.limit {
        rows.truncate(n);
    }
    if rows.is_empty() {
        return Err("manifest has no rows".into());
    }

    let mut out = std::io::BufWriter::new(std::fs::File::create(&args.out_csv)?);
    writeln!(
        out,
        "utt_id,engine,wer,ref_word_count,hyp_snippet"
    )?;

    let force = args.force_cpu;
    let mut summary: Vec<(Engine, Vec<f64>)> = args
        .engines
        .iter()
        .map(|e| (*e, Vec::new()))
        .collect();

    for eng in &args.engines {
        eprintln!("[eval] loading {} ...", eng.as_str());
        match *eng {
            Engine::Whisper => {
                let whisper_id = std::env::var("TAURSCRIBE_WHISPER_MODEL_ID").ok();
                let models = WhisperManager::list_available_models()?;
                if models.is_empty() {
                    return Err("Whisper: no ggml models in models dir".into());
                }
                let id = whisper_id
                    .as_deref()
                    .filter(|id| models.iter().any(|m| m.id == *id))
                    .unwrap_or(models[0].id.as_str());
                let mut w = WhisperManager::new();
                w.initialize(Some(id), force)?;
                for row in &rows {
                    let flac = librispeech_wer::resolve_librispeech_flac(
                        &row.flac_path,
                        &row.utt_id,
                        audio_root.as_deref(),
                    );
                    let pcm = match pcm_for_eval(&flac) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("[eval] {} decode/preprocess: {}", row.utt_id, e);
                            continue;
                        }
                    };
                    let hyp_raw = match transcribe_whisper(&mut w, &pcm) {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("[eval] {} whisper: {}", row.utt_id, e);
                            continue;
                        }
                    };
                    let hyp = clean_transcript(&hyp_raw);
                    let ref_t = librispeech_wer::normalize_for_wer(&row.ref_text);
                    let hyp_t = librispeech_wer::normalize_for_wer(&hyp);
                    let wer = librispeech_wer::word_error_rate(&ref_t, &hyp_t);
                    let snippet: String = hyp.chars().take(120).collect();
                    writeln!(
                        out,
                        "{},{},{},{},{}",
                        csv_cell(&row.utt_id),
                        eng.as_str(),
                        wer,
                        ref_t.len(),
                        csv_cell(&snippet)
                    )?;
                    if let Some((_, v)) = summary.iter_mut().find(|(e, _)| *e == Engine::Whisper) {
                        v.push(wer);
                    }
                }
                w.unload();
            }
            Engine::Parakeet => {
                let parakeet_id = std::env::var("TAURSCRIBE_PARAKEET_MODEL_ID").ok();
                let models = ParakeetManager::list_available_models()?;
                if models.is_empty() {
                    return Err("Parakeet: no ONNX bundle in models dir".into());
                }
                let id = parakeet_id
                    .as_deref()
                    .filter(|id| models.iter().any(|m| m.id == *id));
                let mut p = ParakeetManager::new();
                p.initialize(id, force)?;
                for row in &rows {
                    let flac = librispeech_wer::resolve_librispeech_flac(
                        &row.flac_path,
                        &row.utt_id,
                        audio_root.as_deref(),
                    );
                    let pcm = match pcm_for_eval(&flac) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("[eval] {} decode/preprocess: {}", row.utt_id, e);
                            continue;
                        }
                    };
                    let hyp_raw = match transcribe_parakeet(&mut p, &pcm) {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("[eval] {} parakeet: {}", row.utt_id, e);
                            continue;
                        }
                    };
                    let hyp = clean_transcript(&hyp_raw);
                    let ref_t = librispeech_wer::normalize_for_wer(&row.ref_text);
                    let hyp_t = librispeech_wer::normalize_for_wer(&hyp);
                    let wer = librispeech_wer::word_error_rate(&ref_t, &hyp_t);
                    let snippet: String = hyp.chars().take(120).collect();
                    writeln!(
                        out,
                        "{},{},{},{},{}",
                        csv_cell(&row.utt_id),
                        eng.as_str(),
                        wer,
                        ref_t.len(),
                        csv_cell(&snippet)
                    )?;
                    if let Some((_, v)) = summary.iter_mut().find(|(e, _)| *e == Engine::Parakeet) {
                        v.push(wer);
                    }
                }
                p.unload();
            }
            Engine::Granite => {
                let granite_id = std::env::var("TAURSCRIBE_GRANITE_MODEL_ID").ok();
                let id = granite_id.as_deref();
                let mut g = GraniteSpeechManager::new();
                g.initialize(id, force)?;
                for row in &rows {
                    let flac = librispeech_wer::resolve_librispeech_flac(
                        &row.flac_path,
                        &row.utt_id,
                        audio_root.as_deref(),
                    );
                    let pcm = match pcm_for_eval(&flac) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("[eval] {} decode/preprocess: {}", row.utt_id, e);
                            continue;
                        }
                    };
                    let hyp_raw = match transcribe_granite(&mut g, &pcm) {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("[eval] {} granite: {}", row.utt_id, e);
                            continue;
                        }
                    };
                    let hyp = clean_transcript(&hyp_raw);
                    let ref_t = librispeech_wer::normalize_for_wer(&row.ref_text);
                    let hyp_t = librispeech_wer::normalize_for_wer(&hyp);
                    let wer = librispeech_wer::word_error_rate(&ref_t, &hyp_t);
                    let snippet: String = hyp.chars().take(120).collect();
                    writeln!(
                        out,
                        "{},{},{},{},{}",
                        csv_cell(&row.utt_id),
                        eng.as_str(),
                        wer,
                        ref_t.len(),
                        csv_cell(&snippet)
                    )?;
                    if let Some((_, v)) = summary.iter_mut().find(|(e, _)| *e == Engine::Granite) {
                        v.push(wer);
                    }
                }
                g.unload();
            }
        }
        out.flush()?;
    }

    eprintln!("\n=== Summary (mean / median WER, N utterances) ===");
    for (e, wers) in &summary {
        if wers.is_empty() {
            eprintln!("{}: no successful utterances", e.as_str());
            continue;
        }
        let mean = wers.iter().sum::<f64>() / wers.len() as f64;
        let med = median(wers.clone());
        eprintln!(
            "{}: mean={:.4} median={:.4} n={}",
            e.as_str(),
            mean,
            med,
            wers.len()
        );
    }
    eprintln!("Wrote {}", args.out_csv);
    Ok(())
}
