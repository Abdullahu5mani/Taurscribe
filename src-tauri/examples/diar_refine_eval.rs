//! Runs the diarization stack on a 16 kHz mono WAV and prints the speaker turns
//! after each stage, for comparing against a reference (e.g. DiariZen).
//! usage: diar_refine_eval <nemotron.safetensors> <audio.wav>
//! Output: JSON {"raw":[[s,e,spk]..],"first_pass":[..],"refined":[..],"report":{..}}
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    use taurscribe_lib::{diar_refine, neural_diarizer};
    let a: Vec<String> = std::env::args().collect();
    let (pcm, rate) = taurscribe_lib::audio_decode::decode_audio_mono_f32(std::path::Path::new(&a[2])).unwrap();
    assert_eq!(rate, 16_000);
    let model = taurscribe_lib::nemotron_diar_mlx::NemotronDiarMlx::load(std::path::Path::new(&a[1])).unwrap();
    let raw = neural_diarizer::tidy_turns(model.diarize(&pcm).unwrap());
    let engine = taurscribe_lib::speaker_embedding::get_speaker_engine();
    let mut voiceprint = |audio: &[f32]| engine.lock().ok()?.compute_embedding(audio, 16_000).ok();
    let first = neural_diarizer::merge_voices_with_hint(raw.clone(), &pcm, &mut voiceprint, None);
    let (refined, report) = diar_refine::refine(first.clone(), &pcm, &mut voiceprint, &diar_refine::RefineOptions::default());
    let prints = diar_refine::pooled_prints(&raw, &pcm, &mut voiceprint);
    let ids: Vec<usize> = prints.keys().copied().collect();
    eprintln!("pooled-print similarity (rows/cols = speakers {:?}; speech s = {:?})", ids, ids.iter().map(|i| prints[i].1 / 1000).collect::<Vec<_>>());
    for a in &ids {
        eprintln!("  S{a}: {}", ids.iter().map(|b| format!("{:5.2}", diar_refine::print_similarity(&prints[a].0, &prints[b].0))).collect::<Vec<_>>().join(" "));
    }
    let rows = |t: &[neural_diarizer::SpeakerTurn]| t.iter().map(|x| (x.start_ms as f64 / 1e3, x.end_ms as f64 / 1e3, x.speaker)).collect::<Vec<_>>();
    let count = |t: &[neural_diarizer::SpeakerTurn]| t.iter().map(|x| x.speaker + 1).max().unwrap_or(0);
    eprintln!("raw {} -> first pass {} -> refined {} speakers; {:?}", count(&raw), count(&first), count(&refined), report);
    println!(
        "{}",
        serde_json::json!({"raw": rows(&raw), "first_pass": rows(&first), "refined": rows(&refined),
            "report": {"in": report.speakers_in, "out": report.speakers_out, "merged": report.merged, "absorbed": report.absorbed, "unprinted": report.unprinted}})
    );
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {}
