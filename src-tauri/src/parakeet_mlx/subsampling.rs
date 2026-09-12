//! MLX 8x Depthwise-Separable Convolutional Subsampling for Parakeet Nemotron.
//!
//! Downsamples 128 mel frequency bins by 8x along the time and frequency dimensions
//! into 17 frequency bins (4352 features) and projects to 1024 hidden dimensions.

use mlx_rs::ops::indexing::IndexOp;
use mlx_rs::{error::Exception, ops, Array};
use std::collections::HashMap;

pub type Weights = HashMap<String, Array>;

pub struct ConvSubsampling {
    w0: Array, // [256, 3, 3, 1]
    b0: Array, // [256]
    w2: Array, // [256, 3, 3, 1] groups=256
    b2: Array, // [256]
    w3: Array, // [256, 1, 1, 256]
    b3: Array, // [256]
    w5: Array, // [256, 3, 3, 1] groups=256
    b5: Array, // [256]
    w6: Array, // [256, 1, 1, 256]
    b6: Array, // [256]
    w_out: Array, // [1024, 4352]
    b_out: Array, // [1024]
}

impl ConvSubsampling {
    pub fn load(w: &Weights) -> Result<Self, Exception> {
        let get = |k: &str| -> Result<&Array, Exception> {
            w.get(k).ok_or_else(|| Exception::custom(format!("missing tensor: {k}")))
        };

        // In ONNX weights are [C_out, C_in, kH, kW].
        // In MLX conv2d, weights must be [C_out, kH, kW, C_in] (transpose perm: [0, 2, 3, 1]).
        let w0 = get("encoder.pre_encode.conv.0.weight")?.transpose_axes(&[0, 2, 3, 1])?;
        let b0 = get("encoder.pre_encode.conv.0.bias")?.clone();

        let w2 = get("encoder.pre_encode.conv.2.weight")?.transpose_axes(&[0, 2, 3, 1])?;
        let b2 = get("encoder.pre_encode.conv.2.bias")?.clone();

        let w3 = get("encoder.pre_encode.conv.3.weight")?.transpose_axes(&[0, 2, 3, 1])?;
        let b3 = get("encoder.pre_encode.conv.3.bias")?.clone();

        let w5 = get("encoder.pre_encode.conv.5.weight")?.transpose_axes(&[0, 2, 3, 1])?;
        let b5 = get("encoder.pre_encode.conv.5.bias")?.clone();

        let w6 = get("encoder.pre_encode.conv.6.weight")?.transpose_axes(&[0, 2, 3, 1])?;
        let b6 = get("encoder.pre_encode.conv.6.bias")?.clone();

        let w_out = get("encoder.pre_encode.out.weight")?.clone();
        let b_out = get("encoder.pre_encode.out.bias")?.clone();

        Ok(Self {
            w0,
            b0,
            w2,
            b2,
            w3,
            b3,
            w5,
            b5,
            w6,
            b6,
            w_out,
            b_out,
        })
    }

    /// Pad helper: top=2, bottom=1, left=2, right=1 (axes: batch=0, time=1, freq=2, channel=3)
    fn pad_2x2(&self, x: &Array) -> Result<Array, Exception> {
        ops::pad(x, &[(0, 0), (2, 1), (2, 1), (0, 0)][..], None, None)
    }

    /// Forward pass:
    /// `mel`: [1, 128, T] -> returns `x`: [1, T_out, 1024]
    pub fn forward(&self, mel: &Array) -> Result<Array, Exception> {
        // [1, 128, T] -> transpose to [1, T, 128] -> reshape to [1, T, 128, 1]
        let shape = mel.shape();
        let t_len = shape[2];
        let x_in = mel
            .transpose_axes(&[0, 2, 1])?
            .reshape(&[1, t_len, 128, 1])?;

        // conv.0 (stride 2, pad 2x2) -> ReLU
        let x = self.pad_2x2(&x_in)?;
        let x = ops::conv2d(&x, &self.w0, (2, 2), (0, 0), (1, 1), 1)?.add(&self.b0)?;
        let x = ops::maximum(&x, &Array::from_f32(0.0))?;

        // conv.2 (depthwise stride 2, pad 2x2, groups 256)
        let x = self.pad_2x2(&x)?;
        let x = ops::conv2d(&x, &self.w2, (2, 2), (0, 0), (1, 1), 256)?.add(&self.b2)?;

        // conv.3 (pointwise stride 1, pad 0) -> ReLU
        let x = ops::conv2d(&x, &self.w3, (1, 1), (0, 0), (1, 1), 1)?.add(&self.b3)?;
        let x = ops::maximum(&x, &Array::from_f32(0.0))?;

        // conv.5 (depthwise stride 2, pad 2x2, groups 256)
        let x = self.pad_2x2(&x)?;
        let x = ops::conv2d(&x, &self.w5, (2, 2), (0, 0), (1, 1), 256)?.add(&self.b5)?;

        // conv.6 (pointwise stride 1, pad 0) -> ReLU
        let x = ops::conv2d(&x, &self.w6, (1, 1), (0, 0), (1, 1), 1)?.add(&self.b6)?;
        let x = ops::maximum(&x, &Array::from_f32(0.0))?;

        // x is [1, T_sub, 17, 256] in NHWC format
        // Transpose [0, 1, 3, 2] to [1, T_sub, 256, 17], then flatten to [1, T_sub, 4352]
        let x_trans = x.transpose_axes(&[0, 1, 3, 2])?;
        let sub_shape = x_trans.shape();
        let t_sub = sub_shape[1];
        let x_flat = x_trans.reshape(&[1, t_sub, 4352])?;

        // Out linear projection: [1, T_sub, 4352] @ [4352, 1024] + [1024]
        let projected = x_flat
            .matmul(&self.w_out.transpose_axes(&[1, 0])?)?
            .add(&self.b_out)?;

        // In streaming inference, the first 2 frames are overlap/context from the chunk boundary
        // Slice [:, 2:, :]
        let sliced = projected.index((.., 2.., ..));
        Ok(sliced)
    }
}
