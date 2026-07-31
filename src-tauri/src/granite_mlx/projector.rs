//! Windowed Q-Former projector: encoder hidden states -> audio embeddings.
//!
//! The encoder output is cut into fixed windows; each window is mean-pooled down
//! to `block_size / downsample_rate` learned queries that cross-attend to the
//! full window. That is what turns ~50 frames/s of audio into the far shorter
//! token sequence the editor consumes.

use mlx_rs::{error::Exception, fast, ops, Array};

use super::encoder::{LayerNorm, Linear, Weights};

#[derive(Debug, Clone)]
pub struct ProjectorConfig {
    pub encoder_dim: i32,
    pub num_encoder_layers: i32,
    pub hidden_size: i32,
    pub llm_dim: i32,
    pub num_heads: i32,
    pub num_layers: usize,
    pub block_size: i32,
    pub downsample_rate: i32,
    pub mlp_ratio: f32,
    pub layernorm_eps: f32,
}

impl ProjectorConfig {
    pub fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let p = &v["projector_config"];
        let num = |k: &str| -> Result<i64, String> {
            p[k].as_i64()
                .ok_or_else(|| format!("projector_config.{k} missing"))
        };
        Ok(Self {
            encoder_dim: num("encoder_dim")? as i32,
            num_encoder_layers: num("num_encoder_layers")? as i32,
            hidden_size: num("hidden_size")? as i32,
            llm_dim: num("llm_dim")? as i32,
            num_heads: num("num_heads")? as i32,
            num_layers: num("num_layers")? as usize,
            block_size: num("block_size")? as i32,
            downsample_rate: num("downsample_rate")? as i32,
            mlp_ratio: p["mlp_ratio"].as_f64().unwrap_or(2.0) as f32,
            layernorm_eps: p["layernorm_eps"].as_f64().unwrap_or(1e-6) as f32,
        })
    }
}

struct QFormerLayer {
    attn_norm: LayerNorm,
    q: Linear,
    k: Linear,
    v: Linear,
    o: Linear,
    mlp_norm: LayerNorm,
    fc1: Linear,
    fc2: Linear,
    num_heads: i32,
    head_dim: i32,
    scale: f32,
}

impl QFormerLayer {
    fn load(w: &Weights, i: usize, cfg: &ProjectorConfig) -> Result<Self, Exception> {
        let p = format!("projector.layers.{i}");
        let head_dim = cfg.hidden_size / cfg.num_heads;
        Ok(Self {
            attn_norm: LayerNorm::load_eps(w, &format!("{p}.attn_norm"), cfg.layernorm_eps)?,
            q: Linear::load(w, &format!("{p}.q_proj"), true)?,
            k: Linear::load(w, &format!("{p}.k_proj"), true)?,
            v: Linear::load(w, &format!("{p}.v_proj"), true)?,
            o: Linear::load(w, &format!("{p}.o_proj"), true)?,
            mlp_norm: LayerNorm::load_eps(w, &format!("{p}.mlp_norm"), cfg.layernorm_eps)?,
            fc1: Linear::load(w, &format!("{p}.fc1"), true)?,
            fc2: Linear::load(w, &format!("{p}.fc2"), true)?,
            num_heads: cfg.num_heads,
            head_dim,
            scale: (head_dim as f32).powf(-0.5),
        })
    }

    fn debug_attn(&self, x: &Array, enc: &Array) -> Result<(Array, Array, Array), Exception> {
        let h = self.attn_norm.forward(x)?;
        let shape = h.shape().to_vec();
        let (bsz, qlen, dim) = (shape[0], shape[1], shape[2]);
        let enc_len = enc.shape()[1];
        let heads = |t: &Array, len: i32| -> Result<Array, Exception> {
            t.reshape(&[bsz, len, self.num_heads, self.head_dim])?
                .transpose_axes(&[0, 2, 1, 3])
        };
        let qp = self.q.forward(&h)?;
        let q = heads(&qp, qlen)?;
        let k = heads(&self.k.forward(enc)?, enc_len)?;
        let v = heads(&self.v.forward(enc)?, enc_len)?;
        let attn = attend(&q, &k, &v, self.scale)?
            .transpose_axes(&[0, 2, 1, 3])?
            .reshape(&[bsz, qlen, dim])?;
        let post = x.add(&self.o.forward(&attn)?)?;
        Ok((q, attn, post))
    }

    /// `x` are the learned queries, `enc` the window they attend over.
    fn forward(&self, x: &Array, enc: &Array) -> Result<Array, Exception> {
        let h = self.attn_norm.forward(x)?;
        let shape = h.shape().to_vec();
        let (bsz, qlen, dim) = (shape[0], shape[1], shape[2]);
        let enc_len = enc.shape()[1];

        let heads = |t: &Array, len: i32| -> Result<Array, Exception> {
            t.reshape(&[bsz, len, self.num_heads, self.head_dim])?
                .transpose_axes(&[0, 2, 1, 3])
        };
        let q = heads(&self.q.forward(&h)?, qlen)?;
        let k = heads(&self.k.forward(enc)?, enc_len)?;
        let v = heads(&self.v.forward(enc)?, enc_len)?;

        let attn = attend(&q, &k, &v, self.scale)?
            .transpose_axes(&[0, 2, 1, 3])?
            .reshape(&[bsz, qlen, dim])?;
        let x = x.add(&self.o.forward(&attn)?)?;

        let m = self.mlp_norm.forward(&x)?;
        let gated = super::encoder::silu(&self.fc1.forward(&m)?)?;
        x.add(&self.fc2.forward(&gated)?)
    }
}

impl Projector {
    /// Layer-0 attention internals, for parity bisection.
    pub fn debug_attn(&self, h: &Array, enc: &Array) -> Result<(Array, Array, Array), Exception> {
        self.layers[0].debug_attn(h, enc)
    }
}

pub struct Projector {
    layer_norms: Vec<LayerNorm>,
    layer_projector: Linear,
    layers: Vec<QFormerLayer>,
    out_norm: LayerNorm,
    out_linear: Linear,
    query: Array,
    window_positions: Array,
    cfg: ProjectorConfig,
}

impl Projector {
    pub fn load(w: &Weights, cfg: ProjectorConfig) -> Result<Self, Exception> {
        let layer_norms = (0..cfg.num_encoder_layers)
            .map(|i| {
                LayerNorm::load_eps(w, &format!("projector.layer_norms.{i}"), cfg.layernorm_eps)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let layers = (0..cfg.num_layers)
            .map(|i| QFormerLayer::load(w, i, &cfg))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            layer_norms,
            layer_projector: Linear::load(w, "projector.layer_projector", true)?,
            layers,
            out_norm: LayerNorm::load_eps(w, "projector.out_norm", cfg.layernorm_eps)?,
            out_linear: Linear::load(w, "projector.out_linear", true)?,
            query: super::encoder::get(w, "projector.query")?.clone(),
            window_positions: super::encoder::get(w, "projector.window_positions")?.clone(),
            cfg,
        })
    }

    /// `multilayer` is the concatenation of the selected encoder hidden states,
    /// shaped (B, T, num_encoder_layers * encoder_dim).
    /// Stage 1+2 only, so the parity harness can bisect this module.
    pub fn debug_stages(&self, multilayer: &Array) -> Result<(Array, Array), Exception> {
        let cfg = &self.cfg;
        let shape = multilayer.shape().to_vec();
        let (bsz, seq_len) = (shape[0], shape[1]);
        let split = multilayer.reshape(&[bsz, seq_len, cfg.num_encoder_layers, cfg.encoder_dim])?;
        let parts = self
            .layer_norms
            .iter()
            .enumerate()
            .map(|(i, norm)| {
                let idx = Array::from_int(i as i32);
                let slice = split
                    .take_axis(&idx, 2)?
                    .reshape(&[bsz, seq_len, cfg.encoder_dim])?;
                norm.forward(&slice)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let step1 = ops::concatenate_axis(&parts, -1)?;
        let step2 = gelu(&self.layer_projector.forward(&step1)?)?;
        Ok((step1, step2))
    }

    /// Windowing outputs (query stack, window context) plus the first Q-Former
    /// layer, so a parity failure can be localised.
    pub fn debug_windows(&self, step2: &Array) -> Result<(Array, Array, Array), Exception> {
        let cfg = &self.cfg;
        let shape = step2.shape().to_vec();
        let (bsz, seq_len) = (shape[0], shape[1]);
        let block = cfg.block_size;
        let rest = seq_len % block;
        let mut nblocks = seq_len / block;
        let x = if rest > 0 {
            nblocks += 1;
            ops::pad(step2, &[(0, 0), (0, block - rest), (0, 0)], Array::from_f32(0.0), None)?
        } else {
            step2.clone()
        };
        let hidden = x.reshape(&[bsz * nblocks, block, cfg.hidden_size])?;
        let qlen = self.query.shape()[1];
        let mean_pool = ops::mean_axis(
            &hidden.reshape(&[bsz * nblocks, qlen, cfg.downsample_rate, cfg.hidden_size])?,
            -2,
            false,
        )?;
        let h = self.query.add(&mean_pool)?;
        let enc = hidden.add(&self.window_positions)?;
        let l0 = self.layers[0].forward(&h, &enc)?;
        Ok((h, enc, l0))
    }

    pub fn forward(&self, multilayer: &Array) -> Result<Array, Exception> {
        let cfg = &self.cfg;
        let shape = multilayer.shape().to_vec();
        let (bsz, seq_len) = (shape[0], shape[1]);

        // Each source layer is normalised on its own before they are re-joined.
        let split = multilayer.reshape(&[bsz, seq_len, cfg.num_encoder_layers, cfg.encoder_dim])?;
        let parts = self
            .layer_norms
            .iter()
            .enumerate()
            .map(|(i, norm)| {
                let idx = Array::from_int(i as i32);
                let slice = split
                    .take_axis(&idx, 2)?
                    .reshape(&[bsz, seq_len, cfg.encoder_dim])?;
                norm.forward(&slice)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let x = ops::concatenate_axis(&parts, -1)?;
        let x = gelu(&self.layer_projector.forward(&x)?)?;

        let block = cfg.block_size;
        let rest = seq_len % block;
        let mut nblocks = seq_len / block;
        let x = if rest > 0 {
            nblocks += 1;
            ops::pad(
                &x,
                &[(0, 0), (0, block - rest), (0, 0)],
                Array::from_f32(0.0),
                None,
            )?
        } else {
            x
        };

        let hidden = x.reshape(&[bsz * nblocks, block, cfg.hidden_size])?;
        let qlen = self.query.shape()[1];
        // Mean-pool each window down to one vector per query slot.
        let mean_pool = ops::mean_axis(
            &hidden.reshape(&[
                bsz * nblocks,
                qlen,
                cfg.downsample_rate,
                cfg.hidden_size,
            ])?,
            -2,
            false,
        )?;

        let mut h = self.query.add(&mean_pool)?;
        let enc = hidden.add(&self.window_positions)?;
        for layer in &self.layers {
            h = layer.forward(&h, &enc)?;
        }

        let h = h.reshape(&[bsz, nblocks * qlen, -1])?;
        self.out_linear.forward(&self.out_norm.forward(&h)?)
    }
}

/// Plain scaled dot-product attention, written out rather than using the fused
/// kernel: this is unmasked cross-attention over a 15-slot window, and the
/// fused path did not reproduce the reference here.
fn attend(q: &Array, k: &Array, v: &Array, scale: f32) -> Result<Array, Exception> {
    let scores = q
        .matmul(&k.transpose_axes(&[0, 1, 3, 2])?)?
        .multiply(Array::from_f32(scale))?;
    ops::softmax_axis(&scores, -1, None)?.matmul(v)
}

/// Exact gelu, matching `nn.gelu` in the reference rather than a tanh approximation.
fn gelu(x: &Array) -> Result<Array, Exception> {
    let inv_sqrt2 = Array::from_f32(std::f32::consts::FRAC_1_SQRT_2);
    let cdf = ops::erf(&x.multiply(&inv_sqrt2)?)?
        .add(Array::from_f32(1.0))?
        .multiply(Array::from_f32(0.5))?;
    x.multiply(&cdf)
}
