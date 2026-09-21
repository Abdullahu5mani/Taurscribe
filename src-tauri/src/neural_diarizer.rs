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

fn count_speakers(turns: &[SpeakerTurn]) -> usize {
    turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

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
