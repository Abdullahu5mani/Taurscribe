//! MLX RNN-T Decoder & Joint Network for Parakeet Nemotron.
//!
//! Implements the 2-layer LSTM predictor network, joint projection network,
//! and greedy token emission on Apple Silicon Metal GPU.

use std::collections::HashMap;
use mlx_rs::ops::indexing::IndexOp;
use mlx_rs::{error::Exception, ops, Array};

pub type Weights = HashMap<String, Array>;

pub fn get_tensor<'a>(w: &'a Weights, key: &str) -> Result<&'a Array, Exception> {
    w.get(key)
        .ok_or_else(|| Exception::custom(format!("missing tensor: {key}")))
}

/// Single LSTM Cell on Metal GPU using ONNX-compatible gate order [i, o, f, c].
pub struct LstmCell {
    weight_ih: Array, // [4*H, in_dim]
    weight_hh: Array, // [4*H, H]
    bias_ih: Array,   // [4*H]
    bias_hh: Array,   // [4*H]
    hidden_dim: i32,
}

impl LstmCell {
    pub fn new(w: &Weights, layer_idx: usize, in_dim: i32, hidden_dim: i32) -> Result<Self, Exception> {
        let w_ih_raw = get_tensor(w, &format!("decoder.lstm.{layer_idx}.weight_ih"))?;
        let w_hh_raw = get_tensor(w, &format!("decoder.lstm.{layer_idx}.weight_hh"))?;
        let b_raw = get_tensor(w, &format!("decoder.lstm.{layer_idx}.bias"))?;

        // Squeeze outer batch dimension from ONNX shape [1, 4*H, dim] -> [4*H, dim]
        let weight_ih = w_ih_raw.reshape(&[4 * hidden_dim, in_dim])?;
        let weight_hh = w_hh_raw.reshape(&[4 * hidden_dim, hidden_dim])?;

        // Bias in ONNX is [1, 8*H] containing [W_b, R_b]
        let b_flat = b_raw.reshape(&[8 * hidden_dim])?;
        let bias_ih = b_flat.index((0..(4 * hidden_dim),));
        let bias_hh = b_flat.index(((4 * hidden_dim)..(8 * hidden_dim),));

        Ok(Self {
            weight_ih,
            weight_hh,
            bias_ih,
            bias_hh,
            hidden_dim,
        })
    }

    /// Single time-step LSTM forward: x [1, in_dim], h_prev [1, H], c_prev [1, H]
    /// Returns: (h_t [1, H], c_t [1, H])
    pub fn step(&self, x: &Array, h_prev: &Array, c_prev: &Array) -> Result<(Array, Array), Exception> {
        let x_proj = x.matmul(&self.weight_ih.transpose_axes(&[1, 0])?)?.add(&self.bias_ih)?;
        let h_proj = h_prev.matmul(&self.weight_hh.transpose_axes(&[1, 0])?)?.add(&self.bias_hh)?;
        let gates = x_proj.add(&h_proj)?; // [1, 4*H]

        let h = self.hidden_dim;
        let i_gate = ops::sigmoid(&gates.index((.., 0..h)))?;
        let o_gate = ops::sigmoid(&gates.index((.., h..(2 * h))))?;
        let f_gate = ops::sigmoid(&gates.index((.., (2 * h)..(3 * h))))?;
        let c_gate = ops::tanh(&gates.index((.., (3 * h)..(4 * h))))?;

        // c_t = f * c_prev + i * c_gate
        let c_t = f_gate.multiply(c_prev)?.add(&i_gate.multiply(&c_gate)?)?;
        // h_t = o * tanh(c_t)
        let h_t = o_gate.multiply(&ops::tanh(&c_t)?)?;

        Ok((h_t, c_t))
    }
}

/// 2-Layer LSTM Predictor Network
pub struct PredictorNetwork {
    embed: Array, // [1025, 640]
    lstm0: LstmCell,
    lstm1: LstmCell,
}

impl PredictorNetwork {
    pub fn load(w: &Weights) -> Result<Self, Exception> {
        let embed = get_tensor(w, "decoder.prediction.embed.weight")?.clone();
        let lstm0 = LstmCell::new(w, 0, 640, 640)?;
        let lstm1 = LstmCell::new(w, 1, 640, 640)?;

        Ok(Self {
            embed,
            lstm0,
            lstm1,
        })
    }

    /// Run predictor step: token_id -> (pred_vector [1, 640], new_h0, new_c0, new_h1, new_c1)
    pub fn step(
        &self,
        token_id: i32,
        h0: &Array,
        c0: &Array,
        h1: &Array,
        c1: &Array,
    ) -> Result<(Array, Array, Array, Array, Array), Exception> {
        let x = self.embed.index((token_id..(token_id + 1), ..)); // [1, 640]

        let (new_h0, new_c0) = self.lstm0.step(&x, h0, c0)?;
        let (new_h1, new_c1) = self.lstm1.step(&new_h0, h1, c1)?;

        Ok((new_h1.clone(), new_h0, new_c0, new_h1, new_c1))
    }
}

/// Joint Projection Network combining Encoder + Predictor
pub struct JointNetwork {
    enc_weight: Array,   // [640, 1024]
    enc_bias: Array,     // [640]
    pred_weight: Array,  // [640, 640]
    pred_bias: Array,    // [640]
    joint2_weight: Array,// [1025, 640]
    joint2_bias: Array,  // [1025]
}

impl JointNetwork {
    pub fn load(w: &Weights) -> Result<Self, Exception> {
        Ok(Self {
            enc_weight: get_tensor(w, "joint.enc.weight")?.clone(),
            enc_bias: get_tensor(w, "joint.enc.bias")?.clone(),
            pred_weight: get_tensor(w, "joint.pred.weight")?.clone(),
            pred_bias: get_tensor(w, "joint.pred.bias")?.clone(),
            joint2_weight: get_tensor(w, "joint.joint_net.2.weight")?.clone(),
            joint2_bias: get_tensor(w, "joint.joint_net.2.bias")?.clone(),
        })
    }

    /// enc_out: [1, 1024], pred_out: [1, 640] -> logits: [1, 1025]
    pub fn forward(&self, enc_out: &Array, pred_out: &Array) -> Result<Array, Exception> {
        let enc_proj = enc_out.matmul(&self.enc_weight.transpose_axes(&[1, 0])?)?.add(&self.enc_bias)?;
        let pred_proj = pred_out.matmul(&self.pred_weight.transpose_axes(&[1, 0])?)?.add(&self.pred_bias)?;

        let sum = enc_proj.add(&pred_proj)?;
        let activated = ops::maximum(&sum, &Array::from_f32(0.0))?; // ReLU

        let logits = activated.matmul(&self.joint2_weight.transpose_axes(&[1, 0])?)?.add(&self.joint2_bias)?;
        Ok(logits)
    }
}
