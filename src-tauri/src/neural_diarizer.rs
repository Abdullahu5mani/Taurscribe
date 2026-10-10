//! Neural speaker diarization for the call channel: NVIDIA Nemotron-3
//! Diarization (Streaming Sortformer v3, 100M params, up to 8 speakers).
//! Apple Silicon runs the MLX port (`nemotron_diar_mlx`, BF16); every other
//! platform runs the unquantized F16 GGUF through transcribe.cpp on the best
//! compiled-in device (CUDA / Vulkan / Metal / CPU).
//!
//! It answers "who spoke when" within one recording; speakers are numbered in
//! order of first appearance. Cross-meeting identity stays with CAM++ voiceprints.
//! Without the model the heuristic clusterer in `diarization` is used instead.

use crate::commands::model_registry::{DIARIZATION_MODEL_DIR, DIARIZATION_MODEL_FILE};
use std::path::PathBuf;

/// Where Settings → Models downloads the diarization model.
pub fn model_path() -> Option<PathBuf> {
    let p = crate::utils::get_models_dir()
        .ok()?
        .join(DIARIZATION_MODEL_DIR)
        .join(DIARIZATION_MODEL_FILE);
    p.exists().then_some(p)
}

/// One speaker turn in milliseconds; `speaker` is 0-based, in arrival order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeakerTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: usize,
}

/// Turns shorter than this are dropped (a cough or a word's tail).
const MIN_TURN_MS: u64 = 240;
/// Same-speaker turns closer than this are joined into one.
const JOIN_GAP_MS: u64 = 1_000;

/// Diarizes 16 kHz mono PCM. Loads the model for this call only, so its memory
/// is released afterwards (meetings are diarized once, after recording).
pub fn diarize_16k(pcm: &[f32]) -> Result<Vec<SpeakerTurn>, String> {
    let path = model_path().ok_or("diarization model not installed")?;
    let t = std::time::Instant::now();
    let (raw, backend) = run_model(&path, pcm)?;
    let turns = tidy_turns(raw);
    println!(
        "[DIARIZE] Nemotron-3 on {backend}: {:.1}s of audio in {:.2}s, {} turns, {} speakers",
        pcm.len() as f64 / 16_000.0,
        t.elapsed().as_secs_f64(),
        turns.len(),
        count_speakers(&turns)
    );
    Ok(turns)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn run_model(path: &std::path::Path, pcm: &[f32]) -> Result<(Vec<SpeakerTurn>, String), String> {
    let model = crate::nemotron_diar_mlx::NemotronDiarMlx::load(path)?;
    Ok((model.diarize(pcm)?, "MLX".to_string()))
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn run_model(path: &std::path::Path, pcm: &[f32]) -> Result<(Vec<SpeakerTurn>, String), String> {
    let model = transcribe_cpp::Model::load_with(path, &transcribe_cpp::ModelOptions::default())
        .map_err(|e| format!("load {}: {e}", path.display()))?;
    let backend = model.backend();
    let mut session = model.session().map_err(|e| format!("session: {e}"))?;
    // Default preset = NVIDIA's offline configuration (30.4 s lookahead), the
    // most accurate one; the recording is complete, so latency does not matter.
    let out = session
        .run(pcm, &transcribe_cpp::RunOptions::default())
        .map_err(|e| format!("diarize: {e}"))?;
    let turns = out
        .speaker_segments
        .iter()
        .filter(|s| s.speaker_id > 0 && s.t1_ms > s.t0_ms)
        .map(|s| SpeakerTurn {
            start_ms: s.t0_ms.max(0) as u64,
            end_ms: s.t1_ms.max(0) as u64,
            speaker: (s.speaker_id - 1) as usize,
        })
        .collect();
    Ok((turns, backend))
}

/// Makes the model's 10 ms-resolution segments usable as transcription turns:
/// sorted, no overlap (overlapped speech goes to whoever was talking first),
/// same-speaker neighbours joined, fragments dropped, speakers renumbered 0..n
/// in order of first appearance.
pub fn tidy_turns(mut turns: Vec<SpeakerTurn>) -> Vec<SpeakerTurn> {
    turns.sort_by_key(|t| (t.start_ms, t.end_ms));
    let mut out: Vec<SpeakerTurn> = Vec::new();
    for mut t in turns {
        if let Some(last) = out.last_mut() {
            if t.speaker == last.speaker && t.start_ms <= last.end_ms + JOIN_GAP_MS {
                last.end_ms = last.end_ms.max(t.end_ms);
                continue;
            }
            if t.start_ms < last.end_ms {
                t.start_ms = last.end_ms;
            }
        }
        if t.end_ms > t.start_ms {
            out.push(t);
        }
    }
    out.retain(|t| t.end_ms - t.start_ms >= MIN_TURN_MS);

    // Join again: dropping a fragment can leave same-speaker neighbours.
    let mut joined: Vec<SpeakerTurn> = Vec::new();
    for t in out {
        match joined.last_mut() {
            Some(last) if last.speaker == t.speaker && t.start_ms <= last.end_ms + JOIN_GAP_MS => {
                last.end_ms = last.end_ms.max(t.end_ms)
            }
            _ => joined.push(t),
        }
    }

    let mut order: Vec<usize> = Vec::new();
    for t in &joined {
        if !order.contains(&t.speaker) {
            order.push(t.speaker);
        }
    }
    for t in &mut joined {
        t.speaker = order.iter().position(|&s| s == t.speaker).unwrap();
    }
    joined
}

// ── voiceprint check (in-person meetings) ────────────────────────────────────
//
// On one microphone the separator can hear two similar voices as one person
// (two women with similar pitch, say). The speaker recognition model (CAM++)
// tells them apart far more reliably: on test meetings the same voice scored
// 0.69-0.91 against itself and similar different voices 0.42-0.53. So each
// speaker's turns are cut at their pauses, voiceprinted, and a speaker whose
// pieces form two clearly different voices is split in two.

/// Pieces at least this long get a voiceprint that can start a new voice.
const VOICEPRINT_PIECE_MS: u64 = 2_000;
/// Shorter pieces at least this long are matched to a voice by voiceprint
/// (too short to trust for finding voices, enough to pick between two);
/// even shorter ones follow a neighbour.
const MATCH_PIECE_MS: u64 = 800;
/// A short piece's voiceprint decides only when its best voice beats the
/// next by this much (short voiceprints are noisy).
const MATCH_MARGIN: f32 = 0.15;
/// Pieces whose average voiceprint similarity is at least this are one voice.
/// Below the vault's match threshold (0.60): splitting one person in two is
/// worse than leaving two similar voices together.
const SAME_VOICE_SIMILARITY: f32 = 0.55;
/// A split-off voice needs this much speech to count as another person.
const MIN_SPLIT_SPEECH_MS: u64 = 4_000;
/// Silence that separates two pieces inside a turn.
const PIECE_PAUSE_MS: u64 = 280;

/// The speech pieces of `turn`, cut where it pauses for PIECE_PAUSE_MS.
fn pause_pieces(pcm: &[f32], turn: &SpeakerTurn) -> Vec<(u64, u64)> {
    const FRAME_MS: u64 = 20;
    let at = |ms: u64| ((ms * 16) as usize).min(pcm.len());
    let (a, b) = (at(turn.start_ms), at(turn.end_ms));
    let frame = (FRAME_MS * 16) as usize;
    if b <= a || b - a < frame * 2 {
        return vec![(turn.start_ms, turn.end_ms)];
    }
    let rms: Vec<f32> = pcm[a..b]
        .chunks(frame)
        .map(|f| (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt())
        .collect();
    let mut sorted = rms.clone();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 10];
    let loud = sorted[sorted.len() * 9 / 10];
    // A turn that is mostly speech has no silence at its 10th percentile, so
    // the threshold never goes above half the speech level.
    let threshold = (floor * 3.0).max(loud * 0.12).min(loud * 0.5);
    let pause_frames = (PIECE_PAUSE_MS / FRAME_MS) as usize;

    let mut pieces = Vec::new();
    let (mut start, mut quiet) = (None::<usize>, 0usize);
    for (i, &r) in rms.iter().enumerate() {
        if r >= threshold {
            start.get_or_insert(i);
            quiet = 0;
        } else if let Some(s) = start {
            quiet += 1;
            if quiet >= pause_frames {
                pieces.push((s, i + 1 - quiet));
                start = None;
                quiet = 0;
            }
        }
    }
    if let Some(s) = start {
        pieces.push((s, rms.len() - quiet));
    }
    let to_ms = |f: usize| turn.start_ms + f as u64 * FRAME_MS;
    let pieces: Vec<(u64, u64)> = pieces.into_iter().filter(|(s, e)| e > s).map(|(s, e)| (to_ms(s), to_ms(e).min(turn.end_ms))).collect();
    if pieces.is_empty() { vec![(turn.start_ms, turn.end_ms)] } else { pieces }
}

/// Average-linkage clustering: groups of item indices whose average pairwise
/// similarity stays at or above `threshold`.
fn group_by_similarity(sim: &[Vec<f32>], threshold: f32) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = (0..sim.len()).map(|i| vec![i]).collect();
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for i in 0..groups.len() {
            for j in i + 1..groups.len() {
                let total: f32 = groups[i].iter().flat_map(|&a| groups[j].iter().map(move |&b| (a, b))).map(|(a, b)| sim[a][b]).sum();
                let avg = total / (groups[i].len() * groups[j].len()) as f32;
                if avg >= threshold && best.is_none_or(|(_, _, s)| avg > s) {
                    best = Some((i, j, avg));
                }
            }
        }
        let Some((i, j, _)) = best else { return groups };
        let merged = groups.remove(j);
        groups[i].extend(merged);
    }
}

/// Splits speakers whose turns hold two clearly different voices (see above).
/// `voiceprint` returns the speaker embedding of 16 kHz audio, or None.
pub fn split_mixed_speakers(
    turns: Vec<SpeakerTurn>,
    pcm: &[f32],
    voiceprint: &mut dyn FnMut(&[f32]) -> Option<Vec<f32>>,
) -> Vec<SpeakerTurn> {
    // Every piece, with the index of the turn it was cut from.
    let (mut pieces, turn_of): (Vec<SpeakerTurn>, Vec<usize>) = turns
        .iter()
        .enumerate()
        .flat_map(|(n, t)| pause_pieces(pcm, t).into_iter().map(move |(s, e)| (SpeakerTurn { start_ms: s, end_ms: e, speaker: t.speaker }, n)))
        .unzip();
    let mut next_speaker = count_speakers(&turns);
    let mut changed = false;
    for speaker in 0..count_speakers(&turns) {
        let mine: Vec<usize> = (0..pieces.len()).filter(|&i| pieces[i].speaker == speaker).collect();
        let printed: Vec<(usize, Vec<f32>)> = mine
            .iter()
            .filter(|&&i| pieces[i].end_ms - pieces[i].start_ms >= VOICEPRINT_PIECE_MS)
            .filter_map(|&i| {
                let p = &pieces[i];
                let audio = &pcm[(p.start_ms * 16) as usize..((p.end_ms * 16) as usize).min(pcm.len())];
                voiceprint(audio).filter(|e| !e.is_empty()).map(|e| (i, e))
            })
            .collect();
        if printed.len() < 2 {
            continue;
        }
        let sim: Vec<Vec<f32>> = printed
            .iter()
            .map(|(_, a)| printed.iter().map(|(_, b)| crate::speaker_embedding::cosine_similarity(a, b)).collect())
            .collect();
        let speech = |g: &[usize]| g.iter().map(|&k| pieces[printed[k].0].end_ms - pieces[printed[k].0].start_ms).sum::<u64>();
        let mut groups = group_by_similarity(&sim, SAME_VOICE_SIMILARITY);
        groups.sort_by_key(|g| std::cmp::Reverse(speech(g)));
        // Small leftovers are not another person: they join the closest voice.
        let (mut big, mut small) = (Vec::new(), Vec::new());
        for (n, g) in groups.into_iter().enumerate() {
            if n == 0 || speech(&g) >= MIN_SPLIT_SPEECH_MS { big.push(g) } else { small.push(g) }
        }
        if big.len() < 2 {
            continue;
        }
        let mut groups = big;
        for g in small {
            let avg = |h: &Vec<usize>| {
                g.iter().flat_map(|&a| h.iter().map(move |&b| (a, b))).map(|(a, b)| sim[a][b]).sum::<f32>() / (g.len() * h.len()) as f32
            };
            let target = (0..groups.len()).max_by(|&x, &y| avg(&groups[x]).total_cmp(&avg(&groups[y]))).unwrap();
            groups[target].extend(g);
        }
        // Each voice's pieces get its speaker. A shorter piece takes, in order:
        // a clear voiceprint match; the one voice of the rest of its own
        // turn (a sentence cut at a comma); the closest printed piece in time.
        let mut voice_of: Vec<(usize, u64, u64, usize)> = Vec::new(); // (turn, start, end, speaker)
        for (n, g) in groups.iter().enumerate() {
            let id = if n == 0 { speaker } else { next_speaker + n - 1 };
            for &k in g {
                let i = printed[k].0;
                pieces[i].speaker = id;
                voice_of.push((turn_of[i], pieces[i].start_ms, pieces[i].end_ms, id));
            }
        }
        let centroids: Vec<Vec<f32>> = groups
            .iter()
            .map(|g| {
                let mut c = vec![0.0; printed[g[0]].1.len()];
                for &k in g {
                    c.iter_mut().zip(&printed[k].1).for_each(|(a, b)| *a += b);
                }
                c
            })
            .collect();
        for &i in &mine {
            if printed.iter().any(|(j, _)| *j == i) {
                continue;
            }
            let p = pieces[i];
            let own: std::collections::BTreeSet<usize> = voice_of.iter().filter(|v| v.0 == turn_of[i]).map(|v| v.3).collect();
            if p.end_ms - p.start_ms >= MATCH_PIECE_MS {
                let audio = &pcm[(p.start_ms * 16) as usize..((p.end_ms * 16) as usize).min(pcm.len())];
                if let Some(e) = voiceprint(audio).filter(|e| !e.is_empty()) {
                    let mut sims: Vec<(f32, usize)> = centroids
                        .iter()
                        .enumerate()
                        .map(|(n, c)| (crate::speaker_embedding::cosine_similarity(&e, c), n))
                        .collect();
                    sims.sort_by(|a, b| b.0.total_cmp(&a.0));
                    if sims.len() < 2 || sims[0].0 - sims[1].0 >= MATCH_MARGIN {
                        let best = sims[0].1;
                        pieces[i].speaker = if best == 0 { speaker } else { next_speaker + best - 1 };
                        continue;
                    }
                }
            }
            if own.len() == 1 {
                pieces[i].speaker = *own.first().unwrap();
                continue;
            }
            let gap = |&(_, s, e, _): &(usize, u64, u64, usize)| if e <= p.start_ms { p.start_ms - e } else { s.saturating_sub(p.end_ms) };
            if let Some(&(_, _, _, id)) = voice_of.iter().min_by_key(|v| (v.0 != turn_of[i], gap(v))) {
                pieces[i].speaker = id;
            }
        }
        println!("[DIARIZE] Voiceprints split speaker {} into {} voices", speaker + 1, groups.len());
        next_speaker += groups.len() - 1;
        changed = true;
    }
    if changed { tidy_turns(pieces) } else { turns }
}

/// Merge duplicate voices across separator labels. Missing/ambiguous fingerprints
/// leave speakers intact: a brief contribution alone is not evidence of identity.
pub fn merge_same_voices(turns: Vec<SpeakerTurn>, pcm: &[f32], voiceprint: &mut dyn FnMut(&[f32]) -> Option<Vec<f32>>) -> Vec<SpeakerTurn> {
    merge_voices_with_hint(turns, pcm, voiceprint, None)
}

pub fn merge_voices_with_hint(mut turns: Vec<SpeakerTurn>, pcm: &[f32], voiceprint: &mut dyn FnMut(&[f32]) -> Option<Vec<f32>>, _expected: Option<usize>) -> Vec<SpeakerTurn> {
    use std::collections::BTreeMap;
    let mut prints: BTreeMap<usize, Vec<f32>> = BTreeMap::new();
    let mut speech: BTreeMap<usize, u64> = BTreeMap::new();
    for t in &turns { *speech.entry(t.speaker).or_default() += t.end_ms.saturating_sub(t.start_ms); }
    for &id in speech.keys() {
        let mut candidates: Vec<_> = turns.iter().filter(|t| t.speaker == id).collect();
        candidates.sort_by_key(|t| std::cmp::Reverse(t.end_ms.saturating_sub(t.start_ms)));
        let mut mean = Vec::<f32>::new();
        for t in candidates.into_iter().take(3) {
            let a = (t.start_ms.saturating_mul(16) as usize).min(pcm.len());
            let b = (t.end_ms.saturating_mul(16) as usize).min(pcm.len()).min(a + 16000 * 15);
            if b.saturating_sub(a) < 16000 { continue; }
            if let Some(e) = voiceprint(&pcm[a..b]).filter(|e| !e.is_empty() && e.iter().all(|v| v.is_finite())) {
                let norm = e.iter().map(|v| v*v).sum::<f32>().sqrt();
                if norm <= f32::EPSILON { continue; }
                if mean.is_empty() { mean.resize(e.len(), 0.0); }
                if mean.len() == e.len() { for (m,v) in mean.iter_mut().zip(e) { *m += v / norm; } }
            }
        }
        if !mean.is_empty() { prints.insert(id, mean); }
    }
    loop {
        let ids: Vec<_> = speech.keys().copied().collect();
        let long_meeting = turns.iter().map(|t| t.end_ms).max().unwrap_or(0) >= 60_000;
        let mut best = None;
        for (i, &a) in ids.iter().enumerate() {
            for &b in &ids[i+1..] {
                let (Some(pa), Some(pb)) = (prints.get(&a), prints.get(&b)) else { continue; };
                // Never collapse simultaneous speakers.
                if turns.iter().filter(|t| t.speaker == a).any(|x| turns.iter().filter(|t| t.speaker == b).any(|y| x.start_ms < y.end_ms && y.start_ms < x.end_ms)) { continue; }
                let sim = crate::speaker_embedding::cosine_similarity(pa, pb);
                let small = speech[&a].min(speech[&b]) < 4_000;
                // A count hint must not weaken identity evidence to force a
                // requested total. Only tiny fragments get the existing margin gate.
                let threshold = if long_meeting && small { 0.55 } else { 0.60 };
                // Below the ordinary cutoff demand a clear best match, preventing
                // ambiguous short fragments from being attached to a random voice.
                if sim < 0.60 {
                    let fragment = if speech[&a] < speech[&b] { a } else { b };
                    let other = if fragment == a { b } else { a };
                    let runner_up = ids.iter().filter(|&&id| id != fragment && id != other)
                        .filter_map(|id| prints.get(id)).map(|p| crate::speaker_embedding::cosine_similarity(&prints[&fragment], p))
                        .fold(-1.0f32, f32::max);
                    if sim - runner_up < 0.15 { continue; }
                }
                if sim >= threshold && best.is_none_or(|(_,_,score)| sim > score) { best = Some((a,b,sim)); }
            }
        }
        let Some((a,b,_)) = best else { break; };
        for t in &mut turns { if t.speaker == b { t.speaker = a; } }
        let wb = speech.remove(&b).unwrap(); let wa = speech[&a];
        let pb = prints.remove(&b).unwrap();
        let pa = prints.get_mut(&a).unwrap();
        let norm_a = pa.iter().map(|v| v*v).sum::<f32>().sqrt().max(f32::EPSILON);
        let norm_b = pb.iter().map(|v| v*v).sum::<f32>().sqrt().max(f32::EPSILON);
        for (x,y) in pa.iter_mut().zip(pb) { *x = *x / norm_a * wa as f32 + y / norm_b * wb as f32; }
        speech.insert(a,wa+wb);
    }
    // Preserve all timestamps, including brief turns; compact labels only.
    let mut labels = BTreeMap::new();
    for t in &mut turns { let next = labels.len(); t.speaker = *labels.entry(t.speaker).or_insert(next); }
    turns
}

fn count_speakers(turns: &[SpeakerTurn]) -> usize {
    turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16 kHz audio of `(voice, seconds)` parts separated by 0.5 s pauses;
    /// each voice is a tone at its own pitch.
    fn talk(parts: &[(usize, f32)]) -> (Vec<f32>, Vec<(u64, u64, usize)>) {
        let mut pcm = Vec::new();
        let mut spans = Vec::new();
        for &(voice, secs) in parts {
            pcm.extend(vec![0.0; 8_000]);
            let start = pcm.len() as u64 / 16;
            let hz = 150.0 + 90.0 * voice as f32;
            pcm.extend((0..(secs * 16_000.0) as usize).map(|n| 0.3 * (n as f32 * hz * std::f32::consts::TAU / 16_000.0).sin()));
            spans.push((start, pcm.len() as u64 / 16, voice));
        }
        pcm.extend(vec![0.0; 8_000]);
        (pcm, spans)
    }

    /// A stand-in voiceprint: which pitch the audio is, as a one-hot vector.
    fn pitch_print(audio: &[f32]) -> Option<Vec<f32>> {
        let crossings = audio.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
        let hz = crossings * 16_000.0 / audio.len() as f32;
        let voice = ((hz - 150.0) / 90.0).round().clamp(0.0, 3.0) as usize;
        let mut e = vec![0.05; 4];
        e[voice] = 1.0;
        Some(e)
    }

    #[test]
    fn a_turn_is_cut_at_its_pauses() {
        let (pcm, spans) = talk(&[(0, 2.0), (1, 3.0)]);
        let whole = SpeakerTurn { start_ms: spans[0].0, end_ms: spans[1].1, speaker: 0 };
        let pieces = pause_pieces(&pcm, &whole);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        for ((s, e), (ts, te, _)) in pieces.iter().zip(&spans) {
            assert!(s.abs_diff(*ts) <= 40 && e.abs_diff(*te) <= 40, "{s}-{e} vs {ts}-{te}");
        }
    }

    #[test]
    fn two_voices_heard_as_one_speaker_are_split() {
        // The separator put voices 0 and 1 together as speaker 0; voice 2 is speaker 1.
        let (pcm, spans) = talk(&[(0, 4.0), (1, 3.0), (2, 3.0), (0, 3.0), (1, 4.0)]);
        let turns = vec![
            SpeakerTurn { start_ms: spans[0].0, end_ms: spans[1].1, speaker: 0 },
            SpeakerTurn { start_ms: spans[2].0, end_ms: spans[2].1, speaker: 1 },
            SpeakerTurn { start_ms: spans[3].0, end_ms: spans[4].1, speaker: 0 },
        ];
        let out = split_mixed_speakers(turns, &pcm, &mut pitch_print);
        let speakers: Vec<usize> = out.iter().map(|t| t.speaker).collect();
        assert_eq!(speakers, [0, 1, 2, 0, 1], "{out:?}");
    }

    #[test]
    fn a_short_piece_follows_its_own_sentence_not_the_next_speaker() {
        // Voice 1 says a short phrase, pauses, then a long one; voice 0 answers
        // right after. The separator heard both as speaker 0.
        let (pcm, spans) = talk(&[(0, 4.0), (1, 1.2), (1, 3.0), (0, 3.5), (1, 3.0)]);
        let turns = vec![SpeakerTurn { start_ms: spans[0].0, end_ms: spans[4].1, speaker: 0 }];
        let out = split_mixed_speakers(turns, &pcm, &mut pitch_print);
        let voice_at = |ms: u64| out.iter().find(|t| t.start_ms <= ms && ms < t.end_ms).map(|t| t.speaker);
        // The 1.2 s phrase belongs with the 3 s one after it (same voice).
        assert_eq!(voice_at(spans[1].0 + 500), voice_at(spans[2].0 + 500), "{out:?}");
        assert_ne!(voice_at(spans[1].0 + 500), voice_at(spans[0].0 + 500), "{out:?}");
    }

    #[test]
    fn one_voice_is_never_split_and_short_leftovers_stay() {
        let (pcm, spans) = talk(&[(0, 4.0), (0, 3.0), (3, 2.5), (0, 3.0)]);
        // 2.5 s of a different voice is too little to be another person.
        let turns = vec![SpeakerTurn { start_ms: spans[0].0, end_ms: spans[3].1, speaker: 0 }];
        let out = split_mixed_speakers(turns.clone(), &pcm, &mut pitch_print);
        assert_eq!(out, turns);
        // Without voiceprints nothing changes either.
        assert_eq!(split_mixed_speakers(turns.clone(), &pcm, &mut |_| None), turns);
    }

    #[test]
    fn grouping_keeps_similar_items_together() {
        let sim = vec![
            vec![1.0, 0.8, 0.2, 0.3],
            vec![0.8, 1.0, 0.25, 0.2],
            vec![0.2, 0.25, 1.0, 0.7],
            vec![0.3, 0.2, 0.7, 1.0],
        ];
        let mut groups = group_by_similarity(&sim, 0.55);
        groups.iter_mut().for_each(|g| g.sort());
        groups.sort();
        assert_eq!(groups, vec![vec![0, 1], vec![2, 3]]);
        assert_eq!(group_by_similarity(&sim, 0.1).len(), 1);
    }

    fn t(start_ms: u64, end_ms: u64, speaker: usize) -> SpeakerTurn {
        SpeakerTurn { start_ms, end_ms, speaker }
    }

    #[test]
    fn joins_same_speaker_and_trims_overlap() {
        let got = tidy_turns(vec![t(0, 2000, 3), t(2500, 4000, 3), t(3500, 6000, 5)]);
        assert_eq!(got, vec![t(0, 4000, 0), t(4000, 6000, 1)]);
    }

    #[test]
    fn drops_fragments_then_rejoins() {
        let got = tidy_turns(vec![t(0, 2000, 0), t(2100, 2200, 1), t(2300, 5000, 0)]);
        assert_eq!(got, vec![t(0, 5000, 0)]);
    }

    #[test]
    fn renumbers_by_first_appearance() {
        let got = tidy_turns(vec![t(0, 1000, 4), t(3000, 4000, 2), t(6000, 7000, 4)]);
        assert_eq!(got.iter().map(|t| t.speaker).collect::<Vec<_>>(), vec![0, 1, 0]);
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    fn turn(id: usize, a: u64, b: u64) -> SpeakerTurn { SpeakerTurn { speaker:id,start_ms:a,end_ms:b } }
    #[test]
    fn duplicate_voices_merge_without_losing_timing_or_brief_real_speakers() {
        let turns = vec![turn(0,0,5000),turn(1,6000,11000),turn(2,12000,13000)];
        let mut pcm = vec![0.1;16000*13]; pcm[16000*12..].fill(0.8);
        let mut print = |p: &[f32]| Some(if p[0] > 0.5 { vec![0.0,1.0] } else { vec![1.0,0.0] });
        let out = merge_same_voices(turns.clone(), &pcm, &mut print);
        assert_eq!(out.iter().map(|t|(t.start_ms,t.end_ms)).collect::<Vec<_>>(),turns.iter().map(|t|(t.start_ms,t.end_ms)).collect::<Vec<_>>());
        assert_eq!(out[0].speaker,out[1].speaker); assert_ne!(out[0].speaker,out[2].speaker);
    }
    #[test]
    fn missing_prints_and_overlapping_voices_are_not_collapsed() {
        let turns = vec![turn(0,0,5000),turn(1,3000,7000)]; let pcm=vec![0.1;16000*7];
        assert_eq!(merge_same_voices(turns.clone(),&pcm,&mut |_| None),turns);
        assert_eq!(merge_same_voices(turns.clone(),&pcm,&mut |_| Some(vec![1.0,0.0])),turns);
    }
    #[test]
    fn count_hint_does_not_lower_identity_threshold() {
        let turns = vec![turn(0,0,5000),turn(1,6000,11000)];
        let mut pcm=vec![0.1;16000*11];pcm[16000*6..].fill(0.8);
        // Similar but below 0.60: the old hint-only 0.55 threshold merged them.
        let out=merge_voices_with_hint(turns.clone(),&pcm,&mut |p| Some(if p[0]<0.5 {vec![1.0,0.0]} else {vec![0.57,0.8216447]}),Some(1));
        assert_eq!(out,turns);
    }
    #[test]
    fn speaker_hint_does_not_force_distinct_people_into_one_voice() {
        let turns = vec![turn(0,0,5000),turn(1,6000,11000)];
        let mut pcm=vec![0.1;16000*11];pcm[16000*6..].fill(0.8);
        let out=merge_voices_with_hint(turns.clone(),&pcm,&mut |p| Some(if p[0]<0.5 {vec![1.0,0.0]} else {vec![0.0,1.0]}),Some(1));
        assert_eq!(out,turns);
    }
}
