//! Second-pass speaker re-clustering for a diarized recording.
//!
//! The neural separator (Nemotron-3) labels speech with up to 8 speakers, and on
//! casual multi-person audio it tends to over-split: one person ends up under
//! several labels, usually the short interjections. The first-pass merge in
//! `neural_diarizer` only fingerprints a speaker from three single turns of at
//! least a second each, so a speaker made of many short turns never gets a
//! voiceprint and is never merged.
//!
//! This pass fingerprints every speaker from *pooled* speech (all of their
//! turns, longest first), then merges by evidence:
//!
//! * clusters with plenty of speech merge only on a clear same-voice score;
//! * small clusters merge on a lower score, but only when the match is
//!   unambiguous (clear margin over the runner-up);
//! * a speaker hint can pull a surplus speaker in, but never below a floor, so
//!   a count guess cannot fuse two different people;
//! * what stays tiny and unprinted is attached, turn by turn, to the nearest
//!   real speaker in time, because a second of speech is not another person.
//!
//! Timestamps are never touched: only the speaker labels change.

use crate::neural_diarizer::SpeakerTurn;
use std::collections::BTreeMap;

/// Same-voice cutoff between two well-evidenced speakers (matches the vault).
const SAME_VOICE: f32 = 0.60;
/// A cluster with less speech than this is "small".
const SMALL_SPEECH_MS: u64 = 8_000;
/// A cluster with less speech than this is "tiny".
const TINY_SPEECH_MS: u64 = 3_000;
/// Small clusters merge at this score if the match is clear.
const SMALL_SAME_VOICE: f32 = 0.50;
/// Tiny clusters merge at this score if the match is very clear.
const TINY_SAME_VOICE: f32 = 0.45;
/// How much better than the runner-up a small cluster's match must be.
const SMALL_MARGIN: f32 = 0.10;
const TINY_MARGIN: f32 = 0.08;
/// A speaker hint never merges below this score.
const HINT_FLOOR: f32 = 0.40;
/// Speech needed from a speaker (pooled) before its voiceprint is trusted.
const MIN_PRINT_MS: u64 = 1_500;
/// A turn shorter than this adds nothing to a pooled print.
const MIN_POOL_TURN_MS: u64 = 300;
/// Pooled audio per speaker is capped here (longest turns first).
const MAX_POOL_MS: u64 = 15_000;
/// Speakers with less speech than this and no print are folded into a neighbour.
const ORPHAN_MS: u64 = 2_500;
/// An orphan turn follows a neighbour no further away than this.
const ORPHAN_REACH_MS: u64 = 3_000;

#[derive(Debug, Clone, Copy)]
pub struct RefineOptions {
    /// The caller's guess of how many speakers there are (a soft limit).
    pub expected: Option<usize>,
}

impl Default for RefineOptions {
    fn default() -> Self {
        Self { expected: None }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefineReport {
    pub speakers_in: usize,
    pub speakers_out: usize,
    /// (kept, removed, similarity) in merge order, using the first pass's labels.
    pub merged: Vec<(usize, usize, f32)>,
    /// Speakers folded into their neighbours for lack of evidence.
    pub absorbed: Vec<usize>,
    /// Speakers that had too little speech for a voiceprint.
    pub unprinted: Vec<usize>,
}

/// Whether the second pass runs. `TAURSCRIBE_DIAR_REFINE=0` turns it off and
/// `=1` forces it on; otherwise it follows `DEFAULT_ENABLED`.
pub fn enabled() -> bool {
    const DEFAULT_ENABLED: bool = false;
    match std::env::var("TAURSCRIBE_DIAR_REFINE").ok().as_deref() {
        Some("0") | Some("off") | Some("false") => false,
        Some("1") | Some("on") | Some("true") => true,
        _ => DEFAULT_ENABLED,
    }
}

fn dur(t: &SpeakerTurn) -> u64 {
    t.end_ms.saturating_sub(t.start_ms)
}

fn normalized(e: &[f32]) -> Option<Vec<f32>> {
    if e.is_empty() || !e.iter().all(|v| v.is_finite()) {
        return None;
    }
    let n = e.iter().map(|v| v * v).sum::<f32>().sqrt();
    (n > f32::EPSILON).then(|| e.iter().map(|v| v / n).collect())
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return -1.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// One speaker's pooled speech: its longest turns, back to back, up to the cap.
fn pooled_audio(turns: &[SpeakerTurn], speaker: usize, pcm: &[f32]) -> (Vec<f32>, u64) {
    let mut mine: Vec<&SpeakerTurn> = turns.iter().filter(|t| t.speaker == speaker && dur(t) >= MIN_POOL_TURN_MS).collect();
    mine.sort_by_key(|t| std::cmp::Reverse(dur(t)));
    let (mut audio, mut ms) = (Vec::new(), 0u64);
    for t in mine {
        if ms >= MAX_POOL_MS {
            break;
        }
        let a = ((t.start_ms * 16) as usize).min(pcm.len());
        let take = dur(t).min(MAX_POOL_MS - ms);
        let b = (a + (take * 16) as usize).min(pcm.len());
        if b > a {
            audio.extend_from_slice(&pcm[a..b]);
            ms += take;
        }
    }
    (audio, ms)
}

/// Each speaker's normalised pooled voiceprint and the speech it came from
/// (diagnostics and tuning; `refine` builds the same prints).
pub fn pooled_prints(
    turns: &[SpeakerTurn],
    pcm: &[f32],
    voiceprint: &mut dyn FnMut(&[f32]) -> Option<Vec<f32>>,
) -> BTreeMap<usize, (Vec<f32>, u64)> {
    let ids: std::collections::BTreeSet<usize> = turns.iter().map(|t| t.speaker).collect();
    ids.into_iter()
        .filter_map(|id| {
            let (audio, ms) = pooled_audio(turns, id, pcm);
            (ms >= MIN_PRINT_MS).then(|| voiceprint(&audio).and_then(|e| normalized(&e)).map(|p| (id, (p, ms)))).flatten()
        })
        .collect()
}

/// Cosine similarity of two normalised prints.
pub fn print_similarity(a: &[f32], b: &[f32]) -> f32 {
    cosine(a, b)
}

/// Merges over-split speakers and folds orphans. See the module docs.
pub fn refine(
    mut turns: Vec<SpeakerTurn>,
    pcm: &[f32],
    voiceprint: &mut dyn FnMut(&[f32]) -> Option<Vec<f32>>,
    opts: &RefineOptions,
) -> (Vec<SpeakerTurn>, RefineReport) {
    let mut report = RefineReport::default();
    let mut speech: BTreeMap<usize, u64> = BTreeMap::new();
    for t in &turns {
        *speech.entry(t.speaker).or_default() += dur(t);
    }
    report.speakers_in = speech.len();
    if speech.len() < 2 {
        report.speakers_out = speech.len();
        return (turns, report);
    }

    // Voiceprints from pooled speech.
    let mut prints: BTreeMap<usize, Vec<f32>> = BTreeMap::new();
    for &id in speech.keys() {
        let (audio, ms) = pooled_audio(&turns, id, pcm);
        if ms < MIN_PRINT_MS {
            report.unprinted.push(id);
            continue;
        }
        match voiceprint(&audio).and_then(|e| normalized(&e)) {
            Some(p) => {
                prints.insert(id, p);
            }
            None => report.unprinted.push(id),
        }
    }

    // How strict a merge must be for a pair, given the smaller cluster's speech.
    let rule = |small: u64| -> (f32, f32) {
        if small < TINY_SPEECH_MS {
            (TINY_SAME_VOICE, TINY_MARGIN)
        } else if small < SMALL_SPEECH_MS {
            (SMALL_SAME_VOICE, SMALL_MARGIN)
        } else {
            (SAME_VOICE, 0.0)
        }
    };

    loop {
        let ids: Vec<usize> = speech.keys().copied().collect();
        let over_hint = opts.expected.is_some_and(|n| ids.len() > n.max(1));
        let mut best: Option<(usize, usize, f32)> = None;
        for (i, &a) in ids.iter().enumerate() {
            for &b in &ids[i + 1..] {
                let (Some(pa), Some(pb)) = (prints.get(&a), prints.get(&b)) else { continue };
                let sim = cosine(pa, pb);
                let (fragment, other) = if speech[&a] < speech[&b] { (a, b) } else { (b, a) };
                let (mut need, margin) = rule(speech[&fragment]);
                if over_hint {
                    // The count hint relaxes the bar for a surplus fragment, to a floor.
                    need = need.min(HINT_FLOOR.max(need - 0.10));
                }
                if sim < need {
                    continue;
                }
                if margin > 0.0 {
                    // The fragment must prefer this speaker clearly over every other.
                    let runner_up = ids
                        .iter()
                        .filter(|&&id| id != fragment && id != other)
                        .filter_map(|id| prints.get(id))
                        .map(|p| cosine(&prints[&fragment], p))
                        .fold(-1.0f32, f32::max);
                    if sim - runner_up < margin {
                        continue;
                    }
                }
                if best.is_none_or(|(_, _, s)| sim > s) {
                    best = Some((a, b, sim));
                }
            }
        }
        let Some((a, b, sim)) = best else { break };
        // Keep the label with more speech.
        let (keep, drop) = if speech[&a] >= speech[&b] { (a, b) } else { (b, a) };
        for t in &mut turns {
            if t.speaker == drop {
                t.speaker = keep;
            }
        }
        let (wk, wd) = (speech[&keep] as f32, speech.remove(&drop).unwrap() as f32);
        let pd = prints.remove(&drop).unwrap();
        if let Some(pk) = prints.get_mut(&keep) {
            for (x, y) in pk.iter_mut().zip(pd) {
                *x = *x * wk + y * wd;
            }
            if let Some(n) = normalized(pk) {
                *pk = n;
            }
        }
        *speech.get_mut(&keep).unwrap() += wd as u64;
        report.merged.push((keep, drop, sim));
    }

    // Orphans: little speech, nothing confident to merge with.
    let orphans: Vec<usize> = speech
        .iter()
        .filter(|(id, &ms)| ms < ORPHAN_MS && !prints.contains_key(id))
        .map(|(&id, _)| id)
        .collect();
    if !orphans.is_empty() && speech.len() - orphans.len() >= 1 {
        let anchors = |t: &SpeakerTurn| !orphans.contains(&t.speaker);
        let order: Vec<SpeakerTurn> = {
            let mut o = turns.clone();
            o.sort_by_key(|t| t.start_ms);
            o
        };
        for t in &mut turns {
            if !orphans.contains(&t.speaker) {
                continue;
            }
            let gap = |o: &SpeakerTurn| {
                if o.end_ms <= t.start_ms {
                    t.start_ms - o.end_ms
                } else {
                    o.start_ms.saturating_sub(t.end_ms)
                }
            };
            if let Some(near) = order.iter().filter(|o| anchors(o)).min_by_key(|o| gap(o)) {
                if gap(near) <= ORPHAN_REACH_MS {
                    t.speaker = near.speaker;
                }
            }
        }
        report.absorbed = orphans;
        speech.clear();
        for t in &turns {
            *speech.entry(t.speaker).or_default() += dur(t);
        }
    }

    // Compact labels in order of first appearance.
    let mut order: Vec<usize> = Vec::new();
    let mut sorted: Vec<&SpeakerTurn> = turns.iter().collect();
    sorted.sort_by_key(|t| t.start_ms);
    for t in sorted {
        if !order.contains(&t.speaker) {
            order.push(t.speaker);
        }
    }
    for t in &mut turns {
        t.speaker = order.iter().position(|&s| s == t.speaker).unwrap();
    }
    report.speakers_out = order.len();
    (turns, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(speaker: usize, a: u64, b: u64) -> SpeakerTurn {
        SpeakerTurn { start_ms: a, end_ms: b, speaker }
    }

    /// Audio whose first sample says which voice it is (one-hot print below).
    fn pcm_for(turns: &[SpeakerTurn], voice_of: &dyn Fn(usize) -> f32) -> Vec<f32> {
        let end = turns.iter().map(|t| t.end_ms).max().unwrap_or(0) as usize * 16;
        let mut pcm = vec![0.0; end];
        for t in turns {
            let v = voice_of(t.speaker);
            pcm[(t.start_ms * 16) as usize..(t.end_ms * 16) as usize].fill(v);
        }
        pcm
    }

    /// A print for each voice level: near-orthogonal for different voices,
    /// `mix` of two neighbouring voices when the audio blends them.
    fn print(p: &[f32]) -> Option<Vec<f32>> {
        let level = p.iter().sum::<f32>() / p.len() as f32;
        let mut e = vec![0.0; 4];
        let i = level.floor().clamp(0.0, 3.0) as usize;
        let frac = level - level.floor();
        e[i] = 1.0 - frac;
        if i + 1 < 4 {
            e[i + 1] = frac;
        }
        Some(e)
    }

    #[test]
    fn short_turns_of_one_voice_pool_into_a_print_and_merge() {
        // Speaker 0 talks at length; speaker 2 is the same voice as 0 in a few
        // short turns (each under a second): the old per-turn print never saw it.
        let turns = vec![
            turn(0, 0, 8_000),
            turn(1, 9_000, 17_000),
            turn(2, 18_000, 18_700),
            turn(2, 20_000, 20_600),
            turn(2, 22_000, 22_800),
            turn(0, 25_000, 33_000),
        ];
        let pcm = pcm_for(&turns, &|s| if s == 1 { 2.0 } else { 0.0 });
        let (out, report) = refine(turns, &pcm, &mut print, &RefineOptions::default());
        assert_eq!(report.speakers_in, 3);
        assert_eq!(report.speakers_out, 2, "{report:?}");
        assert_eq!(out[2].speaker, out[0].speaker);
        assert_ne!(out[1].speaker, out[0].speaker);
    }

    #[test]
    fn different_voices_are_never_merged() {
        let turns = vec![turn(0, 0, 8_000), turn(1, 9_000, 17_000), turn(2, 18_000, 26_000)];
        let pcm = pcm_for(&turns, &|s| s as f32);
        let (out, report) = refine(turns.clone(), &pcm, &mut print, &RefineOptions::default());
        assert_eq!(out, turns);
        assert_eq!(report.speakers_out, 3);
    }

    #[test]
    fn a_count_hint_cannot_fuse_distinct_people() {
        let turns = vec![turn(0, 0, 8_000), turn(1, 9_000, 17_000)];
        let pcm = pcm_for(&turns, &|s| s as f32 * 2.0);
        let (out, _) = refine(turns.clone(), &pcm, &mut print, &RefineOptions { expected: Some(1) });
        assert_eq!(out, turns);
    }

    #[test]
    fn timestamps_are_preserved() {
        let turns = vec![turn(0, 0, 6_000), turn(1, 7_000, 7_500), turn(0, 8_000, 14_000)];
        let pcm = pcm_for(&turns, &|_| 0.0);
        let (out, _) = refine(turns.clone(), &pcm, &mut print, &RefineOptions::default());
        let times = |v: &[SpeakerTurn]| v.iter().map(|t| (t.start_ms, t.end_ms)).collect::<Vec<_>>();
        assert_eq!(times(&out), times(&turns));
    }

    #[test]
    fn a_tiny_unprinted_speaker_follows_its_nearest_neighbour() {
        // Speaker 2 has 0.6 s in total and no usable print.
        let turns = vec![turn(0, 0, 8_000), turn(2, 8_200, 8_800), turn(1, 10_000, 18_000)];
        let pcm = pcm_for(&turns, &|s| if s == 1 { 2.0 } else { 0.0 });
        let (out, report) = refine(turns, &pcm, &mut print, &RefineOptions::default());
        assert_eq!(report.absorbed, vec![2]);
        assert_eq!(report.speakers_out, 2);
        assert_eq!(out[1].speaker, out[0].speaker);
    }

    #[test]
    fn nothing_changes_without_voiceprints() {
        let turns = vec![turn(0, 0, 8_000), turn(1, 9_000, 17_000), turn(2, 18_000, 19_000)];
        let pcm = vec![0.1; 19_000 * 16];
        let (out, report) = refine(turns.clone(), &pcm, &mut |_| None, &RefineOptions::default());
        // Without prints the 1 s speaker is an orphan and is attached; the two
        // long speakers stay apart.
        assert_eq!(report.speakers_out, 2);
        assert_ne!(out[0].speaker, out[1].speaker);
    }

    #[test]
    fn a_single_speaker_is_returned_as_is() {
        let turns = vec![turn(0, 0, 5_000), turn(0, 6_000, 9_000)];
        let (out, report) = refine(turns.clone(), &vec![0.1; 9_000 * 16], &mut print, &RefineOptions::default());
        assert_eq!(out, turns);
        assert_eq!(report.speakers_out, 1);
    }
}
