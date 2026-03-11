// granite_features.rs — Audio feature extraction for Granite 4.0 1B Speech ONNX
//
// Computes 160-dimensional log-mel spectrogram features matching the model's
// preprocessor_config.json:
//   sample_rate = 16000, n_fft = 512, hop_length = 160,
//   win_length = 400, n_mels = 80, frame_stacking = 2

use ndarray::Array2;
use rustfft::{num_complex::Complex, FftPlanner};

// ───────────────────────── Constants ──────────────────────────────────────────
const SAMPLE_RATE: usize = 16000;
const N_FFT: usize = 512;
const HOP_LENGTH: usize = 160;
const WIN_LENGTH: usize = 400;
const N_MELS: usize = 80;
const N_FREQ_BINS: usize = N_FFT / 2 + 1; // 257

// ───────────────────────── Hann Window ────────────────────────────────────────

/// Periodic Hann window of length `len`.
fn hann_window(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / len as f32).cos())
        })
        .collect()
}

// ───────────────────────── Hz ↔ Mel ──────────────────────────────────────────

/// Convert frequency in Hz to Mel scale (HTK formula).
#[inline]
fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

/// Convert Mel scale to frequency in Hz (HTK formula).
#[inline]
fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

// ───────────────────────── Mel Filterbank ────────────────────────────────────

/// Build an 80 × 257 triangular Mel filterbank matrix.
///
/// Each row is a triangular filter centered on a Mel-spaced frequency.
fn compute_mel_filterbank() -> Array2<f32> {
    let fmin = 0.0_f32;
    let fmax = SAMPLE_RATE as f32 / 2.0; // 8000 Hz Nyquist

    let mel_min = hz_to_mel(fmin);
    let mel_max = hz_to_mel(fmax);

    // N_MELS + 2 evenly spaced points in Mel space → triangle edges
    let n_points = N_MELS + 2;
    let mel_points: Vec<f32> = (0..n_points)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (n_points - 1) as f32)
        .collect();

    // Convert back to Hz then to FFT bin indices
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();
    let bin_points: Vec<f32> = hz_points
        .iter()
        .map(|&h| (N_FFT as f32 + 1.0) * h / SAMPLE_RATE as f32)
        .collect();

    let mut filterbank = Array2::<f32>::zeros((N_MELS, N_FREQ_BINS));

    for m in 0..N_MELS {
        let f_left = bin_points[m];
        let f_center = bin_points[m + 1];
        let f_right = bin_points[m + 2];

        for k in 0..N_FREQ_BINS {
            let k_f = k as f32;
            if k_f >= f_left && k_f <= f_center && f_center > f_left {
                filterbank[[m, k]] = (k_f - f_left) / (f_center - f_left);
            } else if k_f > f_center && k_f <= f_right && f_right > f_center {
                filterbank[[m, k]] = (f_right - k_f) / (f_right - f_center);
            }
        }
    }

    filterbank
}

// ───────────────────────── STFT ──────────────────────────────────────────────

/// Compute the Short-Time Fourier Transform.
///
/// Returns a 2-D array of shape `(n_frames, N_FREQ_BINS)` containing the
/// magnitude of each frequency bin per frame.
fn stft_magnitude(signal: &[f32]) -> Array2<f32> {
    let window = hann_window(WIN_LENGTH);

    // Pad signal with zeros so every sample is covered
    let pad_len = N_FFT / 2;
    let mut padded = vec![0.0_f32; pad_len];
    padded.extend_from_slice(signal);
    padded.resize(padded.len() + pad_len, 0.0);

    // Number of frames
    let n_frames = if padded.len() >= N_FFT {
        (padded.len() - N_FFT) / HOP_LENGTH + 1
    } else {
        0
    };

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);

    let mut magnitudes = Array2::<f32>::zeros((n_frames, N_FREQ_BINS));

    let mut buffer = vec![Complex::new(0.0_f32, 0.0); N_FFT];

    for frame_idx in 0..n_frames {
        let start = frame_idx * HOP_LENGTH;

        // Fill buffer: apply window to center of N_FFT frame
        for i in 0..N_FFT {
            let sample_idx = start + i;
            let sample = if sample_idx < padded.len() {
                padded[sample_idx]
            } else {
                0.0
            };
            // Window is WIN_LENGTH (400), centered within N_FFT (512)
            let win_offset = (N_FFT - WIN_LENGTH) / 2; // 56
            let win_val = if i >= win_offset && i < win_offset + WIN_LENGTH {
                window[i - win_offset]
            } else {
                0.0
            };
            buffer[i] = Complex::new(sample * win_val, 0.0);
        }

        fft.process(&mut buffer);

        // Take magnitude of first N_FREQ_BINS
        for k in 0..N_FREQ_BINS {
            magnitudes[[frame_idx, k]] = (buffer[k].re * buffer[k].re + buffer[k].im * buffer[k].im).sqrt();
        }
    }

    magnitudes
}

// ───────────────────────── Log-Mel Spectrogram ───────────────────────────────

/// Compute log-mel spectrogram: shape `(n_frames, 80)`.
fn log_mel_spectrogram(audio: &[f32]) -> Array2<f32> {
    let magnitudes = stft_magnitude(audio);
    let filterbank = compute_mel_filterbank();

    let n_frames = magnitudes.nrows();
    let mut mel_spec = Array2::<f32>::zeros((n_frames, N_MELS));

    // mel_spec = magnitudes · filterbank^T
    for t in 0..n_frames {
        for m in 0..N_MELS {
            let mut sum = 0.0_f32;
            for k in 0..N_FREQ_BINS {
                sum += magnitudes[[t, k]] * filterbank[[m, k]];
            }
            // Clamp to avoid log(0)
            mel_spec[[t, m]] = (sum.max(1e-10)).ln();
        }
    }

    mel_spec
}

// ───────────────────────── Feature Extraction ────────────────────────────────

/// Extract Granite Speech input features from raw 16 kHz audio.
///
/// 1. Computes 80-bin log-mel spectrogram
/// 2. Stacks pairs of adjacent frames → 160-dimensional vectors
///
/// Returns an `Array2<f32>` of shape `(n_stacked_frames, 160)`.
pub fn extract_features(audio: &[f32]) -> Array2<f32> {
    let mel = log_mel_spectrogram(audio);
    let n_frames = mel.nrows();

    // Stack pairs of adjacent frames: [frame_t || frame_{t+1}]
    // If odd number of frames, the last unpaired frame is dropped.
    let n_stacked = n_frames / 2;

    let mut features = Array2::<f32>::zeros((n_stacked, N_MELS * 2));

    for t in 0..n_stacked {
        let src_t = t * 2;
        for m in 0..N_MELS {
            features[[t, m]] = mel[[src_t, m]];
            features[[t, N_MELS + m]] = mel[[src_t + 1, m]];
        }
    }

    features
}

/// Convenience: returns the expected sample rate for this feature extractor.
#[allow(dead_code)]
pub const fn expected_sample_rate() -> u32 {
    SAMPLE_RATE as u32
}
