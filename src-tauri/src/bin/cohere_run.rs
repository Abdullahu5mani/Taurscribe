//! One-shot Cohere transcription of a single audio file.
//!
//! Usage:
//!   cargo run --bin cohere_run -- <path/to/audio.wav>
//!
//! CUDA only — errors out if CUDA EP unavailable.
//! Set TAURSCRIBE_COHERE_DEBUG=1 for verbose decode tracing.

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
            eprintln!("Usage: cohere_run <audio_file>");
            std::process::exit(1);
        });

    if !path.exists() {
        eprintln!("File not found: {}", path.display());
        std::process::exit(1);
    }

    println!("[cohere_run] Decoding: {}", path.display());

    // Decode directly to mono to match the lower-memory file-drop pipeline.
    let (mut mono, sample_rate) = audio_decode::decode_audio_mono_f32(&path).unwrap_or_else(|e| {
        eprintln!("Decode error: {e}");
        std::process::exit(1);
    });

    // Resample to 16 kHz
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
        "[cohere_run] Audio ready: {:.2}s @ 16 kHz ({} samples)",
        mono.len() as f32 / 16000.0,
        mono.len()
    );

    // Init Cohere — CUDA only, no fallback
    let mut cohere = CohereManager::new();
    cohere.initialize(None, false).unwrap_or_else(|e| {
        eprintln!("Cohere init error: {e}");
        std::process::exit(1);
    });

    println!("[cohere_run] Model loaded. Transcribing...");

    let t0 = std::time::Instant::now();
    let raw_text = cohere.transcribe_chunk(&mono, 16000).unwrap_or_else(|e| {
        eprintln!("Transcription error: {e}");
        std::process::exit(1);
    });

    let elapsed = t0.elapsed();
    let text = clean_transcript(&raw_text);

    println!("[cohere_run] Done in {:.2}s", elapsed.as_secs_f32());
    println!();
    println!("=== TRANSCRIPT ===");
    println!("{text}");

    cohere.unload();
}
