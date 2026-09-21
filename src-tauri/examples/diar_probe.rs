//! Runs Nemotron-3 Diarization (MLX on Apple Silicon) on a 16 kHz mono WAV.
//! usage: diar_probe <model.safetensors> <audio.wav> [probs_out.f32]
//! Prints raw turns as JSON [[start_s, end_s, speaker], ...]; optionally dumps
//! the per-10 ms speaker probabilities (frames x 8, little-endian f32).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (x, rate) = taurscribe_lib::audio_decode::decode_audio_mono_f32(std::path::Path::new(&a[2])).unwrap();
    assert_eq!(rate, 16_000);
    let t = std::time::Instant::now();
    let m = taurscribe_lib::nemotron_diar_mlx::NemotronDiarMlx::load(std::path::Path::new(&a[1])).unwrap();
    let load = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let probs = m.speaker_probs(&x).unwrap();
    let run = t.elapsed().as_secs_f64();
    eprintln!("load {load:.2}s  run {run:.2}s  ({:.0}x realtime)  frames {}", x.len() as f64 / 16_000.0 / run, probs.len());
    if let Some(out) = a.get(3) {
        let bytes: Vec<u8> = probs.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(out, bytes).unwrap();
    }
    let turns = m.diarize(&x).unwrap();
    let rows: Vec<_> = turns.iter().map(|t| (t.start_ms as f64 / 1e3, t.end_ms as f64 / 1e3, t.speaker)).collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {}
