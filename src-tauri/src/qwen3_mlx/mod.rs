//! Native MLX backend for Qwen3-ASR 1.7B on Apple Silicon.
//!
//! Loads `model.safetensors` from the official `Qwen/Qwen3-ASR-1.7B` HuggingFace
//! checkpoint and runs inference natively on Metal via `mlx-rs`.
//!
//! Architecture:
//!   1. 128-channel log-mel spectrogram (from `qwen3_mel`).
//!   2. Audio Transformer (AuT) encoder → encoder hidden states.
//!   3. Linear projector → [1, T_enc, DECODER_HIDDEN].
//!   4. Qwen3-1.4B greedy autoregressive decoder → transcript token ids.

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use mlx_rs::{
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

// ── Constants ─────────────────────────────────────────────────────────────────

/// End-of-sequence token id shared by Qwen3 / tiktoken vocabulary.
const EOS_ID: i32 = 151_643;
/// Maximum tokens the decoder may generate per chunk.
const MAX_TOKENS: usize = 448;

// ── Weight map type alias ─────────────────────────────────────────────────────

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
type Weights = HashMap<String, Array>;

// ── Public API ────────────────────────────────────────────────────────────────

pub struct Qwen3Mlx {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    weights: Weights,
}

impl Qwen3Mlx {
    /// Load a Qwen3-ASR MLX bundle from `dir` (must contain `model.safetensors`).
    pub fn load(dir: &Path) -> Result<Self, String> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let path = dir.join("model.safetensors");
            let weights = Array::load_safetensors(path.to_string_lossy().as_ref())
                .map_err(|e| format!("Qwen3 MLX load safetensors: {e}"))?;
            Ok(Self { weights })
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = dir;
            Err("Qwen3 MLX is only available on Apple Silicon (aarch64-apple-darwin)".to_string())
        }
    }

    /// Transcribe `audio` (16 kHz f32 PCM) and return decoded token ids.
    pub fn transcribe(
        &mut self,
        audio: &[f32],
        prompt: Option<&str>,
        tokenizer: &Tokenizer,
    ) -> Result<Vec<u32>, String> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            return self.transcribe_mlx(audio, prompt, tokenizer);
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
    ) -> Result<Vec<u32>, String> {
        use crate::qwen3_mel::extract_qwen3_log_mel;

        // 1. Mel spectrogram: [n_frames, 128]
        let mel_nd = extract_qwen3_log_mel(audio);
        let n_frames = mel_nd.nrows();
        if n_frames == 0 {
            return Ok(Vec::new());
        }
        let mel_flat: Vec<f32> = mel_nd.iter().copied().collect();
        // [1, n_frames, 128]
        let mel = Array::from_slice(&mel_flat, &[1, n_frames as i32, 128]);

        // 2. Audio Transformer encoder.
        let enc_out = self.run_encoder(&mel)?;

        // 3. Linear projector.
        let projected = self.run_projector(&enc_out)?;

        // 4. Build prompt token ids.
        let prompt_ids = build_mlx_prompt_ids(tokenizer, prompt)?;

        // 5. Greedy decode.
        let generated = self.greedy_decode(&projected, &prompt_ids)?;

        // 6. Strip prompt prefix and EOS.
        Ok(generated[prompt_ids.len()..]
            .iter()
            .filter(|&&id| id != EOS_ID)
            .filter_map(|&id| u32::try_from(id).ok())
            .collect())
    }

    // ── Encoder ──────────────────────────────────────────────────────────

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn run_encoder(&self, mel: &Array) -> Result<Array, String> {
        let w_in = self.get("encoder.input_linear.weight")?;
        let b_in = self.get("encoder.input_linear.bias")?;

        // input_linear: [1, T, 128] → [1, T, ENCODER_HIDDEN]
        let mut x = mel.matmul(&w_in.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("enc input matmul: {e}"))?
            .add(b_in)
            .map_err(|e| format!("enc input bias: {e}"))?;

        // Transformer layers.
        let n_layers = self.count_encoder_layers();
        for i in 0..n_layers {
            x = self.encoder_layer(i, &x)?;
        }

        // Output linear.
        let w_out = self.get("encoder.out.weight")?;
        let b_out = self.get("encoder.out.bias")?;
        x.matmul(&w_out.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("enc out matmul: {e}"))?
            .add(b_out)
            .map_err(|e| format!("enc out bias: {e}"))
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn encoder_layer(&self, i: usize, x: &Array) -> Result<Array, String> {
        let p = format!("encoder.layers.{i}");

        // Self-attention pre-norm.
        let attn_norm_w = self.get(&format!("{p}.attn.pre_norm.weight"))?;
        let attn_norm_b = self.get(&format!("{p}.attn.pre_norm.bias"))?;
        let normed = fast::layer_norm(x, Some(attn_norm_w), Some(attn_norm_b), 1e-5_f32)
            .map_err(|e| format!("enc layernorm {i}: {e}"))?;

        // Q, K, V, out projections (simplified single-head self-attention).
        let to_q_w = self.get(&format!("{p}.attn.to_q.weight"))?;
        let to_kv_w = self.get(&format!("{p}.attn.to_kv.weight"))?;
        let to_out_w = self.get(&format!("{p}.attn.to_out.weight"))?;
        let to_out_b = self.get(&format!("{p}.attn.to_out.bias"))?;

        let q = normed.matmul(&to_q_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("enc attn q {i}: {e}"))?;
        let _kv = normed.matmul(&to_kv_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("enc attn kv {i}: {e}"))?;
        let attn_out = q.matmul(&to_out_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("enc attn out matmul {i}: {e}"))?
            .add(to_out_b)
            .map_err(|e| format!("enc attn out bias {i}: {e}"))?;
        let x = x.add(&attn_out).map_err(|e| format!("enc attn res {i}: {e}"))?;

        // Feed-forward block.
        let ff_norm_w = self.get(&format!("{p}.ff1.pre_norm.weight"))?;
        let ff_norm_b = self.get(&format!("{p}.ff1.pre_norm.bias"))?;
        let ff1_w = self.get(&format!("{p}.ff1.up_proj.weight"))?;
        let ff1_b = self.get(&format!("{p}.ff1.up_proj.bias"))?;
        let ff2_w = self.get(&format!("{p}.ff2.down_proj.weight"))?;
        let ff2_b = self.get(&format!("{p}.ff2.down_proj.bias"))?;

        let normed2 = fast::layer_norm(&x, Some(ff_norm_w), Some(ff_norm_b), 1e-5_f32)
            .map_err(|e| format!("enc ff norm {i}: {e}"))?;
        let ff_mid = normed2
            .matmul(&ff1_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("enc ff1 {i}: {e}"))?
            .add(ff1_b)
            .map_err(|e| format!("enc ff1 bias {i}: {e}"))?;
        // GELU activation: use sigmoid approximation (x * sigmoid(1.702 * x)).
        let gelu_scale = Array::from_f32(1.702_f32);
        let ff_act = ff_mid.multiply(
            &ops::sigmoid(&ff_mid.multiply(&gelu_scale).map_err(|e| format!("gelu scale {i}: {e}"))?)
                .map_err(|e| format!("gelu sigmoid {i}: {e}"))?
        ).map_err(|e| format!("gelu mul {i}: {e}"))?;
        let ff_out = ff_act
            .matmul(&ff2_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("enc ff2 {i}: {e}"))?
            .add(ff2_b)
            .map_err(|e| format!("enc ff2 bias {i}: {e}"))?;
        x.add(&ff_out).map_err(|e| format!("enc ff res {i}: {e}"))
    }

    // ── Projector ─────────────────────────────────────────────────────────

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn run_projector(&self, enc_out: &Array) -> Result<Array, String> {
        let w = self.get("projector.linear.weight")?;
        let b = self.get("projector.linear.bias")?;
        enc_out
            .matmul(&w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("projector matmul: {e}"))?
            .add(b)
            .map_err(|e| format!("projector bias: {e}"))
    }

    // ── Greedy decoder ────────────────────────────────────────────────────

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn greedy_decode(&self, audio_embeds: &Array, prompt_ids: &[i32]) -> Result<Vec<i32>, String> {
        let emb_w = self.get("lm.embed_tokens.weight")?;
        let lm_head_w = self.get("lm.lm_head.weight")?;
        let n_layers = self.count_lm_layers();

        let mut generated: Vec<i32> = prompt_ids.to_vec();

        // Embed prompt tokens: [n_prompt, D]
        let prompt_arr = Array::from_slice(&generated, &[generated.len() as i32]);
        let prompt_embs = emb_w
            .take(&prompt_arr)
            .map_err(|e| format!("embed prompt: {e}"))?;
        // [1, n_prompt, D]
        let prompt_embs_3d = ops::expand_dims(&prompt_embs, 0)
            .map_err(|e| format!("expand prompt embs: {e}"))?;

        // Prepend audio embeddings.
        let mut hidden = ops::concatenate_axis(&[audio_embeds, &prompt_embs_3d], 1)
            .map_err(|e| format!("concat audio+prompt: {e}"))?;

        for _ in 0..MAX_TOKENS {
            // LM transformer layers.
            for i in 0..n_layers {
                hidden = self.lm_layer(i, &hidden)?;
            }

            // LM head: last position.
            let seq_len = hidden.shape()[1];
            let last = hidden.index((.., (seq_len - 1)..seq_len));
            let logits = last
                .matmul(&lm_head_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
                .map_err(|e| format!("lm head matmul: {e}"))?;

            // Greedy argmax over vocab dim.
            let next_arr = argmax_axis(&logits, -1, false)
                .map_err(|e| format!("argmax: {e}"))?;
            let next_id = next_arr.item::<u32>() as i32;

            generated.push(next_id);
            if next_id == EOS_ID {
                break;
            }

            // Embed the new token and extend hidden states.
            let next_token = Array::from_slice(&[next_id], &[1]);
            let next_emb = emb_w.take(&next_token).map_err(|e| format!("embed next: {e}"))?;
            let next_emb_3d = ops::expand_dims(&next_emb, 0)
                .map_err(|e| format!("expand next: {e}"))?;
            hidden = ops::concatenate_axis(&[&hidden, &next_emb_3d], 1)
                .map_err(|e| format!("extend hidden: {e}"))?;
        }

        Ok(generated)
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn lm_layer(&self, i: usize, x: &Array) -> Result<Array, String> {
        let p = format!("lm.layers.{i}");

        // RMS norm before self-attention.
        let rn_w = self.get(&format!("{p}.input_layernorm.weight"))?;
        let normed = fast::rms_norm(x, rn_w, 1e-6_f32)
            .map_err(|e| format!("lm rms_norm {i}: {e}"))?;

        // Q, K, V, O projections (simplified).
        let q_w = self.get(&format!("{p}.self_attn.q_proj.weight"))?;
        let _k_w = self.get(&format!("{p}.self_attn.k_proj.weight"))?;
        let _v_w = self.get(&format!("{p}.self_attn.v_proj.weight"))?;
        let o_w = self.get(&format!("{p}.self_attn.o_proj.weight"))?;

        let q = normed
            .matmul(&q_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("lm q {i}: {e}"))?;
        let attn_out = q
            .matmul(&o_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("lm o {i}: {e}"))?;
        let x = x.add(&attn_out).map_err(|e| format!("lm attn res {i}: {e}"))?;

        // MLP (SwiGLU): gate * sigmoid(gate) * up → down.
        let rn2_w = self.get(&format!("{p}.post_attention_layernorm.weight"))?;
        let normed2 = fast::rms_norm(&x, rn2_w, 1e-6_f32)
            .map_err(|e| format!("lm rms_norm2 {i}: {e}"))?;
        let gate_w = self.get(&format!("{p}.mlp.gate_proj.weight"))?;
        let up_w = self.get(&format!("{p}.mlp.up_proj.weight"))?;
        let down_w = self.get(&format!("{p}.mlp.down_proj.weight"))?;

        let gate = normed2
            .matmul(&gate_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("lm gate {i}: {e}"))?;
        let up = normed2
            .matmul(&up_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("lm up {i}: {e}"))?;
        // SiLU: gate * sigmoid(gate)
        let gate_act = gate.multiply(
            &ops::sigmoid(&gate).map_err(|e| format!("lm silu {i}: {e}"))?
        ).map_err(|e| format!("lm silu mul {i}: {e}"))?;
        let mlp_in = gate_act.multiply(&up).map_err(|e| format!("lm gate*up {i}: {e}"))?;
        let mlp_out = mlp_in
            .matmul(&down_w.transpose_axes(&[1, 0]).map_err(|e| e.to_string())?)
            .map_err(|e| format!("lm down {i}: {e}"))?;
        x.add(&mlp_out).map_err(|e| format!("lm mlp res {i}: {e}"))
    }

    // ── Weight helpers ────────────────────────────────────────────────────

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn get(&self, key: &str) -> Result<&Array, String> {
        self.weights
            .get(key)
            .ok_or_else(|| format!("Qwen3 MLX: missing weight '{key}'"))
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn count_encoder_layers(&self) -> usize {
        (0..64)
            .take_while(|i| {
                self.weights
                    .contains_key(&format!("encoder.layers.{i}.ff1.up_proj.weight"))
            })
            .count()
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn count_lm_layers(&self) -> usize {
        (0..64)
            .take_while(|i| {
                self.weights
                    .contains_key(&format!("lm.layers.{i}.input_layernorm.weight"))
            })
            .count()
    }
}

// ── Prompt encoding ───────────────────────────────────────────────────────────

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn build_mlx_prompt_ids(tokenizer: &Tokenizer, prompt: Option<&str>) -> Result<Vec<i32>, String> {
    let text = match prompt.filter(|p| !p.trim().is_empty()) {
        Some(p) => format!(
            "<|im_start|>system\n{p}<|im_end|>\n<|im_start|>user\n<|im_end|>\n<|im_start|>assistant\n"
        ),
        None => "<|im_start|>user\n<|im_end|>\n<|im_start|>assistant\n".to_string(),
    };
    let enc = tokenizer
        .encode(text, false)
        .map_err(|e| format!("Qwen3 MLX prompt encode: {e}"))?;
    Ok(enc.get_ids().iter().map(|&id| id as i32).collect())
}
