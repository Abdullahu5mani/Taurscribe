//! Conformer CTC encoder, ported from `scripts/granite_mlx/granite_nar_mlx.py`.
//!
//! Layer-for-layer translation of the validated Python port, so the two can be
//! compared stage by stage. Weight-layout fixes happen once at load: BatchNorm
//! is folded into the depthwise conv, and the 1x1 convs are squeezed to Linear.

use std::collections::HashMap;

use mlx_rs::ops::indexing::IndexOp;
use mlx_rs::{error::Exception, fast, ops, Array, Dtype};

pub type Weights = HashMap<String, Array>;

pub fn get<'a>(w: &'a Weights, key: &str) -> Result<&'a Array, Exception> {
    w.get(key)
        .ok_or_else(|| Exception::custom(format!("missing tensor: {key}")))
}

/// `y = x @ W^T + b`, matching how MLX stores `nn.Linear` weights as (out, in).
pub struct Linear {
    weight: Array,
    bias: Option<Array>,
}

impl Linear {
    pub fn load(w: &Weights, prefix: &str, bias: bool) -> Result<Self, Exception> {
        Ok(Self {
            weight: get(w, &format!("{prefix}.weight"))?.clone(),
            bias: if bias {
                Some(get(w, &format!("{prefix}.bias"))?.clone())
            } else {
                None
            },
        })
    }

    pub fn forward(&self, x: &Array) -> Result<Array, Exception> {
        let y = x.matmul(&self.weight.transpose_axes(&[1, 0])?)?;
        match &self.bias {
            Some(b) => y.add(b),
            None => Ok(y),
        }
    }
}

pub struct LayerNorm {
    weight: Array,
    bias: Array,
    eps: f32,
}

impl LayerNorm {
    fn load(w: &Weights, prefix: &str) -> Result<Self, Exception> {
        Self::load_eps(w, prefix, 1e-5)
    }

    pub fn load_eps(w: &Weights, prefix: &str, eps: f32) -> Result<Self, Exception> {
        Ok(Self {
            weight: get(w, &format!("{prefix}.weight"))?.clone(),
            bias: get(w, &format!("{prefix}.bias"))?.clone(),
            eps,
        })
    }

    pub fn forward(&self, x: &Array) -> Result<Array, Exception> {
        fast::layer_norm(x, Some(&self.weight), Some(&self.bias), self.eps)
    }
}

struct FeedForward {
    pre_norm: LayerNorm,
    up: Linear,
    down: Linear,
}

impl FeedForward {
    fn load(w: &Weights, prefix: &str) -> Result<Self, Exception> {
        Ok(Self {
            pre_norm: LayerNorm::load(w, &format!("{prefix}.pre_norm"))?,
            up: Linear::load(w, &format!("{prefix}.up_proj"), true)?,
            down: Linear::load(w, &format!("{prefix}.down_proj"), true)?,
        })
    }

    fn forward(&self, x: &Array) -> Result<Array, Exception> {
        let h = self.up.forward(&self.pre_norm.forward(x)?)?;
        self.down.forward(&silu(&h)?)
    }
}

pub fn silu(x: &Array) -> Result<Array, Exception> {
    x.multiply(ops::sigmoid(x)?)
}

/// Block-local attention with Shaw-style relative position embeddings.
struct Attention {
    pre_norm: LayerNorm,
    to_q: Linear,
    to_kv: Linear,
    to_out: Linear,
    rel_pos_emb: Array,
    num_heads: i32,
    dim_head: i32,
    context_size: i32,
    scale: f32,
}

impl Attention {
    fn load(w: &Weights, prefix: &str, cfg: &EncoderConfig) -> Result<Self, Exception> {
        Ok(Self {
            pre_norm: LayerNorm::load(w, &format!("{prefix}.pre_norm"))?,
            to_q: Linear::load(w, &format!("{prefix}.to_q"), false)?,
            to_kv: Linear::load(w, &format!("{prefix}.to_kv"), false)?,
            to_out: Linear::load(w, &format!("{prefix}.to_out"), true)?,
            rel_pos_emb: get(w, &format!("{prefix}.rel_pos_emb.weight"))?.clone(),
            num_heads: cfg.num_heads,
            dim_head: cfg.dim_head,
            context_size: cfg.context_size,
            scale: (cfg.dim_head as f32).powf(-0.5),
        })
    }

    /// `dists` is the precomputed (ctx, ctx) table of clipped relative offsets.
    fn forward(&self, x: &Array, dists: &Array) -> Result<Array, Exception> {
        let x = self.pre_norm.forward(x)?;
        let shape = x.shape().to_vec();
        let (bsz, seq_len) = (shape[0], shape[1]);
        let ctx = self.context_size;

        let num_blocks = (seq_len + ctx - 1) / ctx;
        let remainder = seq_len % ctx;
        let x = if remainder > 0 {
            ops::pad(&x, &[(0, 0), (0, ctx - remainder), (0, 0)], Array::from_f32(0.0), None)?
        } else {
            x
        };
        let padded_len = x.shape()[1];

        let q = self.to_q.forward(&x)?;
        let kv = self.to_kv.forward(&x)?;
        let parts = ops::split(&kv, 2, -1)?;
        let (k, v) = (&parts[0], &parts[1]);

        // (B, T, inner) -> (B*blocks, heads, ctx, dim_head): MLX's fused
        // attention kernel takes rank-4 inputs only, so blocks ride the batch axis.
        let to_blocks = |t: &Array| -> Result<Array, Exception> {
            t.reshape(&[bsz * num_blocks, ctx, self.num_heads, self.dim_head])?
                .transpose_axes(&[0, 2, 1, 3])
        };
        let (q, k, v) = (to_blocks(&q)?, to_blocks(k)?, to_blocks(v)?);

        // Shaw bias as an additive mask. einsum("nhcd,crd->nhcr") expressed as a
        // batched matmul over the ctx axis, which mlx-rs has no einsum for.
        let rel = self.rel_pos_emb.index(dists);
        let n = bsz * num_blocks;
        let q_c = q
            .transpose_axes(&[2, 0, 1, 3])? // (ctx, N, H, D)
            .reshape(&[ctx, n * self.num_heads, self.dim_head])?;
        let rel_c = rel.transpose_axes(&[0, 2, 1])?; // (ctx, D, ctx)
        let mut pos_attn = q_c
            .matmul(&rel_c)? // (ctx, N*H, ctx)
            .reshape(&[ctx, n, self.num_heads, ctx])?
            .transpose_axes(&[1, 2, 0, 3])? // (N, H, ctx, ctx)
            .multiply(Array::from_f32(self.scale))?;

        if remainder > 0 {
            // Slots past the true end of the final block must not attend.
            let keep = ops::arange::<_, f32>(None, ctx as f32, None)?
                .lt(Array::from_f32(remainder as f32))?;
            let valid = keep
                .reshape(&[ctx, 1])?
                .logical_and(&keep.reshape(&[1, ctx])?)?
                .as_dtype(pos_attn.dtype())?;
            let neg = Array::from_f32(-65504.0).as_dtype(pos_attn.dtype())?;
            let grouped = pos_attn.reshape(&[bsz, num_blocks, self.num_heads, ctx, ctx])?;
            let head = grouped.index((.., ..num_blocks - 1));
            let last = grouped.index((.., num_blocks - 1));
            let masked = last
                .multiply(&valid)?
                .add(&Array::from_f32(1.0).subtract(&valid)?.multiply(&neg)?)?;
            let last_shape = [bsz, 1, self.num_heads, ctx, ctx];
            pos_attn = ops::concatenate_axis(&[head, masked.reshape(&last_shape)?], 1)?
                .reshape(&[n, self.num_heads, ctx, ctx])?;
        }

        let out = fast::scaled_dot_product_attention(
            &q, &k, &v, self.scale,
            fast::ScaledDotProductAttentionMask::Array(&pos_attn),
        )?;
        let out = out
            .transpose_axes(&[0, 2, 1, 3])?
            .reshape(&[bsz, padded_len, -1])?;
        self.to_out.forward(&out.index((.., ..seq_len)))
    }
}

/// Conv module. BatchNorm is folded into `depth_conv` at load time.
struct ConvModule {
    norm: LayerNorm,
    up: Linear,
    depth_w: Array,
    depth_b: Array,
    down: Linear,
    padding: i32,
    groups: i32,
}

impl ConvModule {
    fn load(w: &Weights, prefix: &str, cfg: &EncoderConfig) -> Result<Self, Exception> {
        let inner = cfg.hidden_dim * cfg.conv_expansion_factor;
        Ok(Self {
            norm: LayerNorm::load(w, &format!("{prefix}.norm"))?,
            up: Linear::load(w, &format!("{prefix}.up_conv"), true)?,
            depth_w: get(w, &format!("{prefix}.depth_conv.weight"))?.clone(),
            depth_b: get(w, &format!("{prefix}.depth_conv.bias"))?.clone(),
            down: Linear::load(w, &format!("{prefix}.down_conv"), true)?,
            padding: cfg.conv_kernel_size / 2,
            groups: inner,
        })
    }

    fn forward(&self, x: &Array) -> Result<Array, Exception> {
        let h = self.up.forward(&self.norm.forward(x)?)?;
        let parts = ops::split(&h, 2, -1)?;
        let gated = parts[0].multiply(ops::sigmoid(&parts[1])?)?;
        let conv = ops::conv1d(&gated, &self.depth_w, 1, self.padding, 1, self.groups)?
            .add(&self.depth_b)?;
        self.down.forward(&silu(&conv)?)
    }
}

struct Block {
    ff1: FeedForward,
    attn: Attention,
    conv: ConvModule,
    ff2: FeedForward,
    post_norm: LayerNorm,
}

impl Block {
    fn load(w: &Weights, i: usize, cfg: &EncoderConfig) -> Result<Self, Exception> {
        let p = format!("encoder.layers.{i}");
        Ok(Self {
            ff1: FeedForward::load(w, &format!("{p}.ff1"))?,
            attn: Attention::load(w, &format!("{p}.attn"), cfg)?,
            conv: ConvModule::load(w, &format!("{p}.conv"), cfg)?,
            ff2: FeedForward::load(w, &format!("{p}.ff2"))?,
            post_norm: LayerNorm::load(w, &format!("{p}.post_norm"))?,
        })
    }

    fn forward(&self, x: &Array, dists: &Array) -> Result<Array, Exception> {
        let half = Array::from_f32(0.5);
        let x = self.ff1.forward(x)?.multiply(&half)?.add(x)?;
        let x = self.attn.forward(&x, dists)?.add(&x)?;
        let x = self.conv.forward(&x)?.add(&x)?;
        let x = self.ff2.forward(&x)?.multiply(&half)?.add(&x)?;
        self.post_norm.forward(&x)
    }
}

#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub hidden_dim: i32,
    pub num_layers: usize,
    pub num_heads: i32,
    pub dim_head: i32,
    pub context_size: i32,
    pub max_pos_emb: i32,
    pub conv_expansion_factor: i32,
    pub conv_kernel_size: i32,
    pub self_conditioning_layer: usize,
    pub bpe_pooling_window: i32,
}

impl EncoderConfig {
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let e = &v["encoder_config"];
        let num = |k: &str| -> Result<i64, String> {
            e[k].as_i64().ok_or_else(|| format!("encoder_config.{k} missing"))
        };
        Ok(Self {
            hidden_dim: num("hidden_dim")? as i32,
            num_layers: num("num_layers")? as usize,
            num_heads: num("num_heads")? as i32,
            dim_head: num("dim_head")? as i32,
            context_size: num("context_size")? as i32,
            max_pos_emb: num("max_pos_emb")? as i32,
            conv_expansion_factor: num("conv_expansion_factor")? as i32,
            conv_kernel_size: num("conv_kernel_size")? as i32,
            // Upstream self-conditions halfway through the stack.
            self_conditioning_layer: e["self_conditioning_layer"]
                .as_i64()
                .unwrap_or(num("num_layers")? / 2) as usize,
            bpe_pooling_window: e["bpe_pooling_window"].as_i64().unwrap_or(4) as i32,
        })
    }
}

pub struct CtcEncoder {
    input_linear: Linear,
    layers: Vec<Block>,
    out: Linear,
    out_mid: Linear,
    out_bpe: Linear,
    dists: Array,
    cfg: EncoderConfig,
}

impl CtcEncoder {
    pub fn load(w: &Weights, cfg: EncoderConfig) -> Result<Self, Exception> {
        let ctx = cfg.context_size;
        // dists[i][j] = clip(i - j, -ctx, ctx) + max_pos_emb
        let seq = ops::arange::<_, f32>(None, ctx as f32, None)?;
        let dists = ops::clip(
            &seq.reshape(&[ctx, 1])?.subtract(&seq.reshape(&[1, ctx])?)?,
            (Array::from_f32(-(ctx as f32)), Array::from_f32(ctx as f32)),
        )?
        .add(Array::from_f32(cfg.max_pos_emb as f32))?
        .as_dtype(Dtype::Int32)?;

        let layers = (0..cfg.num_layers)
            .map(|i| Block::load(w, i, &cfg))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            input_linear: Linear::load(w, "encoder.input_linear", true)?,
            layers,
            out: Linear::load(w, "encoder.out", true)?,
            out_mid: Linear::load(w, "encoder.out_mid", true)?,
            out_bpe: Linear::load(w, "encoder.out_bpe", true)?,
            dists,
            cfg,
        })
    }

    /// Returns `(bpe_logits, hidden_states_for_the_requested_layers)`.
    /// `wanted` uses the upstream convention where entry *i* is the output of
    /// layer *i*, so it is 1-based over this stack.
    pub fn forward(
        &self,
        features: &Array,
        wanted: &[usize],
    ) -> Result<(Array, Vec<Array>), Exception> {
        let mut x = self.input_linear.forward(features)?;
        let mut selected: HashMap<usize, Array> = HashMap::new();
        let mut blank_probs: Option<Array> = None;

        for (idx0, layer) in self.layers.iter().enumerate() {
            let idx = idx0 + 1;
            x = layer.forward(&x, &self.dists)?;

            if idx == self.cfg.self_conditioning_layer {
                let mid = self.out.forward(&x)?;
                let probs = ops::softmax_axis(&mid.as_dtype(Dtype::Float32)?, -1, None)?;
                blank_probs = Some(probs.index((.., .., 0)));
                x = x.add(&self.out_mid.forward(&probs.as_dtype(x.dtype())?)?)?;
            }
            if wanted.contains(&idx) {
                selected.insert(idx, x.clone());
            }
        }

        let blank = blank_probs
            .ok_or_else(|| Exception::custom("self-conditioning layer never ran"))?;
        let importance = Array::from_f32(1.0).subtract(&blank)?;
        let pooled = posterior_weighted_pool(&x, &importance, self.cfg.bpe_pooling_window)?;
        let bpe_logits = self.out_bpe.forward(&pooled)?;

        let hidden = wanted
            .iter()
            .map(|i| {
                selected
                    .get(i)
                    .cloned()
                    .ok_or_else(|| Exception::custom(format!("layer {i} not captured")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((bpe_logits, hidden))
    }
}

fn posterior_weighted_pool(
    hidden: &Array,
    importance: &Array,
    window: i32,
) -> Result<Array, Exception> {
    let shape = hidden.shape().to_vec();
    let (bsz, seq_len, dim) = (shape[0], shape[1], shape[2]);
    let pad = (window - seq_len % window) % window;
    let (hidden, importance) = if pad > 0 {
        (
            ops::pad(hidden, &[(0, 0), (0, pad), (0, 0)], Array::from_f32(0.0), None)?,
            ops::pad(importance, &[(0, 0), (0, pad)], Array::from_f32(0.0), None)?,
        )
    } else {
        (hidden.clone(), importance.clone())
    };
    let windows = hidden.shape()[1] / window;
    let hidden = hidden
        .as_dtype(Dtype::Float32)?
        .reshape(&[bsz, windows, window, dim])?;
    let importance = importance
        .as_dtype(Dtype::Float32)?
        .reshape(&[bsz, windows, window])?;
    let denom = ops::sum_axis(&importance, -1, true)?.add(Array::from_f32(1e-8))?;
    let weights = importance.divide(&denom)?.reshape(&[bsz, windows, window, 1])?;
    ops::sum_axis(&hidden.multiply(&weights)?, 2, false)
}
