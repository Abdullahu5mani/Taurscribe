//! fp16 timing and transcript check on real audio, through the Rust pipeline.
//!
//!   cargo run --release --bin granite_mlx_bench -- <model-dir> <16kHz-wav>

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macOS only");
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use mlx_rs::Dtype;
    use taurscribe_lib::{granite_features, granite_mlx::GraniteMlx};

    let mut args = std::env::args().skip(1);
    let model_dir = args.next().ok_or("usage: <model-dir> <wav>")?;
    let wav_path = args.next().ok_or("usage: <model-dir> <wav>")?;

    let t = std::time::Instant::now();
    let model = GraniteMlx::load(std::path::Path::new(&model_dir), Dtype::Float16)?;
    println!("load: {:.2}s (fp16)", t.elapsed().as_secs_f32());

    let audio = read_wav_mono16k(&wav_path)?;
    let seconds = audio.len() as f32 / 16_000.0;
    let feats = granite_features::extract_features(&audio);
    let frames = feats.nrows();
    let flat: Vec<f32> = feats.iter().copied().collect();
    println!("audio: {seconds:.2}s -> {frames} frames");

    let tok = tokenizers::Tokenizer::from_file(
        std::path::Path::new(&model_dir).join("tokenizer.json"),
    )
    .map_err(|e| format!("load tokenizer: {e}"))?;

    // Warm up: first call pays kernel compilation.
    let ids = model.transcribe_features(&flat, frames)?;
    let text = tok.decode(&ids, true).map_err(|e| format!("decode: {e}"))?;

    let mut runs = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        model.transcribe_features(&flat, frames)?;
        runs.push(t.elapsed().as_secs_f32());
    }
    runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = runs[runs.len() / 2];

    println!("\np50 {:.3}s over {} runs  ->  RTF {:.4}", p50, runs.len(), p50 / seconds);
    println!("tokens: {}", ids.len());
    println!("text:   {}", text.trim());
    Ok(())
}

/// Minimal 16-bit PCM WAV reader; the reference clip is 16 kHz mono.
#[cfg(target_os = "macos")]
fn read_wav_mono16k(path: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let raw = std::fs::read(path)?;
    if &raw[..4] != b"RIFF" || &raw[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let (mut pos, mut channels, mut bits, mut rate) = (12usize, 1u16, 16u16, 16_000u32);
    while pos + 8 <= raw.len() {
        let id = &raw[pos..pos + 4];
        let size = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into()?) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            channels = u16::from_le_bytes(raw[body + 2..body + 4].try_into()?);
            rate = u32::from_le_bytes(raw[body + 4..body + 8].try_into()?);
            bits = u16::from_le_bytes(raw[body + 14..body + 16].try_into()?);
        } else if id == b"data" {
            if rate != 16_000 {
                return Err(format!("expected 16 kHz, got {rate}").into());
            }
            if bits != 16 {
                return Err(format!("expected 16-bit PCM, got {bits}").into());
            }
            let samples: Vec<f32> = raw[body..(body + size).min(raw.len())]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect();
            return Ok(if channels > 1 {
                samples
                    .chunks(channels as usize)
                    .map(|f| f.iter().sum::<f32>() / channels as f32)
                    .collect()
            } else {
                samples
            });
        }
        pos = body + size + (size & 1);
    }
    Err("no data chunk".into())
}
