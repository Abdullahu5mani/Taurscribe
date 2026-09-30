/// VAD (Voice Activity Detection) Manager
///
/// Pure energy-based VAD: RMS threshold per 50ms frame, with hysteresis-based
/// segment detection for file transcription and a simple gate for live recording.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Frame size for energy VAD (50ms at 16kHz).
const CHUNK_SIZE: usize = 800;

pub struct VADManager;

impl VADManager {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }

    /// No-op — kept for call-site compatibility with the live recording path.
    pub fn reset_state(&mut self) {}

    /// Return a speech probability for `audio` (0.0 = silence, 1.0 = speech).
    pub fn is_speech(&mut self, audio: &[f32]) -> Result<f32, String> {
        Ok(Self::energy_vad(audio))
    }

    /// Scan `audio` in CHUNK_SIZE frames and return the peak speech probability.
    /// Stops early once a frame exceeds 0.5 (short-circuit: unambiguous speech found).
    pub fn max_speech_prob(&mut self, audio: &[f32], max_frames: usize) -> f32 {
        if audio.is_empty() || max_frames == 0 {
            return 0.0;
        }
        let mut peak: f32 = 0.0;
        for frame in audio.chunks(CHUNK_SIZE).take(max_frames) {
            let prob = Self::energy_vad(frame);
            if prob > peak {
                peak = prob;
            }
            if peak > 0.5 {
                break;
            }
        }
        peak
    }

    /// Energy-based speech probability.
    /// RMS < 0.005 (~-46 dBFS) → 0.0 (silence).
    /// RMS > 0.025 (~-32 dBFS) → 1.0 (speech).
    /// Linear ramp between.
    fn energy_vad(audio: &[f32]) -> f32 {
        if audio.is_empty() {
            return 0.0;
        }
        let rms = (audio.iter().map(|&x| x * x).sum::<f32>() / audio.len() as f32).sqrt();
        let threshold = 0.005_f32;
        if rms < threshold {
            0.0
        } else if rms > threshold * 5.0 {
            1.0
        } else {
            ((rms - threshold) / (threshold * 4.0)).min(1.0)
        }
    }

    /// Hysteresis-based segment finder used by file transcription.
    ///
    /// A segment STARTS when `prob > onset` and ENDS when `prob stays below offset`
    /// for longer than `padding_ms`. Callers pass in (onset, offset) pairs; for energy
    /// VAD, onset=0.5 / offset=0.2 works well (speech is ~1.0, silence is 0.0).
    pub fn get_speech_timestamps_hysteresis(
        &mut self,
        audio: &[f32],
        padding_ms: usize,
        onset: f32,
        offset: f32,
    ) -> Result<Vec<(f32, f32)>, String> {
        const SAMPLE_RATE: f32 = 16000.0;
        const MIN_SPEECH_FRAMES: usize = 2;

        let frame_ms = (CHUNK_SIZE as f32 / SAMPLE_RATE * 1000.0) as usize;
        let padding_frames = padding_ms / frame_ms.max(1);

        let mut segments = Vec::new();
        let mut speech_start: Option<usize> = None;
        let mut consecutive_speech = 0usize;
        let mut below_offset_frames = 0usize;
        let mut max_prob: f32 = 0.0;
        let mut frame_count: usize = 0;

        for (i, chunk) in audio.chunks(CHUNK_SIZE).enumerate() {
            let prob = Self::energy_vad(chunk);
            max_prob = max_prob.max(prob);
            frame_count += 1;

            match speech_start {
                None => {
                    if prob > onset {
                        speech_start = Some(i);
                        consecutive_speech = 1;
                        below_offset_frames = 0;
                    }
                }
                Some(_) => {
                    if prob >= offset {
                        consecutive_speech += 1;
                        below_offset_frames = 0;
                    } else {
                        below_offset_frames += 1;
                        if below_offset_frames > padding_frames {
                            if consecutive_speech >= MIN_SPEECH_FRAMES {
                                let start_idx =
                                    speech_start.unwrap().saturating_sub(padding_frames);
                                let end_idx = i;
                                segments.push((
                                    (start_idx * CHUNK_SIZE) as f32 / SAMPLE_RATE,
                                    (end_idx * CHUNK_SIZE) as f32 / SAMPLE_RATE,
                                ));
                            }
                            speech_start = None;
                            consecutive_speech = 0;
                            below_offset_frames = 0;
                        }
                    }
                }
            }
        }

        if let Some(start_idx) = speech_start {
            if consecutive_speech >= MIN_SPEECH_FRAMES {
                let start_idx = start_idx.saturating_sub(padding_frames);
                segments.push((
                    (start_idx * CHUNK_SIZE) as f32 / SAMPLE_RATE,
                    audio.len() as f32 / SAMPLE_RATE,
                ));
            }
        }

        let mut merged: Vec<(f32, f32)> = Vec::new();
        for seg in segments {
            if let Some(last) = merged.last_mut() {
                if seg.0 <= last.1 {
                    last.1 = seg.1.max(last.1);
                    continue;
                }
            }
            merged.push(seg);
        }

        println!(
            "[VAD] Found {} speech segment(s) (onset={}, offset={}, max_prob={:.3}, frames={})",
            merged.len(),
            onset,
            offset,
            max_prob,
            frame_count,
        );

        Ok(merged)
    }
}

/// Run **energy-based** VAD on the full audio, collect speech-only segments, and concatenate
/// them into a single buffer for the ASR. Silent sections are omitted.
pub fn assemble_speech_audio(
    mono: &[f32],
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Vec<f32>, String> {
    const SAMPLE_RATE: f32 = 16000.0;
    const ENERGY_FRAME_SAMPLES: usize = 800; // 50ms at 16kHz
    const MIN_SILENCE_FRAMES: usize = 16; // 800ms hangover
    const PAD_FRAMES: usize = 2; // 100ms padding each side

    if let Some(c) = cancel {
        if c.load(Ordering::Relaxed) {
            return Err("Transcription cancelled".to_string());
        }
    }

    // Compute per-frame RMS
    let frames: Vec<f32> = mono
        .chunks(ENERGY_FRAME_SAMPLES)
        .map(|f| (f.iter().map(|&x| x * x).sum::<f32>() / f.len() as f32).sqrt())
        .collect();

    if frames.is_empty() {
        return Ok(Vec::new());
    }

    // Adaptive threshold: set relative to the audio's own noise floor
    let mut sorted = frames.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let p10 = sorted[(n * 10 / 100).min(n.saturating_sub(1))];
    let p25 = sorted[(n * 25 / 100).min(n.saturating_sub(1))];
    let p50 = sorted[(n * 50 / 100).min(n.saturating_sub(1))];
    let p75 = sorted[(n * 75 / 100).min(n.saturating_sub(1))];

    let p95 = sorted[(n * 95 / 100).min(n.saturating_sub(1))];

    // Speech is what stands clearly above the background (p10), but the bar
    // never rises past 20 dB under the loudest speech (p95), so quiet and noisy
    // recordings keep their words. Stationary noise with no speech (p95 barely
    // above p10) stays under the ×2 minimum. Every term scales with the audio,
    // so a file's gain doesn't change what is kept; the absolute floor (−70 dBFS)
    // only drops digital silence.
    let energy_threshold = (p10 * 4.0).min(p95 * 0.1).max(p10 * 2.0).max(0.0003);

    println!(
        "[FILE_TRANSCRIBE] Energy VAD: frames={}, p10={:.5} p25={:.5} p50={:.5} p75={:.5} threshold={:.5}",
        frames.len(), p10, p25, p50, p75, energy_threshold
    );

    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut seg_start: Option<usize> = None;
    let mut silence_run = 0usize;

    for (i, &rms) in frames.iter().enumerate() {
        if i % 2048 == 0 {
            if let Some(c) = cancel {
                if c.load(Ordering::Relaxed) {
                    return Err("Transcription cancelled".to_string());
                }
            }
        }
        if rms >= energy_threshold {
            if seg_start.is_none() {
                seg_start = Some(i);
            }
            silence_run = 0;
        } else if let Some(start) = seg_start {
            silence_run += 1;
            if silence_run >= MIN_SILENCE_FRAMES {
                let seg_end = i - silence_run + 1;
                segments.push((start, seg_end));
                seg_start = None;
                silence_run = 0;
            }
        }
    }
    if let Some(start) = seg_start {
        segments.push((start, frames.len()));
    }

    if segments.is_empty() {
        println!(
            "[FILE_TRANSCRIBE] Energy VAD found no active speech — not sending silence to ASR"
        );
        return Ok(Vec::new());
    }

    println!(
        "[FILE_TRANSCRIBE] Energy VAD: {} segment(s)",
        segments.len()
    );

    let mut assembled = Vec::new();
    let nseg = segments.len();
    const LOG_EACH: usize = 8;
    for (i, (fs, fe)) in segments.iter().enumerate() {
        if i % 32 == 0 {
            if let Some(c) = cancel {
                if c.load(Ordering::Relaxed) {
                    return Err("Transcription cancelled".to_string());
                }
            }
        }
        let sample_start = fs.saturating_sub(PAD_FRAMES) * ENERGY_FRAME_SAMPLES;
        let sample_end = ((fe + PAD_FRAMES) * ENERGY_FRAME_SAMPLES).min(mono.len());
        let log_line =
            nseg <= LOG_EACH || i < LOG_EACH / 2 || i >= nseg.saturating_sub(LOG_EACH / 2);
        if log_line {
            println!(
                "  Energy segment {}: {:.2}s - {:.2}s",
                i + 1,
                sample_start as f32 / SAMPLE_RATE,
                sample_end as f32 / SAMPLE_RATE
            );
        } else if i == LOG_EACH / 2 {
            println!(
                "  Energy segment ... ({} segments omitted) ...",
                nseg.saturating_sub(LOG_EACH)
            );
        }
        assembled.extend_from_slice(&mono[sample_start..sample_end]);
    }

    Ok(assembled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f32, amp: f32) -> Vec<f32> {
        (0..(16_000.0 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16_000.0).sin() * amp)
            .collect()
    }

    fn silence(secs: f32) -> Vec<f32> {
        vec![0.0; (16_000.0 * secs) as usize]
    }

    #[test]
    fn energy_vad_ramps_between_thresholds() {
        let mut vad = VADManager::new().unwrap();
        assert_eq!(vad.is_speech(&[]).unwrap(), 0.0);
        assert_eq!(vad.is_speech(&[0.004; 800]).unwrap(), 0.0);
        assert_eq!(vad.is_speech(&[0.03; 800]).unwrap(), 1.0);
        let mid = vad.is_speech(&[0.015; 800]).unwrap();
        assert!((mid - 0.5).abs() < 1e-3, "{mid}");
    }

    #[test]
    fn max_speech_prob_stops_at_the_frame_limit() {
        let mut vad = VADManager::new().unwrap();
        let mut audio = silence(1.0);
        audio.extend(tone(0.5, 0.5));
        assert_eq!(vad.max_speech_prob(&audio, 10), 0.0);
        assert_eq!(vad.max_speech_prob(&audio, 1000), 1.0);
        assert_eq!(vad.max_speech_prob(&audio, 0), 0.0);
        assert_eq!(vad.max_speech_prob(&[], 10), 0.0);
    }

    #[test]
    fn hysteresis_finds_separate_speech_segments() {
        let mut vad = VADManager::new().unwrap();
        let mut audio = silence(1.0);
        audio.extend(tone(1.0, 0.3));
        audio.extend(silence(2.0));
        audio.extend(tone(0.5, 0.3));
        let segs = vad.get_speech_timestamps_hysteresis(&audio, 200, 0.5, 0.2).unwrap();
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert!((segs[0].0 - 0.8).abs() < 0.06, "{segs:?}");
        assert!(segs[0].1 > 2.0 && segs[0].1 < 2.4, "{segs:?}");
        assert!((segs[1].0 - 3.8).abs() < 0.06, "{segs:?}");
        assert!((segs[1].1 - audio.len() as f32 / 16_000.0).abs() < 1e-3, "open segment runs to the end");
    }

    #[test]
    fn hysteresis_ignores_single_frame_blips_and_merges_short_gaps() {
        let mut vad = VADManager::new().unwrap();
        let mut blip = silence(1.0);
        blip.extend(tone(0.05, 0.5));
        blip.extend(silence(1.0));
        assert!(vad.get_speech_timestamps_hysteresis(&blip, 100, 0.5, 0.2).unwrap().is_empty());

        let mut gap = tone(1.0, 0.3);
        gap.extend(silence(0.1));
        gap.extend(tone(1.0, 0.3));
        let segs = vad.get_speech_timestamps_hysteresis(&gap, 300, 0.5, 0.2).unwrap();
        assert_eq!(segs.len(), 1, "{segs:?}");
    }

    #[test]
    fn assembled_audio_drops_long_silences() {
        let mut audio = silence(2.0);
        audio.extend(tone(1.0, 0.3));
        audio.extend(silence(3.0));
        audio.extend(tone(1.0, 0.3));
        audio.extend(silence(2.0));
        let speech = assemble_speech_audio(&audio, None).unwrap();
        // Two seconds of speech plus 100 ms padding each side and the 800 ms hangover edge.
        assert!(speech.len() >= 32_000 && speech.len() <= 32_000 + 4 * 1_600 + 1_600, "{}", speech.len());
        assert!(speech.len() < audio.len() / 2);
    }

    #[test]
    fn assembled_audio_keeps_short_pauses() {
        let mut audio = tone(1.0, 0.3);
        audio.extend(silence(0.3));
        audio.extend(tone(1.0, 0.3));
        let speech = assemble_speech_audio(&audio, None).unwrap();
        assert_eq!(speech.len(), audio.len());
    }

    #[test]
    fn assembled_audio_of_silence_is_empty() {
        assert!(assemble_speech_audio(&silence(3.0), None).unwrap().is_empty());
        assert!(assemble_speech_audio(&[], None).unwrap().is_empty());
        // Steady hiss is not speech either.
        assert!(assemble_speech_audio(&vec![0.001; 48_000], None).unwrap().is_empty());
    }

    /// Deterministic white noise with peak `amp` (no RNG dependency).
    fn noise(secs: f32, amp: f32, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..(16_000.0 * secs) as usize)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn mix(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b).map(|(x, y)| x + y).collect()
    }

    #[test]
    fn quiet_speech_between_loud_speech_is_kept() {
        // A speaker 20 dB quieter than the others, with short pauses around them.
        let mut speech = tone(2.0, 0.3);
        speech.extend(silence(0.5));
        speech.extend(tone(1.5, 0.03));
        speech.extend(silence(0.5));
        speech.extend(tone(2.0, 0.3));
        let audio = mix(&speech, &noise(6.5, 0.001, 1));
        assert_eq!(assemble_speech_audio(&audio, None).unwrap().len(), audio.len());
    }

    #[test]
    fn what_is_kept_does_not_depend_on_the_recording_level() {
        let mut speech = tone(1.5, 0.2);
        speech.extend(silence(2.0));
        speech.extend(tone(1.5, 0.05));
        speech.extend(silence(2.0));
        speech.extend(tone(1.0, 0.02));
        let audio = mix(&speech, &noise(8.0, 0.002, 2));
        let kept = |gain: f32| {
            let scaled: Vec<f32> = audio.iter().map(|s| s * gain).collect();
            assemble_speech_audio(&scaled, None).unwrap().len()
        };
        let full = kept(1.0);
        assert!(full < audio.len(), "the long pauses should go");
        for gain in [0.5, 0.1, 0.03] {
            assert_eq!(kept(gain), full, "gain {gain}");
        }
    }

    #[test]
    fn speech_in_steady_noise_is_kept_and_noise_alone_is_not() {
        // About 10 dB SNR: the noise never stops, the speech comes in two bursts.
        let background = noise(6.0, 0.05, 3);
        let mut speech = silence(1.0);
        speech.extend(tone(1.5, 0.12));
        speech.extend(silence(2.0));
        speech.extend(tone(1.5, 0.12));
        let kept = assemble_speech_audio(&mix(&speech, &background), None).unwrap().len();
        assert!((48_000..=48_000 + 4 * 1_600).contains(&kept), "{kept}");
        assert!(assemble_speech_audio(&background, None).unwrap().is_empty());
    }

    #[test]
    fn assembled_audio_honours_cancellation() {
        let cancel = Arc::new(AtomicBool::new(true));
        assert!(assemble_speech_audio(&tone(1.0, 0.3), Some(&cancel)).is_err());
        cancel.store(false, Ordering::Relaxed);
        assert!(assemble_speech_audio(&tone(1.0, 0.3), Some(&cancel)).is_ok());
    }
}
