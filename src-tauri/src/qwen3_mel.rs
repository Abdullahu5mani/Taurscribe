//! 128-channel Slaney log-mel spectrogram extractor for Qwen3-ASR.
//!
//! Matches the official `Qwen/Qwen3-ASR-1.7B` feature extractor configuration:
//!   - Sample rate:   16 000 Hz
//!   - FFT size:      400
//!   - Window length: 400 samples (25 ms Hann window)
//!   - Hop length:    160 samples (10 ms)
//!   - Mel bins:      128 (Slaney area-normalized triangular filters)
//!   - Dynamic range: max - 8.0, normalized via `(x + 4.0) / 4.0`
//!
//! Accelerated with `rustfft` and static plan caching.
//! Output shape: [n_frames, 128].

use ndarray::Array2;
use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::{Arc, OnceLock};

// ── Constants ─────────────────────────────────────────────────────────────────

pub const QWEN3_SAMPLE_RATE: u32 = 16_000;
pub const QWEN3_N_FFT: usize = 400;
pub const QWEN3_WIN_LENGTH: usize = 400;
pub const QWEN3_HOP_LENGTH: usize = 160;
pub const QWEN3_N_MELS: usize = 128;
pub const QWEN3_N_FREQ_BINS: usize = QWEN3_N_FFT / 2 + 1; // 201

static FILTERBANK: OnceLock<Array2<f32>> = OnceLock::new();
static HANN_WINDOW: OnceLock<Vec<f32>> = OnceLock::new();
static FFT_PLAN: OnceLock<Arc<dyn rustfft::Fft<f32>>> = OnceLock::new();

// ── Public API ────────────────────────────────────────────────────────────────

/// Extract a log-mel spectrogram from 16 kHz mono f32 PCM.
///
/// Returns a matrix of shape `[n_frames, QWEN3_N_MELS]` matching HuggingFace's
/// `Qwen3ASRFeatureExtractor`.
pub fn extract_qwen3_log_mel(audio: &[f32]) -> Array2<f32> {
    if audio.is_empty() {
        return Array2::<f32>::zeros((0, QWEN3_N_MELS));
    }

    let window = HANN_WINDOW.get_or_init(|| hann_window(QWEN3_WIN_LENGTH));
    let fft = FFT_PLAN.get_or_init(|| {
        let mut planner = FftPlanner::<f32>::new();
        planner.plan_fft_forward(QWEN3_N_FFT)
    });
    let fb = FILTERBANK.get_or_init(slaney_mel_filterbank);

    let pad = QWEN3_N_FFT / 2;
    let n_frames = audio.len() / QWEN3_HOP_LENGTH;
    if n_frames == 0 {
        return Array2::<f32>::zeros((0, QWEN3_N_MELS));
    }

    // 1. STFT magnitudes [n_frames, 201]
    let mut magnitudes = Array2::<f32>::zeros((n_frames, QWEN3_N_FREQ_BINS));
    let mut buf = vec![Complex::new(0.0f32, 0.0f32); QWEN3_N_FFT];

    for f in 0..n_frames {
        let start = f * QWEN3_HOP_LENGTH;
        for i in 0..QWEN3_N_FFT {
            let src = start as isize + i as isize - pad as isize;
            let sample = if src < 0 {
                audio[(-src) as usize]
            } else if src >= audio.len() as isize {
                let diff = src - audio.len() as isize + 1;
                audio[audio.len().saturating_sub(diff as usize + 1)]
            } else {
                audio[src as usize]
            };
            buf[i] = Complex::new(sample * window[i], 0.0);
        }
        fft.process(&mut buf);
        for b in 0..QWEN3_N_FREQ_BINS {
            magnitudes[[f, b]] = buf[b].re * buf[b].re + buf[b].im * buf[b].im;
        }
    }

    // 2. mel_spec = magnitudes @ filterbank -> [n_frames, 128]
    let mut out = Array2::<f32>::zeros((n_frames, QWEN3_N_MELS));
    let mut max_val = f32::NEG_INFINITY;

    for f in 0..n_frames {
        for m in 0..QWEN3_N_MELS {
            let mut sum = 0.0f32;
            for b in 0..QWEN3_N_FREQ_BINS {
                sum += magnitudes[[f, b]] * fb[[b, m]];
            }
            let log_v = sum.max(1e-10).log10();
            if log_v > max_val {
                max_val = log_v;
            }
            out[[f, m]] = log_v;
        }
    }

    // 3. Dynamic range clamp and normalization
    let clamp_val = max_val - 8.0;
    for v in out.iter_mut() {
        *v = ((*v).max(clamp_val) + 4.0) / 4.0;
    }

    out
}

// ── Hann window ───────────────────────────────────────────────────────────────

fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()))
        .collect()
}

// ── Slaney Mel Filterbank ─────────────────────────────────────────────────────

/// Build [201, 128] Slaney area-normalized Mel filterbank.
pub fn slaney_mel_filterbank() -> Array2<f32> {
    // If a pre-dumped binary exists, load it; otherwise compute it analytically.
    if let Ok(models_dir) = crate::utils::get_models_dir() {
        for candidate in [
            models_dir.join("qwen3-asr-1.7b-mlx").join("mel_filters.bin"),
            std::path::PathBuf::from("target/qwen3-model-test/mel_filters.bin"),
        ] {
            if let Ok(bytes) = std::fs::read(&candidate) {
                if bytes.len() == QWEN3_N_FREQ_BINS * QWEN3_N_MELS * 4 {
                    let mut fb = Array2::<f32>::zeros((QWEN3_N_FREQ_BINS, QWEN3_N_MELS));
                    let floats: Vec<f32> = bytes
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                        .collect();
                    for b in 0..QWEN3_N_FREQ_BINS {
                        for m in 0..QWEN3_N_MELS {
                            fb[[b, m]] = floats[b * QWEN3_N_MELS + m];
                        }
                    }
                    return fb;
                }
            }
        }
    }

    compute_slaney_filterbank()
}

fn compute_slaney_filterbank() -> Array2<f32> {
    let num_mel = QWEN3_N_MELS;
    let num_bins = QWEN3_N_FREQ_BINS;
    let sr = QWEN3_SAMPLE_RATE as f32;

    let hertz_to_mel = |hz: f32| -> f32 {
        if hz < 1000.0 {
            3.0 * hz / 200.0
        } else {
            15.0 + (hz / 1000.0).ln() * (27.0 / (6.4_f32).ln())
        }
    };

    let mel_to_hertz = |mel: f32| -> f32 {
        if mel < 15.0 {
            200.0 * mel / 3.0
        } else {
            1000.0 * (((6.4_f32).ln() / 27.0) * (mel - 15.0)).exp()
        }
    };

    let mel_min = hertz_to_mel(0.0);
    let mel_max = hertz_to_mel(sr / 2.0);
    let mel_points: Vec<f32> = (0..num_mel + 2)
        .map(|i| mel_min + (mel_max - mel_min) * (i as f32) / (num_mel as f32 + 1.0))
        .collect();
    let filter_freqs: Vec<f32> = mel_points.iter().map(|&m| mel_to_hertz(m)).collect();

    let fft_freqs: Vec<f32> = (0..num_bins)
        .map(|k| (k as f32) * (sr / 2.0) / (num_bins as f32 - 1.0))
        .collect();

    let mut fb = Array2::<f32>::zeros((num_bins, num_mel));
    for m in 0..num_mel {
        let f_left = filter_freqs[m];
        let f_center = filter_freqs[m + 1];
        let f_right = filter_freqs[m + 2];
        let enorm = 2.0 / (f_right - f_left);

        for k in 0..num_bins {
            let f = fft_freqs[k];
            let val = if f < f_left || f > f_right {
                0.0
            } else if f <= f_center {
                (f - f_left) / (f_center - f_left)
            } else {
                (f_right - f) / (f_right - f_center)
            };
            fb[[k, m]] = val * enorm;
        }
    }
    fb
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filterbank_has_correct_shape() {
        let fb = slaney_mel_filterbank();
        assert_eq!(fb.nrows(), QWEN3_N_FREQ_BINS);
        assert_eq!(fb.ncols(), QWEN3_N_MELS);
    }

    #[test]
    fn filterbank_is_non_negative() {
        let fb = slaney_mel_filterbank();
        for &v in fb.iter() {
            assert!(v >= 0.0, "negative filter coefficient: {v}");
        }
    }

    #[test]
    fn extract_silence_gives_valid_shape() {
        let silence = vec![0.0_f32; 16_000];
        let mel = extract_qwen3_log_mel(&silence);
        assert_eq!(mel.ncols(), QWEN3_N_MELS);
        assert!(mel.nrows() > 0);
    }

    #[test]
    fn extract_empty_audio_returns_empty() {
        let mel = extract_qwen3_log_mel(&[]);
        assert_eq!(mel.ncols(), QWEN3_N_MELS);
        assert_eq!(mel.nrows(), 0);
    }
}
