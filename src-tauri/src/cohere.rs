// cohere.rs — Cohere Transcribe ONNX ASR manager.

use half::f16;
use ort::memory::Allocator;
use ort::session::Session;
use serde::Deserialize;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::cohere_features;
use crate::utils::strip_whitelisted_sound_captions;

const DEFAULT_MODEL_DIR: &str = "granite-speech-1b";
const MODEL_ID_UNIVERSAL: &str = "granite-speech-1b";
const NUM_LAYERS: usize = 8;
const NUM_HEADS: usize = 8;
const HEAD_DIM: usize = 128;
const VOCAB_SIZE: usize = 16384;
const DEFAULT_EOS_TOKEN_ID: i64 = 3;
const DEFAULT_DECODER_START_TOKEN_ID: i64 = 13764;
const DEFAULT_PAD_TOKEN_ID: i64 = 2;
const DEFAULT_MAX_NEW_TOKENS: usize = 256;
// Modern ORT supports 0-sized dynamic dimensions; no dummy-position workaround needed.
const MAX_SAFE_PROMPT_TOKENS: usize = 1024;

#[derive(Debug, Clone, serde::Serialize)]
pub enum GpuBackend {
    Cuda,
    DirectML,
    Cpu,
    Hybrid,
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Cuda => write!(f, "CUDA"),
            GpuBackend::DirectML => write!(f, "DirectML"),
            GpuBackend::Cpu => write!(f, "CPU"),
            GpuBackend::Hybrid => write!(f, "Hybrid"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CohereStatus {
    pub loaded: bool,
    pub model_id: Option<String>,
    pub backend: String,
    pub gpu_only: bool,
}

#[derive(Clone)]
struct KvTensor {
    shape: Vec<usize>,
    data: Vec<f16>,
}

#[derive(Clone)]
struct LayerKv {
    decoder_key: KvTensor,
    decoder_value: KvTensor,
    encoder_key: KvTensor,
    encoder_value: KvTensor,
}

pub struct CohereManager {
    encoder_session: Option<Session>,
    decoder_session: Option<Session>,
    tokenizer: Option<tokenizers::Tokenizer>,
    backend: GpuBackend,
    model_name: Option<String>,
    prompt_token_ids: Vec<i64>,
    decoder_start_token_id: i64,
    eos_token_id: i64,
    pad_token_id: i64,
    max_new_tokens: usize,
    debug_decode: bool,
    // (input_sample_rate, input_len, resampler)
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>,
}

#[inline]
fn should_trace(debug_decode: bool) -> bool {
    debug_decode || cfg!(target_os = "windows")
}

impl CohereManager {
    pub fn new() -> Self {
        Self {
            encoder_session: None,
            decoder_session: None,
            tokenizer: None,
            backend: GpuBackend::Cpu,
            model_name: None,
            prompt_token_ids: Vec::new(),
            decoder_start_token_id: DEFAULT_DECODER_START_TOKEN_ID,
            eos_token_id: DEFAULT_EOS_TOKEN_ID,
            pad_token_id: DEFAULT_PAD_TOKEN_ID,
            max_new_tokens: DEFAULT_MAX_NEW_TOKENS,
            debug_decode: false,
            resampler: None,
        }
    }

    pub fn get_status(&self) -> CohereStatus {
        let hybrid = matches!(self.backend, GpuBackend::Hybrid);
        CohereStatus {
            loaded: self.encoder_session.is_some() && self.decoder_session.is_some(),
            model_id: self.model_name.clone(),
            backend: self.backend.to_string(),
            gpu_only: hybrid,
        }
    }

    pub fn unload(&mut self) {
        if self.encoder_session.is_some() || self.decoder_session.is_some() {
            println!("[COHERE] Unloading model...");
            self.encoder_session = None;
            self.decoder_session = None;
            self.tokenizer = None;
            self.prompt_token_ids.clear();
            self.resampler = None;
            println!("[COHERE] Model unloaded");
        }
    }

    pub fn initialize(&mut self, model_id: Option<&str>, force_cpu: bool) -> Result<String, String> {
        let models_dir = crate::utils::get_models_dir()?;
        let model_dir = resolve_granite_model_dir(&models_dir, model_id)?;
        if !granite_int4_bundle_ready(&model_dir) {
            return Err(format!(
                "Cohere ONNX bundle not found in {}. Download model from Settings > Models.",
                model_dir.display()
            ));
        }
        if force_cpu {
            println!("[COHERE] force_cpu requested but ignored: Cohere runs in fixed Hybrid mode.");
        }

        let tokenizer_path = model_dir.join("tokenizer.json");
        let encoder_fp16 = model_dir.join("encoder_model_fp16.onnx");
        let decoder_fp16 = model_dir.join("decoder_model_merged_fp16.onnx");
        let encoder_q4f16 = model_dir.join("encoder_model_q4f16.onnx");
        let decoder_q4f16 = model_dir.join("decoder_model_merged_q4f16.onnx");
        let (encoder_path, decoder_path) = if encoder_fp16.exists() && decoder_fp16.exists() {
            (encoder_fp16, decoder_fp16)
        } else {
            (encoder_q4f16, decoder_q4f16)
        };
        let generation_config_path = model_dir.join("generation_config.json");
        let model_config_path = model_dir.join("config.json");
        println!(
            "[COHERE] initialize: model_dir={} encoder={} decoder={}",
            model_dir.display(),
            encoder_path.display(),
            decoder_path.display()
        );

        if self.encoder_session.is_some() || self.decoder_session.is_some() {
            self.unload();
        }

        let (backend, enc, dec) = {
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            {
                // Windows workaround: ORT CUDA GQA currently conflicts with this decoder export.
                // Run encoder on CUDA and decoder on CPU (hybrid) for stable inference.
                let enc = self
                    .create_session_cuda(&encoder_path)
                    .map_err(|e| format!("Cohere hybrid init (encoder CUDA) failed: {}", e))?;
                let dec = self
                    .create_session_cpu(&decoder_path)
                    .map_err(|e| format!("Cohere hybrid init (decoder CPU) failed: {}", e))?;
                (GpuBackend::Hybrid, enc, dec)
            }
            #[cfg(any(not(target_os = "windows"), all(target_os = "windows", not(target_arch = "x86_64"))))]
            {
                let enc = self
                    .create_session_cuda(&encoder_path)
                    .map_err(|e| format!("Cohere CUDA initialization failed (encoder): {}", e))?;
                let dec = self
                    .create_session_cuda(&decoder_path)
                    .map_err(|e| format!("Cohere CUDA initialization failed (decoder): {}", e))?;
                (GpuBackend::Cuda, enc, dec)
            }
        };

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;
        let runtime_cfg = load_runtime_config(&generation_config_path, &model_config_path, &tokenizer)?;

        self.encoder_session = Some(enc);
        self.decoder_session = Some(dec);
        self.tokenizer = Some(tokenizer);
        self.prompt_token_ids = runtime_cfg.prompt_token_ids;
        self.decoder_start_token_id = runtime_cfg.decoder_start_token_id;
        self.eos_token_id = runtime_cfg.eos_token_id;
        self.pad_token_id = runtime_cfg.pad_token_id;
        self.max_new_tokens = runtime_cfg.max_new_tokens;
        self.debug_decode = match std::env::var("TAURSCRIBE_COHERE_DEBUG").ok().as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => cfg!(target_os = "windows"),
        };
        self.backend = backend.clone();
        self.model_name = Some(MODEL_ID_UNIVERSAL.to_string());
        if should_trace(self.debug_decode) {
            println!(
                "[COHERE][TRACE] runtime cfg: prompt_tokens={} decoder_start={} eos={} pad={} max_new_tokens={} backend={}",
                self.prompt_token_ids.len(),
                self.decoder_start_token_id,
                self.eos_token_id,
                self.pad_token_id,
                self.max_new_tokens,
                self.backend
            );
        }

        let msg = format!("[COHERE] Model loaded ({})", backend);
        println!("{}", msg);
        Ok(msg)
    }

    pub fn transcribe_chunk(&mut self, samples: &[f32], sample_rate: u32) -> Result<String, String> {
        let audio: Cow<[f32]> = if sample_rate != 16000 {
            Cow::Owned(self.resample(samples, sample_rate)?)
        } else {
            Cow::Borrowed(samples)
        };

        let start = std::time::Instant::now();
        let features = cohere_features::extract_features(&audio);
        let n_frames = features.nrows();
        if n_frames == 0 {
            return Ok(String::new());
        }
        if should_trace(self.debug_decode) {
            println!(
                "[COHERE][TRACE] audio in: sample_rate={} samples={} feature_frames={} feature_dims={}",
                sample_rate,
                audio.len(),
                n_frames,
                features.ncols()
            );
        }
        let feature_data: Vec<f32> = features.iter().cloned().collect();
        let encoder_input = make_tensor_f32(vec![1, n_frames, 128], feature_data)?;

        let encoder = self.encoder_session.as_mut().ok_or("Encoder not loaded")?;
        let decoder = self.decoder_session.as_mut().ok_or("Decoder not loaded")?;
        let tokenizer = self.tokenizer.as_ref().ok_or("Tokenizer not loaded")?;

        let enc_out = encoder
            .run(ort::inputs!["input_features" => encoder_input])
            .map_err(|e| format!("Encoder run: {}", e))?;
        let enc_val = enc_out.values().next().ok_or("No encoder output")?;
        let (enc_shape_i64, enc_data) = enc_val
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract encoder hidden states: {}", e))?;
        let encoder_shape: Vec<usize> = enc_shape_i64.iter().map(|&d| d as usize).collect();
        let encoder_data = enc_data.to_vec();
        let encoder_seq_len = enc_shape_i64
            .get(1)
            .copied()
            .ok_or("Bad encoder output shape")?;
        if should_trace(self.debug_decode) {
            println!(
                "[COHERE][TRACE] encoder out shape={:?} seq_len={} data_len={}",
                encoder_shape,
                encoder_seq_len,
                encoder_data.len()
            );
        }

        // Build encoder_hidden_states once for prefill; decode steps won't resend it.
        let encoder_hidden_states = make_tensor_f32(encoder_shape.clone(), encoder_data.clone())?;

        // Prompt: decoder_start + language token.
        let mut prompt = if self.prompt_token_ids.is_empty() {
            vec![self.decoder_start_token_id]
        } else {
            self.prompt_token_ids.clone()
        };
        if should_trace(self.debug_decode) {
            println!(
                "[COHERE][TRACE] prefill prompt len={} ids_head={:?}",
                prompt.len(),
                &prompt[..prompt.len().min(32)]
            );
        }

        // Prefill: compute encoder cross-attention KV (fresh from encoder_hidden_states,
        // since past_encoder_sequence_length=0) and the first output token.
        let (mut next_token, prefill_cache) = run_decoder_prefill(
            decoder,
            &encoder_hidden_states,
            encoder_seq_len,
            &prompt,
            self.pad_token_id,
            self.debug_decode,
        )?;

        // Freeze encoder KV after prefill — the merged decoder returns 0-sized
        // present.*.encoder.* on subsequent steps (it signals "cached, reuse yours").
        // We must hold our own copy and pass it every step.
        let frozen_encoder_kv: Vec<(KvTensor, KvTensor)> = prefill_cache
            .iter()
            .map(|l| (l.encoder_key.clone(), l.encoder_value.clone()))
            .collect();
        let mut decoder_kv: Vec<(KvTensor, KvTensor)> = prefill_cache
            .into_iter()
            .map(|l| (l.decoder_key, l.decoder_value))
            .collect();

        if self.debug_decode {
            if let Some((ek, _)) = frozen_encoder_kv.first() {
                println!("[COHERE][DEBUG] encoder_kv frozen shape={:?} (reused every step)", ek.shape);
            }
            if let Some((dk, _)) = decoder_kv.first() {
                println!("[COHERE][DEBUG] decoder_kv after prefill shape={:?}", dk.shape);
            }
        }

        let mut generated: Vec<i64> = Vec::new();

        for step in 0..self.max_new_tokens {
            if next_token == self.eos_token_id {
                if self.debug_decode {
                    println!("[COHERE][DEBUG] EOS at step={}", step);
                }
                break;
            }
            generated.push(next_token);
            if self.debug_decode && step < 16 {
                println!("[COHERE][DEBUG] step={} token={}", step, next_token);
            }
            prompt.clear();
            prompt.push(next_token);
            let (t, new_decoder_kv) = run_decoder_step(
                decoder,
                &encoder_shape,
                &encoder_data,
                &decoder_kv,
                &frozen_encoder_kv,
                &prompt,
                self.debug_decode,
            )?;
            next_token = t;
            decoder_kv = new_decoder_kv;
            if self.debug_decode && step < 4 {
                if let Some((dk, _)) = decoder_kv.first() {
                    println!("[COHERE][DEBUG] decoder_kv shape after step={} → {:?}", step, dk.shape);
                }
            }
        }

        if generated.is_empty() {
            return Ok(String::new());
        }
        let text = tokenizer
            .decode(
                &generated.iter().map(|&t| t as u32).collect::<Vec<_>>(),
                true,
            )
            .map_err(|e| format!("Token decode error: {}", e))?;
        let out = strip_whitelisted_sound_captions(text.trim());

        let duration = start.elapsed();
        let audio_duration = audio.len() as f32 / 16000.0;
        let speedup = audio_duration / duration.as_secs_f32().max(1e-6);
        println!(
            "[COHERE] Transcribed {:.2}s audio in {:.0}ms | Speed: {:.1}x | \"{}\"",
            audio_duration,
            duration.as_millis(),
            speedup,
            out.trim()
        );

        Ok(out)
    }

}

fn run_decoder_prefill(
        decoder: &mut Session,
        encoder_hidden_states: &ort::value::DynValue,
        encoder_seq_len: i64,
        input_ids: &[i64],
        _pad_token_id: i64,
        debug_decode: bool,
    ) -> Result<(i64, Vec<LayerKv>), String> {
        let seq_len = input_ids.len();
        if seq_len == 0 {
            return Err("Decoder prefill: empty input_ids".to_string());
        }
        if seq_len > MAX_SAFE_PROMPT_TOKENS {
            return Err(format!(
                "Decoder prefill prompt too long: {} > {}",
                seq_len, MAX_SAFE_PROMPT_TOKENS
            ));
        }
        if should_trace(debug_decode) {
            let est_self_attn = (seq_len as u128) * (seq_len as u128);
            let est_cross_attn = (seq_len as u128) * (encoder_seq_len.max(0) as u128);
            println!(
                "[COHERE][TRACE] prefill inputs: seq_len={} encoder_seq_len={} est_self_attn={} est_cross_attn={} prompt_head={:?}",
                seq_len,
                encoder_seq_len,
                est_self_attn,
                est_cross_attn,
                &input_ids[..input_ids.len().min(16)]
            );
        }
        let mut inputs: Vec<(String, ort::value::DynValue)> = Vec::new();
        inputs.push(("input_ids".into(), make_tensor_i64(vec![1, seq_len], input_ids.to_vec())?));
        inputs.push((
            "attention_mask".into(),
            make_tensor_i64(vec![1, seq_len], vec![1_i64; seq_len])?,
        ));
        // position_ids is required by decoder/pos_emb/Gather_Quant in this export.
        inputs.push((
            "position_ids".into(),
            make_tensor_i64(
                vec![1, seq_len],
                (0_i64..seq_len as i64).collect(),
            )?,
        ));
        inputs.push(("num_logits_to_keep".into(), make_tensor_i64(vec![], vec![1])?));
        // NOTE: ort DynValue isn't Clone; prefill is only called once per chunk, so re-create it here.
        // We keep the heavy optimization for decode steps (where it matters most).
        let (shape, data) = encoder_hidden_states
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract encoder_hidden_states for prefill rebuild: {}", e))?;
        inputs.push((
            "encoder_hidden_states".into(),
            make_tensor_f32(shape.iter().map(|&d| d as usize).collect(), data.to_vec())?,
        ));

        // Initial past KV: empty (past_seq_len=0). Must use Tensor::new (allocator path)
        // because CreateTensorWithDataAsOrtValue rejects 0-sized dims; CreateTensorAsOrtValue
        // (used by Tensor::new) does not have that restriction.
        // past_encoder_sequence_length=0 signals the merged decoder to compute encoder KV
        // fresh from encoder_hidden_states on this first call.
        let alloc = Allocator::default();
        for layer in 0..NUM_LAYERS {
            inputs.push((
                format!("past_key_values.{}.decoder.key", layer),
                make_empty_kv_f16(&alloc)?,
            ));
            inputs.push((
                format!("past_key_values.{}.decoder.value", layer),
                make_empty_kv_f16(&alloc)?,
            ));
            inputs.push((
                format!("past_key_values.{}.encoder.key", layer),
                make_empty_kv_f16(&alloc)?,
            ));
            inputs.push((
                format!("past_key_values.{}.encoder.value", layer),
                make_empty_kv_f16(&alloc)?,
            ));
        }

        let outputs = decoder
            .run(ort::session::SessionInputs::from(
                inputs
                    .into_iter()
                    .map(|(k, v)| (Cow::<str>::from(k), v))
                    .collect::<Vec<_>>(),
            ))
            .map_err(|e| {
                format!(
                    "Decoder prefill run: {} | seq_len={} encoder_seq_len={} prompt_head={:?}",
                    e,
                    seq_len,
                    encoder_seq_len,
                    &input_ids[..input_ids.len().min(16)]
                )
            })?;
        if debug_decode {
            println!("[COHERE][DEBUG] prefill seq_len={} encoder_seq={}", seq_len, encoder_seq_len);
        }

        extract_decoder_outputs(&outputs, encoder_seq_len as usize, debug_decode)
}

/// One autoregressive decode step.
/// `decoder_kv`: per-layer (dec_key, dec_value) growing cache.
/// `frozen_encoder_kv`: per-layer (enc_key, enc_value) fixed from prefill — the merged
/// decoder returns 0-sized present.*.encoder.* on step 2+ so we must keep our own copy.
fn run_decoder_step(
        decoder: &mut Session,
        encoder_shape: &[usize],
        encoder_data: &[f32],
        decoder_kv: &[(KvTensor, KvTensor)],
        frozen_encoder_kv: &[(KvTensor, KvTensor)],
        input_ids: &[i64],
        debug_decode: bool,
    ) -> Result<(i64, Vec<(KvTensor, KvTensor)>), String> {
        let seq_len = input_ids.len();
        let past_decoder_len = decoder_kv
            .first()
            .and_then(|(dk, _)| dk.shape.get(2).copied())
            .unwrap_or(0);
        let total_seq = past_decoder_len + seq_len;
        if total_seq == 0 {
            return Err("Decoder step: computed total_seq=0".to_string());
        }
        if should_trace(debug_decode) {
            let est_self_attn = (total_seq as u128) * (total_seq as u128);
            println!(
                "[COHERE][TRACE] step inputs: seq_len={} past_decoder_len={} total_seq={} est_self_attn={}",
                seq_len, past_decoder_len, total_seq, est_self_attn
            );
        }

        if should_trace(debug_decode) {
            println!(
                "[COHERE][DEBUG] step: past_dec_len={} seq_len={} total_seq={} enc_kv_shape={:?}",
                past_decoder_len, seq_len, total_seq,
                frozen_encoder_kv.first().map(|(ek, _)| &ek.shape)
            );
        }

        let mut inputs: Vec<(String, ort::value::DynValue)> = Vec::new();
        inputs.push(("input_ids".into(), make_tensor_i64(vec![1, seq_len], input_ids.to_vec())?));
        inputs.push((
            "attention_mask".into(),
            make_tensor_i64(vec![1, total_seq], vec![1_i64; total_seq])?,
        ));
        // position_ids is required by decoder/pos_emb/Gather_Quant in this export.
        inputs.push((
            "position_ids".into(),
            make_tensor_i64(
                vec![1, seq_len],
                ((past_decoder_len as i64)..(past_decoder_len as i64 + seq_len as i64)).collect(),
            )?,
        ));
        inputs.push(("num_logits_to_keep".into(), make_tensor_i64(vec![], vec![1])?));
        inputs.push((
            "encoder_hidden_states".into(),
            make_tensor_f32(encoder_shape.to_vec(), encoder_data.to_vec())?,
        ));

        for layer in 0..NUM_LAYERS {
            let (dk, dv) = decoder_kv.get(layer).ok_or_else(|| format!("decoder_kv missing layer {}", layer))?;
            let (ek, ev) = frozen_encoder_kv.get(layer).ok_or_else(|| format!("encoder_kv missing layer {}", layer))?;
            let (dk_shape, dk_data) = normalize_kv_for_input(dk);
            let (dv_shape, dv_data) = normalize_kv_for_input(dv);
            let (ek_shape, ek_data) = normalize_kv_for_input(ek);
            let (ev_shape, ev_data) = normalize_kv_for_input(ev);
            if debug_decode && layer == 0 {
                println!(
                    "[COHERE][DEBUG] layer0 kv shapes: dec_k={:?} enc_k={:?}",
                    dk_shape, ek_shape
                );
            }
            inputs.push((
                format!("past_key_values.{}.decoder.key", layer),
                make_tensor_f16(dk_shape, dk_data)?,
            ));
            inputs.push((
                format!("past_key_values.{}.decoder.value", layer),
                make_tensor_f16(dv_shape, dv_data)?,
            ));
            inputs.push((
                format!("past_key_values.{}.encoder.key", layer),
                make_tensor_f16(ek_shape, ek_data)?,
            ));
            inputs.push((
                format!("past_key_values.{}.encoder.value", layer),
                make_tensor_f16(ev_shape, ev_data)?,
            ));
        }

        let outputs = decoder
            .run(ort::session::SessionInputs::from(
                inputs
                    .into_iter()
                    .map(|(k, v)| (Cow::<str>::from(k), v))
                    .collect::<Vec<_>>(),
            ))
            .map_err(|e| {
                format!(
                    "Decoder step run: {} | seq_len={} past_decoder_len={} total_seq={} token_head={:?}",
                    e,
                    seq_len,
                    past_decoder_len,
                    total_seq,
                    &input_ids[..input_ids.len().min(8)]
                )
            })?;

        // Extract next token and updated decoder KV; discard present.*.encoder.* output
        // (it's 0-sized after step 1 — the model signals "I've already cached this").
        extract_step_outputs(&outputs, debug_decode)
}

impl CohereManager {
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
        let (_, _, resampler) = self.resampler.as_mut().ok_or("resampler missing")?;
        let waves = resampler
            .process(&vec![samples.to_vec()], None)
            .map_err(|e| e.to_string())?;
        Ok(waves[0].clone())
    }

    fn create_session_cpu(&self, path: &Path) -> Result<Session, String> {
        Session::builder()
            .map_err(|e| format!("ORT builder: {}", e))?
            .commit_from_file(path)
            .map_err(|e| format!("CPU session load {}: {}", path.display(), e))
    }

    #[cfg(any(
        not(target_os = "windows"),
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    fn create_session_cuda(&self, path: &Path) -> Result<Session, String> {
        Session::builder()
            .map_err(|e| format!("ORT builder: {}", e))?
            .with_execution_providers([
                ort::execution_providers::CUDAExecutionProvider::default()
                    .build()
                    .error_on_failure(),
            ])
            .map_err(|e| format!("CUDA EP: {}", e))?
            .commit_from_file(path)
            .map_err(|e| format!("CUDA load {}: {}", path.display(), e))
    }

}

/// Creates a zero-element f16 KV tensor of shape [1, NUM_HEADS, 0, HEAD_DIM].
/// Uses Tensor::new (CreateTensorAsOrtValue) which supports 0-sized dims, unlike
/// from_array (CreateTensorWithDataAsOrtValue) which rejects them.
fn make_empty_kv_f16(alloc: &Allocator) -> Result<ort::value::DynValue, String> {
    ort::value::Tensor::<f16>::new(alloc, [1_usize, NUM_HEADS, 0, HEAD_DIM])
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Empty KV tensor creation error: {}", e))
}

fn make_tensor_f32(shape: Vec<usize>, data: Vec<f32>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {}", e))
}

fn make_tensor_f16(shape: Vec<usize>, data: Vec<f16>) -> Result<ort::value::DynValue, String> {
    ort::value::Tensor::<f16>::from_array((shape.clone(), data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error (f16 shape={:?}): {}", shape, e))
}

fn make_tensor_i64(shape: Vec<usize>, data: Vec<i64>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {}", e))
}

fn extract_token_from_logits(
    outputs: &ort::session::SessionOutputs,
    debug_decode: bool,
) -> Result<i64, String> {
    let logits_val = outputs
        .get("logits")
        .ok_or("No 'logits' output from decoder")?;
    let (logits_shape, logits_data) = logits_val
        .try_extract_tensor::<f16>()
        .map_err(|e| format!("Extract logits: {}", e))?;
    if logits_shape.len() != 3 || logits_shape[2] as usize != VOCAB_SIZE {
        return Err(format!("Unexpected logits shape: {:?}", logits_shape));
    }
    let seq = logits_shape[1] as usize;
    let start = (seq - 1) * VOCAB_SIZE;
    let token = logits_data[start..start + VOCAB_SIZE]
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(idx, _)| idx as i64)
        .ok_or("Empty logits")?;
    if should_trace(debug_decode) {
        println!("[COHERE][TRACE] logits shape={:?}", logits_shape);
        let mut top = logits_data[start..start + VOCAB_SIZE]
            .iter()
            .enumerate()
            .map(|(i, v)| (i, v.to_f32()))
            .collect::<Vec<_>>();
        top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        println!("[COHERE][DEBUG] top5={:?}", &top[..5.min(top.len())]);
    }
    Ok(token)
}

/// Extract result from prefill: token + all KV (decoder AND encoder, both non-zero on first call).
fn extract_decoder_outputs(
    outputs: &ort::session::SessionOutputs,
    _fallback_encoder_len: usize,
    debug_decode: bool,
) -> Result<(i64, Vec<LayerKv>), String> {
    let token = extract_token_from_logits(outputs, debug_decode)?;

    let mut layers = Vec::with_capacity(NUM_LAYERS);
    for layer in 0..NUM_LAYERS {
        let dk = get_present(outputs, &format!("present.{}.decoder.key", layer))?;
        let dv = get_present(outputs, &format!("present.{}.decoder.value", layer))?;
        let ek = get_present(outputs, &format!("present.{}.encoder.key", layer))?;
        let ev = get_present(outputs, &format!("present.{}.encoder.value", layer))?;
        if should_trace(debug_decode) && layer == 0 {
            println!(
                "[COHERE][DEBUG] prefill output: dec_k={:?} enc_k={:?}",
                dk.shape, ek.shape
            );
        }
        layers.push(LayerKv {
            decoder_key: dk,
            decoder_value: dv,
            encoder_key: ek,
            encoder_value: ev,
        });
    }
    Ok((token, layers))
}

/// Extract result from a decode step: token + updated decoder KV only.
/// Encoder KV is NOT read from step outputs because the merged decoder returns 0-sized
/// present.*.encoder.* after the first step (it signals "cached, reuse yours").
fn extract_step_outputs(
    outputs: &ort::session::SessionOutputs,
    debug_decode: bool,
) -> Result<(i64, Vec<(KvTensor, KvTensor)>), String> {
    let token = extract_token_from_logits(outputs, debug_decode)?;

    let mut dec_layers = Vec::with_capacity(NUM_LAYERS);
    for layer in 0..NUM_LAYERS {
        let dk = get_present(outputs, &format!("present.{}.decoder.key", layer))?;
        let dv = get_present(outputs, &format!("present.{}.decoder.value", layer))?;
        if should_trace(debug_decode) && layer == 0 {
            println!("[COHERE][DEBUG] step output: dec_k={:?}", dk.shape);
        }
        dec_layers.push((dk, dv));
    }
    Ok((token, dec_layers))
}

fn get_present(outputs: &ort::session::SessionOutputs, name: &str) -> Result<KvTensor, String> {
    let val = outputs
        .get(name)
        .ok_or_else(|| format!("Missing KV output: {}", name))?;
    let (shape, data) = val
        .try_extract_tensor::<f16>()
        .map_err(|e| format!("Extract KV {}: {}", name, e))?;
    Ok(KvTensor {
        shape: shape.iter().map(|&d| d as usize).collect(),
        data: data.to_vec(),
    })
}

fn normalize_kv_for_input(kv: &KvTensor) -> (Vec<usize>, Vec<f16>) {
    let shape = kv.shape.clone();
    let expected = shape.iter().product::<usize>();
    if expected == kv.data.len() {
        return (shape, kv.data.clone());
    }
    if expected == 0 {
        return (shape, Vec::new());
    }
    let mut data = kv.data.clone();
    data.resize(expected, f16::from_f32(0.0));
    (shape, data)
}

#[derive(Debug, Deserialize)]
struct GenerationConfig {
    decoder_start_token_id: Option<i64>,
    eos_token_id: Option<i64>,
    pad_token_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ModelConfig {
    prompt_defaults: Option<Vec<PromptDefault>>,
}

#[derive(Debug, Deserialize)]
struct PromptDefault {
    role: Option<String>,
    slots: Option<std::collections::HashMap<String, String>>,
}

struct RuntimeConfig {
    decoder_start_token_id: i64,
    eos_token_id: i64,
    pad_token_id: i64,
    max_new_tokens: usize,
    prompt_token_ids: Vec<i64>,
}

fn load_runtime_config(
    generation_config_path: &Path,
    model_config_path: &Path,
    tokenizer: &tokenizers::Tokenizer,
) -> Result<RuntimeConfig, String> {
    let gen_cfg: GenerationConfig = serde_json::from_str(
        &std::fs::read_to_string(generation_config_path)
            .map_err(|e| format!("Read generation_config.json: {}", e))?,
    )
    .map_err(|e| format!("Parse generation_config.json: {}", e))?;

    let model_cfg: ModelConfig = serde_json::from_str(
        &std::fs::read_to_string(model_config_path)
            .map_err(|e| format!("Read config.json: {}", e))?,
    )
    .map_err(|e| format!("Parse config.json: {}", e))?;

    let decoder_start = gen_cfg
        .decoder_start_token_id
        .unwrap_or(DEFAULT_DECODER_START_TOKEN_ID);
    let eos = gen_cfg.eos_token_id.unwrap_or(DEFAULT_EOS_TOKEN_ID);
    let pad = gen_cfg.pad_token_id.unwrap_or(DEFAULT_PAD_TOKEN_ID);

    let mut prompt_ids = vec![decoder_start];
    let lang = std::env::var("TAURSCRIBE_COHERE_LANG")
        .unwrap_or_else(|_| "en".to_string())
        .to_lowercase();
    let lang_token = format!("<|{}|>", lang);

    let defaults = model_cfg
        .prompt_defaults
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.role.as_deref() == Some("user"))
        .and_then(|d| d.slots)
        .unwrap_or_default();

    let order = [
        defaults.get("source_lang").cloned().unwrap_or(lang_token.clone()),
        defaults.get("target_lang").cloned().unwrap_or(lang_token),
        defaults.get("pnc").cloned().unwrap_or_else(|| "<|pnc|>".to_string()),
        defaults.get("itn").cloned().unwrap_or_else(|| "<|noitn|>".to_string()),
        defaults
            .get("timestamp")
            .cloned()
            .unwrap_or_else(|| "<|notimestamp|>".to_string()),
        defaults
            .get("diarize")
            .cloned()
            .unwrap_or_else(|| "<|nodiarize|>".to_string()),
        defaults
            .get("emotion")
            .cloned()
            .unwrap_or_else(|| "<|emo:undefined|>".to_string()),
    ];
    for tok in order {
        if let Some(id) = tokenizer.token_to_id(&tok) {
            prompt_ids.push(id as i64);
        }
    }

    Ok(RuntimeConfig {
        decoder_start_token_id: decoder_start,
        eos_token_id: eos,
        pad_token_id: pad,
        max_new_tokens: DEFAULT_MAX_NEW_TOKENS,
        prompt_token_ids: prompt_ids,
    })
}

pub(crate) fn resolve_granite_model_dir(models_dir: &Path, model_id: Option<&str>) -> Result<PathBuf, String> {
    let dir = match model_id {
        None => models_dir.join(DEFAULT_MODEL_DIR),
        Some(id) => {
            let pb = PathBuf::from(id);
            if pb.is_absolute() {
                pb
            } else {
                match id {
                    "granite-speech-1b"
                    | "granite-speech-1b-cpu"
                    | "granite-speech-1b-fp16"
                    | "granite-speech-1b-fp16-cuda" => models_dir.join(DEFAULT_MODEL_DIR),
                    other => {
                        if other.contains('/') || other.contains('\\') {
                            return Err(format!("Invalid model id: {}", other));
                        }
                        models_dir.join(other)
                    }
                }
            }
        }
    };
    if !dir.exists() {
        return Err(format!(
            "Model not found at {}. Download it from Settings > Download Manager.",
            dir.display()
        ));
    }
    Ok(dir)
}

pub(crate) fn granite_logical_model_id_for_dir(_model_dir: &Path) -> String {
    MODEL_ID_UNIVERSAL.to_string()
}

pub(crate) fn granite_int4_bundle_ready(dir: &Path) -> bool {
    let has_fp16 = dir.join("encoder_model_fp16.onnx").exists()
        && dir.join("encoder_model_fp16.onnx_data").exists()
        && dir.join("decoder_model_merged_fp16.onnx").exists()
        && dir.join("decoder_model_merged_fp16.onnx_data").exists();
    let has_q4f16 = dir.join("encoder_model_q4f16.onnx").exists()
        && dir.join("encoder_model_q4f16.onnx_data").exists()
        && dir.join("decoder_model_merged_q4f16.onnx").exists()
        && dir.join("decoder_model_merged_q4f16.onnx_data").exists();
    dir.is_dir()
        && (has_fp16 || has_q4f16)
        && dir.join("tokenizer.json").exists()
        && dir.join("preprocessor_config.json").exists()
}
