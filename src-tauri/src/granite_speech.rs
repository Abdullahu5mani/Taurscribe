// granite_speech.rs — Granite 4.0 1B Speech ONNX orchestrator
//
// Manages three ONNX sessions (audio_encoder, embed_tokens, decoder_model_merged)
// and performs end-to-end speech-to-text inference with KV cache management.
//
// NOTE: We use ort's (shape, Vec<T>) tuple API for Value::from_array instead of
// ndarray arrays because ort 2.0.0-rc.11 re-exports its own ndarray version
// which is incompatible with the project's ndarray 0.15.
//
// KV cache is float32 throughout: the q4 ONNX models only quantize internal
// weight matrices; the past_key_values I/O tensors remain float32.

use ndarray::{Array2, Array3, Array4};
use ort::session::Session;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::borrow::Cow;
use std::path::PathBuf;

use crate::granite_features;

// ───────────────────────── Model Constants ────────────────────────────────────
const EOS_TOKEN_ID: i64 = 100257;
#[allow(dead_code)]
const PAD_TOKEN_ID: i64 = 100256;
const AUDIO_TOKEN_INDEX: i64 = 100352;
const NUM_HIDDEN_LAYERS: usize = 40;
const NUM_KV_HEADS: usize = 4;
const HEAD_DIM: usize = 128;
const MAX_NEW_TOKENS: usize = 448;
const HIDDEN_SIZE: usize = 2048;

// ───────────────────────── GPU Backend ────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub enum GpuBackend {
    Cuda,
    Cpu,
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Cuda => write!(f, "CUDA"),
            GpuBackend::Cpu => write!(f, "CPU"),
        }
    }
}

// ───────────────────────── Status ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraniteSpeechStatus {
    pub loaded: bool,
    pub model_id: Option<String>,
    pub backend: String,
}

// ───────────────────────── Helper: build ORT tensor ──────────────────────────
// Use (shape, Vec<T>) tuple form which always works regardless of ndarray version.

fn make_tensor_f32(shape: Vec<usize>, data: Vec<f32>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {}", e))
}

fn make_tensor_i64(shape: Vec<usize>, data: Vec<i64>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {}", e))
}

/// Create an empty (zero-length sequence) KV cache tensor: [1, num_heads, 0, head_dim].
///
/// ORT's (shape, Vec<T>) raw-data API rejects zero-sized dimensions.
/// ndarray::Array4 handles zero-length axes correctly.
fn make_empty_kv_f32(num_heads: usize, head_dim: usize) -> Result<ort::value::DynValue, String> {
    let arr = Array4::<f32>::from_shape_vec((1, num_heads, 0, head_dim), vec![])
        .map_err(|e| format!("Empty KV shape error: {}", e))?;
    ort::value::Value::from_array(arr)
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Empty KV tensor creation error: {}", e))
}


// ───────────────────────── Manager ────────────────────────────────────────────

pub struct GraniteSpeechManager {
    encoder_session: Option<Session>,
    embed_session: Option<Session>,
    decoder_session: Option<Session>,
    tokenizer: Option<tokenizers::Tokenizer>,
    backend: GpuBackend,
    model_name: Option<String>,
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>,
}

impl GraniteSpeechManager {
    pub fn new() -> Self {
        GraniteSpeechManager {
            encoder_session: None,
            embed_session: None,
            decoder_session: None,
            tokenizer: None,
            backend: GpuBackend::Cpu,
            model_name: None,
            resampler: None,
        }
    }

    pub fn get_status(&self) -> GraniteSpeechStatus {
        GraniteSpeechStatus {
            loaded: self.encoder_session.is_some(),
            model_id: self.model_name.clone(),
            backend: self.backend.to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn unload(&mut self) {
        if self.encoder_session.is_some() {
            println!("[GRANITE] Unloading Granite Speech model...");
            self.encoder_session = None;
            self.embed_session = None;
            self.decoder_session = None;
            self.tokenizer = None;
            self.model_name = None;
            self.resampler = None;
            println!("[GRANITE] Model unloaded");
        }
    }

    pub fn initialize(
        &mut self,
        model_path: Option<&str>,
        force_cpu: bool,
    ) -> Result<String, String> {
        let models_dir = crate::utils::get_models_dir()?;
        let model_dir = if let Some(p) = model_path {
            PathBuf::from(p)
        } else {
            let default_dir = models_dir.join("granite-speech-1b");
            if !default_dir.exists() {
                return Err(
                    "Granite Speech model not found. Please download it from Settings > Download Manager.".to_string()
                );
            }
            default_dir
        };

        if !model_dir.exists() {
            return Err(format!("Model directory not found: {}", model_dir.display()));
        }

        let mode_label = if force_cpu {
            " [CPU-only mode]"
        } else if cfg!(target_os = "macos") {
            " [XNNPACK/CPU]"
        } else {
            ""
        };
        println!("[GRANITE] Loading model from: {}{}", model_dir.display(), mode_label);

        // q4f16 variants: FP16 activations, faster on CUDA tensor cores (~1.5 GB)
        // q4 variants: FP32 I/O, runs on any hardware (~1.8 GB)
        let enc_q4f16 = model_dir.join("audio_encoder_q4f16.onnx");
        let emb_q4f16 = model_dir.join("embed_tokens_q4f16.onnx");
        let dec_q4f16 = model_dir.join("decoder_model_merged_q4f16.onnx");
        let encoder_path = model_dir.join("audio_encoder_q4.onnx");
        let embed_path = model_dir.join("embed_tokens_q4.onnx");
        let decoder_path = model_dir.join("decoder_model_merged_q4.onnx");
        let tokenizer_path = model_dir.join("tokenizer.json");

        let has_q4    = encoder_path.exists() && embed_path.exists() && decoder_path.exists();
        let has_q4f16 = enc_q4f16.exists()    && emb_q4f16.exists()  && dec_q4f16.exists();
        if !has_q4 && !has_q4f16 {
            return Err(
                "No Granite Speech model files found. Download the GPU or CPU variant from Settings > Models.".to_string()
            );
        }
        if !tokenizer_path.exists() {
            return Err(format!("Missing tokenizer.json (expected at {})", tokenizer_path.display()));
        }

        let (backend, encoder, embed, decoder) = if force_cpu || cfg!(target_os = "macos") {
            // Prefer q4 on CPU; fall back to q4f16 if only the GPU download is present.
            let (ep, eb, dp) = if has_q4 { (&encoder_path, &embed_path, &decoder_path) }
                               else       { (&enc_q4f16,   &emb_q4f16,   &dec_q4f16)   };
            let enc = self.create_session_cpu(ep)?;
            let emb = self.create_session_cpu(eb)?;
            let dec = self.create_session_cpu(dp)?;
            (GpuBackend::Cpu, enc, emb, dec)
        } else {
            // Prefer q4f16 on CUDA (FP16 activations, faster on tensor cores).
            // Fall back to q4 on CUDA, then to CPU with whatever files are present.
            let cuda_q4f16 = if has_q4f16 {
                self.try_create_sessions_cuda(&enc_q4f16, &emb_q4f16, &dec_q4f16)
                    .map(|r| { println!("[GRANITE] Using q4f16 on CUDA"); r })
                    .ok()
            } else {
                None
            };

            if let Some((enc, emb, dec)) = cuda_q4f16 {
                (GpuBackend::Cuda, enc, emb, dec)
            } else if has_q4 {
                match self.try_create_sessions_cuda(&encoder_path, &embed_path, &decoder_path) {
                    Ok((enc, emb, dec)) => (GpuBackend::Cuda, enc, emb, dec),
                    Err(e) => {
                        println!("[GRANITE] CUDA failed ({}), falling back to CPU...", e);
                        let enc = self.create_session_cpu(&encoder_path)?;
                        let emb = self.create_session_cpu(&embed_path)?;
                        let dec = self.create_session_cpu(&decoder_path)?;
                        (GpuBackend::Cpu, enc, emb, dec)
                    }
                }
            } else {
                // Only q4f16 available but CUDA failed — run q4f16 on CPU
                println!("[GRANITE] CUDA unavailable, running q4f16 on CPU");
                let enc = self.create_session_cpu(&enc_q4f16)?;
                let emb = self.create_session_cpu(&emb_q4f16)?;
                let dec = self.create_session_cpu(&dec_q4f16)?;
                (GpuBackend::Cpu, enc, emb, dec)
            }
        };

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

        self.encoder_session = Some(encoder);
        self.embed_session = Some(embed);
        self.decoder_session = Some(decoder);
        self.tokenizer = Some(tokenizer);
        self.backend = backend.clone();
        self.model_name = Some("granite-speech-1b".to_string());

        let msg = format!("[GRANITE] Model loaded ({})", backend);
        println!("{}", msg);
        Ok(msg)
    }

    pub fn transcribe_chunk(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<String, String> {
        let audio = if sample_rate != 16000 {
            self.resample(samples, sample_rate)?
        } else {
            samples.to_vec()
        };

        let start = std::time::Instant::now();

        let features = granite_features::extract_features(&audio);
        let n_frames = features.nrows();
        if n_frames == 0 {
            return Ok(String::new());
        }
        println!(
            "[GRANITE] Features: {} frames × {} dims ({:.0}ms)",
            n_frames, features.ncols(), start.elapsed().as_millis()
        );

        let audio_embeddings = self.run_audio_encoder(&features)?;
        println!(
            "[GRANITE] Audio encoded: {} × {} ({:.0}ms)",
            audio_embeddings.shape()[1], audio_embeddings.shape()[2], start.elapsed().as_millis()
        );

        let (first_token_id, kv_cache) = self.prefill(&audio_embeddings)?;
        println!(
            "[GRANITE] Prefill done, first token: {} ({:.0}ms)",
            first_token_id, start.elapsed().as_millis()
        );

        let all_tokens = self.decode_loop(first_token_id, kv_cache)?;

        let tokenizer = self.tokenizer.as_ref().ok_or("Tokenizer not loaded")?;
        let text = tokenizer
            .decode(&all_tokens.iter().map(|&t| t as u32).collect::<Vec<_>>(), true)
            .map_err(|e| format!("Token decode error: {}", e))?;

        let duration = start.elapsed();
        let audio_duration = audio.len() as f32 / 16000.0;
        let speedup = audio_duration / duration.as_secs_f32();
        println!(
            "[GRANITE] Transcribed {:.2}s audio in {:.0}ms | Speed: {:.1}x | \"{}\"",
            audio_duration, duration.as_millis(), speedup, text.trim()
        );

        Ok(text.trim().to_string())
    }

    // ─────────────────────── Session Creation ─────────────────────────────────

    fn try_create_sessions_cuda(
        &self,
        encoder_path: &std::path::Path,
        embed_path: &std::path::Path,
        decoder_path: &std::path::Path,
    ) -> Result<(Session, Session, Session), String> {
        println!("[GRANITE] Attempting CUDA acceleration...");

        let enc = Session::builder()
            .map_err(|e| format!("Session builder: {}", e))?
            .with_execution_providers([ort::execution_providers::CUDAExecutionProvider::default().build()])
            .map_err(|e| format!("CUDA EP: {}", e))?
            .commit_from_file(encoder_path)
            .map_err(|e| format!("Encoder load: {}", e))?;

        let emb = Session::builder()
            .map_err(|e| format!("Session builder: {}", e))?
            .with_execution_providers([ort::execution_providers::CUDAExecutionProvider::default().build()])
            .map_err(|e| format!("CUDA EP: {}", e))?
            .commit_from_file(embed_path)
            .map_err(|e| format!("Embed load: {}", e))?;

        let dec = Session::builder()
            .map_err(|e| format!("Session builder: {}", e))?
            .with_execution_providers([ort::execution_providers::CUDAExecutionProvider::default().build()])
            .map_err(|e| format!("CUDA EP: {}", e))?
            .commit_from_file(decoder_path)
            .map_err(|e| format!("Decoder load: {}", e))?;

        println!("[GRANITE] ✓ CUDA sessions created");
        Ok((enc, emb, dec))
    }

    fn create_session_cpu(&self, path: &std::path::Path) -> Result<Session, String> {
        Session::builder()
            .map_err(|e| format!("Session builder: {}", e))?
            .commit_from_file(path)
            .map_err(|e| format!("CPU session load: {}", e))
    }

    // ─────────────────────── Phase 2: Audio Encoder ──────────────────────────

    fn run_audio_encoder(&mut self, features: &Array2<f32>) -> Result<Array3<f32>, String> {
        let encoder = self.encoder_session.as_mut().ok_or("Encoder not loaded")?;

        let n_frames = features.nrows();
        let feat_dim = features.ncols();

        // Flatten ndarray to Vec and use (shape, Vec) tuple for ort
        let data: Vec<f32> = features.iter().cloned().collect();
        let input_tensor = make_tensor_f32(vec![1, n_frames, feat_dim], data)?;

        let outputs = encoder
            .run(ort::inputs!["input_features" => input_tensor])
            .map_err(|e| format!("Encoder run: {}", e))?;

        let output = outputs.values().next().ok_or("No encoder output")?;
        let (shape, data) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract encoder output: {}", e))?;

        // Shape derefs to [i64]
        let d0 = shape[0] as usize;
        let d1 = shape[1] as usize;
        let d2 = shape[2] as usize;

        let result = Array3::from_shape_vec((d0, d1, d2), data.to_vec())
            .map_err(|e| format!("Encoder output reshape: {}", e))?;

        Ok(result)
    }

    // ─────────────────────── Phase 3: Prefill ────────────────────────────────

    fn prefill(
        &mut self,
        audio_embeddings: &Array3<f32>,
    ) -> Result<(i64, Vec<Vec<f32>>), String> {
        let embed_session = self.embed_session.as_mut().ok_or("Embed session not loaded")?;
        let decoder = self.decoder_session.as_mut().ok_or("Decoder not loaded")?;
        let tokenizer = self.tokenizer.as_ref().ok_or("Tokenizer not loaded")?;

        // Chat template from tokenizer_config.json (granite-4.0-1b-speech):
        //   {% if role == 'user' %}USER: {{ content }}\n ASSISTANT:{% endif %}
        // The <|audio|> placeholder is placed inside the user content, before the question.
        let prompt = "USER: <|audio|>can you transcribe the speech into a written format?\n ASSISTANT:".to_string();
        let encoding = tokenizer
            .encode(prompt, false)
            .map_err(|e| format!("Tokenize error: {}", e))?;
        let prompt_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        println!("[GRANITE] Prompt tokens ({}): {:?}", prompt_ids.len(), &prompt_ids[..prompt_ids.len().min(10)]);

        // Embed the prompt tokens
        let token_tensor = make_tensor_i64(vec![1, prompt_ids.len()], prompt_ids.clone())?;

        let embed_outputs = embed_session
            .run(ort::inputs!["input_ids" => token_tensor])
            .map_err(|e| format!("Embed run: {}", e))?;

        let text_emb_val = embed_outputs.values().next().ok_or("No embed output")?;
        let (text_shape, text_data) = text_emb_val
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract text embeddings: {}", e))?;

        let text_seq_len = text_shape[1] as usize;
        let text_hidden = text_shape[2] as usize;

        // Build text embeddings as Array3 for indexing
        let text_emb = Array3::from_shape_vec((1, text_seq_len, text_hidden), text_data.to_vec())
            .map_err(|e| format!("Text emb reshape: {}", e))?;

        // Find the audio token position and replace with audio embeddings
        let audio_token_pos = prompt_ids.iter().position(|&id| id == AUDIO_TOKEN_INDEX);
        let audio_seq_len = audio_embeddings.shape()[1];

        let combined_embeddings = if let Some(pos) = audio_token_pos {
            let total_len = text_seq_len - 1 + audio_seq_len;
            let mut combined = Array3::<f32>::zeros((1, total_len, HIDDEN_SIZE));

            for t in 0..pos {
                for h in 0..HIDDEN_SIZE {
                    combined[[0, t, h]] = text_emb[[0, t, h]];
                }
            }
            for t in 0..audio_seq_len {
                for h in 0..HIDDEN_SIZE {
                    combined[[0, pos + t, h]] = audio_embeddings[[0, t, h]];
                }
            }
            for t in (pos + 1)..text_seq_len {
                let dst = pos + audio_seq_len + (t - pos - 1);
                for h in 0..HIDDEN_SIZE {
                    combined[[0, dst, h]] = text_emb[[0, t, h]];
                }
            }
            combined
        } else {
            let total_len = audio_seq_len + text_seq_len;
            let mut combined = Array3::<f32>::zeros((1, total_len, HIDDEN_SIZE));
            for t in 0..audio_seq_len {
                for h in 0..HIDDEN_SIZE {
                    combined[[0, t, h]] = audio_embeddings[[0, t, h]];
                }
            }
            for t in 0..text_seq_len {
                for h in 0..HIDDEN_SIZE {
                    combined[[0, audio_seq_len + t, h]] = text_emb[[0, t, h]];
                }
            }
            combined
        };

        let seq_len = combined_embeddings.shape()[1];
        println!("[GRANITE] Combined embeddings: 1 × {} × {}", seq_len, HIDDEN_SIZE);

        // Build decoder inputs — inputs_embeds is float32; KV cache is float16
        let embeds_data: Vec<f32> = combined_embeddings.iter().cloned().collect();
        let attn_data: Vec<i64> = vec![1i64; seq_len];

        let mut decoder_inputs: Vec<(String, ort::value::DynValue)> = Vec::new();

        decoder_inputs.push(("inputs_embeds".into(), make_tensor_f32(vec![1, seq_len, HIDDEN_SIZE], embeds_data)?));
        decoder_inputs.push(("attention_mask".into(), make_tensor_i64(vec![1, seq_len], attn_data)?));

        // Empty KV cache (past_sequence_length = 0) via ndarray::Array4 — ORT's raw-data API
        // rejects zero-sized dimensions, but ndarray handles them correctly.
        for layer in 0..NUM_HIDDEN_LAYERS {
            decoder_inputs.push((format!("past_key_values.{}.key", layer), make_empty_kv_f32(NUM_KV_HEADS, HEAD_DIM)?));
            decoder_inputs.push((format!("past_key_values.{}.value", layer), make_empty_kv_f32(NUM_KV_HEADS, HEAD_DIM)?));
        }

        let decoder_outputs = decoder
            .run(ort::session::SessionInputs::from(
                decoder_inputs.into_iter().map(|(k, v)| (Cow::<str>::from(k), v)).collect::<Vec<_>>(),
            ))
            .map_err(|e| format!("Decoder prefill run: {}", e))?;

        let (first_token, kv_cache) = extract_decoder_outputs(&decoder_outputs)?;
        Ok((first_token, kv_cache))
    }

    // ─────────────────────── Phase 4: Decode Loop ────────────────────────────

    fn decode_loop(
        &mut self,
        first_token_id: i64,
        initial_kv_cache: Vec<Vec<f32>>,
    ) -> Result<Vec<i64>, String> {
        let embed_session = self.embed_session.as_mut().ok_or("Embed not loaded")?;
        let decoder = self.decoder_session.as_mut().ok_or("Decoder not loaded")?;

        let mut generated_tokens = vec![first_token_id];
        let mut current_token = first_token_id;
        // KV cache stored as flat Vec<f32> per layer-kind
        let mut kv_cache = initial_kv_cache;
        // kv_cache_seq_len tracks how many tokens are in the KV cache
        let mut kv_cache_seq_len: usize = if !kv_cache.is_empty() {
            kv_cache[0].len() / (NUM_KV_HEADS * HEAD_DIM)
        } else {
            0
        };

        for step in 0..MAX_NEW_TOKENS {
            if current_token == EOS_TOKEN_ID {
                println!("[GRANITE] EOS reached at step {}", step);
                break;
            }

            // Repetition guard: stop if the last 6 tokens are all identical,
            // or if a 2-token pattern repeats 4 times (e.g. "as as as as as as").
            let n = generated_tokens.len();
            if n >= 6 {
                let tail = &generated_tokens[n - 6..];
                // All-same check
                if tail.iter().all(|&t| t == tail[0]) {
                    println!("[GRANITE] Repetition detected (all-same) at step {}, stopping", step);
                    generated_tokens.truncate(n - 5); // keep only the first of the run
                    break;
                }
                // Alternating-pair check (a b a b a b)
                if n >= 8 {
                    let t = &generated_tokens[n - 8..];
                    if t[0] == t[2] && t[2] == t[4] && t[4] == t[6]
                        && t[1] == t[3] && t[3] == t[5] && t[5] == t[7]
                    {
                        println!("[GRANITE] Repetition detected (bigram loop) at step {}, stopping", step);
                        generated_tokens.truncate(n - 6);
                        break;
                    }
                }
            }

            // Embed the single new token
            let token_tensor = make_tensor_i64(vec![1, 1], vec![current_token])?;

            let embed_outputs = embed_session
                .run(ort::inputs!["input_ids" => token_tensor])
                .map_err(|e| format!("Embed run: {}", e))?;

            let emb_val = embed_outputs.values().next().ok_or("No embed output")?;
            let (_emb_shape, emb_data) = emb_val
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("Extract embedding: {}", e))?;

            // Build decoder inputs — inputs_embeds is float32; KV cache is float16
            let mut decoder_inputs: Vec<(String, ort::value::DynValue)> = Vec::new();

            decoder_inputs.push(("inputs_embeds".into(), make_tensor_f32(vec![1, 1, HIDDEN_SIZE], emb_data.to_vec())?));

            let attn_len = kv_cache_seq_len + 1;
            decoder_inputs.push(("attention_mask".into(), make_tensor_i64(vec![1, attn_len], vec![1i64; attn_len])?));

            // Past KV cache tensors — model expects f32
            for layer in 0..NUM_HIDDEN_LAYERS {
                let key_idx = layer * 2;
                let val_idx = layer * 2 + 1;

                let key_data = std::mem::take(&mut kv_cache[key_idx]);
                let val_data = std::mem::take(&mut kv_cache[val_idx]);

                decoder_inputs.push((
                    format!("past_key_values.{}.key", layer),
                    make_tensor_f32(vec![1, NUM_KV_HEADS, kv_cache_seq_len, HEAD_DIM], key_data)?,
                ));
                decoder_inputs.push((
                    format!("past_key_values.{}.value", layer),
                    make_tensor_f32(vec![1, NUM_KV_HEADS, kv_cache_seq_len, HEAD_DIM], val_data)?,
                ));
            }

            let decoder_outputs = decoder
                .run(ort::session::SessionInputs::from(
                    decoder_inputs.into_iter().map(|(k, v)| (Cow::<str>::from(k), v)).collect::<Vec<_>>(),
                ))
                .map_err(|e| format!("Decoder step {} run: {}", step, e))?;

            let (next_token, new_kv_cache) = extract_decoder_outputs(&decoder_outputs)?;

            if next_token != EOS_TOKEN_ID {
                generated_tokens.push(next_token);
            }

            current_token = next_token;
            kv_cache = new_kv_cache;
            kv_cache_seq_len += 1;
        }

        Ok(generated_tokens)
    }

    // ─────────────────────── Output Extraction ───────────────────────────────

}

// Free function to avoid borrow conflicts when called from methods
// that hold &mut borrows on individual session fields.
fn extract_decoder_outputs(
    outputs: &ort::session::SessionOutputs,
) -> Result<(i64, Vec<Vec<f32>>), String> {
    let logits_val = outputs
        .get("logits")
        .ok_or("No 'logits' output from decoder")?;

    // Try f32 logits first; fall back to f16 and upcast
    let token_id = if let Ok((logits_shape, logits_data)) =
        logits_val.try_extract_tensor::<f32>()
    {
        let seq_len = logits_shape[1] as usize;
        let vocab_size = logits_shape[2] as usize;
        let last_start = (seq_len - 1) * vocab_size;
        logits_data[last_start..last_start + vocab_size]
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx as i64)
            .ok_or_else(|| "Empty logits".to_string())?
    } else {
        let (logits_shape, logits_data) = logits_val
            .try_extract_tensor::<half::f16>()
            .map_err(|e| format!("Extract logits: {}", e))?;
        let seq_len = logits_shape[1] as usize;
        let vocab_size = logits_shape[2] as usize;
        let last_start = (seq_len - 1) * vocab_size;
        logits_data[last_start..last_start + vocab_size]
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx as i64)
            .ok_or_else(|| "Empty logits".to_string())?
    };

    // Extract KV cache — try f32 first, fall back to f16→f32 upcast
    let mut kv_cache: Vec<Vec<f32>> = Vec::new();

    for layer in 0..NUM_HIDDEN_LAYERS {
        for kind in &["key", "value"] {
            let name = format!("present.{}.{}", layer, kind);
            let kv_val = outputs
                .get(&name)
                .ok_or_else(|| format!("Missing KV output: {}", name))?;

            let data: Vec<f32> = if let Ok((_, kv_data)) = kv_val.try_extract_tensor::<f32>() {
                kv_data.to_vec()
            } else {
                let (_, kv_data) = kv_val
                    .try_extract_tensor::<half::f16>()
                    .map_err(|e| format!("Extract KV {}: {}", name, e))?;
                kv_data.iter().map(|x| x.to_f32()).collect()
            };

            kv_cache.push(data);
        }
    }

    Ok((token_id, kv_cache))
}

impl GraniteSpeechManager {

    // ─────────────────────── Resampling ──────────────────────────────────────


    fn resample(&mut self, samples: &[f32], sample_rate: u32) -> Result<Vec<f32>, String> {
        let needs_new = self
            .resampler
            .as_ref()
            .map_or(true, |(r, s, _)| *r != sample_rate || *s != samples.len());

        if needs_new {
            let params = SincInterpolationParameters {
                sinc_len: 64,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 32,
                window: WindowFunction::BlackmanHarris2,
            };
            let resampler = SincFixedIn::<f32>::new(
                16000.0 / sample_rate as f64,
                2.0,
                params,
                samples.len(),
                1,
            )
            .map_err(|e| e.to_string())?;
            self.resampler = Some((sample_rate, samples.len(), Box::new(resampler)));
        }

        let (_, _, resampler) = self.resampler.as_mut().unwrap();
        let waves = resampler
            .process(&vec![samples.to_vec()], None)
            .map_err(|e| e.to_string())?;
        Ok(waves[0].clone())
    }
}
