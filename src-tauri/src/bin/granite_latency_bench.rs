//! Granite-only LibriSpeech timing benchmark.
//!
//! Loads one Granite bundle once, then records per-utterance decode,
//! preprocessing, transcription latency, real-time factor, and WER.

use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::cohere::CohereManager;
use taurscribe_lib::librispeech_wer;
use taurscribe_lib::utils::clean_transcript;

#[derive(Debug, Deserialize)]
struct ManifestRow {
    utt_id: String,
    flac_path: String,
    ref_text: String,
}

struct Args {
    manifest: PathBuf,
    out_csv: PathBuf,
    model_dir: Option<String>,
    limit: Option<usize>,
    audio_root: Option<PathBuf>,
}

fn usage() -> ! {
    eprintln!(
        "granite_latency_bench --manifest <path.jsonl> --out <results.csv> [--model-dir <bundle>] [--limit N] [--audio-root <test-clean-dir>]"
    );
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut manifest = None;
    let mut out_csv = None;
    let mut model_dir = None;
    let mut limit = None;
    let mut audio_root = None;
    let mut it = std::env::args().skip(1);

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--manifest" => manifest = Some(PathBuf::from(it.next().unwrap_or_else(|| usage()))),
            "--out" => out_csv = Some(PathBuf::from(it.next().unwrap_or_else(|| usage()))),
            "--model-dir" => model_dir = Some(it.next().unwrap_or_else(|| usage())),
            "--limit" => {
                limit = Some(
                    it.next()
                        .unwrap_or_else(|| usage())
                        .parse()
                        .unwrap_or_else(|_| usage()),
                );
            }
            "--audio-root" => {
                audio_root = Some(PathBuf::from(it.next().unwrap_or_else(|| usage())))
            }
            "-h" | "--help" => usage(),
            _ => usage(),
        }
    }

    Args {
        manifest: manifest.unwrap_or_else(|| usage()),
        out_csv: out_csv.unwrap_or_else(|| usage()),
        model_dir,
        limit,
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

fn pcm_for_eval(flac_path: &Path) -> Result<(Vec<f32>, f64, f64), String> {
    let decode_start = Instant::now();
    let (mut mono, sample_rate) = audio_decode::decode_audio_mono_f32(flac_path)?;
    if sample_rate != 16000 {
        let resampled = audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
        drop(mono);
        mono = resampled;
    }
    let decode_sec = decode_start.elapsed().as_secs_f64();

    let preprocess_start = Instant::now();
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);
    if mono.is_empty() {
        return Err("edge trim emptied buffer".to_string());
    }
    audio_preprocess::preprocess_assembled_speech_16k(&mut mono);
    if mono.is_empty() {
        return Err("preprocess emptied buffer".to_string());
    }
    let preprocess_sec = preprocess_start.elapsed().as_secs_f64();

    Ok((mono, decode_sec, preprocess_sec))
}

const COHERE_CHUNK_SAMPLES: usize = 16000 * 35;

fn transcribe_cohere(g: &mut CohereManager, pcm: &[f32]) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    for chunk in pcm.chunks(COHERE_CHUNK_SAMPLES) {
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

fn percentile(mut xs: Vec<f64>, p: f64) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((xs.len() - 1) as f64 * p).round() as usize;
    xs[idx.min(xs.len() - 1)]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();
    let audio_root = args.audio_root.or_else(|| {
        std::env::var("TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT")
            .ok()
            .map(PathBuf::from)
    });

    let manifest_text = std::fs::read_to_string(&args.manifest)?;
    let mut rows: Vec<ManifestRow> = manifest_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    if let Some(limit) = args.limit {
        rows.truncate(limit);
    }
    if rows.is_empty() {
        return Err("manifest has no rows".into());
    }

    eprintln!(
        "[granite-latency] loading model {}",
        args.model_dir.as_deref().unwrap_or("<default>")
    );
    let load_start = Instant::now();
    let mut granite = CohereManager::new();
    granite.initialize(args.model_dir.as_deref(), false)?;
    let load_sec = load_start.elapsed().as_secs_f64();
    eprintln!("[granite-latency] model loaded in {load_sec:.3}s");

    let mut out = std::io::BufWriter::new(std::fs::File::create(&args.out_csv)?);
    writeln!(
        out,
        "utt_id,audio_sec,decode_sec,preprocess_sec,transcribe_sec,total_sec,rtf,wer,ref_word_count,hyp_snippet"
    )?;

    let mut wers = Vec::new();
    let mut transcribe_secs = Vec::new();
    let mut rtfs = Vec::new();
    let total_start = Instant::now();

    for row in &rows {
        let flac = librispeech_wer::resolve_librispeech_flac(
            &row.flac_path,
            &row.utt_id,
            audio_root.as_deref(),
        );
        let row_start = Instant::now();
        let (pcm, decode_sec, preprocess_sec) = match pcm_for_eval(&flac) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[granite-latency] {} decode/preprocess: {}", row.utt_id, e);
                continue;
            }
        };
        let audio_sec = pcm.len() as f64 / 16000.0;

        let transcribe_start = Instant::now();
        let hyp_raw = match transcribe_cohere(&mut granite, &pcm) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[granite-latency] {} granite: {}", row.utt_id, e);
                continue;
            }
        };
        let transcribe_sec = transcribe_start.elapsed().as_secs_f64();
        let total_sec = row_start.elapsed().as_secs_f64();
        let rtf = transcribe_sec / audio_sec.max(0.001);

        let hyp = clean_transcript(&hyp_raw);
        let ref_t = librispeech_wer::normalize_for_wer(&row.ref_text);
        let hyp_t = librispeech_wer::normalize_for_wer(&hyp);
        let wer = librispeech_wer::word_error_rate(&ref_t, &hyp_t);
        let snippet: String = hyp.chars().take(120).collect();

        writeln!(
            out,
            "{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.8},{},{}",
            csv_cell(&row.utt_id),
            audio_sec,
            decode_sec,
            preprocess_sec,
            transcribe_sec,
            total_sec,
            rtf,
            wer,
            ref_t.len(),
            csv_cell(&snippet)
        )?;

        wers.push(wer);
        transcribe_secs.push(transcribe_sec);
        rtfs.push(rtf);
    }
    out.flush()?;
    granite.unload();

    let mean_wer = wers.iter().sum::<f64>() / wers.len().max(1) as f64;
    let mean_transcribe = transcribe_secs.iter().sum::<f64>() / transcribe_secs.len().max(1) as f64;
    let mean_rtf = rtfs.iter().sum::<f64>() / rtfs.len().max(1) as f64;
    eprintln!("\n=== Granite latency summary ===");
    eprintln!("rows={}", wers.len());
    eprintln!("load_sec={load_sec:.3}");
    eprintln!("wall_sec={:.3}", total_start.elapsed().as_secs_f64());
    eprintln!("wer_mean={mean_wer:.4} wer_median={:.4}", median(wers));
    eprintln!(
        "transcribe_sec mean={mean_transcribe:.4} median={:.4} p95={:.4}",
        median(transcribe_secs.clone()),
        percentile(transcribe_secs, 0.95)
    );
    eprintln!(
        "rtf mean={mean_rtf:.4} median={:.4} p95={:.4}",
        median(rtfs.clone()),
        percentile(rtfs, 0.95)
    );
    eprintln!("Wrote {}", args.out_csv.display());
    Ok(())
}
