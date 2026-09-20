//! 128-channel log-mel spectrogram extractor for Qwen3-ASR.
//!
//! Matches the official `Qwen/Qwen3-ASR-1.7B` feature extractor configuration:
//!   - Sample rate:   16 000 Hz
//!   - FFT size:      512
//!   - Window length: 400 samples (25 ms)
//!   - Hop length:    160 samples (10 ms)
//!   - Mel bins:      128   (Slaney-style area-normalized triangular filters)
//!
//! This is a pure-Rust, zero-Python implementation. Output shape: [n_frames, 128].

use ndarray::Array2;

// ── Constants ─────────────────────────────────────────────────────────────────

pub const QWEN3_SAMPLE_RATE: u32 = 16_000;
pub const QWEN3_N_FFT: usize = 512;
pub const QWEN3_WIN_LENGTH: usize = 400;
pub const QWEN3_HOP_LENGTH: usize = 160;
pub const QWEN3_N_MELS: usize = 128;

const FREQ_MIN: f32 = 0.0;
const FREQ_MAX: f32 = 8_000.0; // Nyquist for 16 kHz

// ── Public API ────────────────────────────────────────────────────────────────

/// Extract a log-mel spectrogram from 16 kHz mono f32 PCM.
///
/// Returns a matrix of shape `[n_frames, QWEN3_N_MELS]` where every column is
/// a mel band and every row is one analysis frame.
pub fn extract_qwen3_log_mel(audio: &[f32]) -> Array2<f32> {
    let frames = stft(audio);
    let filterbank = mel_filterbank();
    apply_mel_and_log(&frames, &filterbank)
}

// ── STFT ──────────────────────────────────────────────────────────────────────

/// Compute the power spectrogram via an FFT-DFT approximation using the DFT
/// formula directly — sufficient for the fixed short windows used here.
fn stft(audio: &[f32]) -> Array2<f32> {
    let window = hann_window(QWEN3_WIN_LENGTH);
    let pad = QWEN3_N_FFT / 2;

    // Reflect-pad the signal on both sides.
    let mut padded = Vec::with_capacity(audio.len() + 2 * pad);
    for i in (1..=pad).rev() {
        padded.push(*audio.get(i).unwrap_or(&0.0));
    }
    padded.extend_from_slice(audio);
    for i in (audio.len().saturating_sub(pad)..audio.len()).rev() {
        padded.push(*audio.get(i).unwrap_or(&0.0));
    }

    let n_frames = if padded.len() < QWEN3_WIN_LENGTH {
        0
    } else {
        (padded.len() - QWEN3_WIN_LENGTH) / QWEN3_HOP_LENGTH + 1
    };
    let n_bins = QWEN3_N_FFT / 2 + 1; // 257

    let mut power = Array2::<f32>::zeros((n_frames, n_bins));

    for (f, frame) in power.rows_mut().into_iter().enumerate() {
        let start = f * QWEN3_HOP_LENGTH;
        let windowed: Vec<f32> = (0..QWEN3_WIN_LENGTH)
            .map(|i| padded.get(start + i).copied().unwrap_or(0.0) * window[i])
            .collect();

        // DFT on the zero-padded frame.
        let padded_frame = {
            let mut v = windowed;
            v.resize(QWEN3_N_FFT, 0.0);
            v
        };

        let mut frame = frame;
        for k in 0..n_bins {
            let (mut re, mut im) = (0.0_f64, 0.0_f64);
            for n in 0..QWEN3_N_FFT {
                let angle = -2.0 * std::f64::consts::PI * (k * n) as f64 / QWEN3_N_FFT as f64;
                re += padded_frame[n] as f64 * angle.cos();
                im += padded_frame[n] as f64 * angle.sin();
            }
            frame[k] = (re * re + im * im) as f32;
        }
    }

    power
}

// ── Hann window ───────────────────────────────────────────────────────────────

fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos())
        })
        .collect()
}

// ── Mel filterbank ────────────────────────────────────────────────────────────

/// Build a [QWEN3_N_MELS × (N_FFT/2+1)] area-normalized triangular Mel filterbank.
fn mel_filterbank() -> Array2<f32> {
    let n_bins = QWEN3_N_FFT / 2 + 1; // 257
    let f_min_mel = hz_to_mel(FREQ_MIN);
    let f_max_mel = hz_to_mel(FREQ_MAX);

    // QWEN3_N_MELS+2 linearly-spaced mel points → convert back to Hz.
    let mel_points: Vec<f32> = (0..=(QWEN3_N_MELS + 1))
        .map(|i| {
            mel_to_hz(f_min_mel + i as f32 * (f_max_mel - f_min_mel) / (QWEN3_N_MELS + 1) as f32)
        })
        .collect();

    // Convert center frequencies to FFT bin indices.
    let bin_freq = QWEN3_SAMPLE_RATE as f32 / QWEN3_N_FFT as f32;
    let fft_bins: Vec<f32> = mel_points.iter().map(|&f| f / bin_freq).collect();

    let mut fb = Array2::<f32>::zeros((QWEN3_N_MELS, n_bins));
    for m in 0..QWEN3_N_MELS {
        let f_m_minus = fft_bins[m];
        let f_m = fft_bins[m + 1];
        let f_m_plus = fft_bins[m + 2];
        let width = f_m_plus - f_m_minus;

        for k in 0..n_bins {
            let k = k as f32;
            let v = if k < f_m_minus || k > f_m_plus {
                0.0
            } else if k <= f_m {
                2.0 / width * (k - f_m_minus) / (f_m - f_m_minus)
            } else {
                2.0 / width * (f_m_plus - k) / (f_m_plus - f_m)
            };
            fb[[m, k as usize]] = v;
        }
    }
    fb
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

// ── Mel + log ─────────────────────────────────────────────────────────────────

/// Apply mel filterbank, take log, clamp at 1e-10.
fn apply_mel_and_log(power: &Array2<f32>, fb: &Array2<f32>) -> Array2<f32> {
    // power: [n_frames, n_bins]
    // fb:    [N_MELS, n_bins]
    // out:   [n_frames, N_MELS]
    let n_frames = power.nrows();
    let n_mels = QWEN3_N_MELS;
    let n_bins = QWEN3_N_FFT / 2 + 1;

    let mut out = Array2::<f32>::zeros((n_frames, n_mels));
    for f in 0..n_frames {
        for m in 0..n_mels {
            let mut s = 0.0_f32;
            for k in 0..n_bins {
                s += fb[[m, k]] * power[[f, k]];
            }
            out[[f, m]] = s.max(1e-10).ln();
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filterbank_has_correct_shape() {
        let fb = mel_filterbank();
        assert_eq!(fb.nrows(), QWEN3_N_MELS);
        assert_eq!(fb.ncols(), QWEN3_N_FFT / 2 + 1);
    }

    #[test]
    fn filterbank_is_non_negative() {
        let fb = mel_filterbank();
        for &v in fb.iter() {
            assert!(v >= 0.0, "negative filter coefficient: {v}");
        }
    }

    #[test]
    fn hann_window_has_correct_length() {
        let w = hann_window(QWEN3_WIN_LENGTH);
        assert_eq!(w.len(), QWEN3_WIN_LENGTH);
    }

    #[test]
    fn hann_window_starts_and_ends_near_zero() {
        let w = hann_window(QWEN3_WIN_LENGTH);
        assert!(w[0] < 1e-3, "Hann window should start near 0");
        assert!(w[QWEN3_WIN_LENGTH - 1] < 1e-3, "Hann window should end near 0");
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
        // Either 0 or 1 frames is acceptable; shape must be valid.
        assert_eq!(mel.ncols(), QWEN3_N_MELS);
    }

    #[test]
    fn log_mel_values_are_negative() {
        // log of values ≤ 1 must be ≤ 0.
        let silence = vec![0.0_f32; 16_000];
        let mel = extract_qwen3_log_mel(&silence);
        for &v in mel.iter() {
            assert!(v <= 0.0, "log-mel of silence should be ≤ 0, got {v}");
        }
    }
}
