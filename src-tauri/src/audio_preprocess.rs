//! Universal ASR preprocessing (plan: mono → 16 kHz → optional edge trim (files) →
//! optional RNNoise @ 48 kHz (live) → resample → DC removal → conditional high-pass →
//! conditional level assist → clamp). All three engines consume 16 kHz mono f32.
//!
//! RNNoise (`nnnoiseless`) only accepts **48 kHz** frames. Live path denoises at native
//! rate when `sample_rate == 48000`; file path may denoise via exact 16k↔48k (×3) resample
//! when the noise heuristic fires.

use crate::denoise::Denoiser;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::borrow::Cow;

// ── Policy thresholds (tunable) ─────────────────────────────────────────────

/// Frame length for edge / noise analysis (ms at 16 kHz).
pub const FRAME_MS_16K: usize = 20;
/// Minimum contiguous edge silence (ms) before we trim (avoid nipping weak starts).
pub const EDGE_MIN_SILENCE_MS: usize = 400;
/// Frame RMS must exceed `noise_floor * this` to count as non-silence for edge trim.
pub const EDGE_RMS_GATE_FACTOR: f32 = 2.8;
/// If low-frequency proxy energy / total RMS exceeds this, apply a gentle high-pass.
pub const LF_EXCESS_RATIO: f32 = 0.38;
/// Moving-average length for LF proxy (~50 ms at 16 kHz).
pub const LF_MA_SAMPLES: usize = 800;
/// Peak frame RMS / (noise_floor + eps) below this ⇒ treat as noisy (apply denoise when enabled).
pub const SNR_PEAK_TO_FLOOR_MIN: f32 = 12.0;
/// Apply level assist only when global RMS is below this (linear, ~-29 dBFS).
pub const QUIET_RMS_THRESHOLD: f32 = 0.038;
/// Target RMS when applying level assist (-20 dBFS).
pub const LEVEL_TARGET_RMS: f32 = 0.1;
/// Maximum gain in level assist (+20 dB cap).
pub const MAX_GAIN_LINEAR: f32 = 10.0;

const SINC_PARAMS: SincInterpolationParameters = SincInterpolationParameters {
    sinc_len: 64,
    f_cutoff: 0.95,
    interpolation: SincInterpolationType::Linear,
    window: WindowFunction::BlackmanHarris2,
    oversampling_factor: 32,
};

const RESAMPLE_CHUNK: usize = 1024 * 10;

/// Preserve the sinc filter's state while a file is decoded packet by packet.
/// Only a partial input block is retained between calls.
pub struct StreamingResampler16k {
    from_rate: u32,
    resampler: Option<SincFixedIn<f32>>,
    pending: Vec<f32>,
    input: Vec<Vec<f32>>,
    total_input: u64,
    total_output: usize,
}

impl StreamingResampler16k {
    pub fn new(from_rate: u32) -> Result<Self, String> {
        if from_rate == 0 {
            return Err("File has invalid sample rate".into());
        }
        let resampler = if from_rate == 16_000 {
            None
        } else {
            Some(
                SincFixedIn::<f32>::new(
                    16_000.0 / from_rate as f64,
                    2.0,
                    SINC_PARAMS,
                    RESAMPLE_CHUNK,
                    1,
                )
                .map_err(|e| format!("Resampler init failed: {e:?}"))?,
            )
        };
        Ok(Self {
            from_rate,
            resampler,
            pending: Vec::with_capacity(RESAMPLE_CHUNK),
            input: vec![Vec::with_capacity(RESAMPLE_CHUNK)],
            total_input: 0,
            total_output: 0,
        })
    }

    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>, String> {
        self.total_input = self.total_input.saturating_add(samples.len() as u64);
        if self.resampler.is_none() {
            self.total_output += samples.len();
            return Ok(samples.to_vec());
        }
        self.pending.extend_from_slice(samples);
        let mut out = Vec::new();
        while self.pending.len() >= RESAMPLE_CHUNK {
            self.input[0].clear();
            self.input[0].extend(self.pending.drain(..RESAMPLE_CHUNK));
            let block = self
                .resampler
                .as_mut()
                .unwrap()
                .process(&self.input, None)
                .map_err(|e| format!("Resample failed: {e:?}"))?;
            out.extend_from_slice(&block[0]);
        }
        self.total_output += out.len();
        Ok(out)
    }

    pub fn finish(&mut self) -> Result<Vec<f32>, String> {
        if self.resampler.is_none() {
            return Ok(Vec::new());
        }
        let target = ((self.total_input as u128 * 16_000 + self.from_rate as u128 / 2)
            / self.from_rate as u128) as usize;
        if self.total_output >= target {
            self.pending.clear();
            return Ok(Vec::new());
        }
        self.input[0].clear();
        self.input[0].extend(self.pending.drain(..));
        self.input[0].resize(RESAMPLE_CHUNK, 0.0);
        let block = self
            .resampler
            .as_mut()
            .unwrap()
            .process(&self.input, None)
            .map_err(|e| format!("Resample failed: {e:?}"))?;
        let mut out = block[0].clone();
        out.truncate(target - self.total_output);
        self.total_output += out.len();
        Ok(out)
    }
}

fn resample_mono_ratio(samples: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>, String> {
    if from_rate == to_rate {
        return Ok(samples.to_vec());
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let mut resampler = SincFixedIn::<f32>::new(
        to_rate as f64 / from_rate as f64,
        2.0,
        SINC_PARAMS,
        RESAMPLE_CHUNK,
        1,
    )
    .map_err(|e| format!("Resampler init failed: {:?}", e))?;

    // The last chunk is zero-padded to a full chunk, which used to leave up to
    // ~0.2 s of silence after every resampled buffer (every live chunk and every
    // file). Keep exactly len * ratio samples; feed silence if the resampler has
    // not produced that many yet.
    let expected_len = (samples.len() as f64 * to_rate as f64 / from_rate as f64).round() as usize;
    let mut resampled = Vec::with_capacity(expected_len + RESAMPLE_CHUNK);
    let mut input = vec![Vec::with_capacity(RESAMPLE_CHUNK)];
    let mut chunks = samples.chunks(RESAMPLE_CHUNK);
    while resampled.len() < expected_len {
        input[0].clear();
        if let Some(chunk) = chunks.next() {
            input[0].extend_from_slice(chunk);
        }
        input[0].resize(RESAMPLE_CHUNK, 0.0);
        let waves_out = resampler
            .process(&input, None)
            .map_err(|e| format!("Resample failed: {:?}", e))?;
        resampled.extend_from_slice(&waves_out[0]);
    }
    resampled.truncate(expected_len);

    Ok(resampled)
}

/// Resample mono f32 PCM to 16 kHz (shared with file import).
pub fn resample_mono_to_16k(samples: &[f32], from_rate: u32) -> Result<Vec<f32>, String> {
    resample_mono_ratio(samples, from_rate, 16000)
}

/// Downmix interleaved multi-channel audio to mono f32 by averaging across channels.
pub fn downmix_interleaved_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|chunk| chunk.iter().copied().sum::<f32>() / chunk.len() as f32)
        .collect()
}

fn frame_rms_list(samples: &[f32], frame: usize) -> Vec<f32> {
    if frame == 0 || samples.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(samples.len() / frame + 1);
    for w in samples.chunks(frame) {
        let rms = (w.iter().map(|&s| s * s).sum::<f32>() / w.len().max(1) as f32).sqrt();
        out.push(rms);
    }
    out
}

fn percentile_sorted(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f32 - 1.0) * p).clamp(0.0, sorted.len() as f32 - 1.0) as usize;
    sorted[idx]
}

/// Estimate noise floor from the quietest frames (10th percentile RMS).
pub fn estimate_noise_floor_rms(samples: &[f32], sample_rate: u32) -> f32 {
    let frame = (sample_rate as usize * FRAME_MS_16K / 1000).max(1);
    let mut fr = frame_rms_list(samples, frame);
    if fr.is_empty() {
        return 0.0;
    }
    fr.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile_sorted(&fr, 0.10).max(1e-8)
}

fn global_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Peak short-time RMS (90th percentile of frame RMS) vs noise floor → crude SNR proxy.
fn peak_to_floor_snr(samples: &[f32], sample_rate: u32) -> f32 {
    let frame = (sample_rate as usize * FRAME_MS_16K / 1000).max(1);
    let mut fr = frame_rms_list(samples, frame);
    if fr.is_empty() {
        return 100.0;
    }
    fr.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let peak = percentile_sorted(&fr, 0.90);
    let floor = estimate_noise_floor_rms(samples, sample_rate);
    peak / floor.max(1e-8)
}

/// Low-frequency excess: RMS of short-time |x| average / RMS(x). High ⇒ rumble / drift.
fn lf_excess_ratio(samples: &[f32]) -> f32 {
    if samples.len() < LF_MA_SAMPLES {
        return 0.0;
    }
    let w = LF_MA_SAMPLES;
    let mut sum_abs = 0.0_f32;
    let mut ma_energy = 0.0_f32;
    let mut n_ma = 0usize;
    for i in 0..samples.len() {
        sum_abs += samples[i].abs();
        if i >= w {
            sum_abs -= samples[i - w].abs();
        }
        if i + 1 >= w {
            let ma = sum_abs / w as f32;
            ma_energy += ma * ma;
            n_ma += 1;
        }
    }
    let rms_ma = (ma_energy / n_ma.max(1) as f32).sqrt();
    let rms_x = global_rms(samples).max(1e-8);
    (rms_ma / rms_x).min(2.0)
}

fn remove_dc(samples: &mut [f32]) {
    if samples.is_empty() {
        return;
    }
    let mean = samples.iter().copied().sum::<f32>() / samples.len() as f32;
    for s in samples.iter_mut() {
        *s -= mean;
    }
}

/// First-order high-pass ~80 Hz at 16 kHz (removes rumble after DC removal).
fn highpass_80hz_16k(samples: &mut [f32]) {
    if samples.len() < 2 {
        return;
    }
    const FC: f32 = 80.0;
    const FS: f32 = 16000.0;
    let rc = 1.0 / (2.0 * std::f32::consts::PI * FC);
    let dt = 1.0 / FS;
    let alpha = rc / (rc + dt);
    let mut y_prev = 0.0_f32;
    let mut x_prev = samples[0];
    for i in 0..samples.len() {
        let x = samples[i];
        let y = alpha * (y_prev + x - x_prev);
        samples[i] = y;
        y_prev = y;
        x_prev = x;
    }
}

fn apply_level_assist(samples: &mut [f32]) {
    let rms = global_rms(samples);
    if rms < 1e-6 || rms >= QUIET_RMS_THRESHOLD {
        return;
    }
    let gain = (LEVEL_TARGET_RMS / rms).min(MAX_GAIN_LINEAR);
    for s in samples.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
}

fn clamp_unit(samples: &mut [f32]) {
    for s in samples.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
}

/// Trim long leading/trailing silence from a **16 kHz** mono buffer (file import).
pub fn trim_file_edges_16k(samples: &[f32]) -> Vec<f32> {
    let Some((start, end)) = trim_file_edge_bounds_16k(samples) else {
        return Vec::new();
    };
    samples[start..end].to_vec()
}

fn trim_file_edge_bounds_16k(samples: &[f32]) -> Option<(usize, usize)> {
    if samples.is_empty() {
        return None;
    }
    let frame = (16000 * FRAME_MS_16K / 1000).max(1);
    let floor = estimate_noise_floor_rms(samples, 16000);
    let thresh = (floor * EDGE_RMS_GATE_FACTOR).max(1.5e-4);
    let fr = frame_rms_list(samples, frame);
    if fr.is_empty() {
        return Some((0, samples.len()));
    }

    let min_frames = (EDGE_MIN_SILENCE_MS / FRAME_MS_16K).max(1);
    let mut start_f = 0usize;
    while start_f < fr.len() && fr[start_f] < thresh {
        start_f += 1;
    }
    if start_f < min_frames {
        start_f = 0;
    }

    let mut end_f = fr.len();
    while end_f > start_f && fr[end_f - 1] < thresh {
        end_f -= 1;
    }
    if fr.len() - end_f < min_frames {
        end_f = fr.len();
    }

    let start = (start_f * frame).min(samples.len());
    let end = (end_f * frame).min(samples.len());
    if start >= end {
        return Some((0, samples.len()));
    }
    Some((start, end))
}

fn should_apply_denoise(samples: &[f32], sample_rate: u32) -> bool {
    peak_to_floor_snr(samples, sample_rate) < SNR_PEAK_TO_FLOOR_MIN
}

/// 16 kHz → 48 kHz → RNNoise → 16 kHz for file path when noisy.
fn denoise_16k_with_rnnoise(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let Ok(up) = resample_mono_ratio(samples, 16000, 48000) else {
        return samples.to_vec();
    };
    let mut den = Denoiser::new();
    let den48 = den.process(&up);
    if den48.is_empty() {
        return samples.to_vec();
    }
    let Ok(mut out) = resample_mono_ratio(&den48, 48000, 16000) else {
        return samples.to_vec();
    };
    let target = samples.len();
    if out.len() > target {
        out.truncate(target);
    } else if out.len() < target {
        out.resize(target, 0.0);
    }
    out
}

/// In-place preprocessing on **16 kHz** mono (after resample). `allow_file_denoise` uses 16↔48k RNNoise.
fn preprocess_16k_in_place(samples: &mut Vec<f32>, allow_file_denoise: bool) {
    if samples.is_empty() {
        return;
    }
    remove_dc(samples);

    if lf_excess_ratio(samples) >= LF_EXCESS_RATIO {
        highpass_80hz_16k(samples);
        remove_dc(samples);
    }

    if allow_file_denoise && should_apply_denoise(samples, 16000) {
        let d = denoise_16k_with_rnnoise(samples);
        if d.len() == samples.len() {
            *samples = d;
            remove_dc(samples);
        }
    }

    apply_level_assist(samples);
    clamp_unit(samples);
}

/// Live transcriber chunk: optional RNNoise @ 48 kHz, resample to 16 kHz, then universal 16k chain
/// (no file denoise path — already handled at 48k when applicable).
pub fn preprocess_live_transcribe_chunk(
    chunk: &[f32],
    sample_rate: u32,
    user_wants_denoise: bool,
    denoiser: Option<&mut Denoiser>,
) -> Vec<f32> {
    if chunk.is_empty() {
        return Vec::new();
    }

    let working: Cow<[f32]> =
        if user_wants_denoise && sample_rate == 48000 && should_apply_denoise(chunk, sample_rate) {
            if let Some(d) = denoiser {
                Cow::Owned(d.process(chunk))
            } else {
                Cow::Borrowed(chunk)
            }
        } else {
            Cow::Borrowed(chunk)
        };

    let mut pcm16 = match resample_mono_to_16k(&working, sample_rate) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[AUDIO_PRE] Live chunk resample failed: {}", e);
            if sample_rate == 16000 {
                working.into_owned()
            } else {
                return Vec::new();
            }
        }
    };

    preprocess_16k_in_place(&mut pcm16, false);
    pcm16
}

/// Trim long edge silence on a **file** buffer at 16 kHz (run **before** VAD).
pub fn trim_file_buffer_edges_16k(mono_16k: &mut Vec<f32>) {
    if mono_16k.is_empty() {
        return;
    }
    let Some((start, end)) = trim_file_edge_bounds_16k(mono_16k) else {
        mono_16k.clear();
        return;
    };
    mono_16k.truncate(end);
    if start > 0 {
        mono_16k.drain(..start);
    }
}

/// After VAD assembly: high-pass / optional RNNoise / level assist / clamp on speech-only buffer.
pub fn preprocess_assembled_speech_16k(speech: &mut Vec<f32>) {
    preprocess_16k_in_place(speech, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn resampled_length_matches_the_duration() {
        for (rate, len) in [(48_000u32, 48_000usize), (48_000, 288_000), (44_100, 44_100), (44_100, 12_345), (8_000, 8_000), (22_050, 1)] {
            let input = vec![0.1f32; len];
            let out = resample_mono_to_16k(&input, rate).unwrap();
            let expected = (len as f64 * 16_000.0 / rate as f64).round() as usize;
            assert_eq!(out.len(), expected, "{len} samples at {rate} Hz");
        }
        assert!(resample_mono_to_16k(&[], 48_000).unwrap().is_empty());
    }

    #[test]
    fn resampling_keeps_the_signal_in_time() {
        // A click 0.5 s in must still be ~0.5 s in after resampling (no filter delay).
        let mut input = vec![0.0f32; 48_000];
        input[24_000] = 1.0;
        let out = resample_mono_to_16k(&input, 48_000).unwrap();
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .unwrap()
            .0;
        assert!((peak as i64 - 8_000).abs() <= 2, "peak at {peak}");
    }

    #[test]
    fn resampling_preserves_a_tone_to_the_last_sample() {
        let out = resample_mono_to_16k(&sine(440.0, 48_000, 1.0), 48_000).unwrap();
        let tail_rms = global_rms(&out[out.len() - 1600..]);
        assert!((tail_rms - 0.5 / 2f32.sqrt()).abs() < 0.05, "tail rms {tail_rms}");
        let head_rms = global_rms(&out[..1600]);
        assert!((head_rms - 0.5 / 2f32.sqrt()).abs() < 0.05, "head rms {head_rms}");
    }

    #[test]
    fn upsample_round_trip_keeps_length() {
        let input = sine(300.0, 16_000, 0.73);
        let up = resample_mono_ratio(&input, 16_000, 48_000).unwrap();
        assert_eq!(up.len(), input.len() * 3);
        let back = resample_mono_ratio(&up, 48_000, 16_000).unwrap();
        assert_eq!(back.len(), input.len());
        let err: f32 = input.iter().zip(&back).map(|(a, b)| (a - b).abs()).sum::<f32>() / input.len() as f32;
        assert!(err < 0.02, "mean abs error {err}");
    }

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix_interleaved_to_mono(&[1.0, -1.0, 0.5, 0.5], 2), vec![0.0, 0.5]);
        assert!((downmix_interleaved_to_mono(&[0.3, 0.6, 0.9], 3)[0] - 0.6).abs() < 1e-6);
        assert_eq!(downmix_interleaved_to_mono(&[0.1, 0.2], 1), vec![0.1, 0.2]);
        assert_eq!(downmix_interleaved_to_mono(&[0.1, 0.2], 0), vec![0.1, 0.2]);
        // A trailing partial frame is averaged over the samples it has.
        assert_eq!(downmix_interleaved_to_mono(&[0.2, 0.4, 0.8], 2), vec![0.3, 0.8]);
    }

    #[test]
    fn edge_trim_removes_long_silence_but_keeps_short_gaps() {
        let silence = |ms: usize| vec![0.0f32; 16 * ms];
        let speech = sine(300.0, 16_000, 1.0);
        let mut padded = silence(1000);
        padded.extend(&speech);
        padded.extend(silence(1000));
        let trimmed = trim_file_edges_16k(&padded);
        assert!(trimmed.len() >= speech.len() && trimmed.len() < speech.len() + 16 * 60, "{}", trimmed.len());

        let mut short = silence(200);
        short.extend(&speech);
        assert_eq!(trim_file_edges_16k(&short).len(), short.len());

        assert!(trim_file_edges_16k(&[]).is_empty());
        let quiet = silence(2000);
        assert_eq!(trim_file_edges_16k(&quiet).len(), quiet.len());
    }

    #[test]
    fn level_assist_boosts_quiet_audio_within_the_gain_cap() {
        let mut quiet = vec![0.01f32; 1600];
        apply_level_assist(&mut quiet);
        assert!((quiet[0] - 0.1).abs() < 1e-4);
        let mut very_quiet = vec![0.001f32; 1600];
        apply_level_assist(&mut very_quiet);
        assert!((very_quiet[0] - 0.01).abs() < 1e-5, "capped at +20 dB");
        let mut loud = vec![0.2f32; 1600];
        apply_level_assist(&mut loud);
        assert_eq!(loud[0], 0.2);
    }

    #[test]
    fn dc_removal_and_high_pass_centre_the_signal() {
        let mut v: Vec<f32> = sine(440.0, 16_000, 0.5).iter().map(|s| s + 0.3).collect();
        remove_dc(&mut v);
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        assert!(mean.abs() < 1e-3);
        let mut offset = vec![0.5f32; 16_000];
        highpass_80hz_16k(&mut offset);
        assert!(offset[15_999].abs() < 1e-3, "constant input decays to zero");
    }

    #[test]
    fn noise_floor_uses_the_quietest_frames() {
        let mut v = vec![0.001f32; 16_000];
        v.extend(vec![0.5f32; 16_000]);
        let floor = estimate_noise_floor_rms(&v, 16_000);
        assert!((floor - 0.001).abs() < 1e-4, "{floor}");
        assert_eq!(estimate_noise_floor_rms(&[], 16_000), 0.0);
        assert!(peak_to_floor_snr(&v, 16_000) > 100.0);
    }

    /// No fixture file — verifies the assembled-speech chain does not explode and clamps output.
    #[test]
    fn universal_preprocess_synthetic_sine_1s() {
        let mut v: Vec<f32> = (0..16000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin() * 0.02)
            .collect();
        preprocess_assembled_speech_16k(&mut v);
        assert_eq!(v.len(), 16000);
        assert!(v
            .iter()
            .all(|&x| x.is_finite() && (-1.0..=1.0).contains(&x)));
    }

    #[test]
    fn streaming_resampler_keeps_filter_state_across_irregular_packets() {
        for source_rate in [44_100, 48_000] {
            let source: Vec<f32> = (0..source_rate * 5 + 12_345)
                .map(|i| {
                    (2.0 * std::f32::consts::PI * 440.0 * i as f32 / source_rate as f32).sin() * 0.2
                })
                .collect();
            let mut stream = StreamingResampler16k::new(source_rate).unwrap();
            let mut output = Vec::new();
            for packet in source.chunks(1379) {
                output.extend(stream.push(packet).unwrap());
            }
            output.extend(stream.finish().unwrap());
            let expected_len = ((source.len() as u128 * 16_000 + source_rate as u128 / 2)
                / source_rate as u128) as usize;
            assert_eq!(output.len(), expected_len);
            let whole = resample_mono_to_16k(&source, source_rate).unwrap();
            assert_eq!(output, whole[..expected_len]);
        }
    }
}
