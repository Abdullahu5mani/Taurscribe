//! Tests of voice assignment without loading models or touching captured audio.
use taurscribe_lib::neural_diarizer::{split_mixed_speakers, SpeakerTurn};

fn talk(parts: &[(usize, f32)]) -> (Vec<f32>, Vec<(u64, u64, usize)>) {
    let mut pcm = Vec::new(); let mut spans = Vec::new();
    for &(voice, seconds) in parts {
        pcm.extend(vec![0.0; 8_000]);
        let start = pcm.len() as u64 / 16;
        let hz = 150.0 + 90.0 * voice as f32;
        pcm.extend((0..(seconds * 16_000.0) as usize).map(|n| 0.3 * (n as f32 * hz * std::f32::consts::TAU / 16_000.0).sin()));
        spans.push((start, pcm.len() as u64 / 16, voice));
    }
    pcm.extend(vec![0.0; 8_000]); (pcm, spans)
}
fn voiceprint(audio: &[f32]) -> Option<Vec<f32>> {
    let crossings = audio.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
    let hz = crossings * 16_000.0 / audio.len() as f32;
    let voice = ((hz - 150.0) / 90.0).round().clamp(0.0, 3.0) as usize;
    let mut embedding = vec![0.05; 4]; embedding[voice] = 1.0; Some(embedding)
}
fn voice_at(turns: &[SpeakerTurn], start: u64) -> Option<usize> {
    turns.iter().find(|t| t.start_ms <= start + 500 && start + 500 < t.end_ms).map(|t| t.speaker)
}
fn split(parts: &[(usize, f32)], parents: &[(usize, usize)]) -> (Vec<SpeakerTurn>, Vec<(u64,u64,usize)>) {
    let (pcm, spans) = talk(parts);
    let turns = parents.iter().map(|&(first,last)| SpeakerTurn {start_ms:spans[first].0, end_ms:spans[last].1, speaker:0}).collect();
    (split_mixed_speakers(turns, &pcm, &mut voiceprint), spans)
}
#[test]
fn two_established_voices_are_separated_without_short_replies() {
    let (out, s) = split(&[(0,4.0),(1,4.0),(0,4.0),(1,4.0)], &[(0,0),(1,1),(2,2),(3,3)]);
    assert_ne!(voice_at(&out,s[0].0),voice_at(&out,s[1].0));
    assert_eq!(voice_at(&out,s[0].0),voice_at(&out,s[2].0));
    assert_eq!(voice_at(&out,s[1].0),voice_at(&out,s[3].0));
}
#[test]
fn short_reply_can_match_an_established_voice_in_its_own_turn() {
    let (out, s) = split(&[(0,4.0),(1,1.2),(1,4.0),(0,4.0),(1,4.0)], &[(0,0),(1,1),(2,2),(3,3),(4,4)]);
    assert_eq!(voice_at(&out,s[1].0),voice_at(&out,s[2].0),"{out:?}");
    assert_ne!(voice_at(&out,s[1].0),voice_at(&out,s[0].0),"{out:?}");
}
#[test]
fn short_reply_must_not_inherit_a_different_voice_from_its_parent_turn() {
    // The separator merged A's sentence and B's brief reply into one turn;
    // both voices have eight seconds of independent reference speech.
    let (out, s) = split(&[(0,4.0),(1,1.2),(1,4.0),(0,4.0),(1,4.0)], &[(0,1),(2,2),(3,3),(4,4)]);
    assert_eq!(voice_at(&out,s[1].0),voice_at(&out,s[2].0),"A short B reply was attributed to A: {out:?}");
    assert_ne!(voice_at(&out,s[1].0),voice_at(&out,s[0].0),"{out:?}");
}
