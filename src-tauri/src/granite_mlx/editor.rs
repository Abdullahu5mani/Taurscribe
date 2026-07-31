//! Bidirectional Granite editor LM — the non-autoregressive half of the model.
//!
//! Unlike a normal decoder this runs **once** over the whole sequence with no
//! causal mask and no token loop: it edits the encoder's CTC guess in place.
//! That is why quantisation hurts it so much — there is no autoregressive step
//! in which to recover from weight error.

use mlx_rs::ops::indexing::IndexOp;
use mlx_rs::{error::Exception, fast, Array};

use super::encoder::{get, silu, Linear, Weights};

#[derive(Debug, Clone)]
pub struct EditorConfig {
    pub hidden_size: i32,
    pub num_layers: usize,
    pub num_heads: i32,
    pub num_kv_heads: i32,
    pub head_dim: i32,
    pub rms_norm_eps: f32,
    pub rope_theta: f32,
    pub attention_multiplier: f32,
    pub residual_multiplier: f32,
    pub embedding_multiplier: f32,
    pub logits_scaling: f32,
    pub tied_lm_head: bool,
}

impl EditorConfig {
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let t = &v["text_config"];
        let num = |k: &str| -> Result<i64, String> {
            t[k].as_i64().ok_or_else(|| format!("text_config.{k} missing"))
        };
        let hidden_size = num("hidden_size")? as i32;
        let num_heads = num("num_attention_heads")? as i32;
        // RoPE base moved under `rope_parameters` in newer Granite configs.
        let rope_theta = t["rope_parameters"]["rope_theta"]
            .as_f64()
            .or_else(|| t["rope_theta"].as_f64())
            .unwrap_or(10000.0) as f32;
        Ok(Self {
            hidden_size,
            num_layers: num("num_hidden_layers")? as usize,
            num_heads,
            num_kv_heads: num("num_key_value_heads")? as i32,
            head_dim: t["head_dim"].as_i64().unwrap_or((hidden_size / num_heads) as i64) as i32,
            rms_norm_eps: t["rms_norm_eps"].as_f64().unwrap_or(1e-5) as f32,
            rope_theta,
            attention_multiplier: t["attention_multiplier"].as_f64().unwrap_or_else(|| {
                (hidden_size as f64 / num_heads as f64).powf(-0.5)
            }) as f32,
            residual_multiplier: t["residual_multiplier"].as_f64().unwrap_or(1.0) as f32,
            embedding_multiplier: t["embedding_multiplier"].as_f64().unwrap_or(1.0) as f32,
            logits_scaling: t["logits_scaling"].as_f64().unwrap_or(1.0) as f32,
            // Resolved against the checkpoint at load: a separate lm_head wins.
            tied_lm_head: v["tie_word_embeddings"].as_bool().unwrap_or(true),
        })
    }
}

struct EditorLayer {
    input_norm_w: Array,
    post_attn_norm_w: Array,
    q: Linear,
    k: Linear,
    v: Linear,
    o: Linear,
    gate: Linear,
    up: Linear,
    down: Linear,
    cfg: EditorConfig,
}

impl EditorLayer {
    fn load(w: &Weights, i: usize, cfg: &EditorConfig) -> Result<Self, Exception> {
        let p = format!("editor.layers.{i}");
        Ok(Self {
            input_norm_w: get(w, &format!("{p}.input_layernorm_w"))?.clone(),
            post_attn_norm_w: get(w, &format!("{p}.post_attention_layernorm_w"))?.clone(),
            q: Linear::load(w, &format!("{p}.q_proj"), false)?,
            k: Linear::load(w, &format!("{p}.k_proj"), false)?,
            v: Linear::load(w, &format!("{p}.v_proj"), false)?,
            o: Linear::load(w, &format!("{p}.o_proj"), false)?,
            gate: Linear::load(w, &format!("{p}.gate_proj"), false)?,
            up: Linear::load(w, &format!("{p}.up_proj"), false)?,
            down: Linear::load(w, &format!("{p}.down_proj"), false)?,
            cfg: cfg.clone(),
        })
    }

    fn forward(&self, x: &Array) -> Result<Array, Exception> {
        let c = &self.cfg;
        let shape = x.shape().to_vec();
        let (bsz, seq_len) = (shape[0], shape[1]);
        let h = fast::rms_norm(x, &self.input_norm_w, c.rms_norm_eps)?;

        let shape_heads = |t: Array, heads: i32| -> Result<Array, Exception> {
            t.reshape(&[bsz, seq_len, heads, c.head_dim])?
                .transpose_axes(&[0, 2, 1, 3])
        };
        let q = shape_heads(self.q.forward(&h)?, c.num_heads)?;
        let k = shape_heads(self.k.forward(&h)?, c.num_kv_heads)?;
        let v = shape_heads(self.v.forward(&h)?, c.num_kv_heads)?;

        let q = fast::rope(&q, c.head_dim, false, c.rope_theta, 1.0, 0, None)?;
        let k = fast::rope(&k, c.head_dim, false, c.rope_theta, 1.0, 0, None)?;

        // Non-causal on purpose: the editor sees the whole sequence at once.
        // Grouped-query attention (16 q heads over 4 kv heads) is handled by the
        // fused kernel from the head-count mismatch.
        let attn = fast::scaled_dot_product_attention(&q, &k, &v, c.attention_multiplier, None)?
            .transpose_axes(&[0, 2, 1, 3])?
            .reshape(&[bsz, seq_len, -1])?;
        let residual = Array::from_f32(c.residual_multiplier);
        let x = x.add(&self.o.forward(&attn)?.multiply(&residual)?)?;

        let h = fast::rms_norm(&x, &self.post_attn_norm_w, c.rms_norm_eps)?;
        let gated = silu(&self.gate.forward(&h)?)?.multiply(&self.up.forward(&h)?)?;
        x.add(&self.down.forward(&gated)?.multiply(&residual)?)
    }
}

pub struct Editor {
    embed_tokens: Array,
    lm_head: Option<Array>,
    layers: Vec<EditorLayer>,
    norm_w: Array,
    cfg: EditorConfig,
}

impl Editor {
    pub fn load(w: &Weights, mut cfg: EditorConfig) -> Result<Self, Exception> {
        let lm_head = w.get("editor.lm_head.weight").cloned();
        cfg.tied_lm_head = lm_head.is_none();
        let layers = (0..cfg.num_layers)
            .map(|i| EditorLayer::load(w, i, &cfg))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            embed_tokens: get(w, "editor.embed_tokens.weight")?.clone(),
            lm_head,
            layers,
            norm_w: get(w, "editor.norm_w")?.clone(),
            cfg,
        })
    }

    /// Row lookup into the token embedding table.
    pub fn embed(&self, token_ids: &Array) -> Array {
        self.embed_tokens.index(token_ids)
    }

    fn project_to_vocab(&self, x: &Array) -> Result<Array, Exception> {
        match &self.lm_head {
            Some(head) => x.matmul(&head.transpose_axes(&[1, 0])?),
            None => x.matmul(&self.embed_tokens.transpose_axes(&[1, 0])?),
        }
    }

    /// `logits_from` drops leading positions before the vocabulary projection.
    /// Only the text tail is ever read and the projection is 100352 wide, so
    /// projecting the audio prefix builds a large tensor that is thrown away.
    /// Skipping it is exact, not an approximation.
    pub fn forward(&self, embeds: &Array, logits_from: i32) -> Result<Array, Exception> {
        let c = &self.cfg;
        let mut x = embeds.multiply(Array::from_f32(c.embedding_multiplier))?;
        for layer in &self.layers {
            x = layer.forward(&x)?;
        }
        x = fast::rms_norm(&x, &self.norm_w, c.rms_norm_eps)?;
        if logits_from > 0 {
            x = x.index((.., logits_from..));
        }
        self.project_to_vocab(&x)?
            .divide(Array::from_f32(c.logits_scaling))
    }
}

/// Insert blank edit slots around the encoder's CTC tokens so the editor has
/// somewhere to write insertions. Mirrors `_add_insertion_slots`.
pub fn add_insertion_slots(ids: &[i64], blank: i64, min_len: usize) -> Vec<i64> {
    let mut out = Vec::with_capacity(ids.len() * 2 + 1);
    out.push(blank);
    for &id in ids {
        out.push(id);
        out.push(blank);
    }
    while out.len() < min_len {
        out.push(blank);
    }
    out
}

/// Collapse repeats then drop blanks — greedy CTC decoding.
pub fn ctc_greedy(ids: &[i64], blank: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let mut prev: Option<i64> = None;
    for &id in ids {
        if Some(id) != prev {
            if id != blank {
                out.push(id);
            }
            prev = Some(id);
        }
    }
    out
}

/// Row-wise argmax over the vocabulary axis.
pub fn argmax_rows(logits: &Array) -> Result<Vec<i64>, Exception> {
    let ids = mlx_rs::ops::indexing::argmax_axis(logits, -1, false)?;
    Ok(ids.as_slice::<u32>().iter().map(|&v| v as i64).collect())
}
