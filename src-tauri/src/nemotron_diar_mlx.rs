//! Nemotron-3 Diarization on MLX (Apple Silicon), ported from mlx-audio's
//! `nemotron_diarization.py` (itself following NVIDIA NeMo's Streaming
//! Sortformer). Weights: `mlx-community/Nemotron-3-Diarization` (BF16).
//!
//! Split of work: log-mels and the Arrival-Order Speaker Cache (AOSC)
//! bookkeeping run on the CPU (small arrays, exact control over ties); the
//! 31-layer RoPE transformer and speaker head run on the GPU through MLX.
//! Only NVIDIA's offline preset is used (30.4 s lookahead): meetings are
//! diarized once, after recording, so accuracy wins over latency.

use std::collections::HashMap;
use std::path::Path;

use mlx_rs::ops::indexing::{take_axis, IndexOp};
use mlx_rs::{error::Exception, fast, ops, Array, Dtype};
use rustfft::{num_complex::Complex, FftPlanner};

use crate::neural_diarizer::SpeakerTurn;

// Model geometry (config.json of the MLX conversion).
const D_MODEL: i32 = 512;
const N_HEADS: i32 = 8;
const HEAD_DIM: i32 = D_MODEL / N_HEADS;
const N_LAYERS: usize = 31;
const TF_D: i32 = 192;
const N_SPK: usize = 8;
const N_MELS: usize = 128;
const FACTOR: usize = 8; // mel frames per encoder frame (80 ms)

// Front end (processor_config).
const N_FFT: usize = 512;
const HOP: usize = 160;
const PREEMPH: f32 = 0.97;
const PAD_TO: usize = 16;

// Offline streaming preset + AOSC parameters (modules_config).
const CHUNK_LEN: usize = 340;
const RIGHT_CONTEXT: usize = 40;
const FIFO_LEN: usize = 40;
const UPDATE_PERIOD: usize = 300;
const SPKCACHE_LEN: usize = 264;
const SIL_FRAMES_PER_SPK: usize = 1;
const PRED_SCORE_THRESHOLD: f32 = 0.25;
const SCORES_BOOST_LATEST: f32 = 0.05;
const STRONG_BOOST_RATE: f32 = 0.75;
const WEAK_BOOST_RATE: f32 = 1.5;
const MIN_POS_SCORES_RATE: f32 = 0.5;
const ACTIVITY_THRESHOLD: f32 = 0.5;
const CACHE_LIMIT_BYTES: usize = 256 << 20;

type Weights = HashMap<String, Array>;

fn get(w: &Weights, key: &str) -> Result<Array, Exception> {
    w.get(key)
        .cloned()
        .ok_or_else(|| Exception::custom(format!("missing tensor: {key}")))
}

struct Linear {
    weight_t: Array,
    bias: Option<Array>,
}

impl Linear {
    fn load(w: &Weights, prefix: &str, bias: bool) -> Result<Self, Exception> {
        Ok(Self {
            weight_t: get(w, &format!("{prefix}.weight"))?.transpose_axes(&[1, 0])?,
            bias: if bias { Some(get(w, &format!("{prefix}.bias"))?) } else { None },
        })
    }

    fn forward(&self, x: &Array) -> Result<Array, Exception> {
        let y = x.matmul(&self.weight_t)?;
        match &self.bias {
            Some(b) => y.add(b),
            None => Ok(y),
        }
    }
}

struct LayerNorm {
    weight: Array,
    bias: Array,
}

impl LayerNorm {
    fn load(w: &Weights, prefix: &str) -> Result<Self, Exception> {
        Ok(Self { weight: get(w, &format!("{prefix}.weight"))?, bias: get(w, &format!("{prefix}.bias"))? })
    }

    fn forward(&self, x: &Array) -> Result<Array, Exception> {
        fast::layer_norm(x, Some(&self.weight), Some(&self.bias), 1e-5)
    }
}

fn relu(x: &Array) -> Result<Array, Exception> {
    ops::maximum(x, Array::from_f32(0.0))
}

/// Exact (erf) GELU, as `mlx.nn.gelu`.
fn gelu(x: &Array) -> Result<Array, Exception> {
    let inner = ops::erf(x.multiply(Array::from_f32(std::f32::consts::FRAC_1_SQRT_2))?)?;
    x.multiply(inner.add(Array::from_f32(1.0))?)?.multiply(Array::from_f32(0.5))
}

struct Block {
    norm1: LayerNorm,
    qkv: Linear,
    out: Linear,
    norm2: LayerNorm,
    ff1: Linear,
    ff2: Linear,
}

impl Block {
    fn load(w: &Weights, i: usize) -> Result<Self, Exception> {
        let p = format!("encoder.layers.{i}");
        Ok(Self {
            norm1: LayerNorm::load(w, &format!("{p}.norm1"))?,
            qkv: Linear::load(w, &format!("{p}.attn.w_qkv"), false)?,
            out: Linear::load(w, &format!("{p}.attn.out_proj"), true)?,
            norm2: LayerNorm::load(w, &format!("{p}.norm2"))?,
            ff1: Linear::load(w, &format!("{p}.ffn.linear1"), true)?,
            ff2: Linear::load(w, &format!("{p}.ffn.linear2"), true)?,
        })
    }

    fn forward(&self, x: &Array, mask: &Array) -> Result<Array, Exception> {
        let t = x.shape()[1];
        let qkv = self.qkv.forward(&self.norm1.forward(x)?)?.reshape(&[1, t, 3, N_HEADS, HEAD_DIM])?;
        let part = |i: i32| -> Result<Array, Exception> {
            qkv.index((.., .., i)).transpose_axes(&[0, 2, 1, 3])
        };
        let rope = |a: &Array| fast::rope(a, HEAD_DIM, false, 10_000.0f32, 1.0, 0, None);
        let (q, k, v) = (rope(&part(0)?)?, rope(&part(1)?)?, part(2)?);
        let a = fast::scaled_dot_product_attention(&q, &k, &v, (HEAD_DIM as f32).powf(-0.5), mask, None)?;
        let x = x.add(self.out.forward(&a.transpose_axes(&[0, 2, 1, 3])?.reshape(&[1, t, D_MODEL])?)?)?;
        let h = self.ff2.forward(&gelu(&self.ff1.forward(&self.norm2.forward(&x)?)?)?)?;
        x.add(h)
    }
}

pub struct NemotronDiarMlx {
    proj: Linear,
    embed_norm: LayerNorm,
    blocks: Vec<Block>,
    final_norm: LayerNorm,
    encoder_proj: Linear,
    upsample_w: Array,
    upsample_b: Array,
    hidden: Linear,
    to_spks: Linear,
    sil_emb: Array,
    dtype: Dtype,
    window: Vec<f32>,   // 400-tap Hann, centred in N_FFT
    mel_fb: Vec<f32>,   // (N_MELS, N_FFT/2+1)
}

/// Streaming state; embeddings stay on the GPU, predictions on the CPU.
struct State {
    spkcache: Array,
    spkcache_preds: Vec<[f32; N_SPK]>,
    fifo: Array,
    fifo_preds: Vec<[f32; N_SPK]>,
    compressed: bool,
    frames_processed: usize,
}

impl NemotronDiarMlx {
    pub fn load(path: &Path) -> Result<Self, String> {
        let w = Array::load_safetensors(path).map_err(|e| format!("load {}: {e}", path.display()))?;
        let ex = |e: Exception| e.to_string();
        let host = |key: &str| -> Result<Vec<f32>, String> {
            let a = get(&w, key).map_err(ex)?.as_dtype(Dtype::Float32).map_err(ex)?;
            a.eval().map_err(ex)?;
            Ok(a.as_slice::<f32>().to_vec())
        };
        let taps = host("preprocessor.window")?;
        let pad = (N_FFT - taps.len()) / 2;
        let mut window = vec![0.0; N_FFT];
        window[pad..pad + taps.len()].copy_from_slice(&taps);
        let mel_fb = host("preprocessor.fb")?;
        if mel_fb.len() != N_MELS * (N_FFT / 2 + 1) {
            return Err(format!("unexpected mel filterbank size {}", mel_fb.len()));
        }
        let proj = Linear::load(&w, "encoder.pre_encode.proj", false).map_err(ex)?;
        let dtype = proj.weight_t.dtype();
        Ok(Self {
            proj,
            embed_norm: LayerNorm::load(&w, "encoder.embed_norm").map_err(ex)?,
            blocks: (0..N_LAYERS).map(|i| Block::load(&w, i)).collect::<Result<_, _>>().map_err(ex)?,
            final_norm: LayerNorm::load(&w, "encoder.final_norm").map_err(ex)?,
            encoder_proj: Linear::load(&w, "sortformer_modules.encoder_proj", true).map_err(ex)?,
            upsample_w: get(&w, "sortformer_modules.subpixel_upsample.weight").map_err(ex)?,
            upsample_b: get(&w, "sortformer_modules.subpixel_upsample.bias").map_err(ex)?,
            hidden: Linear::load(&w, "sortformer_modules.first_hidden_to_hidden", true).map_err(ex)?,
            to_spks: Linear::load(&w, "sortformer_modules.single_hidden_to_spks", true).map_err(ex)?,
            sil_emb: get(&w, "sortformer_modules.learnable_sil_emb").map_err(ex)?.reshape(&[1, 1, D_MODEL]).map_err(ex)?,
            dtype,
            window,
            mel_fb,
        })
    }

    /// Diarizes 16 kHz mono PCM into speaker turns (arrival-ordered speakers).
    pub fn diarize(&self, pcm: &[f32]) -> Result<Vec<SpeakerTurn>, String> {
        let probs = self.speaker_probs(pcm).map_err(|e| e.to_string())?;
        Ok(probs_to_turns(&probs))
    }

    /// Per-speaker activity every 10 ms, mirroring mlx-audio's `generate`.
    pub fn speaker_probs(&self, pcm: &[f32]) -> Result<Vec<[f32; N_SPK]>, Exception> {
        let total = pcm.len();
        let valid_frames = total / HOP;
        let mut total_frames = valid_frames + 1; // NeMo's extra centred frame, masked
        total_frames += (PAD_TO - total_frames % PAD_TO) % PAD_TO;
        let (central, right) = (CHUNK_LEN * FACTOR, RIGHT_CONTEXT * FACTOR);
        let empty = |d: i32| Array::zeros::<f32>(&[1, 0, d])?.as_dtype(self.dtype);
        let mut st = State {
            spkcache: empty(D_MODEL)?,
            spkcache_preds: Vec::new(),
            fifo: empty(D_MODEL)?,
            fifo_preds: Vec::new(),
            compressed: false,
            frames_processed: 0,
        };
        // Every window has the same shapes, so a small buffer cache is enough;
        // MLX's default keeps up to the whole GPU working set cached.
        mlx_rs::memory::set_cache_limit(CACHE_LIMIT_BYTES)?;
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(N_FFT);
        let mut out: Vec<[f32; N_SPK]> = Vec::with_capacity(valid_frames);
        loop {
            let available = valid_frames.saturating_sub(st.frames_processed);
            if available == 0 {
                break;
            }
            let n = central.min(available);
            let window_frames = (central + right).min(total_frames - st.frames_processed);
            let feats = self.log_mel(pcm, st.frames_processed, window_frames, valid_frames, &fft);
            let probs = self.step(&feats, window_frames, &mut st, n, window_frames.min(available))?;
            out.extend(probs);
        }
        out.truncate(st.frames_processed);
        drop(st);
        mlx_rs::memory::clear_cache()?;
        Ok(out)
    }

    /// NeMo log-mels for frames `start..start+count`, row-major (count, N_MELS),
    /// split across CPU cores.
    fn log_mel(&self, x: &[f32], start: usize, count: usize, valid: usize, fft: &std::sync::Arc<dyn rustfft::Fft<f32>>) -> Vec<f32> {
        let mut out = vec![0.0f32; count * N_MELS];
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
        let rows_per = count.div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            for (i, part) in out.chunks_mut(rows_per * N_MELS).enumerate() {
                let first = start + i * rows_per;
                let (window, fb) = (&self.window[..], &self.mel_fb[..]);
                scope.spawn(move || log_mel_rows(window, fb, x, first, part, valid, &**fft));
            }
        });
        out
    }

}

fn log_mel_rows(window: &[f32], mel_fb: &[f32], x: &[f32], first: usize, out: &mut [f32], valid: usize, fft: &dyn rustfft::Fft<f32>) {
        let bins = N_FFT / 2 + 1;
        let n = x.len() as i64;
        let at = |p: i64| if p >= 0 && p < n { x[p as usize] } else { 0.0 };
        let mut buf = vec![Complex::new(0.0f32, 0.0); N_FFT];
        let mut power = vec![0.0f32; bins];
        for (row, dst) in out.chunks_mut(N_MELS).enumerate() {
            let t = first + row;
            if t >= valid {
                continue; // zeroed, as the reference masks padding frames
            }
            for (i, c) in buf.iter_mut().enumerate() {
                let p = (t * HOP + i) as i64 - (N_FFT / 2) as i64;
                let v = if p >= 0 && p < n { at(p) - PREEMPH * at(p - 1) } else { 0.0 };
                *c = Complex::new(v * window[i], 0.0);
            }
            fft.process(&mut buf);
            for (k, pw) in power.iter_mut().enumerate() {
                *pw = buf[k].norm_sqr();
            }
            for (m, d) in dst.iter_mut().enumerate() {
                let fb = &mel_fb[m * bins..(m + 1) * bins];
                let e: f32 = fb.iter().zip(&power).map(|(a, b)| a * b).sum();
                *d = (e + 2f32.powi(-24)).ln();
            }
        }
    }

impl NemotronDiarMlx {
    /// One offline window: `feats` holds `window` mel frames; returns the
    /// probabilities of its `central` frames and updates the FIFO / speaker cache.
    fn step(
        &self,
        feats: &[f32],
        window: usize,
        st: &mut State,
        central: usize,
        feature_length: usize,
    ) -> Result<Vec<[f32; N_SPK]>, Exception> {
        // Feature stacking: 8 mel frames -> one 1024-d row -> 512-d embedding.
        let w8 = window.div_ceil(FACTOR);
        let mut stacked = vec![0.0f32; w8 * FACTOR * N_MELS];
        stacked[..feats.len()].copy_from_slice(feats);
        let chunk = self.proj.forward(
            &Array::from_slice(&stacked, &[1, w8 as i32, (FACTOR * N_MELS) as i32]).as_dtype(self.dtype)?,
        )?;
        let chunk_len = feature_length.div_ceil(FACTOR);

        let cache_len = st.spkcache.shape()[1] as usize;
        let fifo_len = st.fifo.shape()[1] as usize;
        let combined = ops::concatenate(&[&st.spkcache, &st.fifo, &chunk], 1)?;
        let t = combined.shape()[1] as usize;
        let valid = chunk_len + cache_len + fifo_len;

        let keys: Vec<bool> = (0..t).map(|i| i < valid).collect();
        let mask = Array::from_slice(&keys, &[1, 1, 1, t as i32]);
        let mut x = self.embed_norm.forward(&combined)?;
        for b in &self.blocks {
            x = b.forward(&x, &mask)?;
        }
        let x = self.final_norm.forward(&x)?;

        // Speaker head at 10 ms: project, sub-pixel upsample x8, two ReLU layers.
        let h = self.encoder_proj.forward(&x)?;
        let h = ops::conv1d(&h, &self.upsample_w, 1, 1, 1, 1)?.add(&self.upsample_b)?;
        let h = relu(&h.reshape(&[1, (t * FACTOR) as i32, TF_D])?)?;
        let h = relu(&self.hidden.forward(&h)?)?;
        let high = ops::sigmoid(&self.to_spks.forward(&h)?)?.as_dtype(Dtype::Float32)?;
        high.eval()?;
        let flat = high.as_slice::<f32>();
        let row = |i: usize| -> [f32; N_SPK] {
            if i >= valid * FACTOR {
                return [0.0; N_SPK];
            }
            let mut r = [0.0; N_SPK];
            r.copy_from_slice(&flat[i * N_SPK..(i + 1) * N_SPK]);
            r
        };
        // Encoder-resolution predictions (mean over each frame's 8 sub-frames).
        let low = |f: usize| -> [f32; N_SPK] {
            let mut m = [0.0; N_SPK];
            for s in 0..FACTOR {
                let r = row(f * FACTOR + s);
                for k in 0..N_SPK {
                    m[k] += r[k] / FACTOR as f32;
                }
            }
            m
        };

        let start = cache_len + fifo_len;
        let result: Vec<[f32; N_SPK]> = (0..central).map(|i| row(start * FACTOR + i)).collect();
        let n80 = central.div_ceil(FACTOR);

        st.fifo = ops::concatenate(&[&st.fifo, &chunk.index((.., ..n80 as i32))], 1)?;
        st.fifo_preds = (cache_len..start).chain(start..start + n80).map(low).collect();
        let fifo_now = st.fifo.shape()[1] as usize;
        if fifo_now > FIFO_LEN {
            let pop = fifo_now.min(UPDATE_PERIOD.max(fifo_now - FIFO_LEN));
            st.spkcache = ops::concatenate(&[&st.spkcache, &st.fifo.index((.., ..pop as i32))], 1)?;
            let mut preds: Vec<[f32; N_SPK]> =
                if st.compressed { std::mem::take(&mut st.spkcache_preds) } else { (0..cache_len).map(low).collect() };
            preds.extend_from_slice(&st.fifo_preds[..pop]);
            st.spkcache_preds = preds;
            st.fifo = st.fifo.index((.., pop as i32..));
            st.fifo_preds.drain(..pop);
            if st.spkcache.shape()[1] as usize > SPKCACHE_LEN {
                let (idx, disabled) = aosc_select(&st.spkcache_preds);
                let n = st.spkcache.shape()[1];
                // Disabled slots read the learned silence embedding, appended as row n.
                let ext = ops::concatenate(&[&st.spkcache, &self.sil_emb], 1)?;
                let rows: Vec<i32> = idx.iter().zip(&disabled).map(|(&i, &d)| if d { n } else { i as i32 }).collect();
                st.spkcache = take_axis(&ext, &Array::from_slice(&rows, &[rows.len() as i32]), 1)?;
                st.spkcache_preds = idx
                    .iter()
                    .zip(&disabled)
                    .map(|(&i, &d)| if d { [0.0; N_SPK] } else { st.spkcache_preds[i] })
                    .collect();
                st.compressed = true;
            }
        }
        st.spkcache.eval()?;
        st.fifo.eval()?;
        st.frames_processed += central;
        Ok(result)
    }
}

/// AOSC compression (NeMo `_compress_spkcache_aosc`): picks SPKCACHE_LEN frames,
/// the most confidently single-speaker frames per speaker plus one silence slot
/// each. Returns frame indices (speaker-major order, as the reference) and which
/// slots are disabled (filled with the silence embedding).
fn aosc_select(preds: &[[f32; N_SPK]]) -> (Vec<usize>, Vec<bool>) {
    let n = preds.len();
    let per_spk = SPKCACHE_LEN / N_SPK - SIL_FRAMES_PER_SPK;
    let strong = (per_spk as f32 * STRONG_BOOST_RATE).floor() as usize;
    let weak = (per_spk as f32 * WEAK_BOOST_RATE).floor() as usize;
    let min_pos = (per_spk as f32 * MIN_POS_SCORES_RATE).floor() as usize;
    let ln_half = 0.5f32.ln();

    // 1. Log-likelihood-ratio scores.
    let mut scores: Vec<[f32; N_SPK]> = preds
        .iter()
        .map(|p| {
            let l1: [f32; N_SPK] = std::array::from_fn(|k| (1.0 - p[k]).max(PRED_SCORE_THRESHOLD).ln());
            let sum1: f32 = l1.iter().sum();
            std::array::from_fn(|k| p[k].max(PRED_SCORE_THRESHOLD).ln() - l1[k] + sum1 - ln_half)
        })
        .collect();
    // 2. Non-speech and (for well-represented speakers) overlapped speech -> -inf.
    for (s, p) in scores.iter_mut().zip(preds) {
        for k in 0..N_SPK {
            if p[k] <= 0.5 {
                s[k] = f32::NEG_INFINITY;
            }
        }
    }
    let pos_count: [usize; N_SPK] = std::array::from_fn(|k| scores.iter().filter(|s| s[k] > 0.0).count());
    for (s, p) in scores.iter_mut().zip(preds) {
        for k in 0..N_SPK {
            if pos_count[k] >= min_pos && !(s[k] > 0.0) && p[k] > 0.5 {
                s[k] = f32::NEG_INFINITY;
            }
        }
    }
    // 3. Favour the newest frames slightly.
    if n > SPKCACHE_LEN {
        for s in &mut scores[SPKCACHE_LEN..] {
            for v in s.iter_mut() {
                *v += SCORES_BOOST_LATEST;
            }
        }
    }
    // 4-5. Guarantee each speaker a minimum share (strong, then weak boost).
    for (k_boost, scale) in [(strong, 2.0f32), (weak, 1.0)] {
        let k_boost = k_boost.min(n);
        for spk in 0..N_SPK {
            for f in top_k(n, k_boost, |f| scores[f][spk]) {
                if scores[f][spk] > f32::NEG_INFINITY {
                    scores[f][spk] += -scale * ln_half;
                }
            }
        }
    }
    // 6-7. Flatten speaker-major with SIL_FRAMES_PER_SPK +inf pad rows; top-k.
    let rows = n + SIL_FRAMES_PER_SPK;
    let flat = |i: usize| -> f32 {
        let (spk, f) = (i / rows, i % rows);
        if f >= n { f32::INFINITY } else { scores[f][spk] }
    };
    let k = SPKCACHE_LEN.min(N_SPK * rows);
    const MAX_INDEX: usize = usize::MAX;
    let mut picked: Vec<usize> = top_k(N_SPK * rows, k, flat)
        .into_iter()
        .map(|i| if flat(i) > f32::NEG_INFINITY { i } else { MAX_INDEX })
        .collect();
    picked.sort_unstable();
    let mut idx = Vec::with_capacity(k);
    let mut disabled = Vec::with_capacity(k);
    for i in picked {
        let frame = if i == MAX_INDEX { 0 } else { i % rows };
        let off = i == MAX_INDEX || frame >= n;
        idx.push(if off { 0 } else { frame });
        disabled.push(off);
    }
    (idx, disabled)
}

/// Indices of the `k` largest values among `0..n`, ties to the lower index.
fn top_k(n: usize, k: usize, value: impl Fn(usize) -> f32) -> Vec<usize> {
    let mut ids: Vec<usize> = (0..n).collect();
    ids.sort_by(|&a, &b| value(b).partial_cmp(&value(a)).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
    ids.truncate(k);
    ids
}

/// Thresholds each speaker's 10 ms activity into turns.
fn probs_to_turns(probs: &[[f32; N_SPK]]) -> Vec<SpeakerTurn> {
    let mut turns = Vec::new();
    for spk in 0..N_SPK {
        let mut start: Option<usize> = None;
        for (i, p) in probs.iter().chain(std::iter::once(&[0.0; N_SPK])).enumerate() {
            match (p[spk] > ACTIVITY_THRESHOLD, start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    turns.push(SpeakerTurn { start_ms: s as u64 * 10, end_ms: i as u64 * 10, speaker: spk });
                    start = None;
                }
                _ => {}
            }
        }
    }
    turns.sort_by_key(|t| t.start_ms);
    turns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aosc_keeps_cache_length_and_silence_slots() {
        // 300 frames alternating between two clean speakers.
        let preds: Vec<[f32; N_SPK]> = (0..300)
            .map(|i| {
                let mut p = [0.01; N_SPK];
                p[(i / 20) % 2] = 0.95;
                p
            })
            .collect();
        let (idx, disabled) = aosc_select(&preds);
        assert_eq!(idx.len(), SPKCACHE_LEN);
        let used: usize = disabled.iter().filter(|d| !**d).count();
        assert!(used >= 2 * 24, "both speakers keep their strong-boost share");
        assert!(idx.iter().all(|&i| i < preds.len()));
    }

    #[test]
    fn probs_become_turns() {
        let mut probs = vec![[0.0; N_SPK]; 100];
        for p in &mut probs[10..40] {
            p[0] = 0.9;
        }
        for p in &mut probs[50..100] {
            p[1] = 0.8;
        }
        let t = probs_to_turns(&probs);
        assert_eq!(t, vec![
            SpeakerTurn { start_ms: 100, end_ms: 400, speaker: 0 },
            SpeakerTurn { start_ms: 500, end_ms: 1000, speaker: 1 },
        ]);
    }
}
