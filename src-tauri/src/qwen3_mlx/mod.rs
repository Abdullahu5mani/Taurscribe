//! Native MLX backend for Qwen3-ASR 1.7B on Apple Silicon.
//!
//! Loads `model.safetensors` from the official `Qwen/Qwen3-ASR-1.7B` HuggingFace
//! checkpoint and runs inference natively on Metal via `mlx-rs`.
//!
//! Zero Python, Zero Quantization (100% full precision floating-point weights).
//! Features:
//!   - 128-channel Slaney log-mel frontend with SIMD FFT.
//!   - 24-layer Audio Transformer (AuT) encoder + multimodal projector.
//!   - Grouped Query Attention (GQA: 16 query heads, 8 key-value heads) with RoPE (theta = 1,000,000).
//!   - Stateful KV-Caching turning autoregressive generation from O(N^2) to O(N).

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use mlx_rs::{
    error::Exception,
    fast,
    ops::{
        self,
        indexing::{argmax_axis, IndexOp},
    },
    Array,
};
use std::collections::HashMap;
use std::path::Path;
use tokenizers::Tokenizer;

// ── Special Token IDs ─────────────────────────────────────────────────────────

const TOKEN_IM_START: i32 = 151_644;
const TOKEN_IM_END: i32 = 151_645;
const TOKEN_END_OF_TEXT: i32 = 151_643;
const TOKEN_SYSTEM: i32 = 8_948;
const TOKEN_USER: i32 = 872;
const TOKEN_ASSISTANT: i32 = 77_091;
const TOKEN_AUDIO_START: i32 = 151_669;
const TOKEN_AUDIO_END: i32 = 151_670;
const TOKEN_AUDIO_PAD: i32 = 151_676;
const TOKEN_ASR_TEXT: i32 = 151_704;
const TOKEN_NEWLINE: i32 = 198;

/// Maximum tokens the decoder may generate per chunk.
const MAX_TOKENS: usize = 448;

// ── Weight map type alias ─────────────────────────────────────────────────────

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
type Weights = HashMap<String, Array>;

// ── Public API ────────────────────────────────────────────────────────────────

pub struct Qwen3Mlx {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    weights: Weights,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    c1_w: Array,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    c2_w: Array,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    c3_w: Array,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    pos_emb: Array,
}

impl Qwen3Mlx {
    /// Load a Qwen3-ASR MLX bundle from `dir` (must contain `model.safetensors`).
    pub fn load(dir: &Path) -> Result<Self, String> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let path = dir.join("model.safetensors");
            if !path.is_file() {
                return Err(format!("model.safetensors not found in {}", dir.display()));
            }

            let weights = Array::load_safetensors(path.to_string_lossy().as_ref())
                .map_err(|e| format!("Qwen3 MLX load safetensors: {e}"))?;

            // Pre-transpose conv2d weights from PyTorch [C_out, C_in, kH, kW]
            // to MLX [C_out, kH, kW, C_in] (perm: [0, 2, 3, 1]).
            let get = |k: &str| -> Result<&Array, String> {
                weights.get(k).ok_or_else(|| format!("missing weight {k}"))
            };

            let c1_w = get("model.audio_tower.conv2d1.weight")?
                .transpose_axes(&[0, 2, 3, 1])
                .map_err(|e| e.to_string())?;
            let c2_w = get("model.audio_tower.conv2d2.weight")?
                .transpose_axes(&[0, 2, 3, 1])
                .map_err(|e| e.to_string())?;
            let c3_w = get("model.audio_tower.conv2d3.weight")?
                .transpose_axes(&[0, 2, 3, 1])
                .map_err(|e| e.to_string())?;

            // Precompute sinusoids positional embedding [13, 1024]
            let channels = 1024;
            let length = 13;
            let half = channels / 2;
            let log_incr = (10000.0_f64).ln() / (half as f64 - 1.0);
            let mut pe_vec = vec![0.0f32; length * channels];
            for t in 0..length {
                for i in 0..half {
                    let inv_ts = (-log_incr * i as f64).exp();
                    let angle = (t as f64) * inv_ts;
                    pe_vec[t * channels + i] = angle.sin() as f32;
                    pe_vec[t * channels + half + i] = angle.cos() as f32;
                }
            }
            let pos_emb = Array::from_slice(&pe_vec, &[13, 1024]);

            Ok(Self {
                weights,
                c1_w,
                c2_w,
                c3_w,
                pos_emb,
            })
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = dir;
            Err("Qwen3 MLX is only available on Apple Silicon (aarch64-apple-darwin)".to_string())
        }
    }

    /// Transcribe `audio` (16 kHz f32 PCM) and return decoded text.
    pub fn transcribe_to_string(
        &mut self,
        audio: &[f32],
        prompt: Option<&str>,
        tokenizer: &Tokenizer,
    ) -> Result<String, String> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            self.transcribe_mlx(audio, prompt, tokenizer)
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (audio, prompt, tokenizer);
            Err("Qwen3 MLX not available on this platform".to_string())
        }
    }

    /// Backwards-compatible token ID transcription.
    pub fn transcribe(
        &mut self,
        audio: &[f32],
        prompt: Option<&str>,
        tokenizer: &Tokenizer,
    ) -> Result<Vec<u32>, String> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let ids = self.transcribe_mlx_ids(audio, prompt, tokenizer)?;
            Ok(ids)
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (audio, prompt, tokenizer);
            Err("Qwen3 MLX not available on this platform".to_string())
        }
    }

    // ── MLX inference (Apple Silicon only) ───────────────────────────────

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn transcribe_mlx(
        &mut self,
        audio: &[f32],
        prompt: Option<&str>,
        tokenizer: &Tokenizer,
    ) -> Result<String, String> {
        let generated_ids = self.transcribe_mlx_ids(audio, prompt, tokenizer)?;
        if generated_ids.is_empty() {
            return Ok(String::new());
        }

        // Post-process generated tokens: strip up to <asr_text> if present.
        let final_ids: &[u32] = if let Some(pos) = generated_ids.iter().position(|&id| id == TOKEN_ASR_TEXT as u32) {
            &generated_ids[pos + 1..]
        } else {
            &generated_ids
        };

        tokenizer
            .decode(final_ids, true)
            .map_err(|e| format!("Qwen3 MLX tokenizer decode: {e}"))
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn transcribe_mlx_ids(
        &mut self,
        audio: &[f32],
        prompt: Option<&str>,
        tokenizer: &Tokenizer,
    ) -> Result<Vec<u32>, String> {
        use crate::qwen3_mel::extract_qwen3_log_mel;

        // 1. Slaney log-mel spectrogram: [n_frames, 128]
        let mel = extract_qwen3_log_mel(audio);
        let n_frames = mel.nrows();
        let n_chunks = n_frames / 100;
        if n_chunks == 0 {
            return Ok(Vec::new());
        }

        // Truncate to multiple of 100 frames for 3-stage stride-2 convolutions
        let mel_slice: Vec<f32> = mel.iter().copied().take(n_chunks * 100 * 128).collect();
        let mel_arr = Array::from_slice(&mel_slice, &[n_chunks as i32, 100, 128]);
        let mel_nhwc = mel_arr
            .transpose_axes(&[0, 2, 1])
            .map_err(|e| e.to_string())?
            .reshape(&[n_chunks as i32, 128, 100, 1])
            .map_err(|e| e.to_string())?;

        // 2. Audio Tower + Projector
        let audio_feats = self.run_audio_tower(&mel_nhwc, n_chunks)?;
        let n_audio_tokens = (n_chunks * 13) as i32;

        // 3. Build multimodal prompt tokens
        let mut prompt_ids: Vec<i32> = Vec::new();
        if let Some(p) = prompt.filter(|s| !s.trim().is_empty()) {
            prompt_ids.extend_from_slice(&[TOKEN_IM_START, TOKEN_SYSTEM, TOKEN_NEWLINE]);
            let enc_p = tokenizer
                .encode(p, false)
                .map_err(|e| format!("prompt encode: {e}"))?;
            prompt_ids.extend(enc_p.get_ids().iter().map(|&x| x as i32));
            prompt_ids.extend_from_slice(&[TOKEN_IM_END, TOKEN_NEWLINE]);
        }

        prompt_ids.extend_from_slice(&[TOKEN_IM_START, TOKEN_USER, TOKEN_NEWLINE, TOKEN_AUDIO_START]);
        let audio_start_idx = prompt_ids.len();
        for _ in 0..n_audio_tokens {
            prompt_ids.push(TOKEN_AUDIO_PAD);
        }
        let audio_end_idx = prompt_ids.len();
        prompt_ids.extend_from_slice(&[
            TOKEN_AUDIO_END,
            TOKEN_IM_END,
            TOKEN_NEWLINE,
            TOKEN_IM_START,
            TOKEN_ASSISTANT,
            TOKEN_NEWLINE,
        ]);

        // 4. Token embeddings with audio feature injection
        let embed_tokens_w = self.get("model.language_model.embed_tokens.weight")?;
        let before_ids = Array::from_slice(&prompt_ids[..audio_start_idx], &[audio_start_idx as i32]);
        let after_ids = Array::from_slice(
            &prompt_ids[audio_end_idx..],
            &[(prompt_ids.len() - audio_end_idx) as i32],
        );

        let emb_before = embed_tokens_w.index(&before_ids);
        let emb_after = embed_tokens_w.index(&after_ids);

        let total_embeds = ops::concatenate_axis(&[&emb_before, &audio_feats, &emb_after], 0)
            .map_err(|e| format!("concatenate audio+text embeddings: {e}"))?;
        let seq_len = prompt_ids.len();
        let x = ops::expand_dims(&total_embeds, 0)
            .map_err(|e| format!("expand embeds: {e}"))?;

        // 5. Stateful LLM Prefill & Generation
        self.run_llm_stateful(&x, seq_len, embed_tokens_w)
    }

    // ── Audio Tower & Projector ───────────────────────────────────────────

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn run_audio_tower(&self, mel_nhwc: &Array, n_chunks: usize) -> Result<Array, String> {
        let gelu = |x: &Array| -> Result<Array, Exception> {
            let s = Array::from_f32(1.702);
            let sig = ops::sigmoid(&x.multiply(&s)?)?;
            x.multiply(&sig)
        };

        let c1_b = self.get("model.audio_tower.conv2d1.bias")?;
        let c2_b = self.get("model.audio_tower.conv2d2.bias")?;
        let c3_b = self.get("model.audio_tower.conv2d3.bias")?;

        let y1 = gelu(&ops::conv2d(mel_nhwc, &self.c1_w, (2, 2), (1, 1), (1, 1), 1)
            .map_err(|e| e.to_string())?
            .add(c1_b)
            .map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

        let y2 = gelu(&ops::conv2d(&y1, &self.c2_w, (2, 2), (1, 1), (1, 1), 1)
            .map_err(|e| e.to_string())?
            .add(c2_b)
            .map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

        let y3 = gelu(&ops::conv2d(&y2, &self.c3_w, (2, 2), (1, 1), (1, 1), 1)
            .map_err(|e| e.to_string())?
            .add(c3_b)
            .map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

        // [n_chunks, 16, 13, 480] -> [n_chunks, 13, 7680]
        let y3_trans = y3
            .transpose_axes(&[0, 2, 3, 1])
            .map_err(|e| e.to_string())?
            .reshape(&[n_chunks as i32, 13, 7680])
            .map_err(|e| e.to_string())?;

        let conv_out_w = self.get("model.audio_tower.conv_out.weight")?;
        let mut h = y3_trans
            .matmul(&conv_out_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
            .add(&self.pos_emb)
            .map_err(|e| e.to_string())?;

        let n_audio_tokens = (n_chunks * 13) as i32;
        h = h.reshape(&[n_audio_tokens, 1024]).map_err(|e| e.to_string())?;

        // 24 Audio Transformer Layers
        let scale_audio = 1.0 / (64.0_f32).sqrt();
        for i in 0..24 {
            let p = format!("model.audio_tower.layers.{i}");
            let sa_norm_w = self.get(&format!("{p}.self_attn_layer_norm.weight"))?;
            let sa_norm_b = self.get(&format!("{p}.self_attn_layer_norm.bias"))?;
            let normed = fast::layer_norm(&h, Some(sa_norm_w), Some(sa_norm_b), 1e-5)
                .map_err(|e| format!("AuT sa norm {i}: {e}"))?;

            let q_w = self.get(&format!("{p}.self_attn.q_proj.weight"))?;
            let q_b = self.get(&format!("{p}.self_attn.q_proj.bias"))?;
            let k_w = self.get(&format!("{p}.self_attn.k_proj.weight"))?;
            let k_b = self.get(&format!("{p}.self_attn.k_proj.bias"))?;
            let v_w = self.get(&format!("{p}.self_attn.v_proj.weight"))?;
            let v_b = self.get(&format!("{p}.self_attn.v_proj.bias"))?;
            let out_w = self.get(&format!("{p}.self_attn.out_proj.weight"))?;
            let out_b = self.get(&format!("{p}.self_attn.out_proj.bias"))?;

            let q = normed
                .matmul(&q_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .add(q_b)
                .map_err(|e| e.to_string())?
                .reshape(&[1, n_audio_tokens, 16, 64])
                .map_err(|e| e.to_string())?
                .transpose_axes(&[0, 2, 1, 3])
                .map_err(|e| e.to_string())?;

            let k = normed
                .matmul(&k_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .add(k_b)
                .map_err(|e| e.to_string())?
                .reshape(&[1, n_audio_tokens, 16, 64])
                .map_err(|e| e.to_string())?
                .transpose_axes(&[0, 2, 1, 3])
                .map_err(|e| e.to_string())?;

            let v = normed
                .matmul(&v_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .add(v_b)
                .map_err(|e| e.to_string())?
                .reshape(&[1, n_audio_tokens, 16, 64])
                .map_err(|e| e.to_string())?
                .transpose_axes(&[0, 2, 1, 3])
                .map_err(|e| e.to_string())?;

            let attn = fast::scaled_dot_product_attention(&q, &k, &v, scale_audio, None)
                .map_err(|e| format!("AuT sdpa {i}: {e}"))?
                .transpose_axes(&[0, 2, 1, 3])
                .map_err(|e| e.to_string())?
                .reshape(&[n_audio_tokens, 1024])
                .map_err(|e| e.to_string())?;

            let attn_out = attn
                .matmul(&out_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .add(out_b)
                .map_err(|e| e.to_string())?;
            h = h.add(&attn_out).map_err(|e| e.to_string())?;

            let fn_norm_w = self.get(&format!("{p}.final_layer_norm.weight"))?;
            let fn_norm_b = self.get(&format!("{p}.final_layer_norm.bias"))?;
            let normed2 = fast::layer_norm(&h, Some(fn_norm_w), Some(fn_norm_b), 1e-5)
                .map_err(|e| format!("AuT fn norm {i}: {e}"))?;

            let fc1_w = self.get(&format!("{p}.fc1.weight"))?;
            let fc1_b = self.get(&format!("{p}.fc1.bias"))?;
            let fc2_w = self.get(&format!("{p}.fc2.weight"))?;
            let fc2_b = self.get(&format!("{p}.fc2.bias"))?;

            let mid = gelu(&normed2
                .matmul(&fc1_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .add(fc1_b)
                .map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;

            let ffn_out = mid
                .matmul(&fc2_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .add(fc2_b)
                .map_err(|e| e.to_string())?;
            h = h.add(&ffn_out).map_err(|e| e.to_string())?;
        }

        let post_w = self.get("model.audio_tower.ln_post.weight")?;
        let post_b = self.get("model.audio_tower.ln_post.bias")?;
        h = fast::layer_norm(&h, Some(post_w), Some(post_b), 1e-5)
            .map_err(|e| format!("AuT post norm: {e}"))?;

        // Multimodal Projector
        let p1_w = self.get("model.multi_modal_projector.linear_1.weight")?;
        let p1_b = self.get("model.multi_modal_projector.linear_1.bias")?;
        let p2_w = self.get("model.multi_modal_projector.linear_2.weight")?;
        let p2_b = self.get("model.multi_modal_projector.linear_2.bias")?;

        let p1_out = gelu(&h
            .matmul(&p1_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
            .add(p1_b)
            .map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

        let audio_feats = p1_out
            .matmul(&p2_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
            .add(p2_b)
            .map_err(|e| e.to_string())?;

        Ok(audio_feats)
    }

    // ── Language Model with Stateful KV Cache ─────────────────────────────

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn run_llm_stateful(
        &self,
        embeds: &Array,
        seq_len: usize,
        embed_tokens_w: &Array,
    ) -> Result<Vec<u32>, String> {
        let mut x = embeds.clone();

        // 1. Causal attention mask for prefill
        let mut mask_vec = vec![0.0f32; seq_len * seq_len];
        for r in 0..seq_len {
            for c in 0..seq_len {
                if c > r {
                    mask_vec[r * seq_len + c] = -1e9;
                }
            }
        }
        let causal_mask = Array::from_slice(&mask_vec, &[seq_len as i32, seq_len as i32]);
        let scale_lm = 1.0 / (128.0_f32).sqrt();

        // 2. Prefill: populate KV cache for all 28 layers
        let mut kv_cache: Vec<(Array, Array)> = Vec::with_capacity(28);

        for l in 0..28 {
            let p = format!("model.language_model.layers.{l}");
            let in_norm_w = self.get(&format!("{p}.input_layernorm.weight"))?;
            let normed = fast::rms_norm(&x, in_norm_w, 1e-6)
                .map_err(|e| format!("LLM in norm {l}: {e}"))?;

            let q_w = self.get(&format!("{p}.self_attn.q_proj.weight"))?;
            let k_w = self.get(&format!("{p}.self_attn.k_proj.weight"))?;
            let v_w = self.get(&format!("{p}.self_attn.v_proj.weight"))?;
            let o_w = self.get(&format!("{p}.self_attn.o_proj.weight"))?;
            let q_norm_w = self.get(&format!("{p}.self_attn.q_norm.weight"))?;
            let k_norm_w = self.get(&format!("{p}.self_attn.k_norm.weight"))?;

            let q = normed
                .matmul(&q_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .reshape(&[1, seq_len as i32, 16, 128])
                .map_err(|e| e.to_string())?;
            let q = fast::rms_norm(&q, q_norm_w, 1e-6)
                .map_err(|e| e.to_string())?
                .transpose_axes(&[0, 2, 1, 3])
                .map_err(|e| e.to_string())?;
            let q = fast::rope(&q, 128, false, 1_000_000.0, 1.0, 0, None)
                .map_err(|e| format!("LLM q rope {l}: {e}"))?;

            let k = normed
                .matmul(&k_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .reshape(&[1, seq_len as i32, 8, 128])
                .map_err(|e| e.to_string())?;
            let k = fast::rms_norm(&k, k_norm_w, 1e-6)
                .map_err(|e| e.to_string())?
                .transpose_axes(&[0, 2, 1, 3])
                .map_err(|e| e.to_string())?;
            let k = fast::rope(&k, 128, false, 1_000_000.0, 1.0, 0, None)
                .map_err(|e| format!("LLM k rope {l}: {e}"))?;

            let v = normed
                .matmul(&v_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .reshape(&[1, seq_len as i32, 8, 128])
                .map_err(|e| e.to_string())?
                .transpose_axes(&[0, 2, 1, 3])
                .map_err(|e| e.to_string())?;

            let attn = fast::scaled_dot_product_attention(
                &q,
                &k,
                &v,
                scale_lm,
                fast::ScaledDotProductAttentionMask::Array(&causal_mask),
            )
            .map_err(|e| format!("LLM sdpa {l}: {e}"))?
            .transpose_axes(&[0, 2, 1, 3])
            .map_err(|e| e.to_string())?
            .reshape(&[1, seq_len as i32, 2048])
            .map_err(|e| e.to_string())?;

            let attn_out = attn
                .matmul(&o_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            x = x.add(&attn_out).map_err(|e| e.to_string())?;

            let post_norm_w = self.get(&format!("{p}.post_attention_layernorm.weight"))?;
            let normed2 = fast::rms_norm(&x, post_norm_w, 1e-6)
                .map_err(|e| format!("LLM post norm {l}: {e}"))?;

            let gate_w = self.get(&format!("{p}.mlp.gate_proj.weight"))?;
            let up_w = self.get(&format!("{p}.mlp.up_proj.weight"))?;
            let down_w = self.get(&format!("{p}.mlp.down_proj.weight"))?;

            let gate = normed2
                .matmul(&gate_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let up = normed2
                .matmul(&up_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let silu_gate = gate
                .multiply(&ops::sigmoid(&gate).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let mlp_mid = silu_gate.multiply(&up).map_err(|e| e.to_string())?;
            let mlp_out = mlp_mid
                .matmul(&down_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            x = x.add(&mlp_out).map_err(|e| e.to_string())?;

            kv_cache.push((k, v));
        }

        // 3. Prefill final prediction
        let final_norm_w = self.get("model.language_model.norm.weight")?;
        let normed_final = fast::rms_norm(&x, final_norm_w, 1e-6)
            .map_err(|e| format!("LLM final norm: {e}"))?;
        let s_idx = seq_len as i32;
        let last = normed_final.index((.., (s_idx - 1)..s_idx));
        let logits = last
            .matmul(&embed_tokens_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let next_arr = argmax_axis(&logits, -1, false)
            .map_err(|e| format!("LLM prefill argmax: {e}"))?;
        let mut next_id = next_arr.item::<u32>() as i32;

        let mut generated_ids: Vec<u32> = Vec::with_capacity(MAX_TOKENS);
        generated_ids.push(next_id as u32);

        // 4. Autoregressive Loop using Stateful KV-Cache
        for _step in 0..MAX_TOKENS {
            if next_id == TOKEN_IM_END || next_id == TOKEN_END_OF_TEXT {
                break;
            }

            let past_len = kv_cache[0].0.shape()[2];
            let tok_arr = Array::from_slice(&[next_id], &[1]);
            let tok_emb = embed_tokens_w.index(&tok_arr);
            let mut x_step = ops::expand_dims(&tok_emb, 0)
                .map_err(|e| format!("expand tok emb: {e}"))?;

            for l in 0..28 {
                let p = format!("model.language_model.layers.{l}");
                let in_norm_w = self.get(&format!("{p}.input_layernorm.weight"))?;
                let normed = fast::rms_norm(&x_step, in_norm_w, 1e-6)
                    .map_err(|e| format!("step in norm {l}: {e}"))?;

                let q_w = self.get(&format!("{p}.self_attn.q_proj.weight"))?;
                let k_w = self.get(&format!("{p}.self_attn.k_proj.weight"))?;
                let v_w = self.get(&format!("{p}.self_attn.v_proj.weight"))?;
                let o_w = self.get(&format!("{p}.self_attn.o_proj.weight"))?;
                let q_norm_w = self.get(&format!("{p}.self_attn.q_norm.weight"))?;
                let k_norm_w = self.get(&format!("{p}.self_attn.k_norm.weight"))?;

                let q = normed
                    .matmul(&q_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?
                    .reshape(&[1, 1, 16, 128])
                    .map_err(|e| e.to_string())?;
                let q = fast::rms_norm(&q, q_norm_w, 1e-6)
                    .map_err(|e| e.to_string())?
                    .transpose_axes(&[0, 2, 1, 3])
                    .map_err(|e| e.to_string())?;
                let q = fast::rope(&q, 128, false, 1_000_000.0, 1.0, past_len as i32, None)
                    .map_err(|e| format!("step q rope {l}: {e}"))?;

                let k_new = normed
                    .matmul(&k_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?
                    .reshape(&[1, 1, 8, 128])
                    .map_err(|e| e.to_string())?;
                let k_new = fast::rms_norm(&k_new, k_norm_w, 1e-6)
                    .map_err(|e| e.to_string())?
                    .transpose_axes(&[0, 2, 1, 3])
                    .map_err(|e| e.to_string())?;
                let k_new = fast::rope(&k_new, 128, false, 1_000_000.0, 1.0, past_len as i32, None)
                    .map_err(|e| format!("step k rope {l}: {e}"))?;

                let v_new = normed
                    .matmul(&v_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?
                    .reshape(&[1, 1, 8, 128])
                    .map_err(|e| e.to_string())?
                    .transpose_axes(&[0, 2, 1, 3])
                    .map_err(|e| e.to_string())?;

                let k_cached = ops::concatenate_axis(&[&kv_cache[l].0, &k_new], 2)
                    .map_err(|e| format!("step concat k {l}: {e}"))?;
                let v_cached = ops::concatenate_axis(&[&kv_cache[l].1, &v_new], 2)
                    .map_err(|e| format!("step concat v {l}: {e}"))?;

                let attn = fast::scaled_dot_product_attention(
                    &q,
                    &k_cached,
                    &v_cached,
                    scale_lm,
                    None,
                )
                .map_err(|e| format!("step sdpa {l}: {e}"))?
                .transpose_axes(&[0, 2, 1, 3])
                .map_err(|e| e.to_string())?
                .reshape(&[1, 1, 2048])
                .map_err(|e| e.to_string())?;

                let attn_out = attn
                    .matmul(&o_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                x_step = x_step.add(&attn_out).map_err(|e| e.to_string())?;

                let post_norm_w = self.get(&format!("{p}.post_attention_layernorm.weight"))?;
                let normed2 = fast::rms_norm(&x_step, post_norm_w, 1e-6)
                    .map_err(|e| format!("step post norm {l}: {e}"))?;

                let gate_w = self.get(&format!("{p}.mlp.gate_proj.weight"))?;
                let up_w = self.get(&format!("{p}.mlp.up_proj.weight"))?;
                let down_w = self.get(&format!("{p}.mlp.down_proj.weight"))?;

                let gate = normed2
                    .matmul(&gate_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                let up = normed2
                    .matmul(&up_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                let silu_gate = gate
                    .multiply(&ops::sigmoid(&gate).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                let mlp_mid = silu_gate.multiply(&up).map_err(|e| e.to_string())?;
                let mlp_out = mlp_mid
                    .matmul(&down_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
                x_step = x_step.add(&mlp_out).map_err(|e| e.to_string())?;

                kv_cache[l] = (k_cached, v_cached);
            }

            let normed_final = fast::rms_norm(&x_step, final_norm_w, 1e-6)
                .map_err(|e| format!("step final norm: {e}"))?;
            let logits = normed_final
                .matmul(&embed_tokens_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let next_arr = argmax_axis(&logits, -1, false)
                .map_err(|e| format!("step argmax: {e}"))?;
            next_id = next_arr.item::<u32>() as i32;
            generated_ids.push(next_id as u32);
        }

        Ok(generated_ids)
    }

    // ── Weight helper ─────────────────────────────────────────────────────

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn get(&self, key: &str) -> Result<&Array, String> {
        self.weights
            .get(key)
            .ok_or_else(|| format!("Qwen3 MLX: missing weight '{key}'"))
    }
}
