//! One-shot Granite transcription against an explicit model directory.
//!
//! Usage:
//!   TAURSCRIBE_GRANITE_MODEL_DIR=<bundle> cargo run --release --bin granite_transcribe_probe -- <audio.wav>

use std::path::PathBuf;

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::cohere::CohereManager;
use taurscribe_lib::utils::clean_transcript;

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("Usage: granite_transcribe_probe <audio_file>");
            std::process::exit(1);
        });

    if !path.exists() {
        eprintln!("File not found: {}", path.display());
        std::process::exit(1);
    }

    let model_dir = std::env::var("TAURSCRIBE_GRANITE_MODEL_DIR").ok();

    println!("[granite_transcribe_probe] Decoding: {}", path.display());
    println!(
        "[granite_transcribe_probe] Model dir: {}",
        model_dir.as_deref().unwrap_or("<default>")
    );

    let (mut mono, sample_rate) = audio_decode::decode_audio_mono_f32(&path).unwrap_or_else(|e| {
        eprintln!("Decode error: {e}");
        std::process::exit(1);
    });

    if sample_rate != 16000 {
        let resampled =
            audio_preprocess::resample_mono_to_16k(&mono, sample_rate).unwrap_or_else(|e| {
                eprintln!("Resample error: {e}");
                std::process::exit(1);
            });
        drop(mono);
        mono = resampled;
    }

    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);
    audio_preprocess::preprocess_assembled_speech_16k(&mut mono);

    println!(
        "[granite_transcribe_probe] Audio ready: {:.2}s @ 16 kHz ({} samples)",
        mono.len() as f32 / 16000.0,
        mono.len()
    );

    let mut granite = CohereManager::new();
    granite
        .initialize(model_dir.as_deref(), false)
        .unwrap_or_else(|e| {
            eprintln!("Granite init error: {e}");
            std::process::exit(1);
        });

    println!("[granite_transcribe_probe] Model loaded. Transcribing...");
    let t0 = std::time::Instant::now();
    let raw_text = granite.transcribe_chunk(&mono, 16000).unwrap_or_else(|e| {
        eprintln!("Transcription error: {e}");
        std::process::exit(1);
    });
    let elapsed = t0.elapsed();

    println!(
        "[granite_transcribe_probe] Done in {:.2}s",
        elapsed.as_secs_f32()
    );
    println!();
    println!("=== TRANSCRIPT ===");
    println!("{}", clean_transcript(&raw_text));

    granite.unload();
}
