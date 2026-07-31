//! Native MLX backend for Granite Speech NAR on Apple silicon.
//!
//! The ONNX path runs this model on CPU (RTF ~1.2). The MLX pipeline measured
//! roughly 11x faster at better WER, so on macOS we drive MLX directly rather
//! than going through ONNX Runtime.
//!
//! This is a port of the validated `scripts/granite_mlx/granite_nar_mlx.py`.
//! Keep the two in step: `granite_mlx_parity` compares them stage by stage.

pub mod editor;
pub mod encoder;
pub mod pipeline;
pub mod projector;

use std::collections::HashMap;
use std::path::Path;

use mlx_rs::{error::Exception, ops, Array, Dtype};

pub use editor::{Editor, EditorConfig};
pub use pipeline::GraniteMlx;
pub use encoder::{CtcEncoder, EncoderConfig, Weights};
pub use projector::{Projector, ProjectorConfig};

/// Reads `model.safetensors` and remaps it onto the layout this port expects.
///
/// Three things change versus the checkpoint on disk, all of them once, here:
///   * tensors are renamed onto this module tree,
///   * the 1x1 convs lose their trailing kernel axis so they can run as Linear,
///   * BatchNorm is folded into the preceding bias-free depthwise conv.
pub fn load_weights(
    model_dir: &Path,
    cfg: &EncoderConfig,
    proj: &ProjectorConfig,
    editor: &EditorConfig,
    dtype: Dtype,
) -> Result<Weights, String> {
    let path = model_dir.join("model.safetensors");
    if !path.exists() {
        return Err(format!("missing {}", path.display()));
    }
    let src = Array::load_safetensors(path.to_string_lossy().as_ref())
        .map_err(|e| format!("load safetensors: {e}"))?;

    let mut out: Weights = HashMap::new();
    let take = |w: &HashMap<String, Array>, k: &str| -> Result<Array, String> {
        w.get(k).cloned().ok_or_else(|| format!("missing tensor {k}"))
    };

    for name in ["input_linear", "out", "out_mid", "out_bpe"] {
        for part in ["weight", "bias"] {
            let key = format!("encoder.{name}.{part}");
            out.insert(key.clone(), take(&src, &key)?);
        }
    }

    for i in 0..cfg.num_layers {
        let p = format!("encoder.layers.{i}");

        for ff in ["ff1", "ff2"] {
            for part in ["pre_norm", "up_proj", "down_proj"] {
                for kind in ["weight", "bias"] {
                    let key = format!("{p}.{ff}.{part}.{kind}");
                    out.insert(key.clone(), take(&src, &key)?);
                }
            }
        }

        for key in [
            format!("{p}.attn.pre_norm.weight"),
            format!("{p}.attn.pre_norm.bias"),
            format!("{p}.attn.to_q.weight"),
            format!("{p}.attn.to_kv.weight"),
            format!("{p}.attn.to_out.weight"),
            format!("{p}.attn.to_out.bias"),
            format!("{p}.attn.rel_pos_emb.weight"),
            format!("{p}.conv.norm.weight"),
            format!("{p}.conv.norm.bias"),
            format!("{p}.conv.up_conv.bias"),
            format!("{p}.conv.down_conv.bias"),
            format!("{p}.post_norm.weight"),
            format!("{p}.post_norm.bias"),
        ] {
            let v = take(&src, &key)?;
            out.insert(key, v);
        }

        // 1x1 convs become Linear: drop the trailing kernel axis.
        for name in ["up_conv", "down_conv"] {
            let key = format!("{p}.conv.{name}.weight");
            let w = take(&src, &key)?;
            let squeezed = w
                .squeeze_axes(&[-1])
                .map_err(|e| format!("squeeze {key}: {e}"))?;
            out.insert(key, squeezed);
        }

        let (folded_w, folded_b) = fold_batch_norm(&src, &format!("{p}.conv"))?;
        out.insert(format!("{p}.conv.depth_conv.weight"), folded_w);
        out.insert(format!("{p}.conv.depth_conv.bias"), folded_b);
    }

    // ---- projector ----
    for i in 0..proj.num_encoder_layers {
        for part in ["weight", "bias"] {
            let key = format!("projector.layer_norms.{i}.{part}");
            out.insert(key.clone(), take(&src, &key)?);
        }
    }
    for name in ["layer_projector", "out_norm", "out_linear"] {
        for part in ["weight", "bias"] {
            let key = format!("projector.{name}.{part}");
            out.insert(key.clone(), take(&src, &key)?);
        }
    }
    for key in ["projector.query", "projector.window_positions"] {
        out.insert(key.to_string(), take(&src, key)?);
    }
    for i in 0..proj.num_layers {
        let s_ = format!("projector.qformer.layers.{i}");
        let d = format!("projector.layers.{i}");
        for part in ["attn_norm", "mlp_norm"] {
            for kind in ["weight", "bias"] {
                out.insert(
                    format!("{d}.{part}.{kind}"),
                    take(&src, &format!("{s_}.{part}.{kind}"))?,
                );
            }
        }
        for part in ["q_proj", "k_proj", "v_proj", "o_proj"] {
            out.insert(
                format!("{d}.{part}.weight"),
                take(&src, &format!("{s_}.cross_attention.{part}.weight"))?,
            );
            if let Some(b) = src.get(&format!("{s_}.cross_attention.{part}.bias")) {
                out.insert(format!("{d}.{part}.bias"), b.clone());
            }
        }
        for part in ["fc1", "fc2"] {
            out.insert(
                format!("{d}.{part}.weight"),
                take(&src, &format!("{s_}.mlp.{part}.weight"))?,
            );
            if let Some(b) = src.get(&format!("{s_}.mlp.{part}.bias")) {
                out.insert(format!("{d}.{part}.bias"), b.clone());
            }
        }
    }

    // ---- editor ----
    out.insert(
        "editor.embed_tokens.weight".into(),
        take(&src, "language_model.model.embed_tokens.weight")?,
    );
    if let Some(head) = src.get("language_model.lm_head.weight") {
        out.insert("editor.lm_head.weight".into(), head.clone());
    }
    out.insert(
        "editor.norm_w".into(),
        take(&src, "language_model.model.norm.weight")?,
    );
    for i in 0..editor.num_layers {
        let s_ = format!("language_model.model.layers.{i}");
        let d = format!("editor.layers.{i}");
        out.insert(
            format!("{d}.input_layernorm_w"),
            take(&src, &format!("{s_}.input_layernorm.weight"))?,
        );
        out.insert(
            format!("{d}.post_attention_layernorm_w"),
            take(&src, &format!("{s_}.post_attention_layernorm.weight"))?,
        );
        for part in ["q_proj", "k_proj", "v_proj", "o_proj"] {
            out.insert(
                format!("{d}.{part}.weight"),
                take(&src, &format!("{s_}.self_attn.{part}.weight"))?,
            );
        }
        for part in ["gate_proj", "up_proj", "down_proj"] {
            out.insert(
                format!("{d}.{part}.weight"),
                take(&src, &format!("{s_}.mlp.{part}.weight"))?,
            );
        }
    }

    let cast = out
        .into_iter()
        .map(|(k, v)| match v.as_dtype(dtype) {
            Ok(v) => Ok((k, v)),
            Err(e) => Err(format!("cast {k}: {e}")),
        })
        .collect::<Result<Weights, String>>()?;
    Ok(cast)
}

/// Fold BatchNorm1d into the preceding bias-free depthwise conv:
///   y = gamma * (conv(x) - mean) / sqrt(var + eps) + beta
///     = conv_scaled(x) + (beta - gamma * mean / sqrt(var + eps))
fn fold_batch_norm(src: &HashMap<String, Array>, prefix: &str) -> Result<(Array, Array), String> {
    let eps = 1e-5_f32;
    let f32_of = |key: &str| -> Result<Array, String> {
        src.get(key)
            .ok_or_else(|| format!("missing tensor {key}"))?
            .as_dtype(Dtype::Float32)
            .map_err(|e| format!("cast {key}: {e}"))
    };

    let gamma = f32_of(&format!("{prefix}.batch_norm.weight"))?;
    let beta = f32_of(&format!("{prefix}.batch_norm.bias"))?;
    let mean = f32_of(&format!("{prefix}.batch_norm.running_mean"))?;
    let var = f32_of(&format!("{prefix}.batch_norm.running_var"))?;
    let conv = f32_of(&format!("{prefix}.depth_conv.conv.weight"))?;

    let inv_std = gamma
        .multiply(ops::rsqrt(&var.add(Array::from_f32(eps)).map_err(err)?).map_err(err)?)
        .map_err(err)?;

    // PyTorch depthwise weight is (channels, 1, kernel); MLX Conv1d wants
    // (channels, kernel, 1).
    let channels = inv_std.shape()[0];
    let scaled = conv
        .multiply(&inv_std.reshape(&[channels, 1, 1]).map_err(err)?)
        .map_err(err)?;
    let folded = scaled.transpose_axes(&[0, 2, 1]).map_err(err)?;
    let bias = beta
        .subtract(&mean.multiply(&inv_std).map_err(err)?)
        .map_err(err)?;
    Ok((folded, bias))
}

fn err(e: Exception) -> String {
    e.to_string()
}

/// Which encoder layers the projector consumes, resolved against the upstream
/// hidden-state tuple whose entry 0 is the pre-layer input.
pub fn resolve_layer_indices(raw: &[i64], num_layers: usize) -> Vec<usize> {
    raw.iter()
        .map(|&i| {
            if i >= 0 {
                i as usize
            } else {
                num_layers + 1 - i.unsigned_abs() as usize
            }
        })
        .collect()
}
