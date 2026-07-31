//! End-to-end Granite NAR transcription on MLX.
//!
//! Ties the three ported stages together with the host-side glue: CTC collapse,
//! insertion slots, and the audio/text embedding concat. Mirrors
//! `transcribe_ids` in the Python reference.

use std::path::Path;

use mlx_rs::ops::indexing::IndexOp;
use mlx_rs::{error::Exception, ops, Array, Dtype};

use super::editor::{add_insertion_slots, argmax_rows, ctc_greedy, Editor, EditorConfig};
use super::encoder::{CtcEncoder, EncoderConfig};
use super::projector::{Projector, ProjectorConfig};
use super::{load_weights, resolve_layer_indices};

pub struct GraniteMlx {
    encoder: CtcEncoder,
    projector: Projector,
    editor: Editor,
    layer_indices: Vec<usize>,
    downsample_rate: i32,
    embedding_multiplier: f32,
    scale_projected: bool,
    blank_token_id: i64,
    min_edit_len: usize,
    dtype: Dtype,
}

impl GraniteMlx {
    pub fn load(model_dir: &Path, dtype: Dtype) -> Result<Self, String> {
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(model_dir.join("config.json")).map_err(
                |e| format!("read config.json: {e}"),
            )?)
            .map_err(|e| format!("parse config.json: {e}"))?;

        let enc_cfg = EncoderConfig::from_json(&config)?;
        let proj_cfg = ProjectorConfig::from_json(&config)?;
        let ed_cfg = EditorConfig::from_json(&config)?;

        let weights = load_weights(model_dir, &enc_cfg, &proj_cfg, &ed_cfg, dtype)?;
        let raw: Vec<i64> = config["encoder_layer_indices"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
            .unwrap_or_else(|| vec![4, 8, 12, -1]);
        let layer_indices = resolve_layer_indices(&raw, enc_cfg.num_layers);

        let to_str = |e: Exception| e.to_string();
        Ok(Self {
            encoder: CtcEncoder::load(&weights, enc_cfg).map_err(to_str)?,
            projector: Projector::load(&weights, proj_cfg.clone()).map_err(to_str)?,
            editor: Editor::load(&weights, ed_cfg.clone()).map_err(to_str)?,
            layer_indices,
            downsample_rate: proj_cfg.downsample_rate,
            embedding_multiplier: ed_cfg.embedding_multiplier,
            scale_projected: config["scale_projected_embeddings"]
                .as_bool()
                .unwrap_or(true),
            blank_token_id: config["blank_token_id"].as_i64().unwrap_or(100_257),
            min_edit_len: config["min_edit_sequence_length"].as_i64().unwrap_or(8) as usize,
            dtype,
        })
    }

    /// `features` is (frames, 160) log-mel, laid out row-major.
    pub fn transcribe_features(&self, features: &[f32], frames: usize) -> Result<Vec<u32>, String> {
        self.run(features, frames).map_err(|e| e.to_string())
    }

    fn run(&self, features: &[f32], frames: usize) -> Result<Vec<u32>, Exception> {
        let feats = Array::from_slice(features, &[1, frames as i32, 160]).as_dtype(self.dtype)?;

        let (bpe_logits, hidden) = self.encoder.forward(&feats, &self.layer_indices)?;
        let ctc_ids = ctc_greedy(&argmax_rows(&bpe_logits.index(0))?, self.blank_token_id);

        let multilayer = ops::concatenate_axis(&hidden, -1)?;
        let audio = self.projector.forward(&multilayer)?;
        let audio = if self.scale_projected {
            audio.divide(Array::from_f32(self.embedding_multiplier))?
        } else {
            audio
        };
        // Trim the projector's padding tail back to the real audio length.
        let audio_len = (frames as i32) / self.downsample_rate;
        let audio = audio.index((.., ..audio_len));

        let slots = add_insertion_slots(&ctc_ids, self.blank_token_id, self.min_edit_len);
        let slot_arr = Array::from_slice(
            &slots.iter().map(|&v| v as i32).collect::<Vec<_>>(),
            &[slots.len() as i32],
        );
        let text = self
            .editor
            .embed(&slot_arr)
            .reshape(&[1, slots.len() as i32, -1])?;

        let embeds = ops::concatenate_axis(&[audio, text], 1)?;
        let logits = self.editor.forward(&embeds, audio_len)?;

        let final_ids = ctc_greedy(&argmax_rows(&logits.index(0))?, self.blank_token_id);
        Ok(final_ids
            .into_iter()
            .filter_map(|id| u32::try_from(id).ok())
            .collect())
    }
}
