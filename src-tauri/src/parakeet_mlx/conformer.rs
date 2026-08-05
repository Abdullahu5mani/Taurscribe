//! FastConformer Encoder for Parakeet Nemotron on Apple Silicon Metal GPU.
//!
//! Implements 24 FastConformer layers with:
//! - Macaron-style dual FeedForward with SiLU and 0.5 residual scaling
//! - Relative Positional Multi-Head Attention with streaming lookback cache (70 frames)
//! - Depthwise-Separable 1D Convolution with streaming convolution context (8 frames)
//! - Fast Metal LayerNorm

use mlx_rs::ops::indexing::IndexOp;
use mlx_rs::{error::Exception, fast, ops, Array};
use std::collections::HashMap;

pub type Weights = HashMap<String, Array>;

pub struct LayerNorm {
    weight: Array,
    bias: Array,
    eps: f32,
}

impl LayerNorm {
    pub fn new(weight: Array, bias: Array) -> Self {
        Self {
            weight,
            bias,
            eps: 1e-5,
        }
    }

    pub fn forward(&self, x: &Array) -> Result<Array, Exception> {
        fast::layer_norm(x, Some(&self.weight), Some(&self.bias), self.eps)
    }
}

pub fn silu(x: &Array) -> Result<Array, Exception> {
    x.multiply(&ops::sigmoid(x)?)
}

pub struct FeedForward {
    norm: LayerNorm,
    linear1: Array, // [4096, 1024]
    linear2: Array, // [1024, 4096]
}

impl FeedForward {
    pub fn load(w: &Weights, norm_pfx: &str, ff_pfx: &str) -> Result<Self, Exception> {
        let get = |k: &str| -> Result<&Array, Exception> {
            w.get(k).ok_or_else(|| Exception::custom(format!("missing tensor: {k}")))
        };

        let norm_w = get(&format!("{norm_pfx}.weight"))?.clone();
        let norm_b = get(&format!("{norm_pfx}.bias"))?.clone();
        let linear1 = get(&format!("{ff_pfx}.linear1.weight"))?.clone();
        let linear2 = get(&format!("{ff_pfx}.linear2.weight"))?.clone();

        Ok(Self {
            norm: LayerNorm::new(norm_w, norm_b),
            linear1,
            linear2,
        })
    }

    pub fn forward(&self, x: &Array) -> Result<Array, Exception> {
        let h = self.norm.forward(x)?;
        let h = silu(&h.matmul(&self.linear1.transpose_axes(&[1, 0])?)?)?;
        h.matmul(&self.linear2.transpose_axes(&[1, 0])?)
    }
}

pub struct RelPosSelfAttention {
    norm: LayerNorm,
    linear_q: Array,   // [1024, 1024]
    linear_k: Array,   // [1024, 1024]
    linear_v: Array,   // [1024, 1024]
    linear_pos: Array, // [1024, 1024]
    linear_out: Array, // [1024, 1024]
    pos_bias_u: Array, // [1, 1, 8, 128]
    pos_bias_v: Array, // [1, 1, 8, 128]
}

impl RelPosSelfAttention {
    pub fn load(w: &Weights, layer_idx: usize) -> Result<Self, Exception> {
        let get = |k: &str| -> Result<&Array, Exception> {
            w.get(k).ok_or_else(|| Exception::custom(format!("missing tensor: {k}")))
        };

        let pfx = format!("encoder.layers.{layer_idx}");
        let norm_w = get(&format!("{pfx}.norm_self_att.weight"))?.clone();
        let norm_b = get(&format!("{pfx}.norm_self_att.bias"))?.clone();

        let linear_q = get(&format!("{pfx}.self_attn.linear_q.weight"))?.clone();
        let linear_k = get(&format!("{pfx}.self_attn.linear_k.weight"))?.clone();
        let linear_v = get(&format!("{pfx}.self_attn.linear_v.weight"))?.clone();
        let linear_pos = get(&format!("{pfx}.self_attn.linear_pos.weight"))?.clone();
        let linear_out = get(&format!("{pfx}.self_attn.linear_out.weight"))?.clone();

        let u_raw = get(&format!("{pfx}.self_attn.pos_bias_u"))?; // [8, 128]
        let v_raw = get(&format!("{pfx}.self_attn.pos_bias_v"))?; // [8, 128]
        let pos_bias_u = u_raw.reshape(&[1, 1, 8, 128])?;
        let pos_bias_v = v_raw.reshape(&[1, 1, 8, 128])?;

        Ok(Self {
            norm: LayerNorm::new(norm_w, norm_b),
            linear_q,
            linear_k,
            linear_v,
            linear_pos,
            linear_out,
            pos_bias_u,
            pos_bias_v,
        })
    }

    /// Forward pass:
    /// - `x`: [1, T_q, 1024]
    /// - `cache_channel`: [1, 70, 1024]
    /// - `cache_len`: number of valid frames in cache (0..=70)
    /// - `pos_slice`: [1, T_pos, 1024]
    /// Returns: (attn_out [1, T_q, 1024], new_cache_channel [1, 70, 1024])
    pub fn forward(
        &self,
        x: &Array,
        cache_channel: &Array,
        cache_len: i32,
        pos_slice: &Array,
    ) -> Result<(Array, Array), Exception> {
        let x_norm = self.norm.forward(x)?;
        let t_q = x.shape()[1];

        // Concatenate lookback cache with current chunk along time (axis 1): [1, 70 + T_q, 1024]
        let k_in = ops::concatenate_axis(&[cache_channel, &x_norm], 1)?;
        let t_k = k_in.shape()[1];

        // Next cache is the last 70 frames
        let next_cache = k_in.index((.., (t_k - 70).., ..));

        // Projections
        let q = x_norm
            .matmul(&self.linear_q.transpose_axes(&[1, 0])?)?
            .reshape(&[1, t_q, 8, 128])?;
        let k = k_in
            .matmul(&self.linear_k.transpose_axes(&[1, 0])?)?
            .reshape(&[1, t_k, 8, 128])?;
        let v = k_in
            .matmul(&self.linear_v.transpose_axes(&[1, 0])?)?
            .reshape(&[1, t_k, 8, 128])?;
        let t_pos = pos_slice.shape()[1];
        let p = pos_slice
            .matmul(&self.linear_pos.transpose_axes(&[1, 0])?)?
            .reshape(&[1, t_pos, 8, 128])?;

        // Q with biases
        let q_u = q.add(&self.pos_bias_u)?.transpose_axes(&[0, 2, 1, 3])?; // [1, 8, T_q, 128]
        let q_v = q.add(&self.pos_bias_v)?.transpose_axes(&[0, 2, 1, 3])?; // [1, 8, T_q, 128]

        // Content score: [1, 8, T_q, 128] @ [1, 8, 128, T_k] = [1, 8, T_q, T_k]
        let matrix_ac = q_u.matmul(&k.transpose_axes(&[0, 2, 3, 1])?)?;

        // Position score: [1, 8, T_q, 128] @ [1, 8, 128, T_pos] = [1, 8, T_q, T_pos]
        let matrix_bd = q_v.matmul(&p.transpose_axes(&[0, 2, 3, 1])?)?;

        // rel_shift on matrix_bd:
        // 1. Pad 1 column on left: [1, 8, T_q, T_pos + 1]
        let bd_padded = ops::pad(&matrix_bd, &[(0, 0), (0, 0), (0, 0), (1, 0)][..], None, None)?;
        // 2. Reshape to [1, 8, T_pos + 1, T_q]
        let bd_reshaped = bd_padded.reshape(&[1, 8, t_pos + 1, t_q])?;
        // 3. Drop first row: [1, 8, T_pos, T_q]
        let bd_sliced = bd_reshaped.index((.., .., 1.., ..));
        // 4. Reshape to [1, 8, T_q, T_pos]
        let bd_back = bd_sliced.reshape(&[1, 8, t_q, t_pos])?;
        // 5. Take first T_k columns: [1, 8, T_q, T_k]
        let bd_final = bd_back.index((.., .., .., ..t_k));

        // Total attention score scaled by 1/sqrt(128)
        let scale = Array::from_f32(1.0 / (128.0f32).sqrt());
        let scores = matrix_ac.add(&bd_final)?.multiply(&scale)?;

        // Apply attention mask if lookback cache is not fully saturated (< 70 frames)
        let masked_scores = if cache_len < 70 {
            let invalid_cols = (70 - cache_len) as i32;
            let neg_inf = ops::full::<f32>(&[1, 8, t_q, invalid_cols], &Array::from_f32(-10000.0))?;
            let valid_part = scores.index((.., .., .., invalid_cols..));
            ops::concatenate_axis(&[&neg_inf, &valid_part], -1)?
        } else {
            scores
        };

        let attn_weights = ops::softmax_axis(&masked_scores, -1, None)?;
        // [1, 8, T_q, T_k] @ [1, 8, T_k, 128] = [1, 8, T_q, 128]
        let v_trans = v.transpose_axes(&[0, 2, 1, 3])?;
        let context = attn_weights.matmul(&v_trans)?;

        // Transpose to [1, T_q, 8, 128] -> reshape to [1, T_q, 1024] -> linear_out
        let out = context
            .transpose_axes(&[0, 2, 1, 3])?
            .reshape(&[1, t_q, 1024])?
            .matmul(&self.linear_out.transpose_axes(&[1, 0])?)?;

        Ok((out, next_cache))
    }
}

pub struct ConformerConv {
    norm: LayerNorm,
    pw1_weight: Array, // [2048, 1024]
    dw_weight: Array,  // [1024, 9, 1]
    batch_norm: LayerNorm,
    pw2_weight: Array, // [1024, 1024]
}

impl ConformerConv {
    pub fn load(w: &Weights, layer_idx: usize) -> Result<Self, Exception> {
        let get = |k: &str| -> Result<&Array, Exception> {
            w.get(k).ok_or_else(|| Exception::custom(format!("missing tensor: {k}")))
        };

        let pfx = format!("encoder.layers.{layer_idx}");
        let norm_w = get(&format!("{pfx}.norm_conv.weight"))?.clone();
        let norm_b = get(&format!("{pfx}.norm_conv.bias"))?.clone();

        // Pointwise conv 1: [2048, 1024, 1] -> squeeze to [2048, 1024]
        let pw1_raw = get(&format!("{pfx}.conv.pointwise_conv1.weight"))?;
        let pw1_weight = pw1_raw.reshape(&[2048, 1024])?;

        // Depthwise conv: [1024, 1, 9] -> MLX conv1d expects [1024, 9, 1]
        let dw_raw = get(&format!("{pfx}.conv.depthwise_conv.weight"))?;
        let dw_weight = dw_raw.transpose_axes(&[0, 2, 1])?;

        // Batch norm in ONNX is a LayerNorm across channels [1024]
        let bn_w = get(&format!("{pfx}.conv.batch_norm.weight"))?.clone();
        let bn_b = get(&format!("{pfx}.conv.batch_norm.bias"))?.clone();

        // Pointwise conv 2: [1024, 1024, 1] -> squeeze to [1024, 1024]
        let pw2_raw = get(&format!("{pfx}.conv.pointwise_conv2.weight"))?;
        let pw2_weight = pw2_raw.reshape(&[1024, 1024])?;

        Ok(Self {
            norm: LayerNorm::new(norm_w, norm_b),
            pw1_weight,
            dw_weight,
            batch_norm: LayerNorm::new(bn_w, bn_b),
            pw2_weight,
        })
    }

    /// Forward pass:
    /// - `x`: [1, T_q, 1024]
    /// - `cache_time`: [1, 8, 1024]
    /// Returns: (conv_out [1, T_q, 1024], new_cache_time [1, 8, 1024])
    pub fn forward(&self, x: &Array, cache_time: &Array) -> Result<(Array, Array), Exception> {
        let x_norm = self.norm.forward(x)?;

        // Pointwise Conv 1 + Gated Linear Unit (GLU)
        let glu = x_norm.matmul(&self.pw1_weight.transpose_axes(&[1, 0])?)?; // [1, T_q, 2048]
        let parts = ops::split(&glu, 2, -1)?;
        let glu_out = parts[0].multiply(&ops::sigmoid(&parts[1])?)?; // [1, T_q, 1024]

        // Concatenate 8 frames from cache along time (axis 1): [1, 8 + T_q, 1024]
        let dw_in = ops::concatenate_axis(&[cache_time, &glu_out], 1)?;
        let t_dw = dw_in.shape()[1];

        // Next cache is the last 8 frames
        let next_cache = dw_in.index((.., (t_dw - 8).., ..));

        // Depthwise Conv1d (groups=1024, kernel=9)
        let dw_out = ops::conv1d(&dw_in, &self.dw_weight, 1, 0, 1, 1024)?; // [1, T_q, 1024]

        // Batch norm + SiLU
        let bn_out = self.batch_norm.forward(&dw_out)?;
        let bn_act = silu(&bn_out)?;

        // Pointwise Conv 2
        let conv_out = bn_act.matmul(&self.pw2_weight.transpose_axes(&[1, 0])?)?;

        Ok((conv_out, next_cache))
    }
}

pub struct ConformerBlock {
    ff1: FeedForward,
    attn: RelPosSelfAttention,
    conv: ConformerConv,
    ff2: FeedForward,
    norm_out: LayerNorm,
}

impl ConformerBlock {
    pub fn load(w: &Weights, layer_idx: usize) -> Result<Self, Exception> {
        let get = |k: &str| -> Result<&Array, Exception> {
            w.get(k).ok_or_else(|| Exception::custom(format!("missing tensor: {k}")))
        };

        let pfx = format!("encoder.layers.{layer_idx}");
        let ff1 = FeedForward::load(w, &format!("{pfx}.norm_feed_forward1"), &format!("{pfx}.feed_forward1"))?;
        let attn = RelPosSelfAttention::load(w, layer_idx)?;
        let conv = ConformerConv::load(w, layer_idx)?;
        let ff2 = FeedForward::load(w, &format!("{pfx}.norm_feed_forward2"), &format!("{pfx}.feed_forward2"))?;

        let norm_out_w = get(&format!("{pfx}.norm_out.weight"))?.clone();
        let norm_out_b = get(&format!("{pfx}.norm_out.bias"))?.clone();
        let norm_out = LayerNorm::new(norm_out_w, norm_out_b);

        Ok(Self {
            ff1,
            attn,
            conv,
            ff2,
            norm_out,
        })
    }

    pub fn forward(
        &self,
        x: &Array,
        cache_channel: &Array,
        cache_time: &Array,
        cache_len: i32,
        pos_slice: &Array,
    ) -> Result<(Array, Array, Array), Exception> {
        let half = Array::from_f32(0.5);

        // FF1 with 0.5 residual
        let ff1_out = self.ff1.forward(x)?;
        let x = x.add(&ff1_out.multiply(&half)?)?;

        // Self Attn
        let (attn_out, next_cache_ch) = self.attn.forward(&x, cache_channel, cache_len, pos_slice)?;
        let x = x.add(&attn_out)?;

        // Conv
        let (conv_out, next_cache_tm) = self.conv.forward(&x, cache_time)?;
        let x = x.add(&conv_out)?;

        // FF2 with 0.5 residual
        let ff2_out = self.ff2.forward(&x)?;
        let x = x.add(&ff2_out.multiply(&half)?)?;

        // Out Norm
        let out = self.norm_out.forward(&x)?;

        Ok((out, next_cache_ch, next_cache_tm))
    }
}

pub struct FastConformerEncoder {
    layers: Vec<ConformerBlock>,
    pos_emb: Array, // [1, 9999, 1024]
}

impl FastConformerEncoder {
    pub fn load(w: &Weights) -> Result<Self, Exception> {
        let mut layers = Vec::with_capacity(24);
        for i in 0..24 {
            layers.push(ConformerBlock::load(w, i)?);
        }

        let pos_emb = w
            .get("encoder.pos_enc.pos_emb")
            .ok_or_else(|| Exception::custom("missing encoder.pos_enc.pos_emb"))?
            .clone();

        Ok(Self { layers, pos_emb })
    }

    /// Forward pass through all 24 Conformer layers:
    /// - `x`: [1, T_q, 1024]
    /// - `caches_channel`: Vec of 24 [1, 70, 1024]
    /// - `caches_time`: Vec of 24 [1, 8, 1024]
    /// - `cache_len`: current valid history length (0..=70)
    /// Returns:
    /// - `encoded`: [1, 1024, T_q] (transposed channels-first for RNN-T decoder)
    /// - `new_caches_channel`: Vec of 24 [1, 70, 1024]
    /// - `new_caches_time`: Vec of 24 [1, 8, 1024]
    pub fn forward(
        &self,
        x: &Array,
        caches_channel: &[Array],
        caches_time: &[Array],
        cache_len: i32,
    ) -> Result<(Array, Vec<Array>, Vec<Array>), Exception> {
        let t_q = x.shape()[1];
        let t_k = 70 + t_q; // 77

        // Slice positional embeddings from 5000 - T_k to 5000 + T_k - 1
        let start_idx = 5000 - t_k;
        let end_idx = 5000 + t_k - 1;
        let pos_slice = self.pos_emb.index((.., start_idx..end_idx, ..));

        let mut curr_x = x.clone();
        let mut new_caches_channel = Vec::with_capacity(24);
        let mut new_caches_time = Vec::with_capacity(24);

        for i in 0..24 {
            let (next_x, next_ch, next_tm) = self.layers[i].forward(
                &curr_x,
                &caches_channel[i],
                &caches_time[i],
                cache_len,
                &pos_slice,
            )?;
            curr_x = next_x;
            new_caches_channel.push(next_ch);
            new_caches_time.push(next_tm);
        }

        // Transpose [1, T_q, 1024] -> [1, 1024, T_q]
        let encoded = curr_x.transpose_axes(&[0, 2, 1])?;
        Ok((encoded, new_caches_channel, new_caches_time))
    }
}
