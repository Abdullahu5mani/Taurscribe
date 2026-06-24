//! Synthetic audio decode/preprocess benchmark.
//!
//! Usage:
//!   cargo run --release --bin audio_pipeline_bench -- 120
//!
//! The optional argument is duration in seconds. This does not run ASR models;
//! it isolates the file-drop audio pipeline hot path.

use std::path::{Path, PathBuf};
use std::time::Instant;

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::memory::{process_memory_stats, trim_process_memory};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;

fn main() {
    let duration_secs = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(90)
        .max(1);

    let path = bench_wav_path(duration_secs);
    write_synthetic_wav(&path, duration_secs).unwrap_or_else(|e| {
        eprintln!("failed to write fixture {}: {e}", path.display());
        std::process::exit(1);
    });

    println!(
        "[audio-bench] fixture={} duration={}s sample_rate={} channels={}",
        path.display(),
        duration_secs,
        SAMPLE_RATE,
        CHANNELS
    );

    let old = run_old_pipeline(&path).unwrap_or_else(|e| {
        eprintln!("old pipeline failed: {e}");
        std::process::exit(1);
    });
    trim_process_memory();
    let new = run_new_pipeline(&path).unwrap_or_else(|e| {
        eprintln!("new pipeline failed: {e}");
        std::process::exit(1);
    });

    println!(
        "[audio-bench] old: decode={}ms total={}ms mono_samples={} final_samples={} rss={}MB private={}MB",
        old.decode_ms,
        old.total_ms,
        old.mono_samples,
        old.final_samples,
        old.working_set_mb,
        old.private_mb
    );
    println!(
        "[audio-bench] new: decode={}ms total={}ms mono_samples={} final_samples={} rss={}MB private={}MB",
        new.decode_ms,
        new.total_ms,
        new.mono_samples,
        new.final_samples,
        new.working_set_mb,
        new.private_mb
    );

    if old.total_ms > 0 {
        let speed_delta =
            ((old.total_ms as f64 - new.total_ms as f64) / old.total_ms as f64) * 100.0;
        println!("[audio-bench] total_time_delta={speed_delta:.1}% positive=faster");
    }

    let _ = std::fs::remove_file(path);
}

struct BenchStats {
    decode_ms: u128,
    total_ms: u128,
    mono_samples: usize,
    final_samples: usize,
    working_set_mb: u64,
    private_mb: u64,
}

fn run_old_pipeline(path: &Path) -> Result<BenchStats, String> {
    let total_start = Instant::now();
    let decode_start = Instant::now();
    let (raw, sample_rate, channels) = audio_decode::decode_audio_interleaved_f32(path)?;
    let decode_ms = decode_start.elapsed().as_millis();

    let mut mono = if channels > 1 {
        let ch = channels as usize;
        raw.chunks(ch)
            .map(|frame| frame.iter().copied().sum::<f32>() / frame.len().max(1) as f32)
            .collect::<Vec<f32>>()
    } else {
        raw
    };
    let mono_samples = mono.len();

    if sample_rate != 16_000 {
        let resampled = audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
        drop(mono);
        mono = resampled;
    }
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);
    audio_preprocess::preprocess_assembled_speech_16k(&mut mono);

    Ok(snapshot(
        decode_ms,
        total_start.elapsed().as_millis(),
        mono_samples,
        mono.len(),
    ))
}

fn run_new_pipeline(path: &Path) -> Result<BenchStats, String> {
    let total_start = Instant::now();
    let decode_start = Instant::now();
    let (mut mono, sample_rate) = audio_decode::decode_audio_mono_f32(path)?;
    let decode_ms = decode_start.elapsed().as_millis();
    let mono_samples = mono.len();

    if sample_rate != 16_000 {
        let resampled = audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
        drop(mono);
        mono = resampled;
    }
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);
    audio_preprocess::preprocess_assembled_speech_16k(&mut mono);

    Ok(snapshot(
        decode_ms,
        total_start.elapsed().as_millis(),
        mono_samples,
        mono.len(),
    ))
}

fn snapshot(
    decode_ms: u128,
    total_ms: u128,
    mono_samples: usize,
    final_samples: usize,
) -> BenchStats {
    let mem = process_memory_stats();
    BenchStats {
        decode_ms,
        total_ms,
        mono_samples,
        final_samples,
        working_set_mb: mem.working_set_bytes / 1_048_576,
        private_mb: mem.private_bytes.unwrap_or(0) / 1_048_576,
    }
}

fn bench_wav_path(duration_secs: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "taurscribe_audio_pipeline_bench_{}_{}s.wav",
        std::process::id(),
        duration_secs
    ))
}

fn write_synthetic_wav(path: &Path, duration_secs: u32) -> Result<(), hound::Error> {
    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    let total_frames = SAMPLE_RATE as usize * duration_secs as usize;
    for i in 0..total_frames {
        let t = i as f32 / SAMPLE_RATE as f32;
        let voiced = ((2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.20
            + (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.08)
            * i16::MAX as f32;
        let left = voiced as i16;
        let right = (voiced * 0.85) as i16;
        writer.write_sample(left)?;
        writer.write_sample(right)?;
    }
    writer.finalize()
}
