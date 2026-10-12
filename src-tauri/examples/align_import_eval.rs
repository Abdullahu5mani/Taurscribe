//! Transcript-first import with and without word alignment (Granite Speech 5).
//! usage: align_import_eval <nemotron.safetensors> <audio.wav>
//! Prints how many speaker turns can reuse the whole-chunk transcript, and the
//! text each strategy yields (JSON on stdout) for WER scoring elsewhere.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    use taurscribe_lib::{gguf_asr::GgufAsrManager, meeting_transcript::{chunk_ranges, TimedTranscript}, neural_diarizer};
    let a: Vec<String> = std::env::args().collect();
    let (pcm, rate) = taurscribe_lib::audio_decode::decode_audio_mono_f32(std::path::Path::new(&a[2])).unwrap();
    assert_eq!(rate, 16_000);
    let model = taurscribe_lib::nemotron_diar_mlx::NemotronDiarMlx::load(std::path::Path::new(&a[1])).unwrap();
    let raw = neural_diarizer::tidy_turns(model.diarize(&pcm).unwrap());
    let engine = taurscribe_lib::speaker_embedding::get_speaker_engine();
    let mut voiceprint = |audio: &[f32]| engine.lock().ok()?.compute_embedding(audio, 16_000).ok();
    let turns = neural_diarizer::merge_voices_with_hint(raw, &pcm, &mut voiceprint, None);
    let intervals: Vec<(u64, u64)> = turns.iter().map(|t| (t.start_ms, t.end_ms)).collect();

    let mut granite = GgufAsrManager::granite();
    granite.initialize(Some("granite-speech-5-nc"), false).unwrap();

    let mut run = |align: bool| {
        // SAFETY: single-threaded example.
        unsafe { std::env::set_var("TAURSCRIBE_WORD_ALIGN", if align { "1" } else { "0" }) };
        let t = std::time::Instant::now();
        let mut whole = TimedTranscript::default();
        for (s, e) in chunk_ranges(&pcm) {
            let part = granite.transcribe_timed(&pcm[s..e]).unwrap();
            whole.append(part, s as u64 / 16);
        }
        let chunk_secs = t.elapsed().as_secs_f32();
        let covered = whole.covers_text();
        // Per turn: reuse the whole-chunk text when it is unambiguous, else re-decode.
        let t = std::time::Instant::now();
        let (mut reused, mut redecoded) = (0, 0);
        let mut texts = Vec::new();
        for (i, (s, e)) in intervals.iter().enumerate() {
            if let Some(text) = whole.for_turn(i, &intervals) {
                reused += 1;
                texts.push(text);
            } else {
                redecoded += 1;
                let (a0, b0) = ((*s as usize * 16).min(pcm.len()), (*e as usize * 16).min(pcm.len()));
                texts.push(granite.transcribe_chunk(&pcm[a0..b0], 16_000, None).unwrap_or_default());
            }
        }
        eprintln!("align={align}: units={} covers={covered} chunk pass {chunk_secs:.1}s; turns {} reused / {} re-decoded ({:.1}s)", whole.units.len(), reused, redecoded, t.elapsed().as_secs_f32());
        serde_json::json!({"reused": reused, "redecoded": redecoded, "turns": intervals.len(), "text": texts.join(" "), "chunk_text": whole.text})
    };
    let off = run(false);
    let on = run(true);
    println!("{}", serde_json::json!({"off": off, "on": on}));
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {}
