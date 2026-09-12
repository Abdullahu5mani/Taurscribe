//! Full End-to-End Native Apple Silicon Metal GPU Engine for Parakeet Nemotron.
//!
//! Replaces CPU ONNX Runtime on macOS with native MLX Metal operations:
//! - Mel Spectrogram Preprocessing (Slaney filterbank + FFT)
//! - 8x Depthwise-Separable Convolution Subsampling
//! - 24-Layer FastConformer Encoder with Rel-Pos Attention & Conv Streaming Caches
//! - 2-Layer LSTM Predictor Network
//! - Joint Network & Greedy RNN-T Decoding
//! - SentencePiece Vocabulary Decoding

use std::f32::consts::PI;
use std::path::Path;
use std::sync::Arc;

use mlx_rs::ops::indexing::IndexOp;
use mlx_rs::{Array, Dtype};
use ndarray::Array2;
use rustfft::{num_complex::Complex, Fft, FftPlanner};

use super::conformer::FastConformerEncoder;
use super::decoder::{JointNetwork, PredictorNetwork};
use super::subsampling::ConvSubsampling;
use parakeet_rs::SentencePieceVocab;

// Nemotron 0.6B audio constants
const SAMPLE_RATE: usize = 16000;
const N_FFT: usize = 512;
const WIN_LENGTH: usize = 400;
const HOP_LENGTH: usize = 160;
const N_MELS: usize = 128;
const PREEMPH: f32 = 0.97;
const LOG_ZERO_GUARD: f32 = 5.960_464_5e-8;
const FMAX: f32 = 8000.0;

// Streaming chunk config
const CHUNK_SIZE: usize = 56;
const PRE_ENCODE_CACHE: usize = 9;

// RNN-T constants
const VOCAB_SIZE: usize = 1024;
const BLANK_ID: usize = 1024;
const MAX_SYMBOLS_PER_STEP: usize = 10;
const RNNT_CONFIDENCE_THRESHOLD: f32 = 0.3;

pub struct ParakeetNemotronMlx {
    subsampling: ConvSubsampling,
    encoder: FastConformerEncoder,
    predictor: PredictorNetwork,
    joint: JointNetwork,
    vocab: SentencePieceVocab,

    // Streaming caches for Conformer encoder
    caches_channel: Vec<Array>, // 24 of [1, 70, 1024]
    caches_time: Vec<Array>,    // 24 of [1, 8, 1024]
    cache_len: i32,

    // Streaming states for LSTM decoder
    h0: Array, // [1, 640]
    c0: Array, // [1, 640]
    h1: Array, // [1, 640]
    c1: Array, // [1, 640]
    last_token: i32,

    // Audio preprocessing buffers
    mel_basis: Array2<f32>,
    window: Vec<f32>,
    fft_plan: Arc<dyn Fft<f32>>,
    audio_buffer: Vec<f32>,
    audio_processed: usize,
    chunk_idx: usize,
    last_preemph_sample: f32,
}

impl ParakeetNemotronMlx {
    pub fn load<P: AsRef<Path>>(model_dir: P) -> Result<Self, String> {
        let model_dir = model_dir.as_ref();
        let safetensors_path = model_dir.join("model.safetensors");
        let tokenizer_path = model_dir.join("tokenizer.model");

        if !safetensors_path.exists() {
            return Err(format!("Missing {}", safetensors_path.display()));
        }
        if !tokenizer_path.exists() {
            return Err(format!("Missing {}", tokenizer_path.display()));
        }

        println!("[ParakeetNemotronMlx] Loading safetensors from {}", safetensors_path.display());
        let tensors = Array::load_safetensors(safetensors_path.to_str().unwrap())
            .map_err(|e| format!("Failed to load safetensors: {e}"))?;

        let subsampling = ConvSubsampling::load(&tensors)
            .map_err(|e| format!("Failed to load subsampling: {e}"))?;
        let encoder = FastConformerEncoder::load(&tensors)
            .map_err(|e| format!("Failed to load encoder: {e}"))?;
        let predictor = PredictorNetwork::load(&tensors)
            .map_err(|e| format!("Failed to load predictor: {e}"))?;
        let joint = JointNetwork::load(&tensors)
            .map_err(|e| format!("Failed to load joint network: {e}"))?;

        let vocab = SentencePieceVocab::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer: {e:?}"))?;

        let mut caches_channel = Vec::with_capacity(24);
        let mut caches_time = Vec::with_capacity(24);
        for _ in 0..24 {
            caches_channel.push(
                Array::zeros::<f32>(&[1, 70, 1024])
                    .map_err(|e| format!("mlx zeros error: {e}"))?
                    .as_dtype(Dtype::Float16)
                    .map_err(|e| format!("mlx dtype error: {e}"))?,
            );
            caches_time.push(
                Array::zeros::<f32>(&[1, 8, 1024])
                    .map_err(|e| format!("mlx zeros error: {e}"))?
                    .as_dtype(Dtype::Float16)
                    .map_err(|e| format!("mlx dtype error: {e}"))?,
            );
        }

        let h0 = Array::zeros::<f32>(&[1, 640]).map_err(|e| format!("mlx zeros: {e}"))?;
        let c0 = Array::zeros::<f32>(&[1, 640]).map_err(|e| format!("mlx zeros: {e}"))?;
        let h1 = Array::zeros::<f32>(&[1, 640]).map_err(|e| format!("mlx zeros: {e}"))?;
        let c1 = Array::zeros::<f32>(&[1, 640]).map_err(|e| format!("mlx zeros: {e}"))?;

        let fft_plan = {
            let mut planner = FftPlanner::<f32>::new();
            planner.plan_fft_forward(N_FFT)
        };

        Ok(Self {
            subsampling,
            encoder,
            predictor,
            joint,
            vocab,
            caches_channel,
            caches_time,
            cache_len: 0,
            h0,
            c0,
            h1,
            c1,
            last_token: BLANK_ID as i32,
            mel_basis: Self::create_mel_filterbank(),
            window: Self::create_window(),
            fft_plan,
            audio_buffer: Vec::new(),
            audio_processed: 0,
            chunk_idx: 0,
            last_preemph_sample: 0.0,
        })
    }

    /// Reset internal streaming contexts for new utterance
    pub fn reset(&mut self) {
        for ch in &mut self.caches_channel {
            if let Ok(z) = Array::zeros::<f32>(&[1, 70, 1024]) {
                if let Ok(h) = z.as_dtype(Dtype::Float16) {
                    *ch = h;
                }
            }
        }
        for tm in &mut self.caches_time {
            if let Ok(z) = Array::zeros::<f32>(&[1, 8, 1024]) {
                if let Ok(h) = z.as_dtype(Dtype::Float16) {
                    *tm = h;
                }
            }
        }
        self.cache_len = 0;
        if let Ok(z) = Array::zeros::<f32>(&[1, 640]) {
            self.h0 = z.clone();
            self.c0 = z.clone();
            self.h1 = z.clone();
            self.c1 = z;
        }
        self.last_token = BLANK_ID as i32;
        self.audio_buffer.clear();
        self.audio_processed = 0;
        self.chunk_idx = 0;
        self.last_preemph_sample = 0.0;
    }

    /// Transcribe a streaming chunk of audio
    pub fn transcribe_chunk(&mut self, audio_chunk: &[f32]) -> Result<String, String> {
        self.audio_buffer.extend_from_slice(audio_chunk);

        let total_audio = self.audio_buffer.len();
        if total_audio < WIN_LENGTH {
            return Ok(String::new());
        }

        let full_mel = self.compute_mel_spectrogram(&self.audio_buffer, self.last_preemph_sample);
        let total_mel_frames = full_mel.shape()[1];

        let processed_mel_frames = self.audio_processed / HOP_LENGTH;
        let available_new_frames = total_mel_frames.saturating_sub(processed_mel_frames);
        if available_new_frames < CHUNK_SIZE {
            return Ok(String::new());
        }

        let expected_size = PRE_ENCODE_CACHE + CHUNK_SIZE;
        let mut chunk_data = vec![0.0f32; N_MELS * expected_size];

        let is_first_chunk = self.chunk_idx == 0;
        let main_start = processed_mel_frames;

        if is_first_chunk {
            for f in 0..CHUNK_SIZE.min(total_mel_frames) {
                for m in 0..N_MELS {
                    chunk_data[m * expected_size + PRE_ENCODE_CACHE + f] = full_mel[[m, f]];
                }
            }
        } else {
            let cache_start = main_start.saturating_sub(PRE_ENCODE_CACHE);
            let cache_frames = main_start - cache_start;
            let cache_offset = PRE_ENCODE_CACHE - cache_frames;

            for f in 0..cache_frames {
                for m in 0..N_MELS {
                    chunk_data[m * expected_size + cache_offset + f] = full_mel[[m, cache_start + f]];
                }
            }

            for f in 0..CHUNK_SIZE.min(total_mel_frames - main_start) {
                for m in 0..N_MELS {
                    chunk_data[m * expected_size + PRE_ENCODE_CACHE + f] = full_mel[[m, main_start + f]];
                }
            }
        }

        // Convert mel chunk to MLX array [1, 128, expected_size]
        let mel_arr = Array::from_slice(&chunk_data, &[1, N_MELS as i32, expected_size as i32]);

        // 1. Subsampling: [1, 128, 65] -> [1, 7, 1024]
        let x_sub = self
            .subsampling
            .forward(&mel_arr)
            .map_err(|e| format!("Subsampling error: {e}"))?;

        // 2. FastConformer 24 layers: [1, 7, 1024] -> [1, 1024, 7]
        let (encoded, new_caches_ch, new_caches_tm) = self
            .encoder
            .forward(&x_sub, &self.caches_channel, &self.caches_time, self.cache_len)
            .map_err(|e| format!("Encoder error: {e}"))?;

        self.caches_channel = new_caches_ch;
        self.caches_time = new_caches_tm;
        self.cache_len = (self.cache_len + 7).min(70);

        // 3. Greedy RNN-T decoding
        let enc_frames = encoded.shape()[2];
        let mut emitted_tokens = Vec::new();

        for t in 0..enc_frames {
            // Frame: [1, 1024, 1] -> transpose to [1, 1, 1024] -> reshape [1, 1024]
            let frame = encoded
                .index((.., .., t..(t + 1)))
                .transpose_axes(&[0, 2, 1])
                .map_err(|e| format!("Index frame error: {e}"))?
                .reshape(&[1, 1024])
                .map_err(|e| format!("Reshape frame error: {e}"))?;

            for _ in 0..MAX_SYMBOLS_PER_STEP {
                let (pred_out, next_h0, next_c0, next_h1, next_c1) = self
                    .predictor
                    .step(self.last_token, &self.h0, &self.c0, &self.h1, &self.c1)
                    .map_err(|e| format!("Predictor error: {e}"))?;

                let logits = self
                    .joint
                    .forward(&frame, &pred_out)
                    .map_err(|e| format!("Joint error: {e}"))?;

                let logits_slice = logits.as_slice::<f32>();
                let mut max_idx = 0;
                let mut max_val = f32::NEG_INFINITY;
                for (i, &v) in logits_slice.iter().enumerate() {
                    if v > max_val {
                        max_val = v;
                        max_idx = i;
                    }
                }

                if max_idx == BLANK_ID {
                    break;
                }

                let blank_logit = logits_slice[BLANK_ID];
                if max_val - blank_logit < RNNT_CONFIDENCE_THRESHOLD {
                    break;
                }

                emitted_tokens.push(max_idx);
                self.last_token = max_idx as i32;
                self.h0 = next_h0;
                self.c0 = next_c0;
                self.h1 = next_h1;
                self.c1 = next_c1;
            }
        }

        // Advance processed position
        self.audio_processed += CHUNK_SIZE * HOP_LENGTH;
        self.chunk_idx += 1;

        // Trim audio buffer
        let keep_samples = (PRE_ENCODE_CACHE + CHUNK_SIZE) * HOP_LENGTH + WIN_LENGTH;
        if self.audio_buffer.len() > keep_samples * 2 {
            let remove = self.audio_buffer.len() - keep_samples;
            let actual_remove = remove.min(self.audio_processed);
            if actual_remove > 0 {
                self.last_preemph_sample = self.audio_buffer[actual_remove - 1];
                self.audio_buffer.drain(0..actual_remove);
                self.audio_processed -= actual_remove;
            }
        }

        let mut result = String::new();
        for &t in &emitted_tokens {
            if t < VOCAB_SIZE {
                result.push_str(&self.vocab.decode_single(t));
            }
        }
        Ok(result)
    }

    fn compute_mel_spectrogram(&self, audio: &[f32], prior_sample: f32) -> Array2<f32> {
        if audio.is_empty() {
            return Array2::zeros((N_MELS, 0));
        }
        let preemph = Self::apply_preemphasis(audio, prior_sample);
        let spec = self.stft_center(&preemph);
        let mel = self.mel_basis.dot(&spec);
        mel.mapv(|x| (x.max(0.0) + LOG_ZERO_GUARD).ln())
    }

    fn apply_preemphasis(audio: &[f32], prior_sample: f32) -> Vec<f32> {
        if audio.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(audio.len());
        let first = audio[0] - PREEMPH * prior_sample;
        result.push(if first.is_finite() { first } else { 0.0 });
        for i in 1..audio.len() {
            let v = audio[i] - PREEMPH * audio[i - 1];
            result.push(if v.is_finite() { v } else { 0.0 });
        }
        result
    }

    fn stft_center(&self, audio: &[f32]) -> Array2<f32> {
        let pad_amount = N_FFT / 2;
        let mut padded = vec![0.0f32; pad_amount];
        padded.extend_from_slice(audio);
        padded.extend(std::iter::repeat_n(0.0f32, pad_amount));

        let num_frames = if padded.len() >= WIN_LENGTH {
            1 + (padded.len() - WIN_LENGTH) / HOP_LENGTH
        } else {
            0
        };

        let freq_bins = N_FFT / 2 + 1;
        let mut spec = Array2::zeros((freq_bins, num_frames));
        let mut fft_buf: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); N_FFT];

        for frame_idx in 0..num_frames {
            let start = frame_idx * HOP_LENGTH;
            if start + WIN_LENGTH > padded.len() {
                break;
            }
            for v in fft_buf.iter_mut() {
                *v = Complex::new(0.0, 0.0);
            }
            for i in 0..WIN_LENGTH {
                fft_buf[i] = Complex::new(padded[start + i] * self.window[i], 0.0);
            }
            self.fft_plan.process(&mut fft_buf);
            for (i, val) in fft_buf.iter().take(freq_bins).enumerate() {
                let mag_sq = val.norm_sqr();
                spec[[i, frame_idx]] = if mag_sq.is_finite() { mag_sq } else { 0.0 };
            }
        }
        spec
    }

    fn create_window() -> Vec<f32> {
        (0..WIN_LENGTH)
            .map(|i| 0.5 - 0.5 * ((2.0 * PI * i as f32) / ((WIN_LENGTH - 1) as f32)).cos())
            .collect()
    }

    fn create_mel_filterbank() -> Array2<f32> {
        let num_freqs = N_FFT / 2 + 1;
        const F_SP: f32 = 200.0 / 3.0;
        const MIN_LOG_HZ: f32 = 1000.0;
        const MIN_LOG_MEL: f32 = MIN_LOG_HZ / F_SP;
        const LOG_STEP: f32 = 0.06875177742094912;

        let hz_to_mel = |hz: f32| -> f32 {
            if hz < MIN_LOG_HZ {
                hz / F_SP
            } else {
                MIN_LOG_MEL + (hz / MIN_LOG_HZ).ln() / LOG_STEP
            }
        };

        let mel_to_hz = |mel: f32| -> f32 {
            if mel < MIN_LOG_MEL {
                mel * F_SP
            } else {
                MIN_LOG_HZ * ((mel - MIN_LOG_MEL) * LOG_STEP).exp()
            }
        };

        let mel_min = hz_to_mel(0.0);
        let mel_max = hz_to_mel(FMAX);

        let mel_points: Vec<f32> = (0..=N_MELS + 1)
            .map(|i| mel_to_hz(mel_min + (mel_max - mel_min) * i as f32 / (N_MELS + 1) as f32))
            .collect();

        let fft_freqs: Vec<f32> = (0..num_freqs)
            .map(|i| (SAMPLE_RATE as f32 / N_FFT as f32) * i as f32)
            .collect();

        let mut weights = Array2::zeros((N_MELS, num_freqs));

        for i in 0..N_MELS {
            let left = mel_points[i];
            let center = mel_points[i + 1];
            let right = mel_points[i + 2];

            for (j, &freq) in fft_freqs.iter().enumerate() {
                if freq >= left && freq <= center && center != left {
                    weights[[i, j]] = (freq - left) / (center - left);
                } else if freq > center && freq <= right && right != center {
                    weights[[i, j]] = (right - freq) / (right - center);
                }
            }
        }
        weights
    }
}
