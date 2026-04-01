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

    let pad = samples.len() % RESAMPLE_CHUNK;
    let mut padded: Vec<f32> = samples.to_vec();
    if pad > 0 {
        padded.extend(std::iter::repeat(0.0_f32).take(RESAMPLE_CHUNK - pad));
    }

    let mut resampled = Vec::new();
    for chunk in padded.chunks(RESAMPLE_CHUNK) {
        let waves_in = vec![chunk.to_vec()];
        let waves_out = resampler
            .process(&waves_in, None)
            .map_err(|e| format!("Resample failed: {:?}", e))?;
        resampled.extend_from_slice(&waves_out[0]);
    }

    Ok(resampled)
}

/// Resample mono f32 PCM to 16 kHz (shared with file import).
pub fn resample_mono_to_16k(samples: &[f32], from_rate: u32) -> Result<Vec<f32>, String> {
    resample_mono_ratio(samples, from_rate, 16000)
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
    if samples.is_empty() {
        return Vec::new();
    }
    let frame = (16000 * FRAME_MS_16K / 1000).max(1);
    let floor = estimate_noise_floor_rms(samples, 16000);
    let thresh = (floor * EDGE_RMS_GATE_FACTOR).max(1.5e-4);
    let fr = frame_rms_list(samples, frame);
    if fr.is_empty() {
        return samples.to_vec();
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
        return samples.to_vec();
    }
    samples[start..end].to_vec()
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

    let mut working = chunk.to_vec();
    if user_wants_denoise && sample_rate == 48000 && should_apply_denoise(chunk, sample_rate) {
        if let Some(d) = denoiser {
            working = d.process(chunk);
        }
    }

    let mut pcm16 = match resample_mono_to_16k(&working, sample_rate) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[AUDIO_PRE] Live chunk resample failed: {}", e);
            if sample_rate == 16000 {
                working
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
    *mono_16k = trim_file_edges_16k(mono_16k);
}

/// After VAD assembly: high-pass / optional RNNoise / level assist / clamp on speech-only buffer.
pub fn preprocess_assembled_speech_16k(speech: &mut Vec<f32>) {
    preprocess_16k_in_place(speech, true);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
