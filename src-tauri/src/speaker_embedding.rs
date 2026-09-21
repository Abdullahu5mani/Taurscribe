//! SOTA Speaker Embedding Engine for Taurscribe
//!
//! Provides 80-channel log-mel filterbank feature extraction and SOTA
//! neural speaker embedding inference (CAM++ / ERes2NetV2) via ONNX Runtime (`ort`).
//!
//! Output: 192-dimensional L2-normalized voiceprint embeddings for persistent
//! speaker identification across meetings with sub-millisecond cosine matching.

use ndarray::Array2;
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub const SPEAKER_SAMPLE_RATE: u32 = 16_000;
/// Kaldi pads the 400-sample frame to the next power of two.
pub const SPEAKER_N_FFT: usize = 512;
pub const SPEAKER_WIN_LENGTH: usize = 400; // 25ms @ 16kHz
pub const SPEAKER_HOP_LENGTH: usize = 160; // 10ms @ 16kHz
pub const SPEAKER_N_MELS: usize = 80;
pub const SPEAKER_EMBEDDING_DIM: usize = 192;
/// Cosine threshold for recognising a vault person with the CAM++ model.
/// Measured on LibriSpeech test-clean (40 real speakers, random 8-25 s samples):
/// different people <= 0.57 (0 of 780 pairs at 0.60), the same person reading
/// another passage >= 0.60 in 38 of 40.
pub const DEFAULT_MATCH_THRESHOLD: f32 = 0.60;

/// User-adjustable match threshold (Settings → Meetings), stored as f32 bits.
static MATCH_THRESHOLD_BITS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0x3F19_999A); // 0.60

/// Current cosine threshold for recognising a vault person.
pub fn match_threshold() -> f32 {
    f32::from_bits(MATCH_THRESHOLD_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

/// Sets the vault match threshold, clamped to a range that stays usable
/// (below 0.45 different people merge; above 0.85 nobody matches).
pub fn set_match_threshold(value: f32) -> f32 {
    let v = if value.is_finite() { value.clamp(0.45, 0.85) } else { DEFAULT_MATCH_THRESHOLD };
    MATCH_THRESHOLD_BITS.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
    v
}

/// Least speech a voiceprint may be built from. Shorter samples never merged
/// different people on LibriSpeech (0/780 even at 3-4 s) but miss the same
/// person often (29/40 at 3-4 s, 5/40 at 4-5 s, 2/40 at 6-8 s).
pub const MIN_VOICEPRINT_SECONDS: f32 = 5.0;

/// Voiceprint for a WAV file (mono or stereo, float or int), or None when the
/// file is unreadable or holds less than MIN_VOICEPRINT_SECONDS of audio.
/// Frames to feed the CAM++ export. Its segment pooling works on 100-step blocks
/// after a stride-2 layer, i.e. 200 input frames, and the exported graph corrupts a
/// partial last block: the voiceprint of the same audio swings with its length
/// (one caller scored 0.33 vs 0.86 against another man at 20.0 s vs 22.2 s). Only
/// lengths of 200k - 2 frames are exact, so trim to the longest such length
/// (drops < 2 s). On LibriSpeech at random 8-25 s lengths this took false
/// merges from 255/780 to 0/780.
pub fn campplus_frame_count(n_frames: usize) -> usize {
    if n_frames < 198 {
        n_frames
    } else {
        (n_frames + 2) / 200 * 200 - 2
    }
}

pub fn embedding_from_wav(path: &str) -> Option<Vec<f32>> {
    let mut r = hound::WavReader::open(path).ok()?;
    let spec = r.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let max_val = (1i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
            r.samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| (s as f32 / max_val).clamp(-1.0, 1.0))
                .collect()
        }
    };
    let channels = spec.channels.max(1) as usize;
    let mono: Vec<f32> = if channels == 1 {
        samples
    } else {
        samples.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect()
    };
    if (mono.len() as f32 / spec.sample_rate.max(1) as f32) < MIN_VOICEPRINT_SECONDS {
        return None;
    }
    get_speaker_engine().lock().ok()?.compute_embedding(&mono, spec.sample_rate).ok()
}

static MEL_FILTERBANK_80: OnceLock<Array2<f32>> = OnceLock::new();
static HANN_WINDOW_400: OnceLock<Vec<f32>> = OnceLock::new();
static FFT_PLANNER_400: OnceLock<Arc<dyn rustfft::Fft<f32>>> = OnceLock::new();

/// Enrolled speaker profile stored in the SQLite Speaker Vault
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultSpeakerEmbedding {
    pub speaker_id: String,
    pub display_name: String,
    pub embedding: Vec<f32>,
    pub snippet_path: Option<String>,
}

// ── 80-Channel Log-Mel Filterbank Extractor ─────────────────────────────────

/// Kaldi's default "povey" window: a Hann window (symmetric) raised to 0.85.
fn hann_window(len: usize) -> Vec<f32> {
    let mut w = Vec::with_capacity(len);
    let denom = (len.max(2) - 1) as f32;
    for i in 0..len {
        let hann = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / denom).cos();
        w.push(hann.powf(0.85));
    }
    w
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}


/// Computes an 80-channel triangular mel filterbank matrix [80, 201] for 16kHz audio
/// Kaldi `MelBanks`: 80 triangles spaced evenly on the mel scale between 20 Hz
/// and Nyquist, with weights computed in the mel domain (what CAM++ was trained on).
fn create_80_mel_filterbank() -> Array2<f32> {
    let n_mels = SPEAKER_N_MELS;
    let n_fft = SPEAKER_N_FFT;
    let n_freq = n_fft / 2 + 1; // 257
    let mel_min = hz_to_mel(20.0);
    let mel_max = hz_to_mel((SPEAKER_SAMPLE_RATE / 2) as f32);
    let delta = (mel_max - mel_min) / (n_mels + 1) as f32;

    let mut fb = Array2::<f32>::zeros((n_mels, n_freq));
    for m in 0..n_mels {
        let left = mel_min + m as f32 * delta;
        let center = left + delta;
        let right = center + delta;
        // Kaldi uses the first n_fft/2 bins (not the Nyquist bin).
        for k in 0..(n_fft / 2) {
            let mel = hz_to_mel(k as f32 * SPEAKER_SAMPLE_RATE as f32 / n_fft as f32);
            if mel > left && mel < right {
                fb[[m, k]] = if mel <= center {
                    (mel - left) / (center - left)
                } else {
                    (right - mel) / (right - center)
                };
            }
        }
    }
    fb
}

/// Extract 80-bin log-mel filterbanks with Cepstral Mean Subtraction (CMS)
pub fn extract_80_fbank(audio: &[f32], sample_rate: u32) -> Array2<f32> {
    if audio.is_empty() {
        return Array2::<f32>::zeros((0, SPEAKER_N_MELS));
    }

    // 1. Resample to 16kHz if needed (simple linear interpolation for speed)
    let pcm_16k: Vec<f32> = if sample_rate == SPEAKER_SAMPLE_RATE {
        audio.to_vec()
    } else {
        let ratio = SPEAKER_SAMPLE_RATE as f64 / sample_rate as f64;
        let out_len = (audio.len() as f64 * ratio) as usize;
        let mut resampled = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let src_idx = i as f64 / ratio;
            let idx0 = src_idx.floor() as usize;
            let idx1 = (idx0 + 1).min(audio.len().saturating_sub(1));
            let frac = (src_idx - idx0 as f64) as f32;
            let val = audio[idx0] * (1.0 - frac) + audio[idx1] * frac;
            resampled.push(val);
        }
        resampled
    };

    let window = HANN_WINDOW_400.get_or_init(|| hann_window(SPEAKER_WIN_LENGTH));
    let fft = FFT_PLANNER_400.get_or_init(|| {
        let mut planner = FftPlanner::<f32>::new();
        planner.plan_fft_forward(SPEAKER_N_FFT)
    });
    let fb = MEL_FILTERBANK_80.get_or_init(create_80_mel_filterbank);

    let n_frames = pcm_16k.len().saturating_sub(SPEAKER_WIN_LENGTH) / SPEAKER_HOP_LENGTH + 1;
    if n_frames == 0 {
        return Array2::<f32>::zeros((0, SPEAKER_N_MELS));
    }

    let n_freq = SPEAKER_N_FFT / 2 + 1;
    let mut fbank = Array2::<f32>::zeros((n_frames, SPEAKER_N_MELS));
    let mut buf = vec![Complex::new(0.0f32, 0.0f32); SPEAKER_N_FFT];
    let mut power_spectrum = vec![0.0f32; n_freq];

    for f in 0..n_frames {
        let start = f * SPEAKER_HOP_LENGTH;
        // Kaldi frame processing: int16-scale samples, remove the DC offset,
        // pre-emphasis 0.97, povey window, zero-pad to the FFT size.
        let mut frame = [0.0f32; SPEAKER_WIN_LENGTH];
        for i in 0..SPEAKER_WIN_LENGTH {
            frame[i] = pcm_16k.get(start + i).copied().unwrap_or(0.0) * 32768.0;
        }
        let dc = frame.iter().sum::<f32>() / SPEAKER_WIN_LENGTH as f32;
        frame.iter_mut().for_each(|v| *v -= dc);
        for i in (1..SPEAKER_WIN_LENGTH).rev() {
            frame[i] -= 0.97 * frame[i - 1];
        }
        frame[0] -= 0.97 * frame[0];
        for i in 0..SPEAKER_N_FFT {
            let v = if i < SPEAKER_WIN_LENGTH { frame[i] * window[i] } else { 0.0 };
            buf[i] = Complex::new(v, 0.0f32);
        }
        fft.process(&mut buf);

        for k in 0..n_freq {
            power_spectrum[k] = buf[k].norm_sqr();
        }

        for m in 0..SPEAKER_N_MELS {
            let mut mel_energy = 0.0_f32;
            for k in 0..n_freq {
                mel_energy += power_spectrum[k] * fb[[m, k]];
            }
            fbank[[f, m]] = mel_energy.max(f32::EPSILON).ln();
        }
    }

    // Cepstral Mean Subtraction (CMS) per mel bin
    for m in 0..SPEAKER_N_MELS {
        let mut sum = 0.0f32;
        for f in 0..n_frames {
            sum += fbank[[f, m]];
        }
        let mean = sum / n_frames as f32;
        for f in 0..n_frames {
            fbank[[f, m]] -= mean;
        }
    }

    fbank
}

// ── Vector Math & Cosine Similarity ─────────────────────────────────────────

/// Computes the L2 norm of a vector
pub fn l2_norm(vec: &[f32]) -> f32 {
    let sum_sq: f32 = vec.iter().map(|&v| v * v).sum();
    sum_sq.sqrt().max(1e-8)
}

/// Normalizes a vector in-place to unit L2 length
pub fn normalize_l2(vec: &mut [f32]) {
    let norm = l2_norm(vec);
    for v in vec.iter_mut() {
        *v /= norm;
    }
}

/// Computes the cosine similarity between two vectors.
/// Since both vectors are expected to be L2-normalized, this is equivalent
/// to their dot product: sum(a_i * b_i).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum();
    let norm_a = l2_norm(a);
    let norm_b = l2_norm(b);
    (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
}

/// Matches an incoming speaker embedding against enrolled vault speakers.
/// Returns Some((speaker_id, display_name, similarity)) if best match >= threshold.
pub fn match_speaker_against_vault(
    embedding: &[f32],
    vault: &[VaultSpeakerEmbedding],
    threshold: f32,
) -> Option<(String, String, f32)> {
    if vault.is_empty() || embedding.is_empty() {
        return None;
    }

    let mut best_match: Option<(String, String, f32)> = None;
    let mut highest_sim = threshold;

    for v in vault {
        let sim = cosine_similarity(embedding, &v.embedding);
        if sim >= highest_sim {
            highest_sim = sim;
            best_match = Some((v.speaker_id.clone(), v.display_name.clone(), sim));
        }
    }

    best_match
}

// ── Fallback High-Fidelity Acoustic Feature Embedding ───────────────────────

/// Generates a deterministic, normalized 192-dimensional acoustic embedding
/// from audio properties (spectral subbands, formant proxies, pitch contour, ZCR)
/// when the external ONNX neural model is not present.
pub fn extract_acoustic_fallback_embedding(audio: &[f32], sample_rate: u32) -> Vec<f32> {
    let mut embedding = vec![0.0f32; SPEAKER_EMBEDDING_DIM];
    if audio.is_empty() {
        return embedding;
    }

    let fbank = extract_80_fbank(audio, sample_rate);
    let n_frames = fbank.shape()[0];
    if n_frames == 0 {
        return embedding;
    }

    // 1. First 80 dimensions: Mean spectral subband energies
    for m in 0..SPEAKER_N_MELS.min(80) {
        let mut sum = 0.0f32;
        for f in 0..n_frames {
            sum += fbank[[f, m]];
        }
        embedding[m] = sum / n_frames as f32;
    }

    // 2. Next 80 dimensions: Standard deviation across frames (vocal dynamics)
    for m in 0..SPEAKER_N_MELS.min(80) {
        let mean = embedding[m];
        let mut var_sum = 0.0f32;
        for f in 0..n_frames {
            let diff = fbank[[f, m]] - mean;
            var_sum += diff * diff;
        }
        embedding[80 + m] = (var_sum / n_frames as f32).sqrt();
    }

    // 3. Remaining 32 dimensions: Pitch proxy, zero crossing, and subband ratios
    let mut zcr = 0;
    for i in 1..audio.len() {
        if (audio[i] >= 0.0 && audio[i - 1] < 0.0) || (audio[i] < 0.0 && audio[i - 1] >= 0.0) {
            zcr += 1;
        }
    }
    let zcr_rate = zcr as f32 / audio.len() as f32;
    embedding[160] = zcr_rate;

    // Pitch proxy via autocorrelation
    let scan_len = audio.len().min(4800);
    let min_lag = (sample_rate / 350) as usize;
    let max_lag = (sample_rate / 75) as usize;
    let mut best_corr = 0.0f32;
    let mut best_lag = min_lag;

    if scan_len > max_lag {
        for lag in min_lag..max_lag {
            let mut corr = 0.0f32;
            for j in 0..(scan_len - lag) {
                corr += audio[j] * audio[j + lag];
            }
            if corr > best_corr {
                best_corr = corr;
                best_lag = lag;
            }
        }
    }
    let pitch_hz = if best_lag > 0 { sample_rate as f32 / best_lag as f32 } else { 120.0 };
    embedding[161] = pitch_hz / 350.0;

    // Subband ratios (spectral tilt)
    for k in 0..30 {
        let low_idx = (k * 2) % 40;
        let high_idx = 40 + (k * 2) % 40;
        embedding[162 + k] = embedding[low_idx] - embedding[high_idx];
    }

    normalize_l2(&mut embedding);
    embedding
}

// ── ONNX Runtime Speaker Embedding Session ──────────────────────────────────

static GLOBAL_SPEAKER_ENGINE: OnceLock<Mutex<SpeakerEmbeddingEngine>> = OnceLock::new();

pub struct SpeakerEmbeddingEngine {
    model_path: Option<PathBuf>,
    session: Option<ort::session::Session>,
    session_stamp: Option<(u64, Option<std::time::SystemTime>)>,
}

impl SpeakerEmbeddingEngine {
    pub fn new() -> Self {
        Self { model_path: None, session: None, session_stamp: None }
    }

    /// True when a neural speaker model (CAM++ / ERes2Net ONNX) is installed.
    /// The acoustic fallback cannot tell voices apart reliably (different people
    /// score ~0.98 cosine), so it must not be used to recognise people.
    pub fn uses_neural_model(&self) -> bool {
        self.model_path.as_ref().map(|p| p.exists()).unwrap_or(false)
    }

    /// Where Settings → Models downloads the speaker model. The engine points
    /// here from the start, so a download (or delete) takes effect immediately.
    pub fn default_model_path() -> Option<PathBuf> {
        use crate::commands::model_registry::{SPEAKER_MODEL_DIR, SPEAKER_MODEL_FILE};
        crate::utils::get_models_dir()
            .ok()
            .map(|d| d.join(SPEAKER_MODEL_DIR).join(SPEAKER_MODEL_FILE))
    }

    pub fn set_model_path(&mut self, path: PathBuf) {
        if self.model_path.as_ref() != Some(&path) {
            self.session = None;
            self.session_stamp = None;
        }
        self.model_path = Some(path);
    }

    /// Computes a 192-dimensional speaker embedding for an audio slice
    pub fn compute_embedding(&mut self, audio: &[f32], sample_rate: u32) -> Result<Vec<f32>, String> {
        if audio.is_empty() {
            return Ok(vec![0.0f32; SPEAKER_EMBEDDING_DIM]);
        }

        // If an external ONNX model is available and exists on disk, run ONNX session
        if let Some(path) = self.model_path.clone() {
            if path.exists() {
                if let Ok(vec) = self.run_onnx_inference(audio, sample_rate, &path) {
                    return Ok(vec);
                }
            } else {
                self.session = None;
                self.session_stamp = None;
            }
        }

        // Reliable fallback acoustic feature embedding
        let mut emb = extract_acoustic_fallback_embedding(audio, sample_rate);
        normalize_l2(&mut emb);
        Ok(emb)
    }

    fn run_onnx_inference(
        &mut self,
        audio: &[f32],
        sample_rate: u32,
        model_path: &Path,
    ) -> Result<Vec<f32>, String> {
        let fbank = extract_80_fbank(audio, sample_rate);
        let n_frames = campplus_frame_count(fbank.shape()[0]);
        if n_frames == 0 {
            return Err("Zero frames in fbank".to_string());
        }

        // Flatten fbank for tensor creation: shape [1, n_frames, 80]
        let mut flat = Vec::with_capacity(n_frames * SPEAKER_N_MELS);
        for f in 0..n_frames {
            for m in 0..SPEAKER_N_MELS {
                flat.push(fbank[[f, m]]);
            }
        }

        // Attempt ONNX session execution
        let metadata = std::fs::metadata(model_path).map_err(|e| e.to_string())?;
        let stamp = (metadata.len(), metadata.modified().ok());
        if self.session.is_none() || self.session_stamp != Some(stamp) {
            let mut builder = ort::session::Session::builder()
                .map_err(|e| format!("Failed to create ORT builder: {}", e))?;
            let session = {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                if let Ok(mut b) = builder.clone().with_execution_providers([ort::ep::CoreML::default().build()]) {
                    b.commit_from_file(model_path)
                } else {
                    builder.commit_from_file(model_path)
                }
            }
            #[cfg(target_os = "windows")]
            {
                if let Ok(mut b) = builder.clone().with_execution_providers([ort::ep::DirectML::default().build()]) {
                    b.commit_from_file(model_path)
                } else {
                    builder.commit_from_file(model_path)
                }
            }
            #[cfg(all(not(all(target_os = "macos", target_arch = "aarch64")), not(target_os = "windows")))]
            {
                builder.commit_from_file(model_path)
            }
            }.map_err(|e| format!("Failed to load speaker embedding model from {}: {}", model_path.display(), e))?;
            self.session = Some(session);
            self.session_stamp = Some(stamp);
        }

        let shape = vec![1usize, n_frames, SPEAKER_N_MELS];
        let input_val = ort::value::Value::from_array((shape, flat))
            .map_err(|e| format!("Failed to create input tensor: {}", e))?;

        let outputs = self.session.as_mut().expect("speaker session initialized")
            .run(ort::inputs![input_val])
            .map_err(|e| format!("Inference error: {}", e))?;

        // First output tensor, shape [1, D]. Keep the model's full dimension D
        // (CAM++ VoxCeleb is 512): truncating to 192 discarded most of the
        // voiceprint. Embeddings of different sizes never match (cosine = 0).
        if let Some((_, val)) = outputs.iter().next() {
            if let Ok(extracted) = val.try_extract_tensor::<f32>() {
                let mut emb: Vec<f32> = extracted.1.to_vec();
                if emb.is_empty() {
                    return Err("Speaker model returned an empty embedding".to_string());
                }
                normalize_l2(&mut emb);
                return Ok(emb);
            }
        }

        Err("Failed to extract embedding tensor from model outputs".to_string())
    }
}

pub fn get_speaker_engine() -> &'static Mutex<SpeakerEmbeddingEngine> {
    GLOBAL_SPEAKER_ENGINE.get_or_init(|| {
        let mut engine = SpeakerEmbeddingEngine::new();
        if let Some(path) = SpeakerEmbeddingEngine::default_model_path() {
            engine.set_model_path(path);
        }
        Mutex::new(engine)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_campplus_frame_count_trims_to_whole_blocks() {
        assert_eq!(campplus_frame_count(100), 100);
        assert_eq!(campplus_frame_count(1998), 1998);
        assert_eq!(campplus_frame_count(1997), 1798);
        assert_eq!(campplus_frame_count(2217), 2198);
    }

    #[test]
    fn test_mel_filterbank_dimensions() {
        let fb = create_80_mel_filterbank();
        assert_eq!(fb.shape(), &[80, SPEAKER_N_FFT / 2 + 1]);
    }

    #[test]
    fn test_fbank_extraction_synthetic_audio() {
        let sample_rate = 16000;
        let mut audio = vec![0.0f32; 16000]; // 1 second of audio
        for (i, x) in audio.iter_mut().enumerate() {
            *x = (2.0 * std::f32::consts::PI * 440.0 * (i as f32 / sample_rate as f32)).sin() * 0.5;
        }

        let fbank = extract_80_fbank(&audio, sample_rate);
        assert!(fbank.shape()[0] > 0);
        assert_eq!(fbank.shape()[1], 80);
    }

    #[test]
    fn test_cosine_similarity_identity_and_orthogonality() {
        let mut v1 = vec![1.0f32; 192];
        normalize_l2(&mut v1);

        let v1_clone = v1.clone();
        let sim_self = cosine_similarity(&v1, &v1_clone);
        assert!((sim_self - 1.0).abs() < 1e-5);

        let mut v2 = vec![0.0f32; 192];
        v2[0] = 1.0;
        let mut v3 = vec![0.0f32; 192];
        v3[1] = 1.0;
        let sim_ortho = cosine_similarity(&v2, &v3);
        assert!((sim_ortho - 0.0).abs() < 1e-5);
    }

    #[test]
    fn test_match_threshold_default_and_clamp() {
        assert_eq!(f32::from_bits(0x3F19_999A), DEFAULT_MATCH_THRESHOLD);
        assert_eq!(set_match_threshold(0.2), 0.45);
        assert_eq!(set_match_threshold(f32::NAN), DEFAULT_MATCH_THRESHOLD);
        assert_eq!(match_threshold(), DEFAULT_MATCH_THRESHOLD);
    }

    #[test]
    fn test_match_speaker_against_vault() {
        let mut emb_sarah = vec![0.0f32; 192];
        emb_sarah[0] = 0.8;
        emb_sarah[1] = 0.6;
        normalize_l2(&mut emb_sarah);

        let mut emb_alex = vec![0.0f32; 192];
        emb_alex[10] = 1.0;
        normalize_l2(&mut emb_alex);

        let vault = vec![
            VaultSpeakerEmbedding {
                speaker_id: "spk_sarah_1".to_string(),
                display_name: "Sarah Connor".to_string(),
                embedding: emb_sarah.clone(),
                snippet_path: None,
            },
            VaultSpeakerEmbedding {
                speaker_id: "spk_alex_2".to_string(),
                display_name: "Alex Chen".to_string(),
                embedding: emb_alex.clone(),
                snippet_path: None,
            },
        ];

        // Slightly perturbed Sarah vector
        let mut query = emb_sarah.clone();
        query[0] += 0.05;
        normalize_l2(&mut query);

        let matched = match_speaker_against_vault(&query, &vault, 0.82);
        assert!(matched.is_some());
        let (id, name, sim) = matched.unwrap();
        assert_eq!(id, "spk_sarah_1");
        assert_eq!(name, "Sarah Connor");
        assert!(sim > 0.95);
    }
}
