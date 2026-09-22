//! Runs a GGUF model through transcribe.cpp and a Whisper model through
//! whisper-rs in the same process, to prove the two ggml copies coexist
//! (diagnostic).
//! usage: cargo run --release --example gguf_probe -- <model.gguf> <ggml-whisper.bin> a.wav ...
fn main() {
    use std::path::Path;
    let a: Vec<String> = std::env::args().collect();
    let load = |p: &str| {
        let (x, rate) = taurscribe_lib::audio_decode::decode_audio_mono_f32(Path::new(p)).unwrap();
        if rate == 16_000 { x } else { taurscribe_lib::audio_preprocess::resample_mono_to_16k(&x, rate).unwrap() }
    };

    let t = std::time::Instant::now();
    // PROBE_BACKEND=cpu|cuda|vulkan|rocm|metal pins the device; default picks the best.
    let backend = match std::env::var("PROBE_BACKEND").as_deref() {
        Ok("cpu") => transcribe_cpp::Backend::Cpu,
        Ok("cuda") => transcribe_cpp::Backend::Cuda,
        Ok("vulkan") => transcribe_cpp::Backend::Vulkan,
        Ok("rocm") => transcribe_cpp::Backend::Rocm,
        Ok("metal") => transcribe_cpp::Backend::Metal,
        _ => transcribe_cpp::Backend::Auto,
    };
    // PROBE_DEVICE=<n> picks the n-th device of that backend (e.g. a second GPU).
    let device = std::env::var("PROBE_DEVICE").ok().and_then(|n| n.parse::<usize>().ok()).map(|n| {
        let kind = format!("{backend:?}").to_lowercase();
        transcribe_cpp::devices()
            .into_iter()
            .filter(|d| format!("{d:?}").to_lowercase().contains(&kind))
            .nth(n)
            .expect("no such device")
    });
    for d in transcribe_cpp::devices() {
        println!("device: {d:?}");
    }
    let options = transcribe_cpp::ModelOptions { backend, device };
    let mut gguf = taurscribe_lib::gguf_asr::GgufAsr::load_with(Path::new(&a[1]), &options).expect("gguf load");
    println!("gguf loaded on {} in {:.2}s", gguf.backend(), t.elapsed().as_secs_f32());

    let ctx = whisper_rs::WhisperContext::new_with_params(&a[2], whisper_rs::WhisperContextParameters::default())
        .expect("whisper load");
    let mut wstate = ctx.create_state().unwrap();

    for f in &a[3..] {
        let audio = load(f);
        let _ = gguf.transcribe(&vec![0.0; 16_000]); // warm
        let t = std::time::Instant::now();
        let text = gguf.transcribe(&audio).unwrap();
        println!("gguf    {:6.0} ms  {text}", t.elapsed().as_secs_f64() * 1e3);

        let t = std::time::Instant::now();
        let params = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        wstate.full(params, &audio).unwrap();
        let text: String = (0..wstate.full_n_segments())
            .filter_map(|i| wstate.get_segment(i).map(|s| s.to_string()))
            .collect();
        println!("whisper {:6.0} ms  {}", t.elapsed().as_secs_f64() * 1e3, text.trim());
    }
}
