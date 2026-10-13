//! Forced-aligns a transcript to audio with the word-alignment model (diagnostic).
//! usage: align_probe <audio.wav> <text-file>   -> JSON [[start_ms,end_ms,"word"],..]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (pcm, rate) = taurscribe_lib::audio_decode::decode_audio_mono_f32(std::path::Path::new(&a[1])).unwrap();
    assert_eq!(rate, 16_000);
    let text = std::fs::read_to_string(&a[2]).unwrap();
    let t = std::time::Instant::now();
    // The aligner takes <= 45 s; chunk like the importer does.
    let mut all = Vec::new();
    let words: Vec<&str> = text.split_whitespace().collect();
    let ranges = taurscribe_lib::meeting_transcript::chunk_ranges(&pcm);
    // Distribute words to chunks by a first pass of equal-density guess is wrong;
    // for the probe, align the whole file in <=30 s windows given pre-split text files.
    if ranges.len() == 1 {
        match taurscribe_lib::word_align::align_transcript(&pcm, &text) {
            Some(u) => all = u.into_iter().map(|x| (x.start_ms, x.end_ms, x.text)).collect(),
            None => eprintln!("alignment unavailable"),
        }
    } else {
        eprintln!("clip longer than one chunk ({} chunks); use a <=30 s clip", ranges.len());
    }
    eprintln!("{} words aligned in {:.2}s (of {})", all.len(), t.elapsed().as_secs_f32(), words.len());
    println!("{}", serde_json::to_string(&all).unwrap());
}
