//! In-person meeting end to end: a one-microphone room recording through the
//! speaker separator (Nemotron-3 Diarization), the user's voiceprint and
//! Granite, checked against the script it was made from.
//!
//! Requires the speaker separation and speaker recognition models and Granite
//! in Taurscribe's models folder, plus a recording:
//!   IN_PERSON_WAV    mono WAV of the meeting
//!   IN_PERSON_ME     WAV of the user reading the voice-setup passage
//!   IN_PERSON_TRUTH  JSON list of { speaker, start_s, end_s, text }
//!   IN_PERSON_YOU    the `speaker` in the truth who is the user
//! Run with:
//!   cargo test --test in_person_meeting -- --ignored --nocapture

use std::collections::HashMap;
use taurscribe_lib::diarization::{diarize_in_person_recording, DiarizedTurn};
use taurscribe_lib::gguf_asr::GgufAsrManager;

#[derive(serde::Deserialize)]
struct Truth {
    speaker: String,
    start_s: f64,
    end_s: f64,
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

fn overlap_ms(t: &DiarizedTurn, truth: &Truth) -> u64 {
    let (a, b) = ((truth.start_s * 1000.0) as u64, (truth.end_s * 1000.0) as u64);
    t.end_ms.min(b).saturating_sub(t.start_ms.max(a))
}

#[test]
#[ignore]
fn a_room_meeting_is_split_into_its_speakers_and_finds_you() {
    let truth: Vec<Truth> = serde_json::from_str(&std::fs::read_to_string(env("IN_PERSON_TRUTH")).unwrap()).unwrap();
    let you = env("IN_PERSON_YOU");
    let me = taurscribe_lib::speaker_embedding::embedding_from_wav(&env("IN_PERSON_ME")).expect("voiceprint of IN_PERSON_ME");

    let mut granite = GgufAsrManager::granite();
    granite.initialize(None, false).expect("Granite installed");
    let mut transcribe = |pcm: &[f32], rate: u32| granite.transcribe_chunk(pcm, rate, None);

    let snippets = std::env::temp_dir().join(format!("in-person-{}", std::process::id()));
    std::fs::create_dir_all(&snippets).unwrap();
    let turns = diarize_in_person_recording(
        std::path::Path::new(&env("IN_PERSON_WAV")),
        "",
        &snippets,
        1,
        Some(&mut transcribe),
        Some(&me),
    )
    .expect("diarize");
    let _ = std::fs::remove_dir_all(&snippets);

    for t in &turns {
        println!("{:>6.1}-{:>6.1}s  ch{} {:<12} {}", t.start_ms as f64 / 1000.0, t.end_ms as f64 / 1000.0, t.channel, t.speaker_name, t.text);
    }

    // Each found speaker stands for the real speaker it overlaps most.
    let mut votes: HashMap<&str, HashMap<&str, u64>> = HashMap::new();
    for t in &turns {
        for r in &truth {
            *votes.entry(t.speaker_name.as_str()).or_default().entry(r.speaker.as_str()).or_default() += overlap_ms(t, r);
        }
    }
    let label_of: HashMap<&str, &str> = votes
        .iter()
        .map(|(found, v)| (*found, v.iter().max_by_key(|(_, ms)| **ms).map(|(s, _)| *s).unwrap_or("?")))
        .collect();
    let (mut right, mut total) = (0u64, 0u64);
    for t in &turns {
        for r in &truth {
            let ms = overlap_ms(t, r);
            total += ms;
            if label_of[t.speaker_name.as_str()] == r.speaker {
                right += ms;
            }
        }
    }
    let accuracy = right as f64 / total.max(1) as f64;
    let speakers: Vec<&&str> = label_of.keys().collect();
    println!("speakers found: {speakers:?} → {label_of:?}; speech attributed correctly: {:.0}%", accuracy * 100.0);

    let real = truth.iter().map(|r| r.speaker.as_str()).collect::<std::collections::HashSet<_>>().len();
    assert_eq!(label_of.len(), real, "found {} speakers, the meeting has {real}", label_of.len());
    assert_eq!(label_of.get("You").copied(), Some(you.as_str()), "\"You\" must be {you}");
    assert!(turns.iter().filter(|t| t.speaker_name == "You").all(|t| t.channel == 0), "the user's turns are on channel 0");
    assert!(accuracy >= 0.85, "only {:.0}% of the speech went to the right speaker", accuracy * 100.0);
    assert!(turns.iter().all(|t| !t.text.trim().is_empty()));
}

/// Prints how alike the voiceprints of the truth's turns are (a diagnostic).
#[test]
#[ignore]
fn voiceprint_similarity_of_the_truth_turns() {
    let truth: Vec<Truth> = serde_json::from_str(&std::fs::read_to_string(env("IN_PERSON_TRUTH")).unwrap()).unwrap();
    let mut reader = hound::WavReader::open(env("IN_PERSON_WAV")).unwrap();
    let rate = reader.spec().sample_rate;
    let pcm: Vec<f32> = reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
    let engine = taurscribe_lib::speaker_embedding::get_speaker_engine();
    let embs: Vec<Vec<f32>> = truth
        .iter()
        .map(|r| {
            let (a, b) = ((r.start_s * rate as f64) as usize, ((r.end_s * rate as f64) as usize).min(pcm.len()));
            engine.lock().unwrap().compute_embedding(&pcm[a..b], rate).unwrap()
        })
        .collect();
    for (i, r) in truth.iter().enumerate() {
        let row: Vec<String> = embs.iter().map(|e| format!("{:.2}", taurscribe_lib::speaker_embedding::cosine_similarity(&embs[i], e))).collect();
        println!("{:<9} {}", r.speaker, row.join(" "));
    }
}
