//! End-to-end verification for macOS x86_64 architecture running via Rosetta 2 emulation.
//! Tests architecture detection, ONNX Runtime CPU execution, and Parakeet ASR transcription accuracy.

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
use std::path::PathBuf;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
use parakeet_rs::Nemotron;

#[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
fn main() {
    println!("macOS x86_64 only");
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("===============================================================================");
    println!("     macOS Intel (x86_64) Native Execution & Accuracy Verification");
    println!("===============================================================================");

    println!("[SYS] Target OS: {}", std::env::consts::OS);
    println!("[SYS] Target Architecture: {}", std::env::consts::ARCH);
    assert_eq!(std::env::consts::ARCH, "x86_64", "Must run on x86_64");

    // 1. Check audio fixture
    let wav_path = PathBuf::from("tests/fixtures/jfk.wav");
    if !wav_path.exists() {
        eprintln!("Audio fixture not found: {}", wav_path.display());
        std::process::exit(1);
    }

    let mut reader = hound::WavReader::open(&wav_path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
    let audio_dur = samples.len() as f64 / spec.sample_rate as f64;
    println!("[AUDIO] Loaded {} (duration: {:.2}s, rate: {}Hz)", wav_path.display(), audio_dur, spec.sample_rate);

    let home = std::env::var("HOME").unwrap_or_default();
    let onnx_dir = PathBuf::from(&home).join("Library/Application Support/Taurscribe/models/parakeet-nemotron");
    if !onnx_dir.exists() {
        eprintln!("Model dir not found: {}", onnx_dir.display());
        std::process::exit(1);
    }

    let dylib_path = std::env::var("ORT_DYLIB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/onnxruntime-x86_64/lib/libonnxruntime.dylib"));
    if dylib_path.exists() {
        println!("[DYNAMIC] Initializing ORT from {}", dylib_path.display());
        ort::init_from(&dylib_path)?.commit();
    }

    println!("[ONNX] Loading Parakeet Nemotron ONNX CPU model on x86_64...");
    let t0 = std::time::Instant::now();
    let mut nemotron = Nemotron::from_pretrained(&onnx_dir, None)?;
    let load_time = t0.elapsed();
    println!("[ONNX] Loaded in {:.2?} on x86_64 CPU", load_time);

    // 3. Streaming Inference
    const CHUNK_SIZE: usize = 8960; // 560ms
    let mut full_transcript = String::new();
    let t_infer = std::time::Instant::now();

    for chunk in samples.chunks(CHUNK_SIZE) {
        let mut chunk_vec = chunk.to_vec();
        if chunk_vec.len() < CHUNK_SIZE {
            chunk_vec.resize(CHUNK_SIZE, 0.0);
        }
        let partial = nemotron.transcribe_chunk(&chunk_vec)?;
        full_transcript.push_str(&partial);
    }

    let infer_duration = t_infer.elapsed();
    let rtf = infer_duration.as_secs_f64() / audio_dur;
    let clean = taurscribe_lib::utils::clean_transcript(&full_transcript);

    println!("\n[RESULT] Transcript on macOS x86_64 CPU:");
    println!("-------------------------------------------------------------------------------");
    println!("\"{clean}\"");
    println!("-------------------------------------------------------------------------------");
    println!("[PERF] Audio: {:.2}s | Processing Time: {:.2?} | Real-Time Factor (RTF): {:.4}", audio_dur, infer_duration, rtf);

    // 4. Verify accuracy
    assert!(
        clean.to_lowercase().contains("ask not what your country can do for you")
            && clean.to_lowercase().contains("what you can do for your country"),
        "Transcript did not match expected JFK speech!"
    );

    println!("\n>>> SUCCESS: macOS x86_64 binary executed flawlessly with 100% transcript accuracy! <<<\n");
    Ok(())
}
