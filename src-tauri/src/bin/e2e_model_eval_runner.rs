use std::env;
use std::path::PathBuf;
use std::time::Instant;

use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::parakeet::ParakeetManager;
use taurscribe_lib::parakeet_loaders::ParakeetLoadPath;
use taurscribe_lib::whisper::WhisperManager;
use taurscribe_lib::utils::clean_transcript;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut engine = String::from("parakeet");
    let mut model_id = String::new();
    let mut audio_path = PathBuf::new();
    let mut force_cpu = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--engine" => {
                i += 1;
                if i < args.len() { engine = args[i].clone(); }
            }
            "--model" => {
                i += 1;
                if i < args.len() { model_id = args[i].clone(); }
            }
            "--audio" => {
                i += 1;
                if i < args.len() { audio_path = PathBuf::from(&args[i]); }
            }
            "--force-cpu" => {
                force_cpu = true;
            }
            _ => {}
        }
        i += 1;
    }

    if audio_path.as_os_str().is_empty() || !audio_path.exists() {
        eprintln!("Audio path does not exist: {:?}", audio_path);
        std::process::exit(1);
    }

    let (mut mono, sr) = audio_decode::decode_audio_mono_f32(&audio_path)?;
    if sr != 16000 {
        mono = audio_preprocess::resample_mono_to_16k(&mono, sr)?;
    }
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);
    audio_preprocess::preprocess_assembled_speech_16k(&mut mono);

    let t_start = Instant::now();
    let transcript = match engine.as_str() {
        "parakeet" => {
            let mut mgr = ParakeetManager::new();
            if model_id.contains("mlx") {
                mgr.initialize_with_load_path(
                    Some(&model_id),
                    false,
                    ParakeetLoadPath::StrictGpu,
                )?;
            } else {
                mgr.initialize_with_load_path(
                    Some(&model_id),
                    true,
                    ParakeetLoadPath::Cpu,
                )?;
            }

            let mut full = String::new();
            // 15-second chunks
            for chunk in mono.chunks(16000 * 15) {
                let part = mgr.transcribe_chunk(chunk, 16000)?;
                if !part.trim().is_empty() {
                    full.push_str(&part);
                    full.push(' ');
                }
            }
            clean_transcript(&full)
        }
        "whisper" => {
            let normalized_id = if model_id.contains("q5_1") {
                "tiny-q5_1"
            } else {
                "tiny"
            };
            let mut mgr = WhisperManager::new();
            mgr.initialize(Some(normalized_id), force_cpu)?;
            let mut full = String::new();
            // 180-second chunks
            for chunk in mono.chunks(16000 * 180) {
                let part = mgr.transcribe_audio_data(chunk, None)?;
                if !part.trim().is_empty() {
                    full.push_str(&part);
                    full.push(' ');
                }
            }
            clean_transcript(&full)
        }
        _ => {
            eprintln!("Unknown engine: {}", engine);
            std::process::exit(2);
        }
    };
    let elapsed_ms = t_start.elapsed().as_millis();

    println!("RESULT_JSON:{}", serde_json::json!({
        "engine": engine,
        "model": model_id,
        "elapsed_ms": elapsed_ms,
        "audio_samples": mono.len(),
        "audio_duration_sec": mono.len() as f64 / 16000.0,
        "transcript": transcript
    }));

    Ok(())
}
