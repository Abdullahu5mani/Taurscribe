//! Speaker Diarization & Audio Snippet Engine for Taurscribe
//!
//! Provides deterministic dual-channel speaker separation:
//! - Channel 0 (Mic / Left): 100% deterministic ground truth for local user ("You")
//! - Channel 1 (System Loopback / Right): Remote meeting participants
//!
//! For remote participants, acoustic energy segmentation and spectral feature
//! clustering separate conversational turns into distinct speakers, extracting
//! clean 3-second isolated audio snippets for instant naming and voiceprint vault storage.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single conversational turn spoken by an identified speaker
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiarizedTurn {
    pub speaker_id: String,
    pub speaker_name: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub channel: u8, // 0 = Mic (You), 1 = System Loopback (Callers)
    pub text: String,
    pub snippet_path: Option<String>,
    #[serde(default)]
    pub candidate_snippets: Vec<String>,
    #[serde(default)]
    pub current_snippet_idx: usize,
}

/// Extracted acoustic characteristics used to cluster speakers on the call channel
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AcousticFeatures {
    pub energy_rms: f32,
    pub spectral_centroid: f32,
    pub zero_crossing_rate: f32,
    pub pitch_proxy: f32,
}

impl AcousticFeatures {
    /// Normalized Euclidean distance between two acoustic feature sets
    pub fn distance(&self, other: &AcousticFeatures) -> f32 {
        let d_centroid = (self.spectral_centroid - other.spectral_centroid).abs() / 2000.0;
        let d_zcr = (self.zero_crossing_rate - other.zero_crossing_rate).abs() / 0.2;
        let d_pitch = (self.pitch_proxy - other.pitch_proxy).abs() / 150.0;
        (d_centroid * 0.45 + d_zcr * 0.35 + d_pitch * 0.20).min(1.0)
    }
}

/// Detected speaker cluster profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerProfile {
    pub speaker_id: String,
    pub speaker_name: String,
    pub channel: u8,
    pub features: AcousticFeatures,
    #[serde(default)]
    pub embedding: Vec<f32>,
    pub turn_count: usize,
    pub total_duration_ms: u64,
    pub snippet_path: Option<String>,
    #[serde(default)]
    pub candidate_snippets: Vec<String>,
}

/// Raw audio turn slice before text alignment
#[derive(Debug, Clone)]
pub struct AudioTurnSegment {
    pub channel: u8,
    pub start_sample: usize,
    pub end_sample: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub features: AcousticFeatures,
    pub cluster_id: usize,
}

/// Calculates RMS energy, zero-crossing rate, and spectral proxy for a mono audio slice
pub fn extract_acoustic_features(samples: &[f32], sample_rate: u32) -> AcousticFeatures {
    if samples.is_empty() {
        return AcousticFeatures::default();
    }

    // 1. RMS Energy
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    let energy_rms = (sum_sq / samples.len() as f32).sqrt();

    // 2. Zero-crossing rate
    let mut zcr_count = 0;
    for i in 1..samples.len() {
        if (samples[i] >= 0.0 && samples[i - 1] < 0.0) || (samples[i] < 0.0 && samples[i - 1] >= 0.0) {
            zcr_count += 1;
        }
    }
    let zero_crossing_rate = zcr_count as f32 / samples.len() as f32;

    // 3. Spectral Centroid approximation using first-order difference filter
    let mut num = 0.0_f32;
    let mut den = 0.0_f32;
    for i in 1..samples.len() {
        let diff = (samples[i] - samples[i - 1]).abs();
        let amp = samples[i].abs();
        num += diff * (sample_rate as f32 * 0.25);
        den += amp + 1e-6;
    }
    let spectral_centroid = (num / den).clamp(200.0, 5000.0);

    // 4. Pitch proxy via short-time normalized autocorrelation
    let max_lag = ((sample_rate as f32) / 75.0) as usize; // min pitch 75 Hz
    let min_lag = ((sample_rate as f32) / 350.0) as usize; // max pitch 350 Hz
    let mut best_corr = 0.0_f32;
    let mut best_lag = min_lag;

    let scan_len = samples.len().min(4800); // 100ms window
    if scan_len > max_lag {
        for lag in min_lag..max_lag {
            let mut corr = 0.0_f32;
            for j in 0..(scan_len - lag) {
                corr += samples[j] * samples[j + lag];
            }
            if corr > best_corr {
                best_corr = corr;
                best_lag = lag;
            }
        }
    }
    let pitch_proxy = if best_lag > 0 {
        sample_rate as f32 / best_lag as f32
    } else {
        150.0
    };

    AcousticFeatures {
        energy_rms,
        spectral_centroid,
        zero_crossing_rate,
        pitch_proxy,
    }
}

/// Detects conversational speech segments on a single channel of 48kHz audio
/// Speech/silence level for one channel. A fixed level drops quiet talkers:
/// call audio arrives well below a local mic (a caller through Meet sat mostly
/// under -36 dBFS, so two thirds of his words were never transcribed). Uses the
/// channel's own noise floor and speech level, never stricter than `max_rms`
/// and never below -54 dBFS.
pub fn adaptive_speech_threshold(frame_rms: &[f32], max_rms: f32) -> f32 {
    const MIN_RMS: f32 = 0.002;
    let mut sorted: Vec<f32> = frame_rms.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |p: f32| sorted[((sorted.len() - 1) as f32 * p) as usize];
    let noise_floor = pct(0.20);
    let speech_level = pct(0.95);
    (noise_floor * 4.0).max(speech_level * 0.08).clamp(MIN_RMS, max_rms.max(MIN_RMS))
}

pub fn detect_channel_speech_segments(
    samples: &[f32],
    sample_rate: u32,
    channel: u8,
    silence_threshold_rms: f32,
) -> Vec<AudioTurnSegment> {
    if samples.is_empty() {
        return Vec::new();
    }

    let frame_size = (sample_rate as usize * 40) / 1000; // 40ms frame (1920 samples @ 48kHz)
    let min_speech_frames = 6; // At least 240ms of continuous speech
    let max_silence_gap_frames = 12; // Merge pauses shorter than 480ms

    let mut segments = Vec::new();
    let num_frames = samples.len() / frame_size;
    if num_frames == 0 {
        return segments;
    }
    let frame_rms: Vec<f32> = (0..num_frames)
        .map(|f| {
            let frame = &samples[f * frame_size..((f + 1) * frame_size).min(samples.len())];
            (frame.iter().map(|&s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
        })
        .collect();
    let threshold = adaptive_speech_threshold(&frame_rms, silence_threshold_rms);
    let mut in_speech = false;
    let mut speech_start_frame = 0;
    let mut silence_frames = 0;

    for f in 0..num_frames {
        let rms = frame_rms[f];

        if rms >= threshold {
            if !in_speech {
                in_speech = true;
                speech_start_frame = f;
            }
            silence_frames = 0;
        } else if in_speech {
            silence_frames += 1;
            if silence_frames >= max_silence_gap_frames {
                // Speech ended
                let speech_end_frame = f - silence_frames + 1;
                if speech_end_frame > speech_start_frame + min_speech_frames {
                    let s_idx = speech_start_frame * frame_size;
                    let e_idx = (speech_end_frame * frame_size).min(samples.len());
                    let turn_slice = &samples[s_idx..e_idx];
                    let features = extract_acoustic_features(turn_slice, sample_rate);

                    let start_ms = ((s_idx as f64 / sample_rate as f64) * 1000.0) as u64;
                    let end_ms = ((e_idx as f64 / sample_rate as f64) * 1000.0) as u64;

                    segments.push(AudioTurnSegment {
                        channel,
                        start_sample: s_idx,
                        end_sample: e_idx,
                        start_ms,
                        end_ms,
                        features,
                        cluster_id: 0,
                    });
                }
                in_speech = false;
                silence_frames = 0;
            }
        }
    }

    // Flush trailing speech segment if audio ended during speech
    if in_speech {
        let speech_end_frame = num_frames;
        if speech_end_frame > speech_start_frame + min_speech_frames {
            let s_idx = speech_start_frame * frame_size;
            let e_idx = (speech_end_frame * frame_size).min(samples.len());
            let turn_slice = &samples[s_idx..e_idx];
            let features = extract_acoustic_features(turn_slice, sample_rate);

            let start_ms = ((s_idx as f64 / sample_rate as f64) * 1000.0) as u64;
            let end_ms = ((e_idx as f64 / sample_rate as f64) * 1000.0) as u64;

            segments.push(AudioTurnSegment {
                channel,
                start_sample: s_idx,
                end_sample: e_idx,
                start_ms,
                end_ms,
                features,
                cluster_id: 0,
            });
        }
    }

    // Merge close consecutive segments on the same channel if gap < 1000ms
    let mut merged: Vec<AudioTurnSegment> = Vec::new();
    for seg in segments {
        if let Some(last) = merged.last_mut() {
            if seg.start_ms.saturating_sub(last.end_ms) < 1000 {
                last.end_sample = seg.end_sample;
                last.end_ms = seg.end_ms;
                let s_idx = last.start_sample;
                let e_idx = last.end_sample.min(samples.len());
                if s_idx < e_idx {
                    last.features = extract_acoustic_features(&samples[s_idx..e_idx], sample_rate);
                }
                continue;
            }
        }
        merged.push(seg);
    }

    merged
}

/// Splits text into individual sentences while preserving sentence punctuation
pub fn split_into_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if ch == '.' || ch == '?' || ch == '!' || ch == '\n' {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_string());
            }
            current.clear();
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }

    if sentences.is_empty() && !text.trim().is_empty() {
        sentences.push(text.trim().to_string());
    }

    sentences
}

/// Clusters remote turn segments on Channel 1 into distinct speakers based on acoustic features
pub fn cluster_remote_turns(
    segments: &mut [AudioTurnSegment],
    distance_threshold: f32,
) -> Vec<SpeakerProfile> {
    if segments.is_empty() {
        return Vec::new();
    }

    let mut clusters: Vec<(AcousticFeatures, usize, u64)> = Vec::new(); // (centroid, count, total_ms)

    for seg in segments.iter_mut() {
        let mut best_cluster = None;
        let mut min_dist = distance_threshold;

        for (idx, (centroid, _, _)) in clusters.iter().enumerate() {
            let dist = seg.features.distance(centroid);
            if dist < min_dist {
                min_dist = dist;
                best_cluster = Some(idx);
            }
        }

        let cluster_idx = match best_cluster {
            Some(idx) => {
                // Update running centroid
                let (ref mut centroid, ref mut count, ref mut total_ms) = clusters[idx];
                let dur = seg.end_ms.saturating_sub(seg.start_ms);
                *count += 1;
                *total_ms += dur;
                centroid.spectral_centroid = (centroid.spectral_centroid * 0.7) + (seg.features.spectral_centroid * 0.3);
                centroid.zero_crossing_rate = (centroid.zero_crossing_rate * 0.7) + (seg.features.zero_crossing_rate * 0.3);
                centroid.pitch_proxy = (centroid.pitch_proxy * 0.7) + (seg.features.pitch_proxy * 0.3);
                idx
            }
            None => {
                let idx = clusters.len();
                let dur = seg.end_ms.saturating_sub(seg.start_ms);
                clusters.push((seg.features.clone(), 1, dur));
                idx
            }
        };

        seg.cluster_id = cluster_idx;
    }

    // A cluster with under 2 s of speech is a fragment (a word's tail, a cough, a
    // burst the acoustic features misread), not another person: fold it into the
    // closest real speaker. One caller came out as "Speaker 1" + a 0.8 s
    // "Speaker 2" on a Teams call.
    const MIN_SPEAKER_MS: u64 = 2_000;
    if clusters.iter().any(|c| c.2 >= MIN_SPEAKER_MS) {
        let mut remap: Vec<usize> = (0..clusters.len()).collect();
        for i in 0..clusters.len() {
            if clusters[i].2 >= MIN_SPEAKER_MS {
                continue;
            }
            let target = (0..clusters.len())
                .filter(|&j| clusters[j].2 >= MIN_SPEAKER_MS)
                .min_by(|&a, &b| {
                    clusters[i].0.distance(&clusters[a].0)
                        .partial_cmp(&clusters[i].0.distance(&clusters[b].0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap();
            remap[i] = target;
            clusters[target].1 += clusters[i].1;
            clusters[target].2 += clusters[i].2;
        }
        // Renumber the surviving clusters 0..n in order.
        let kept: Vec<usize> = (0..clusters.len()).filter(|&i| remap[i] == i).collect();
        for seg in segments.iter_mut() {
            seg.cluster_id = kept.iter().position(|&k| k == remap[seg.cluster_id]).unwrap_or(0);
        }
        clusters = kept.iter().map(|&k| clusters[k].clone()).collect();
    }

    clusters
        .into_iter()
        .enumerate()
        .map(|(i, (feat, count, dur))| SpeakerProfile {
            speaker_id: format!("speaker_remote_{}", i + 1),
            speaker_name: if clusters_count(segments) == 1 {
                "Remote Participant".to_string()
            } else {
                format!("Speaker {}", i + 1)
            },
            channel: 1,
            features: feat,
            embedding: Vec::new(),
            turn_count: count,
            total_duration_ms: dur,
            snippet_path: None,
            candidate_snippets: Vec::new(),
        })
        .collect()
}

fn clusters_count(segments: &[AudioTurnSegment]) -> usize {
    let mut ids: Vec<usize> = segments.iter().map(|s| s.cluster_id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

/// Extracts a clean 3-second isolated audio snippet (mono 16-bit PCM WAV) for a speaker
pub fn extract_3s_audio_snippet(
    samples: &[f32],
    sample_rate: u32,
    target_start_ms: u64,
    target_end_ms: u64,
    output_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let clip_duration_ms = 3000_u64;
    let turn_dur = target_end_ms.saturating_sub(target_start_ms);

    // Pick 3-second window centered in the turn or starting at turn start
    let (window_start_ms, window_end_ms) = if turn_dur <= clip_duration_ms {
        (target_start_ms, target_start_ms + clip_duration_ms)
    } else {
        let mid = (target_start_ms + target_end_ms) / 2;
        (
            mid.saturating_sub(clip_duration_ms / 2),
            mid + (clip_duration_ms / 2),
        )
    };

    let start_sample = ((window_start_ms as f64 / 1000.0) * sample_rate as f64) as usize;
    let end_sample = ((window_end_ms as f64 / 1000.0) * sample_rate as f64) as usize;

    let total_samples = sample_rate as usize * 3; // exactly 3s @ 48kHz = 144,000 samples
    let mut snippet_samples = Vec::with_capacity(total_samples);

    for idx in start_sample..end_sample {
        if idx < samples.len() {
            snippet_samples.push(samples[idx]);
        } else {
            snippet_samples.push(0.0_f32);
        }
    }

    // Write out 16-bit PCM WAV via hound
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(output_path, spec)
        .map_err(|e| format!("Failed to create WAV writer for snippet: {}", e))?;

    for &s in &snippet_samples {
        let val_i16 = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        writer
            .write_sample(val_i16)
            .map_err(|e| format!("Failed to write snippet sample: {}", e))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize snippet WAV: {}", e))?;

    Ok(())
}

/// Where a caller's combined speech for voiceprints lives, next to their
/// playback clip: `snippet_<meeting>_<speaker>[_cand_N].wav` -> `snippet_<meeting>_<speaker>_voice.wav`.
pub fn voice_sample_path(snippet_path: &str) -> std::path::PathBuf {
    let p = Path::new(snippet_path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("snippet");
    let base = match stem.rfind("_cand_") {
        Some(i) if stem[i + 6..].chars().all(|c| c.is_ascii_digit()) => &stem[..i],
        _ => stem,
    };
    p.with_file_name(format!("{}_voice.wav", base))
}

/// Writes the caller's turns back to back (up to 30 s) as 16-bit mono WAV.
/// Returns the seconds of speech written.
pub fn write_voice_sample(samples: &[f32], sample_rate: u32, turns: &[AudioTurnSegment], out: &Path) -> Option<f32> {
    const MAX_SECONDS: usize = 30;
    let cap = sample_rate as usize * MAX_SECONDS;
    let mut speech: Vec<f32> = Vec::new();
    for t in turns {
        let a = t.start_sample.min(samples.len());
        let b = t.end_sample.min(samples.len());
        speech.extend_from_slice(&samples[a..b]);
        if speech.len() >= cap {
            speech.truncate(cap);
            break;
        }
    }
    if speech.is_empty() {
        return None;
    }
    let spec = hound::WavSpec { channels: 1, sample_rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(out, spec).ok()?;
    for &x in &speech {
        w.write_sample((x.clamp(-1.0, 1.0) * 32767.0) as i16).ok()?;
    }
    w.finalize().ok()?;
    Some(speech.len() as f32 / sample_rate as f32)
}

/// Scores the purity of an audio turn segment for isolated voice snippet selection.
/// Penalizes crosstalk (if other channel has speech during the same window)
/// and prefers segments close to 3000ms duration with strong speech energy.
pub fn score_turn_purity(
    seg: &AudioTurnSegment,
    primary_samples: &[f32],
    crosstalk_samples: &[f32],
    _sample_rate: u32,
) -> f32 {
    let dur_ms = seg.end_ms.saturating_sub(seg.start_ms);
    if dur_ms < 600 {
        return 0.0;
    }

    // 1. Duration score (peak around 2500ms - 4500ms)
    let dur_score = if dur_ms >= 2000 && dur_ms <= 5000 {
        1.0
    } else if dur_ms < 2000 {
        dur_ms as f32 / 2000.0
    } else {
        (10000.0 - (dur_ms as f32).min(10000.0)) / 5000.0
    }
    .clamp(0.1, 1.0);

    // 2. Primary speech energy RMS
    let s_idx = seg.start_sample.min(primary_samples.len());
    let e_idx = seg.end_sample.min(primary_samples.len());
    let primary_rms = if s_idx < e_idx {
        let sum_sq: f32 = primary_samples[s_idx..e_idx].iter().map(|&s| s * s).sum();
        (sum_sq / (e_idx - s_idx) as f32).sqrt()
    } else {
        0.0
    };

    let energy_score = (primary_rms / 0.15).clamp(0.0, 1.0);

    // 3. Crosstalk penalty: evaluate other channel energy over the same sample window
    let mut crosstalk_penalty = 1.0_f32;
    if !crosstalk_samples.is_empty() {
        let c_s_idx = seg.start_sample.min(crosstalk_samples.len());
        let c_e_idx = seg.end_sample.min(crosstalk_samples.len());
        if c_s_idx < c_e_idx {
            let sum_sq: f32 = crosstalk_samples[c_s_idx..c_e_idx].iter().map(|&s| s * s).sum();
            let cross_rms = (sum_sq / (c_e_idx - c_s_idx) as f32).sqrt();
            if cross_rms > 0.025 {
                // Severe crosstalk penalty if the other channel is speaking simultaneously
                crosstalk_penalty = (0.025 / cross_rms).powi(2).clamp(0.01, 0.5);
            }
        }
    }

    (dur_score * 0.6 + energy_score * 0.4) * crosstalk_penalty
}

/// Extracts up to `max_candidates` (default 3) distinct, non-overlapping clean 3s voice snippets
/// for a speaker cluster, sorted by highest purity.
pub fn extract_candidate_snippets(
    primary_samples: &[f32],
    crosstalk_samples: &[f32],
    turns: &[AudioTurnSegment],
    sample_rate: u32,
    snippets_dir: &Path,
    prefix: &str,
    max_candidates: usize,
) -> Vec<String> {
    if turns.is_empty() {
        return Vec::new();
    }

    // Score all turns
    let mut scored_turns: Vec<(usize, f32)> = turns
        .iter()
        .enumerate()
        .map(|(idx, t)| {
            let score = score_turn_purity(t, primary_samples, crosstalk_samples, sample_rate);
            (idx, score)
        })
        .collect();

    // Sort descending by purity score
    scored_turns.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut candidate_paths = Vec::new();
    let mut chosen_intervals: Vec<(u64, u64)> = Vec::new();

    for (turn_idx, _score) in scored_turns {
        if candidate_paths.len() >= max_candidates {
            break;
        }

        let turn = &turns[turn_idx];
        // Ensure candidates don't excessively overlap (at least 1.5s apart)
        let overlaps = chosen_intervals.iter().any(|(s, e)| {
            let overlap_s = turn.start_ms.max(*s);
            let overlap_e = turn.end_ms.min(*e);
            overlap_s < overlap_e && overlap_e - overlap_s > 1000
        });

        if overlaps && !candidate_paths.is_empty() {
            continue;
        }

        let cand_num = candidate_paths.len() + 1;
        let filename = if cand_num == 1 {
            format!("{}.wav", prefix)
        } else {
            format!("{}_cand_{}.wav", prefix, cand_num)
        };
        let file_path = snippets_dir.join(&filename);

        if extract_3s_audio_snippet(
            primary_samples,
            sample_rate,
            turn.start_ms,
            turn.end_ms,
            &file_path,
        )
        .is_ok()
        {
            chosen_intervals.push((turn.start_ms, turn.end_ms));
            candidate_paths.push(file_path.to_string_lossy().to_string());
        }
    }

    // Fallback: if no candidates passed (e.g. all scored 0), take the longest turn
    if candidate_paths.is_empty() {
        if let Some(longest) = turns.iter().max_by_key(|t| t.end_ms.saturating_sub(t.start_ms)) {
            let file_path = snippets_dir.join(format!("{}.wav", prefix));
            if extract_3s_audio_snippet(
                primary_samples,
                sample_rate,
                longest.start_ms,
                longest.end_ms,
                &file_path,
            )
            .is_ok()
            {
                candidate_paths.push(file_path.to_string_lossy().to_string());
            }
        }
    }

    candidate_paths
}

/// Aligns a transcribed string or sentences to diarized turns
pub fn align_transcript_to_turns(
    mic_turns: &[AudioTurnSegment],
    remote_turns: &[AudioTurnSegment],
    remote_profiles: &[SpeakerProfile],
    raw_transcript: &str,
    snippets_dir: &Path,
    meeting_id: i64,
    mic_samples: &[f32],
    sys_samples: &[f32],
    sample_rate: u32,
    transcriber: Option<TurnTranscriber<'_>>,
) -> Vec<DiarizedTurn> {
    let mut all_turns: Vec<(u64, u64, u8, String, String, Option<String>, Vec<String>)> = Vec::new();

    // 1. Process local mic turns (Channel 0 = You)
    let you_candidates = extract_candidate_snippets(
        mic_samples,
        sys_samples,
        mic_turns,
        sample_rate,
        snippets_dir,
        &format!("snippet_{}_you", meeting_id),
        3,
    );
    let you_primary = you_candidates.first().cloned();

    for t in mic_turns {
        all_turns.push((
            t.start_ms,
            t.end_ms,
            0,
            "speaker_you".to_string(),
            "You".to_string(),
            you_primary.clone(),
            you_candidates.clone(),
        ));
    }

    // 2. Process remote call turns (Channel 1)
    for t in remote_turns {
        let profile = remote_profiles.get(t.cluster_id);
        let speaker_id = profile
            .map(|p| p.speaker_id.clone())
            .unwrap_or_else(|| "speaker_remote_1".to_string());
        let speaker_name = profile
            .map(|p| p.speaker_name.clone())
            .unwrap_or_else(|| "Remote Speaker".to_string());
        let snippet = profile.and_then(|p| p.snippet_path.clone());
        let candidates = profile
            .map(|p| p.candidate_snippets.clone())
            .unwrap_or_default();

        all_turns.push((
            t.start_ms,
            t.end_ms,
            1,
            speaker_id,
            speaker_name,
            snippet,
            candidates,
        ));
    }

    // Sort chronologically by start time
    all_turns.sort_by_key(|t| t.0);

    // If no acoustic turns were found, create a fallback single turn
    if all_turns.is_empty() {
        return vec![DiarizedTurn {
            speaker_id: "speaker_you".to_string(),
            speaker_name: "You".to_string(),
            start_ms: 0,
            end_ms: 1000,
            channel: 0,
            text: raw_transcript.trim().to_string(),
            snippet_path: None,
            candidate_snippets: Vec::new(),
            current_snippet_idx: 0,
        }];
    }

    // Consolidate consecutive turns by the same speaker if gap is small (< 1500ms)
    let mut consolidated: Vec<(u64, u64, u8, String, String, Option<String>, Vec<String>)> = Vec::new();
    for t in all_turns {
        if let Some(last) = consolidated.last_mut() {
            if last.3 == t.3 && t.0.saturating_sub(last.1) <= 1500 {
                last.1 = t.1.max(last.1);
                if last.5.is_none() {
                    last.5 = t.5;
                }
                if last.6.is_empty() {
                    last.6 = t.6;
                }
                continue;
            }
        }
        consolidated.push(t);
    }

    // Preferred: transcribe every turn from its OWN channel of the recording.
    // Text then belongs to the right speaker by construction (overlapping speech
    // is transcribed on both sides), and nothing the live transcriber dropped is
    // lost. The duration-weighted distribution below is only the fallback.
    if let Some(transcribe) = transcriber {
        let pad = (sample_rate as usize) / 4; // 250 ms of context either side
        let min_len = (sample_rate as usize) * 2 / 5; // skip < 400 ms blips (hallucination-prone)
        let mut per_turn = Vec::new();
        for t in &consolidated {
            let source = if t.2 == 0 { mic_samples } else { sys_samples };
            let start = ((t.0 as usize) * sample_rate as usize / 1000).saturating_sub(pad);
            let end = (((t.1 as usize) * sample_rate as usize / 1000) + pad).min(source.len());
            if end <= start || end - start < min_len {
                continue;
            }
            if let Some(text) = transcribe(&source[start..end], sample_rate) {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    per_turn.push(DiarizedTurn {
                        speaker_id: t.3.clone(),
                        speaker_name: t.4.clone(),
                        start_ms: t.0,
                        end_ms: t.1,
                        channel: t.2,
                        text,
                        snippet_path: t.5.clone(),
                        candidate_snippets: t.6.clone(),
                        current_snippet_idx: 0,
                    });
                }
            }
        }
        if !per_turn.is_empty() {
            println!("[DIARIZE] Transcribed {} turns from their own channels", per_turn.len());
            return per_turn;
        }
        println!("[DIARIZE] Per-turn transcription produced no text; falling back to transcript distribution");
    }

    let mut sentences = split_into_sentences(raw_transcript);

    // With fewer sentences than turns (unpunctuated or run-on text) whole
    // sentences cannot follow the speakers: one sentence used to land entirely
    // on the first turn and every other speaker's turns were dropped. Split by
    // words instead, so each turn gets its share in order.
    if sentences.len() < consolidated.len() {
        sentences = raw_transcript.split_whitespace().map(str::to_string).collect();
    }
    if sentences.is_empty() {
        let first = &consolidated[0];
        return vec![DiarizedTurn {
            speaker_id: first.3.clone(),
            speaker_name: first.4.clone(),
            start_ms: first.0,
            end_ms: first.1,
            channel: first.2,
            text: raw_transcript.trim().to_string(),
            snippet_path: first.5.clone(),
            candidate_snippets: first.6.clone(),
            current_snippet_idx: 0,
        }];
    }

    // Duration-weighted assignment of sentences to speech turns
    let total_chars: usize = sentences.iter().map(|s| s.len().max(1)).sum();
    let total_dur_ms: u64 = consolidated
        .iter()
        .map(|t| t.1.saturating_sub(t.0).max(1000))
        .sum();

    let mut turn_texts: Vec<Vec<String>> = vec![Vec::new(); consolidated.len()];
    let mut cum_chars = 0usize;

    for sent in &sentences {
        let sent_mid = cum_chars + (sent.len() / 2);
        cum_chars += sent.len();

        let progress = if total_chars > 0 {
            sent_mid as f64 / total_chars as f64
        } else {
            0.0
        };

        // Find corresponding turn based on cumulative duration
        let target_turn_idx = {
            let mut cum_dur = 0u64;
            let mut matched_idx = 0;
            for (idx, t) in consolidated.iter().enumerate() {
                let dur = t.1.saturating_sub(t.0).max(1000);
                cum_dur += dur;
                let turn_prog = cum_dur as f64 / total_dur_ms as f64;
                if progress <= turn_prog || idx == consolidated.len() - 1 {
                    matched_idx = idx;
                    break;
                }
            }
            matched_idx
        };

        turn_texts[target_turn_idx].push(sent.clone());
    }

    let mut result_turns = Vec::new();
    for (idx, t) in consolidated.into_iter().enumerate() {
        let joined_text = turn_texts[idx].join(" ");
        if !joined_text.trim().is_empty() {
            result_turns.push(DiarizedTurn {
                speaker_id: t.3,
                speaker_name: t.4,
                start_ms: t.0,
                end_ms: t.1,
                channel: t.2,
                text: joined_text,
                snippet_path: t.5,
                candidate_snippets: t.6,
                current_snippet_idx: 0,
            });
        }
    }

    if result_turns.is_empty() {
        result_turns.push(DiarizedTurn {
            speaker_id: "speaker_you".to_string(),
            speaker_name: "You".to_string(),
            start_ms: 0,
            end_ms: 1000,
            channel: 0,
            text: raw_transcript.trim().to_string(),
            snippet_path: None,
            candidate_snippets: Vec::new(),
            current_snippet_idx: 0,
        });
    }

    result_turns
}

/// Splits the call channel into speakers: Nemotron-3 Diarization when its model
/// is installed, otherwise energy segmentation + acoustic clustering.
fn split_remote_speakers(sys_samples: &[f32], sample_rate: u32) -> (Vec<AudioTurnSegment>, Vec<SpeakerProfile>) {
    if sys_samples.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if crate::neural_diarizer::model_path().is_some() {
        let neural = crate::audio_preprocess::resample_mono_to_16k(sys_samples, sample_rate)
            .and_then(|pcm| crate::neural_diarizer::diarize_16k(&pcm));
        match neural {
            Ok(turns) if !turns.is_empty() => return turns_to_segments(&turns, sys_samples, sample_rate),
            Ok(_) => println!("[DIARIZE] Nemotron-3 found no speech on the call channel; using energy segmentation"),
            Err(e) => eprintln!("[DIARIZE] Nemotron-3 failed ({e}); using acoustic clustering"),
        }
    }
    let mut turns = detect_channel_speech_segments(sys_samples, sample_rate, 1, 0.015);
    let profiles = cluster_remote_turns(&mut turns, 0.40);
    (turns, profiles)
}

/// Builds channel-1 turn segments and one profile per speaker from neural turns.
fn turns_to_segments(
    turns: &[crate::neural_diarizer::SpeakerTurn],
    samples: &[f32],
    sample_rate: u32,
) -> (Vec<AudioTurnSegment>, Vec<SpeakerProfile>) {
    let to_sample = |ms: u64| ((ms as u128 * sample_rate as u128 / 1000) as usize).min(samples.len());
    let segments: Vec<AudioTurnSegment> = turns
        .iter()
        .filter_map(|t| {
            let (s, e) = (to_sample(t.start_ms), to_sample(t.end_ms));
            (e > s).then(|| AudioTurnSegment {
                channel: 1,
                start_sample: s,
                end_sample: e,
                start_ms: t.start_ms,
                end_ms: t.end_ms,
                features: extract_acoustic_features(&samples[s..e], sample_rate),
                cluster_id: t.speaker,
            })
        })
        .collect();
    let n = segments.iter().map(|s| s.cluster_id + 1).max().unwrap_or(0);
    let profiles = (0..n)
        .map(|i| {
            let mine: Vec<&AudioTurnSegment> = segments.iter().filter(|s| s.cluster_id == i).collect();
            let longest = mine.iter().max_by_key(|s| s.end_ms - s.start_ms);
            SpeakerProfile {
                speaker_id: format!("speaker_remote_{}", i + 1),
                speaker_name: if n == 1 { "Remote Participant".to_string() } else { format!("Speaker {}", i + 1) },
                channel: 1,
                features: longest.map(|s| s.features.clone()).unwrap_or_default(),
                embedding: Vec::new(),
                turn_count: mine.len(),
                total_duration_ms: mine.iter().map(|s| s.end_ms - s.start_ms).sum(),
                snippet_path: None,
                candidate_snippets: Vec::new(),
            }
        })
        .collect();
    (segments, profiles)
}

/// Transcribes one turn's audio (mono samples at the given rate); None/empty = no text.
pub type TurnTranscriber<'a> = &'a mut dyn FnMut(&[f32], u32) -> Option<String>;

/// Convenience pipeline: loads a recorded WAV (mono or stereo), extracts channels,
/// detects speech segments, clusters remote callers, extracts 3s snippets, and aligns text.
pub fn diarize_meeting_recording(
    wav_path: &Path,
    raw_transcript: &str,
    snippets_dir: &Path,
    meeting_id: i64,
) -> Vec<DiarizedTurn> {
    diarize_meeting_recording_with(wav_path, raw_transcript, snippets_dir, meeting_id, None)
}

/// Like [`diarize_meeting_recording`], but transcribes each detected turn from its
/// own channel with `transcriber` instead of distributing `raw_transcript`.
pub fn diarize_meeting_recording_with(
    wav_path: &Path,
    raw_transcript: &str,
    snippets_dir: &Path,
    meeting_id: i64,
    transcriber: Option<TurnTranscriber<'_>>,
) -> Vec<DiarizedTurn> {
    let Ok(mut reader) = hound::WavReader::open(wav_path) else {
        return vec![DiarizedTurn {
            speaker_id: "speaker_you".to_string(),
            speaker_name: "You".to_string(),
            start_ms: 0,
            end_ms: 1000,
            channel: 0,
            text: raw_transcript.trim().to_string(),
            snippet_path: None,
            candidate_snippets: Vec::new(),
            current_snippet_idx: 0,
        }];
    };

    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels;

    let mut mic_samples = Vec::new();
    let mut sys_samples = Vec::new();

    let raw_samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let max_val = (1i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
            reader
                .samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| (s as f32 / max_val).clamp(-1.0, 1.0))
                .collect()
        }
    };

    if channels >= 2 {
        for (idx, sample) in raw_samples.iter().enumerate() {
            if idx % (channels as usize) == 0 {
                mic_samples.push(*sample);
            } else if idx % (channels as usize) == 1 {
                sys_samples.push(*sample);
            }
        }
    } else {
        mic_samples = raw_samples;
    }

    let mic_turns = detect_channel_speech_segments(&mic_samples, sample_rate, 0, 0.015);
    let (remote_turns, mut remote_profiles) = split_remote_speakers(&sys_samples, sample_rate);

    // Query enrolled vault speaker profiles for cross-meeting recognition
    let vault_speakers = crate::commands::meetings::open_connection()
        .ok()
        .and_then(|conn| crate::commands::meetings::load_vault_embeddings_internal(&conn).ok())
        .unwrap_or_default();

    // Vault person id -> (profile index, similarity): one person per meeting cluster.
    let mut vault_links: std::collections::HashMap<String, (usize, f32)> = std::collections::HashMap::new();

    // Extract candidate snippets and compute embeddings for remote profiles
    for (idx, profile) in remote_profiles.iter_mut().enumerate() {
        let cluster_turns: Vec<AudioTurnSegment> = remote_turns
            .iter()
            .filter(|t| t.cluster_id == idx)
            .cloned()
            .collect();

        let cands = extract_candidate_snippets(
            &sys_samples,
            &mic_samples,
            &cluster_turns,
            sample_rate,
            snippets_dir,
            &format!("snippet_{}_{}", meeting_id, profile.speaker_id),
            3,
        );

        if let Some(first) = cands.first() {
            profile.snippet_path = Some(first.clone());

            // The voiceprint comes from all of this caller's speech, not the 3 s
            // playback clip: more speech, fewer missed matches (see
            // MIN_VOICEPRINT_SECONDS).
            let voice_path = voice_sample_path(first);
            if let Some(secs) = write_voice_sample(&sys_samples, sample_rate, &cluster_turns, &voice_path) {
                if secs >= crate::speaker_embedding::MIN_VOICEPRINT_SECONDS {
                    if let Some(emb) = crate::speaker_embedding::embedding_from_wav(&voice_path.to_string_lossy()) {
                        profile.embedding = emb;
                    }
                } else {
                    println!(
                        "[INFO] Cluster {}: {:.1}s of speech, too little for a reliable voiceprint (need {:.0}s); not voice-matched",
                        profile.speaker_id, secs, crate::speaker_embedding::MIN_VOICEPRINT_SECONDS
                    );
                }
            }
        }
        profile.candidate_snippets = cands;

        // Auto-match against enrolled speaker vault: only with a neural speaker
        // model. The acoustic fallback scores different people ~0.98 alike and
        // would merge them; without a model, people are linked by name only.
        let neural = crate::speaker_embedding::get_speaker_engine()
            .lock()
            .map(|e| e.uses_neural_model())
            .unwrap_or(false);
        if neural && !profile.embedding.is_empty() && !vault_speakers.is_empty() {
            if let Some((v_id, v_name, sim)) = crate::speaker_embedding::match_speaker_against_vault(
                &profile.embedding,
                &vault_speakers,
                crate::speaker_embedding::match_threshold(),
            ) {
                println!(
                    "[INFO] Auto-recognized speaker: cluster {} matched enrolled vault speaker '{}' ({}) (similarity: {:.3})",
                    profile.speaker_id, v_name, v_id, sim
                );
                // Two different callers in one meeting are two people: if a
                // person already matched an earlier cluster better, keep that.
                let better_elsewhere = vault_links.get(&v_id).map(|(_, s)| *s >= sim).unwrap_or(false);
                if !better_elsewhere {
                    vault_links.insert(v_id.clone(), (idx, sim));
                    profile.speaker_id = v_id;
                    profile.speaker_name = v_name;
                }
            }
        }
    }

    // Undo weaker links that a later, closer cluster took over.
    for (idx, profile) in remote_profiles.iter_mut().enumerate() {
        if let Some((winner, _)) = vault_links.get(&profile.speaker_id) {
            if *winner != idx {
                profile.speaker_id = format!("speaker_remote_{}", idx + 1);
                profile.speaker_name = format!("Speaker {}", idx + 1);
            }
        }
    }

    align_transcript_to_turns(
        &mic_turns,
        &remote_turns,
        &remote_profiles,
        raw_transcript,
        snippets_dir,
        meeting_id,
        &mic_samples,
        &sys_samples,
        sample_rate,
        transcriber,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_acoustic_features_synthetic() {
        let sample_rate = 48000;
        let mut sine = vec![0.0_f32; 48000]; // 1s sine at 440 Hz
        for (i, s) in sine.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * 440.0 * (i as f32) / (sample_rate as f32)).sin() * 0.5;
        }

        let feat = extract_acoustic_features(&sine, sample_rate);
        assert!(feat.energy_rms > 0.3);
        assert!(feat.zero_crossing_rate > 0.01);
        assert!(feat.spectral_centroid > 200.0);
    }

    #[test]
    fn test_detect_channel_speech_segments() {
        let sample_rate = 48000;
        let mut audio = vec![0.0_f32; 48000 * 3]; // 3s total

        // Insert 1s of speech from 0.5s to 1.5s
        for i in 24000..72000 {
            audio[i] = ((i as f32) * 0.05).sin() * 0.4;
        }

        let segments = detect_channel_speech_segments(&audio, sample_rate, 0, 0.05);
        assert!(!segments.is_empty());
        assert!(segments[0].start_ms >= 400 && segments[0].start_ms <= 600);
        assert!(segments[0].end_ms >= 1400);
    }

    #[test]
    fn test_cluster_remote_turns() {
        let mut turns = vec![
            AudioTurnSegment {
                channel: 1,
                start_sample: 0,
                end_sample: 48000,
                start_ms: 0,
                end_ms: 1000,
                features: AcousticFeatures {
                    energy_rms: 0.2,
                    spectral_centroid: 1200.0,
                    zero_crossing_rate: 0.04,
                    pitch_proxy: 130.0,
                },
                cluster_id: 0,
            },
            AudioTurnSegment {
                channel: 1,
                start_sample: 96000,
                end_sample: 144000,
                start_ms: 2000,
                end_ms: 3000,
                features: AcousticFeatures {
                    energy_rms: 0.22,
                    spectral_centroid: 2800.0, // Significantly higher pitch/centroid
                    zero_crossing_rate: 0.12,
                    pitch_proxy: 240.0,
                },
                cluster_id: 0,
            },
        ];

        let profiles = cluster_remote_turns(&mut turns, 0.25);
        assert_eq!(profiles.len(), 2);
        assert_ne!(turns[0].cluster_id, turns[1].cluster_id);
    }

    #[test]
    fn test_extract_3s_audio_snippet() {
        let sample_rate = 48000;
        let audio = vec![0.1_f32; 48000 * 5]; // 5 seconds
        let out_path = std::env::temp_dir().join("test_snippet_3s.wav");

        let res = extract_3s_audio_snippet(&audio, sample_rate, 1000, 2000, &out_path);
        assert!(res.is_ok());
        assert!(out_path.exists());

        let reader = hound::WavReader::open(&out_path).unwrap();
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 48000);
        assert_eq!(reader.duration(), 48000 * 3); // exactly 3 seconds

        let _ = std::fs::remove_file(&out_path);
    }

    #[test]
    fn test_split_into_sentences() {
        let text = "Hello everyone! Welcome to the meeting. Can everyone hear me?";
        let sentences = split_into_sentences(text);
        assert_eq!(sentences.len(), 3);
        assert_eq!(sentences[0], "Hello everyone!");
        assert_eq!(sentences[1], "Welcome to the meeting.");
        assert_eq!(sentences[2], "Can everyone hear me?");
    }

    #[test]
    fn test_align_transcript_to_turns_duration_weighted() {
        let mic_turns = vec![AudioTurnSegment {
            channel: 0,
            start_sample: 0,
            end_sample: 48000 * 6,
            start_ms: 0,
            end_ms: 6000, // 6 seconds long
            features: AcousticFeatures::default(),
            cluster_id: 0,
        }];

        let remote_turns = vec![AudioTurnSegment {
            channel: 1,
            start_sample: 48000 * 7,
            end_sample: 48000 * 9,
            start_ms: 7000,
            end_ms: 9000, // 2 seconds long
            features: AcousticFeatures::default(),
            cluster_id: 0,
        }];

        let profiles = vec![SpeakerProfile {
            speaker_id: "speaker_remote_1".to_string(),
            speaker_name: "Alice".to_string(),
            channel: 1,
            features: AcousticFeatures::default(),
            embedding: Vec::new(),
            turn_count: 1,
            total_duration_ms: 2000,
            snippet_path: None,
            candidate_snippets: Vec::new(),
        }];

        let raw_transcript = "We completed the architecture plan. All services are migrated. Sounds awesome!";
        let snippets_dir = std::env::temp_dir();
        let mic_dummy = vec![0.0_f32; 48000 * 10];
        let sys_dummy = vec![0.0_f32; 48000 * 10];

        let turns = align_transcript_to_turns(
            &mic_turns,
            &remote_turns,
            &profiles,
            raw_transcript,
            &snippets_dir,
            999,
            &mic_dummy,
            &sys_dummy,
            48000,
            None,
        );

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].speaker_name, "You");
        assert!(turns[0].text.contains("architecture plan"));
        assert_eq!(turns[1].speaker_name, "Alice");
        assert!(turns[1].text.contains("awesome"));
    }

    #[test]
    fn test_score_turn_purity_crosstalk_penalty() {
        let seg = AudioTurnSegment {
            channel: 1,
            start_sample: 0,
            end_sample: 48000 * 3,
            start_ms: 0,
            end_ms: 3000,
            features: AcousticFeatures::default(),
            cluster_id: 0,
        };

        let primary_audio = vec![0.1_f32; 48000 * 3];
        let clean_crosstalk = vec![0.0_f32; 48000 * 3];
        let loud_crosstalk = vec![0.2_f32; 48000 * 3]; // Loud user speaking over caller

        let clean_score = score_turn_purity(&seg, &primary_audio, &clean_crosstalk, 48000);
        let noisy_score = score_turn_purity(&seg, &primary_audio, &loud_crosstalk, 48000);

        assert!(clean_score > 0.6);
        assert!(noisy_score < clean_score);
        assert!(noisy_score < 0.45);
    }

    #[test]
    fn test_unpunctuated_transcript_keeps_every_speaker() {
        let seg = |ch: u8, start: u64, end: u64| AudioTurnSegment {
            channel: ch, start_sample: 0, end_sample: 0, start_ms: start, end_ms: end,
            features: AcousticFeatures::default(), cluster_id: 0,
        };
        let mic = vec![seg(0, 0, 9_000)];
        let remote = vec![seg(1, 9_500, 29_000)];
        let profiles = cluster_remote_turns(&mut remote.clone(), 0.40);
        let text = "also a popular contrivance whereby love making may be suspended he hoped there would be stew for dinner turnips and carrots and bruised potatoes and fat mutton pieces to be ladled out in thick peppered flour fattened sauce";
        let dir = std::env::temp_dir().join(format!("taurscribe_align_{}", std::process::id()));
        let turns = align_transcript_to_turns(&mic, &remote, &profiles, text, &dir, 1, &[], &[], 16_000, None);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(turns.iter().any(|t| t.channel == 0) && turns.iter().any(|t| t.channel == 1),
                "both speakers kept: {:?}", turns.iter().map(|t| (t.channel, t.text.len())).collect::<Vec<_>>());
        let words: usize = turns.iter().map(|t| t.text.split_whitespace().count()).sum();
        assert_eq!(words, text.split_whitespace().count(), "no words lost");
    }

    #[test]
    fn test_short_fragment_is_not_a_second_speaker() {
        let feat = |c: f32| AcousticFeatures { spectral_centroid: c, zero_crossing_rate: 0.1, pitch_proxy: 120.0, ..Default::default() };
        let seg = |start: u64, end: u64, c: f32| AudioTurnSegment {
            channel: 1, start_sample: 0, end_sample: 0, start_ms: start, end_ms: end, features: feat(c), cluster_id: 0,
        };
        // 6 s of one voice, then a 0.8 s burst that looks acoustically different.
        let mut segs = vec![seg(0, 3_000, 1500.0), seg(3_500, 6_500, 1520.0), seg(7_000, 7_800, 3400.0)];
        let profiles = cluster_remote_turns(&mut segs, 0.40);
        assert_eq!(profiles.len(), 1);
        assert!(segs.iter().all(|s| s.cluster_id == 0));
        assert_eq!(profiles[0].speaker_name, "Remote Participant");
    }

    #[test]
    fn test_quiet_caller_is_detected() {
        // 3 s of speech-like tone at -46 dBFS between silences: under the old
        // fixed 0.015 level this was dropped entirely.
        let sr = 48_000u32;
        let mut samples = vec![0.0f32; sr as usize];
        samples.extend((0..3 * sr as usize).map(|i| 0.007 * (i as f32 * 0.05).sin()));
        samples.extend(vec![0.0f32; sr as usize]);
        let segs = detect_channel_speech_segments(&samples, sr, 1, 0.015);
        let speech_ms: u64 = segs.iter().map(|s| s.end_ms - s.start_ms).sum();
        assert!(speech_ms >= 2_800, "detected {} ms of 3000", speech_ms);
        // Silence stays silence.
        assert!(detect_channel_speech_segments(&vec![0.0f32; 3 * sr as usize], sr, 1, 0.015).is_empty());
    }

    #[test]
    fn test_voice_sample_path_follows_snippet_and_candidates() {
        let want = Path::new("/m/snippets/snippet_7_speaker_remote_1_voice.wav");
        assert_eq!(voice_sample_path("/m/snippets/snippet_7_speaker_remote_1.wav"), want);
        assert_eq!(voice_sample_path("/m/snippets/snippet_7_speaker_remote_1_cand_3.wav"), want);
    }

    #[test]
    fn test_extract_candidate_snippets_ranking() {
        let sample_rate = 48000;
        let mut primary = vec![0.0_f32; 48000 * 15]; // 15 seconds
        // Turn 1: 0..3s - low volume
        for i in 0..48000 * 3 {
            primary[i] = 0.01;
        }
        // Turn 2: 5..8s - clear vocal speech
        for i in 48000 * 5..48000 * 8 {
            primary[i] = 0.15;
        }

        let crosstalk = vec![0.0_f32; primary.len()];

        let turns = vec![
            AudioTurnSegment {
                channel: 1,
                start_sample: 0,
                end_sample: 48000 * 3,
                start_ms: 0,
                end_ms: 3000,
                features: AcousticFeatures::default(),
                cluster_id: 0,
            },
            AudioTurnSegment {
                channel: 1,
                start_sample: 48000 * 5,
                end_sample: 48000 * 8,
                start_ms: 5000,
                end_ms: 8000,
                features: AcousticFeatures::default(),
                cluster_id: 0,
            },
        ];

        let tmp_dir = std::env::temp_dir();
        let cands = extract_candidate_snippets(
            &primary,
            &crosstalk,
            &turns,
            sample_rate,
            &tmp_dir,
            "test_cand_rank",
            2,
        );

        assert!(!cands.is_empty());
        // Clean up
        for p in cands {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn per_turn_transcription_uses_each_turns_own_channel() {
        // Mic channel is constant 0.1, call channel constant 0.3: the fake
        // transcriber reports which channel it was handed, so attribution is
        // checkable without an ASR model. The turns overlap in time (crosstalk).
        let rate = 16000u32;
        let mic = vec![0.1f32; rate as usize * 4];
        let sys = vec![0.3f32; rate as usize * 4];
        let seg = |channel: u8, start_ms: u64, end_ms: u64| AudioTurnSegment {
            channel,
            start_sample: (start_ms * rate as u64 / 1000) as usize,
            end_sample: (end_ms * rate as u64 / 1000) as usize,
            start_ms,
            end_ms,
            features: AcousticFeatures::default(),
            cluster_id: 0,
        };
        let mic_turns = vec![seg(0, 0, 2000)];
        let remote_turns = vec![seg(1, 1000, 3500)];
        let dir = std::env::temp_dir().join("taurscribe_per_turn_test");
        let _ = std::fs::create_dir_all(&dir);

        let mut calls = 0;
        let mut fake = |samples: &[f32], _rate: u32| -> Option<String> {
            calls += 1;
            let mean = samples.iter().sum::<f32>() / samples.len() as f32;
            Some(if (mean - 0.1).abs() < 0.01 { "said on mic".into() } else { "said on call".into() })
        };
        let turns = align_transcript_to_turns(
            &mic_turns, &remote_turns, &[], "ignored when per-turn text exists",
            &dir, 1, &mic, &sys, rate, Some(&mut fake),
        );

        assert_eq!(calls, 2);
        assert_eq!(turns.len(), 2);
        assert_eq!((turns[0].channel, turns[0].text.as_str()), (0, "said on mic"));
        assert_eq!((turns[1].channel, turns[1].text.as_str()), (1, "said on call"));
    }
}
