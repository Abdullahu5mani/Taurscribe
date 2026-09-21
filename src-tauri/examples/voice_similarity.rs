//! Pairwise speaker-embedding similarity between WAV clips (diagnostic).
//! usage: cargo run --example voice_similarity -- a.wav b.wav ...
fn load(path: &str) -> (Vec<f32>, u32) {
    let mut r = hound::WavReader::open(path).expect("wav");
    let spec = r.spec();
    let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
    let s: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => r.samples::<i32>().map(|x| x.unwrap() as f32 / max).collect(),
    };
    (s, spec.sample_rate)
}
fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let engine = taurscribe_lib::speaker_embedding::get_speaker_engine();
    let embs: Vec<Vec<f32>> = files
        .iter()
        .map(|f| {
            let (s, rate) = load(f);
            engine.lock().unwrap().compute_embedding(&s, rate).unwrap()
        })
        .collect();
    for i in 0..files.len() {
        for j in (i + 1)..files.len() {
            let sim = taurscribe_lib::speaker_embedding::cosine_similarity(&embs[i], &embs[j]);
            println!("{:>28} vs {:<28} {:.3}", files[i].rsplit('/').next().unwrap(), files[j].rsplit('/').next().unwrap(), sim);
        }
    }
}
