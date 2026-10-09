//! Time from "stop" to final text: today's full final pass vs. committing
//! pause-cut segments while recording and transcribing only the tail at stop.
//!
//! Builds long "recordings" by joining LibriSpeech test-clean utterances of one
//! speaker (300 ms gaps), so both strategies can also be scored for WER.
//!
//! Usage:
//!   cargo run --release --features dev-tools --bin stop_latency_bench -- \
//!       --corpus ../taurscribe-runtime/librispeech/LibriSpeech/test-clean \
//!       [--whisper small.en-q5_1] [--granite] [--lengths 15,30,60,120,240] [--runs 3]

use std::path::{Path, PathBuf};
use std::time::Instant;

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::gguf_asr::GgufAsrManager;
use taurscribe_lib::librispeech_wer::{normalize_for_wer, word_error_rate};
use taurscribe_lib::utils::clean_transcript;
use taurscribe_lib::vad::VADManager;
use taurscribe_lib::whisper::WhisperManager;

const SR: usize = 16_000;
const GAP_SAMPLES: usize = SR * 3 / 10;
/// Commit a segment at the first pause once it is at least this long...
const MIN_COMMIT: usize = SR * 3;
/// ...or force a cut here when the speaker never pauses.
const MAX_COMMIT: usize = SR * 12;
/// A pause is this much low-energy audio in a row.
const PAUSE_SAMPLES: usize = SR * 3 / 10;
const FRAME: usize = 800;

struct Recording {
    pcm: Vec<f32>,
    reference: String,
}

/// Joins utterances of one speaker until `secs` of audio.
fn build_recording(corpus: &Path, secs: usize) -> Recording {
    let target = secs * SR;
    let mut pcm = Vec::with_capacity(target + SR * 30);
    let mut reference = String::new();
    let mut speakers: Vec<PathBuf> = std::fs::read_dir(corpus).expect("corpus").flatten().map(|e| e.path()).collect();
    speakers.sort();
    'outer: for speaker in speakers {
        let mut chapters: Vec<PathBuf> = std::fs::read_dir(&speaker).into_iter().flatten().flatten().map(|e| e.path()).collect();
        chapters.sort();
        for chapter in chapters {
            let name = chapter.file_name().unwrap().to_string_lossy().to_string();
            let spk = speaker.file_name().unwrap().to_string_lossy().to_string();
            let trans = chapter.join(format!("{spk}-{name}.trans.txt"));
            let Ok(text) = std::fs::read_to_string(&trans) else { continue };
            for line in text.lines() {
                let Some((id, words)) = line.split_once(' ') else { continue };
                let (mono, rate) = audio_decode::decode_audio_mono_f32(&chapter.join(format!("{id}.flac"))).expect("flac");
                let mono = if rate as usize == SR { mono } else { audio_preprocess::resample_mono_to_16k(&mono, rate).unwrap() };
                pcm.extend_from_slice(&mono);
                pcm.extend(std::iter::repeat(0.0).take(GAP_SAMPLES));
                reference.push_str(words);
                reference.push(' ');
                if pcm.len() >= target {
                    break 'outer;
                }
            }
        }
    }
    Recording { pcm, reference }
}

/// Cuts the recording the way a live committer would: walking forward in
/// 50 ms frames, it commits at the first pause after MIN_COMMIT, or at MAX_COMMIT.
/// Returns (committed segments, uncommitted tail at stop).
fn commit_segments(pcm: &[f32]) -> (Vec<&[f32]>, &[f32]) {
    let mut vad = VADManager::new().unwrap();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quiet = 0;
    let mut pos = 0;
    while pos + FRAME <= pcm.len() {
        let speech = vad.is_speech(&pcm[pos..pos + FRAME]).unwrap() > 0.2;
        pos += FRAME;
        quiet = if speech { 0 } else { quiet + FRAME };
        let len = pos - start;
        if (len >= MIN_COMMIT && quiet >= PAUSE_SAMPLES) || len >= MAX_COMMIT {
            segments.push(&pcm[start..pos]);
            start = pos;
            quiet = 0;
        }
    }
    (segments, &pcm[start..])
}

fn wer(reference: &str, hyp: &str) -> f64 {
    word_error_rate(&normalize_for_wer(reference), &normalize_for_wer(hyp))
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Today's Whisper stop path (recording.rs): pad, preprocess, VAD, one full decode.
fn whisper_old(w: &mut WhisperManager, pcm: &[f32]) -> String {
    let mut audio = pcm.to_vec();
    audio.extend(std::iter::repeat(0.0_f32).take(SR * 400 / 1000));
    audio_preprocess::preprocess_assembled_speech_16k(&mut audio);
    let mut vad = VADManager::new().unwrap();
    let (pad, thr) = if audio.len() < SR * 4 { (800, 0.2) } else { (500, 0.35) };
    let ts = vad.get_speech_timestamps_hysteresis(&audio, pad, thr, thr * 0.5).unwrap();
    let mut clean = Vec::with_capacity(audio.len());
    if ts.is_empty() {
        clean.extend_from_slice(&audio);
    }
    for (s, e) in ts {
        let (s, e) = ((s * SR as f32) as usize, (e * SR as f32) as usize);
        clean.extend_from_slice(&audio[s.min(audio.len())..e.min(audio.len())]);
    }
    clean_transcript(&w.transcribe_audio_data(&clean, None).expect("whisper"))
}

/// The live recorder's gate (recording.rs vad_gated_transcribe): skip silent chunks.
fn has_speech(pcm16: &[f32]) -> bool {
    VADManager::new().unwrap().max_speech_prob(pcm16, usize::MAX) > 0.25
}

fn whisper_chunk(w: &mut WhisperManager, seg: &[f32]) -> String {
    let pcm16 = audio_preprocess::preprocess_live_transcribe_chunk(seg, SR as u32, false, None);
    if !has_speech(&pcm16) {
        return String::new();
    }
    w.transcribe_chunk(&pcm16, SR as u32).expect("whisper chunk")
}

fn gguf_chunk(g: &mut GgufAsrManager, seg: &[f32]) -> String {
    let pcm16 = audio_preprocess::preprocess_live_transcribe_chunk(seg, SR as u32, false, None);
    if !has_speech(&pcm16) {
        return String::new();
    }
    g.transcribe_chunk(&pcm16, SR as u32, None).expect("gguf chunk")
}

struct Row {
    secs: f64,
    old_ms: f64,
    new_ms: f64,
    tail_s: f64,
    live_rtf: f64,
    old_wer: f64,
    new_wer: f64,
}

/// `old` runs the whole-recording pass; `chunk` transcribes one live segment.
fn measure(
    rec: &Recording,
    runs: usize,
    old: &mut dyn FnMut(&[f32]) -> String,
    chunk: &mut dyn FnMut(&[f32]) -> String,
    reset: &mut dyn FnMut(),
) -> Row {
    let (segments, tail) = commit_segments(&rec.pcm);
    let (mut old_t, mut new_t, mut live_t) = (vec![], vec![], vec![]);
    let (mut old_text, mut new_text) = (String::new(), String::new());
    for _ in 0..runs {
        let t = Instant::now();
        old_text = old(&rec.pcm);
        old_t.push(t.elapsed().as_secs_f64() * 1e3);

        reset();
        let mut text = String::new();
        let t = Instant::now();
        for seg in &segments {
            text.push_str(chunk(seg).trim());
            text.push(' ');
        }
        live_t.push(t.elapsed().as_secs_f64());
        // Stop: only the uncommitted tail is left to transcribe.
        let t = Instant::now();
        if tail.len() > SR / 10 {
            text.push_str(chunk(tail).trim());
        }
        new_t.push(t.elapsed().as_secs_f64() * 1e3);
        new_text = clean_transcript(&text);
    }
    if std::env::var("BENCH_DEBUG").is_ok() {
        println!("REF: {}\nOLD: {}\nNEW: {}", rec.reference, old_text, new_text);
    }
    let secs = rec.pcm.len() as f64 / SR as f64;
    let committed_secs = secs - tail.len() as f64 / SR as f64;
    Row {
        secs,
        old_ms: median(old_t),
        new_ms: median(new_t),
        tail_s: tail.len() as f64 / SR as f64,
        live_rtf: committed_secs / median(live_t),
        old_wer: wer(&rec.reference, &old_text),
        new_wer: wer(&rec.reference, &new_text),
    }
}

fn print_rows(label: &str, rows: &[Row]) {
    println!("\n== {label} ==");
    println!("{:>7} | {:>10} | {:>10} | {:>7} | {:>6} | {:>13} | {:>7} | {:>7}", "audio", "old stop", "new stop", "speedup", "tail", "live speed", "old WER", "new WER");
    for r in rows {
        println!(
            "{:>6.0}s | {:>8.0}ms | {:>8.0}ms | {:>6.1}x | {:>5.1}s | {:>11.0}x RT | {:>6.1}% | {:>6.1}%",
            r.secs, r.old_ms, r.new_ms, r.old_ms / r.new_ms.max(1.0), r.tail_s, r.live_rtf, r.old_wer * 100.0, r.new_wer * 100.0
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let corpus = PathBuf::from(arg("--corpus").expect("--corpus <test-clean dir>"));
    let runs: usize = arg("--runs").and_then(|v| v.parse().ok()).unwrap_or(3);
    let lengths: Vec<usize> = arg("--lengths").unwrap_or("15,30,60,120,240".into()).split(',').filter_map(|s| s.parse().ok()).collect();
    let recordings: Vec<Recording> = lengths.iter().map(|&s| build_recording(&corpus, s)).collect();

    if let Some(model) = arg("--whisper") {
        let w = std::cell::RefCell::new(WhisperManager::new());
        let backend = w.borrow_mut().initialize(Some(&model), args.iter().any(|a| a == "--cpu")).expect("whisper init");
        println!("[bench] whisper {model}: {backend}");
        // Warm-up: first decode compiles GPU kernels.
        whisper_chunk(&mut w.borrow_mut(), &recordings[0].pcm[..SR * 5]);
        let rows: Vec<Row> = recordings
            .iter()
            .map(|rec| {
                measure(
                    rec,
                    runs,
                    &mut |pcm| whisper_old(&mut w.borrow_mut(), pcm),
                    &mut |seg| whisper_chunk(&mut w.borrow_mut(), seg),
                    &mut || w.borrow_mut().clear_context(),
                )
            })
            .collect();
        print_rows(&format!("Whisper {model} ({backend})"), &rows);
    }

    if args.iter().any(|a| a == "--granite") {
        let g = std::cell::RefCell::new(GgufAsrManager::granite());
        let backend = g.borrow_mut().initialize(None, args.iter().any(|a| a == "--cpu")).expect("granite init");
        println!("[bench] granite: {backend}");
        gguf_chunk(&mut g.borrow_mut(), &recordings[0].pcm[..SR * 5]);
        let rows: Vec<Row> = recordings
            .iter()
            .map(|rec| {
                // Today's Granite stop path: one pass over the whole saved recording
                // (recording.rs transcribe_final_pass; whole file up to 4 minutes).
                measure(
                    rec,
                    runs,
                    &mut |pcm| {
                        let mut audio = pcm.to_vec();
                        audio.extend(std::iter::repeat(0.0_f32).take(SR * 400 / 1000));
                        audio_preprocess::preprocess_assembled_speech_16k(&mut audio);
                        clean_transcript(&g.borrow_mut().transcribe_chunk(&audio, SR as u32, None).expect("granite"))
                    },
                    &mut |seg| gguf_chunk(&mut g.borrow_mut(), seg),
                    &mut || {},
                )
            })
            .collect();
        print_rows(&format!("Granite Speech 5 ({backend})"), &rows);
    }
}
