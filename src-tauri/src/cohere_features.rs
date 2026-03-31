// cohere_features.rs — Audio feature extraction for Cohere Transcribe ONNX
//
// Based on the published preprocessor settings from
// `onnx-community/cohere-transcribe-03-2026-ONNX`:
//   sample_rate=16000, n_fft=512, win_length=400, hop_length=160, feature_size=128,
//   preemphasis=0.97, dither=1e-5, normalize=per_feature, log=true
//
// Output shape matches encoder input: (frames, 128).

use ndarray::Array2;
use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::{Arc, OnceLock};

static FILTERBANK: OnceLock<Array2<f32>> = OnceLock::new();
static HANN_WINDOW: OnceLock<Vec<f32>> = OnceLock::new();
static FFT_PLAN: OnceLock<Arc<dyn rustfft::Fft<f32>>> = OnceLock::new();

const SAMPLE_RATE: usize = 16000;
const N_FFT: usize = 512;
const HOP_LENGTH: usize = 160;
const WIN_LENGTH: usize = 400;
const N_MELS: usize = 128;
const N_FREQ_BINS: usize = N_FFT / 2 + 1; // 257
const PREEMPHASIS: f32 = 0.97;
const DITHER: f32 = 1e-5;
const EPS: f32 = 1e-10;

#[inline]
fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

#[inline]
fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

fn hann_window(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / len as f32).cos()))
        .collect()
}

fn compute_mel_filterbank() -> Array2<f32> {
    let fmin = 0.0_f32;
    let fmax = SAMPLE_RATE as f32 / 2.0;
    let mel_min = hz_to_mel(fmin);
    let mel_max = hz_to_mel(fmax);

    let n_points = N_MELS + 2;
    let mel_points: Vec<f32> = (0..n_points)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (n_points - 1) as f32)
        .collect();
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();
    let bin_points: Vec<f32> = hz_points
        .iter()
        .map(|&h| (N_FFT as f32 + 1.0) * h / SAMPLE_RATE as f32)
        .collect();

    let mut filterbank = Array2::<f32>::zeros((N_MELS, N_FREQ_BINS));
    for m in 0..N_MELS {
        let left = bin_points[m];
        let center = bin_points[m + 1];
        let right = bin_points[m + 2];
        for k in 0..N_FREQ_BINS {
            let kf = k as f32;
            if kf >= left && kf <= center && center > left {
                filterbank[[m, k]] = (kf - left) / (center - left);
            } else if kf > center && kf <= right && right > center {
                filterbank[[m, k]] = (right - kf) / (right - center);
            }
        }
    }
    filterbank
}

fn preemphasis_and_dither(signal: &[f32]) -> Vec<f32> {
    if signal.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(signal.len());
    for (i, &x) in signal.iter().enumerate() {
        let prev = if i == 0 { 0.0 } else { signal[i - 1] };
        // Lightweight deterministic pseudo-random dither (closer to noise than
        // alternating +/- pattern, while still deterministic for tests).
        let noise = ((((i as u32).wrapping_mul(1664525).wrapping_add(1013904223)) & 1023) as f32
            / 1023.0
            - 0.5)
            * 2.0
            * DITHER;
        out.push((x - PREEMPHASIS * prev) + noise);
    }
    out
}

fn stft_power(signal: &[f32]) -> Array2<f32> {
    let window = HANN_WINDOW.get_or_init(|| hann_window(WIN_LENGTH));
    let fft = FFT_PLAN.get_or_init(|| {
        let mut planner = FftPlanner::<f32>::new();
        planner.plan_fft_forward(N_FFT)
    });

    let pad = N_FFT / 2;
    let mut padded = vec![0.0_f32; pad];
    padded.extend_from_slice(signal);
    padded.resize(padded.len() + pad, 0.0);

    let n_frames = if padded.len() >= N_FFT {
        (padded.len() - N_FFT) / HOP_LENGTH + 1
    } else {
        0
    };

    let mut powers = Array2::<f32>::zeros((n_frames, N_FREQ_BINS));
    let mut buf = vec![Complex::new(0.0_f32, 0.0_f32); N_FFT];
    let win_offset = (N_FFT - WIN_LENGTH) / 2;

    for frame_idx in 0..n_frames {
        let start = frame_idx * HOP_LENGTH;
        for i in 0..N_FFT {
            let sample = padded.get(start + i).copied().unwrap_or(0.0);
            let win = if i >= win_offset && i < win_offset + WIN_LENGTH {
                window[i - win_offset]
            } else {
                0.0
            };
            buf[i] = Complex::new(sample * win, 0.0);
        }
        fft.process(&mut buf);
        for k in 0..N_FREQ_BINS {
            powers[[frame_idx, k]] = buf[k].re * buf[k].re + buf[k].im * buf[k].im;
        }
    }

    powers
}

fn log_mel_spectrogram(audio: &[f32]) -> Array2<f32> {
    let pre = preemphasis_and_dither(audio);
    let powers = stft_power(&pre);
    let filterbank = FILTERBANK.get_or_init(compute_mel_filterbank);
    let mut mel = powers.dot(&filterbank.t());
    // HF processors typically use natural log for mel energies.
    mel.mapv_inplace(|v| v.max(EPS).ln());
    mel
}

/// Cohere encoder features: log-mel with per-feature normalization.
/// Output shape: (frames, 128)
pub fn extract_features(audio: &[f32]) -> Array2<f32> {
    let mut mel = log_mel_spectrogram(audio);
    let n_frames = mel.nrows();
    if n_frames == 0 {
        return Array2::<f32>::zeros((0, N_MELS));
    }

    // normalize=per_feature: each mel bin gets zero-mean / unit-variance over time.
    for m in 0..N_MELS {
        let mut sum = 0.0_f32;
        for t in 0..n_frames {
            sum += mel[[t, m]];
        }
        let mean = sum / n_frames as f32;
        let mut var = 0.0_f32;
        for t in 0..n_frames {
            let d = mel[[t, m]] - mean;
            var += d * d;
        }
        let std = (var / n_frames as f32).sqrt().max(1e-6);
        for t in 0..n_frames {
            mel[[t, m]] = (mel[[t, m]] - mean) / std;
        }
    }

    mel
}

#[allow(dead_code)]
pub const fn expected_sample_rate() -> u32 {
    SAMPLE_RATE as u32
}
