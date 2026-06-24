// granite_features.rs — Audio feature extraction for IBM Granite Speech NAR.
//
// Matches the model's HF extractor shape contract:
// 16 kHz audio -> 80-bin log-mel -> clamp/compress -> stack adjacent frames
// into 160-dim encoder frames at half the 10 ms mel frame rate.

use ndarray::Array2;
use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::{Arc, OnceLock};

static FILTERBANK: OnceLock<Array2<f32>> = OnceLock::new();
static HANN_WINDOW: OnceLock<Vec<f32>> = OnceLock::new();
static FFT_PLAN: OnceLock<Arc<dyn rustfft::Fft<f32>>> = OnceLock::new();

const SAMPLE_RATE: usize = 16_000;
const N_FFT: usize = 512;
const WIN_LENGTH: usize = 400;
const HOP_LENGTH: usize = 160;
const N_MELS: usize = 80;
const STACKED_FEATURES: usize = N_MELS * 2;
const N_FREQ_BINS: usize = N_FFT / 2 + 1;
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
    for mel_idx in 0..N_MELS {
        let left = bin_points[mel_idx];
        let center = bin_points[mel_idx + 1];
        let right = bin_points[mel_idx + 2];
        for bin in 0..N_FREQ_BINS {
            let x = bin as f32;
            if x >= left && x <= center && center > left {
                filterbank[[mel_idx, bin]] = (x - left) / (center - left);
            } else if x > center && x <= right && right > center {
                filterbank[[mel_idx, bin]] = (right - x) / (right - center);
            }
        }
    }
    filterbank
}

fn reflect_index(i: isize, len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    let len = len as isize;
    let mut x = i;
    while x < 0 || x >= len {
        if x < 0 {
            x = -x;
        }
        if x >= len {
            x = 2 * len - 2 - x;
        }
    }
    x as usize
}

fn stft_power(audio: &[f32]) -> Array2<f32> {
    let window = HANN_WINDOW.get_or_init(|| hann_window(WIN_LENGTH));
    let fft = FFT_PLAN.get_or_init(|| {
        let mut planner = FftPlanner::<f32>::new();
        planner.plan_fft_forward(N_FFT)
    });

    if audio.is_empty() {
        return Array2::<f32>::zeros((0, N_FREQ_BINS));
    }

    // torch/torchaudio center=True pads by n_fft/2 with reflection.
    let pad = (N_FFT / 2) as isize;
    let padded_len = audio.len() + N_FFT;
    let n_frames = if padded_len >= N_FFT {
        (padded_len - N_FFT) / HOP_LENGTH + 1
    } else {
        0
    };
    let mut powers = Array2::<f32>::zeros((n_frames, N_FREQ_BINS));
    let mut buf = vec![Complex::new(0.0_f32, 0.0); N_FFT];
    let win_offset = (N_FFT - WIN_LENGTH) / 2;

    for frame in 0..n_frames {
        let start = frame * HOP_LENGTH;
        for i in 0..N_FFT {
            let src = start as isize + i as isize - pad;
            let sample = audio[reflect_index(src, audio.len())];
            let win = if i >= win_offset && i < win_offset + WIN_LENGTH {
                window[i - win_offset]
            } else {
                0.0
            };
            buf[i] = Complex::new(sample * win, 0.0);
        }
        fft.process(&mut buf);
        for bin in 0..N_FREQ_BINS {
            powers[[frame, bin]] = buf[bin].re * buf[bin].re + buf[bin].im * buf[bin].im;
        }
    }
    powers
}

pub fn extract_features(audio_16k: &[f32]) -> Array2<f32> {
    let valid_mel_frames = 2 * (audio_16k.len() / (2 * HOP_LENGTH));
    if valid_mel_frames < 2 {
        return Array2::<f32>::zeros((0, STACKED_FEATURES));
    }

    let powers = stft_power(audio_16k);
    let filterbank = FILTERBANK.get_or_init(compute_mel_filterbank);
    let mut mel = powers.dot(&filterbank.t());
    let frames = valid_mel_frames.min(mel.nrows());
    if frames < 2 {
        return Array2::<f32>::zeros((0, STACKED_FEATURES));
    }

    mel = mel.slice(ndarray::s![0..frames, ..]).to_owned();
    mel.mapv_inplace(|v| v.max(EPS).log10());

    let max_logmel = mel.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let floor = max_logmel - 8.0;
    mel.mapv_inplace(|v| v.max(floor) / 4.0 + 1.0);

    let out_frames = frames / 2;
    let mut stacked = Array2::<f32>::zeros((out_frames, STACKED_FEATURES));
    for t in 0..out_frames {
        for m in 0..N_MELS {
            stacked[[t, m]] = mel[[2 * t, m]];
            stacked[[t, N_MELS + m]] = mel[[2 * t + 1, m]];
        }
    }
    stacked
}
