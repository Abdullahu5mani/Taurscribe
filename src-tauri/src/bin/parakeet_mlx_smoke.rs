
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    use taurscribe_lib::parakeet_mlx::ParakeetNemotronMlx;
    use parakeet_rs::{ExecutionConfig, Nemotron};

    let home = std::env::var("HOME").unwrap_or_default();
    let mlx_dir = PathBuf::from(&home)
        .join("Library/Application Support/Taurscribe/models/parakeet-nemotron-mlx");
    let onnx_dir = PathBuf::from(&home)
        .join("Library/Application Support/Taurscribe/models/parakeet-nemotron");

    // 1. Read jfk.wav
    let wav_path = PathBuf::from("tests/fixtures/jfk.wav");
    println!("[benchmark] Reading WAV file: {}", wav_path.display());
    let mut reader = hound::WavReader::open(&wav_path)?;
    let spec = reader.spec();
    println!("[benchmark] WAV spec: channels={}, sample_rate={}, bits={}", spec.channels, spec.sample_rate, spec.bits_per_sample);
    let samples: Vec<f32> = reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
    let audio_duration_sec = samples.len() as f64 / spec.sample_rate as f64;
    println!("[benchmark] Audio duration: {:.2}s ({} samples)", audio_duration_sec, samples.len());

    const CHUNK_SIZE: usize = 8960; // 560 ms streaming chunks (same as Taurscribe production)

    // 2. Benchmark MLX Metal GPU
    println!("\n========================================");
    println!(">>> BENCHMARKING APPLE SILICON MLX (METAL GPU) <<<");
    println!("========================================");
    let t0 = std::time::Instant::now();
    let mut mlx_engine = ParakeetNemotronMlx::load(&mlx_dir)?;
    println!("[MLX] Loaded in {:.2?}", t0.elapsed());

    let mut mlx_transcript = String::new();
    let t_mlx_start = std::time::Instant::now();
    for chunk in samples.chunks(CHUNK_SIZE) {
        let mut chunk_vec = chunk.to_vec();
        if chunk_vec.len() < CHUNK_SIZE {
            chunk_vec.resize(CHUNK_SIZE, 0.0);
        }
        let chunk_res = mlx_engine.transcribe_chunk(&chunk_vec)?;
        mlx_transcript.push_str(&chunk_res);
    }
    let mlx_duration = t_mlx_start.elapsed();
    let mlx_rtf = mlx_duration.as_secs_f64() / audio_duration_sec;
    println!("[MLX] Transcript: \"{}\"", mlx_transcript.trim());
    println!("[MLX] Latency: {:.2?}, RTF: {:.4}", mlx_duration, mlx_rtf);

    // 3. Benchmark ONNX CPU Baseline
    println!("\n========================================");
    println!(">>> BENCHMARKING ONNX RUNTIME (CPU BASELINE) <<<");
    println!("========================================");
    let t0 = std::time::Instant::now();
    let mut onnx_engine = Nemotron::from_pretrained(&onnx_dir, Some(ExecutionConfig::new()))?;
    println!("[ONNX CPU] Loaded in {:.2?}", t0.elapsed());

    let mut onnx_transcript = String::new();
    let t_onnx_start = std::time::Instant::now();
    for chunk in samples.chunks(CHUNK_SIZE) {
        let mut chunk_vec = chunk.to_vec();
        if chunk_vec.len() < CHUNK_SIZE {
            chunk_vec.resize(CHUNK_SIZE, 0.0);
        }
        let chunk_res = onnx_engine.transcribe_chunk(&chunk_vec)?;
        onnx_transcript.push_str(&chunk_res);
    }
    let onnx_duration = t_onnx_start.elapsed();
    let onnx_rtf = onnx_duration.as_secs_f64() / audio_duration_sec;
    println!("[ONNX CPU] Transcript: \"{}\"", onnx_transcript.trim());
    println!("[ONNX CPU] Latency: {:.2?}, RTF: {:.4}", onnx_duration, onnx_rtf);

    // 4. Comparison
    println!("\n========================================");
    println!(">>> FINAL VERIFICATION SUMMARY <<<");
    println!("========================================");
    println!("MLX Transcript:      \"{}\"", mlx_transcript.trim());
    println!("ONNX CPU Transcript: \"{}\"", onnx_transcript.trim());
    let match_exact = mlx_transcript.trim() == onnx_transcript.trim();
    println!("Exact Transcript Match: {}", if match_exact { "YES! 100% BIT-PERFECT MATCH" } else { "PARTIAL (WER check required)" });

    Ok(())
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {}
