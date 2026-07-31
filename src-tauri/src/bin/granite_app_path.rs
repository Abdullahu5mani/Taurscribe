//! Drives CohereManager exactly as the app does, to prove the wiring works:
//! initialize() must pick MLX, and transcribe_chunk() must return real text.
//!
//!   cargo run --bin granite_app_path -- <wav>

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macOS only");
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use taurscribe_lib::cohere::CohereManager;

    let wav = std::env::args().nth(1).ok_or("usage: <wav>")?;
    let audio = read_wav(&wav)?;
    let seconds = audio.len() as f32 / 16_000.0;

    let mut mgr = CohereManager::new();
    let t = std::time::Instant::now();
    let msg = mgr.initialize(None, false)?;
    println!("initialize: {msg}  ({:.2}s)", t.elapsed().as_secs_f32());
    println!("status: {:?}", mgr.get_status());

    for pass_no in 1..=3 {
        let t = std::time::Instant::now();
        let text = mgr.transcribe_chunk(&audio, 16_000)?;
        let elapsed = t.elapsed().as_secs_f32();
        println!(
            "pass {pass_no}: {elapsed:.3}s for {seconds:.2}s audio -> RTF {:.4}",
            elapsed / seconds
        );
        if pass_no == 3 {
            println!("text: {text}");
        }
    }
    mgr.unload();
    println!("unloaded cleanly");
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_wav(path: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let raw = std::fs::read(path)?;
    let mut pos = 12usize;
    while pos + 8 <= raw.len() {
        let id = &raw[pos..pos + 4];
        let size = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into()?) as usize;
        if id == b"data" {
            return Ok(raw[pos + 8..(pos + 8 + size).min(raw.len())]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect());
        }
        pos += 8 + size + (size & 1);
    }
    Err("no data chunk".into())
}
