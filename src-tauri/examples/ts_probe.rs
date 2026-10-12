//! What timestamp granularity does each GGUF model return? (diagnostic)
//! usage: ts_probe <model.gguf> <audio.wav>
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (pcm, rate) = taurscribe_lib::audio_decode::decode_audio_mono_f32(std::path::Path::new(&a[2])).unwrap();
    assert_eq!(rate, 16_000);
    let model = transcribe_cpp::Model::load_with(std::path::Path::new(&a[1]), &transcribe_cpp::ModelOptions::default()).unwrap();
    println!("backend {}  caps.max_timestamp_kind = {:?}", model.backend(), model.capabilities().max_timestamp_kind);
    let mut session = model.session().unwrap();
    for kind in [transcribe_cpp::TimestampKind::Auto, transcribe_cpp::TimestampKind::Word, transcribe_cpp::TimestampKind::Token, transcribe_cpp::TimestampKind::Segment] {
        let opts = transcribe_cpp::RunOptions { timestamps: kind, ..Default::default() };
        let t = std::time::Instant::now();
        match session.run(&pcm, &opts) {
            Ok(o) => {
                println!("\n== requested {kind:?} -> returned {:?}: {} segments, {} words, {} tokens ({:.1}s)", o.timestamp_kind, o.segments.len(), o.words.len(), o.tokens.len(), t.elapsed().as_secs_f32());
                for w in o.words.iter().take(6) { println!("   word {:>6}-{:<6} {}", w.t0_ms, w.t1_ms, w.text); }
                for s in o.segments.iter().take(3) { println!("   seg  {:>6}-{:<6} {}", s.t0_ms, s.t1_ms, s.text.chars().take(50).collect::<String>()); }
                for k in o.tokens.iter().take(6) { println!("   tok  {:>6}-{:<6} {:?}", k.t0_ms, k.t1_ms, k.text); }
            }
            Err(e) => println!("\n== requested {kind:?} -> error {e}"),
        }
    }
}
