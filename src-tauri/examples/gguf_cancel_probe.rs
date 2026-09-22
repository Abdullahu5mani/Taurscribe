//! Checks that setting the cancel flag aborts an in-flight GGUF run (diagnostic).
//! usage: cargo run --release --example gguf_cancel_probe -- <model.gguf> <long.wav> <cancel_after_ms>
fn main() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let a: Vec<String> = std::env::args().collect();
    let (audio, rate) = taurscribe_lib::audio_decode::decode_audio_mono_f32(std::path::Path::new(&a[2])).unwrap();
    assert_eq!(rate, 16_000);
    let model = transcribe_cpp::Model::load(&a[1]).unwrap();
    println!("supports cancellation: {}", model.supports(transcribe_cpp::Feature::Cancellation));
    drop(model);
    let mut asr = taurscribe_lib::gguf_asr::GgufAsr::load(std::path::Path::new(&a[1])).unwrap();
    let after: u64 = a[3].parse().unwrap();

    let t = std::time::Instant::now();
    let full = asr.transcribe(&audio).unwrap();
    println!("uncancelled: {:.2}s, {} chars", t.elapsed().as_secs_f32(), full.len());

    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(after));
        c.store(true, Ordering::Relaxed);
    });
    let t = std::time::Instant::now();
    let r = asr.transcribe_cancellable(&audio, &cancel);
    println!("cancel after {after} ms: returned in {:.2}s -> {:?}", t.elapsed().as_secs_f32(), r.map(|s| s.len()));
    let t = std::time::Instant::now();
    let again = asr.transcribe(&audio).unwrap();
    println!("session still usable: {:.2}s, same text: {}", t.elapsed().as_secs_f32(), again == full);
}
