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
    text: String,
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

    let labels = check_meeting(&turns, &truth).expect("meeting accuracy and coverage");
    assert_eq!(labels.get("You").copied(), Some(you.as_str()), "\"You\" must be {you}");
    assert!(turns.iter().filter(|t| t.speaker_name == "You").all(|t| t.channel == 0), "the user's turns are on channel 0");
}

/// Measure against ALL reference speech. Union intersections so repeated or
/// overlapping hypothesis turns cannot inflate coverage.
fn covered_ms(turns: &[DiarizedTurn], truth: &Truth, label: Option<&str>) -> u64 {
    let a = (truth.start_s * 1000.0) as u64;
    let b = (truth.end_s * 1000.0) as u64;
    let mut intervals: Vec<_> = turns.iter()
        .filter(|t| label.is_none_or(|label| t.speaker_name == label))
        .map(|t| (t.start_ms.max(a), t.end_ms.min(b)))
        .filter(|(a, b)| b > a).collect();
    intervals.sort_unstable();
    let (mut covered, mut end) = (0, a);
    for (a, b) in intervals {
        covered += b.saturating_sub(a.max(end));
        end = end.max(b);
    }
    covered
}

fn check_meeting<'a>(turns: &'a [DiarizedTurn], truth: &'a [Truth]) -> Result<HashMap<&'a str, &'a str>, String> {
    use taurscribe_lib::librispeech_wer::{normalize_for_wer, word_error_rate};
    if truth.is_empty() || truth.iter().any(|r| !r.start_s.is_finite() || !r.end_s.is_finite()
        || r.start_s < 0.0 || r.end_s <= r.start_s || r.text.trim().is_empty()) {
        return Err("reference needs timed speech and text".into());
    }
    let mut votes: HashMap<&str, HashMap<&str, u64>> = HashMap::new();
    for t in turns {
        if t.text.trim().is_empty() { return Err("empty hypothesis turn".into()); }
        for r in truth {
            *votes.entry(&t.speaker_name).or_default().entry(&r.speaker).or_default() += overlap_ms(t, r);
        }
    }
    let labels: HashMap<_, _> = votes.iter().map(|(found, votes)| {
        (*found, votes.iter().max_by_key(|(name, ms)| (**ms, **name)).map(|(s, _)| *s).unwrap_or("?"))
    }).collect();
    let real: std::collections::HashSet<_> = truth.iter().map(|r| r.speaker.as_str()).collect();
    let mapped: std::collections::HashSet<_> = labels.values().copied().collect();
    if labels.len() != real.len() || mapped != real { return Err("speaker mapping must be one to one".into()); }
    let total: u64 = truth.iter().map(|r| (r.end_s * 1000.0) as u64 - (r.start_s * 1000.0) as u64).sum();
    let coverage = truth.iter().map(|r| covered_ms(turns, r, None)).sum::<u64>() as f64 / total.max(1) as f64;
    let correct = truth.iter().map(|r| {
        let found = labels.iter().find(|(_, real)| **real == r.speaker).unwrap().0;
        covered_ms(turns, r, Some(found))
    }).sum::<u64>() as f64 / total.max(1) as f64;
    println!("labels: {labels:?}; coverage={coverage:.3}; correctly attributed reference speech={correct:.3}");
    if coverage < 0.95 { return Err(format!("speech coverage {coverage:.3} below 0.95")); }
    if correct < 0.85 { return Err(format!("correct attribution {correct:.3} below 0.85")); }
    for speaker in real {
        let mut reference: Vec<_> = truth.iter().filter(|r| r.speaker == speaker).collect();
        reference.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
        let expected = reference.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join(" ");
        let mut hypothesis: Vec<_> = turns.iter().filter(|t| labels[t.speaker_name.as_str()] == speaker).collect();
        hypothesis.sort_by_key(|t| t.start_ms);
        let actual = hypothesis.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ");
        let wer = word_error_rate(&normalize_for_wer(&expected), &normalize_for_wer(&actual));
        println!("{speaker}: WER={wer:.3}");
        if wer > 0.25 { return Err(format!("{speaker}: WER {wer:.3} exceeds 0.25")); }
    }
    Ok(labels)
}

#[test]
fn acceptance_rejects_wrong_words_missing_speech_and_duplicate_coverage() {
    let truth = vec![Truth { speaker: "Alice".into(), start_s: 0.0, end_s: 10.0, text: "one two three four".into() }];
    let mut turns = vec![DiarizedTurn {
        speaker_id: "a".into(), speaker_name: "Speaker 1".into(), start_ms: 0, end_ms: 10_000,
        channel: 1, text: "One, two three four!".into(), snippet_path: None,
        candidate_snippets: Vec::new(), current_snippet_idx: 0,
    }];
    assert!(check_meeting(&turns, &truth).is_ok());
    turns[0].text = "completely unrelated arbitrary output".into();
    assert!(check_meeting(&turns, &truth).unwrap_err().contains("WER"));
    turns[0].text = truth[0].text.clone();
    turns[0].end_ms = 2000;
    assert!(check_meeting(&turns, &truth).unwrap_err().contains("coverage"));
    turns.push(turns[0].clone());
    assert!(check_meeting(&turns, &truth).unwrap_err().contains("coverage"));
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
