// granite_speech.rs — Granite 4.0 1B Speech ONNX orchestrator
//
// Manages three ONNX sessions (audio_encoder, embed_tokens, decoder_model_merged)
// and performs end-to-end speech-to-text inference with KV cache management.
//
// NOTE: We use ort's (shape, Vec<T>) tuple API for Value::from_array instead of
// ndarray arrays because ort 2.0.0-rc.11 re-exports its own ndarray version
// which is incompatible with the project's ndarray 0.15.
//
// KV cache is kept in f32 in Rust. The exported full-FP16 decoder ONNX uses float32
// inputs_embeds and float16 past_key_values (weights are FP16 internally); q4 / q4f16 use f32 KV I/O.

use half::f16;
use ndarray::{s, Array2, Array3, Array4};
use ort::session::Session;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::granite_features;
use crate::utils::strip_whitelisted_sound_captions;

/// FP16 (`granite-speech-1b-fp16`) has no CPU path — ORT CPU EP is unsupported for this bundle.
fn err_granite_fp16_gpu_required(detail: &str) -> String {
    format!(
        "{} The FP16 (GPU-only) Granite package cannot run on CPU. \
Download the CPU INT4 bundle “Granite 4.0 1B Speech” from Settings → Models.",
        detail
    )
}

/// **Linux only.** When set, Granite will not fall back to CPU if CUDA session creation fails.
#[cfg(not(target_os = "windows"))]
fn granite_require_cuda_env() -> bool {
    std::env::var("TAURSCRIBE_GRANITE_REQUIRE_CUDA")
        .map(|v| {
            matches!(
                v.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

// ───────────────────────── Model Constants ────────────────────────────────────
const EOS_TOKEN_ID: i64 = 100257;
#[allow(dead_code)]
const PAD_TOKEN_ID: i64 = 100256;
const AUDIO_TOKEN_INDEX: i64 = 100352;
const NUM_HIDDEN_LAYERS: usize = 40;
const NUM_KV_HEADS: usize = 4;
const HEAD_DIM: usize = 128;
const MAX_NEW_TOKENS: usize = 200;
const HIDDEN_SIZE: usize = 2048;
/// Fused GQA attention builds O(seq²) buffers; cap seq to catch ORT shape bugs before multi-TB alloc attempts.
const MAX_GRANITE_DECODER_SEQ_LEN: usize = 8192;

// ───────────────────────── GPU Backend ────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub enum GpuBackend {
    /// Encoder + embed on CUDA, decoder on CPU (CUDA GQA limitation). Linux, or Windows x86_64 fallback when DirectML fails.
    Cuda,
    /// ONNX Runtime via DirectX 12 / DirectML (Windows). Does not require cuDNN like the CUDA EP.
    DirectML,
    Cpu,
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Cuda => write!(f, "CUDA"),
            GpuBackend::DirectML => write!(f, "DirectML"),
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
    /// FP16 bundle is loaded — CPU inference is not supported; UI should lock ASR to GPU.
    pub gpu_only: bool,
}

// ───────────────────────── Helper: build ORT tensor ──────────────────────────
// Use (shape, Vec<T>) tuple form which always works regardless of ndarray version.

fn make_tensor_f32(shape: Vec<usize>, data: Vec<f32>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {}", e))
}

fn make_tensor_f16(shape: Vec<usize>, data: Vec<f16>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {}", e))
}

fn make_tensor_i64(shape: Vec<usize>, data: Vec<i64>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {}", e))
}

/// ORT may report unresolved dynamic axes as 0 or negative; casting those to `usize` corrupts shapes.
fn ort_positive_dim(d: i64, axis: &str) -> Result<usize, String> {
    if d <= 0 {
        return Err(format!(
            "Granite ONNX tensor axis '{}' has invalid dim {} (unresolved dynamic axis?)",
            axis, d
        ));
    }
    Ok(d as usize)
}

/// Granite uses variable-length audio → variable `inputs_embeds` seq; ORT memory patterns must be off.
fn granite_session_builder() -> Result<ort::session::builder::SessionBuilder, String> {
    Session::builder()
        .map_err(|e| format!("Session builder: {}", e))?
        .with_memory_pattern(false)
        .map_err(|e| format!("ORT DisableMemPattern (Granite dynamic shapes): {}", e))
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

fn make_empty_kv_f16(num_heads: usize, head_dim: usize) -> Result<ort::value::DynValue, String> {
    let arr = Array4::<f16>::from_shape_vec((1, num_heads, 0, head_dim), vec![])
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
    /// Full FP16 bundle: decoder expects float32 `inputs_embeds`, float16 `past_key_values`.
    decoder_io_fp16: bool,
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
            decoder_io_fp16: false,
            resampler: None,
        }
    }

    pub fn get_status(&self) -> GraniteSpeechStatus {
        let loaded = self.encoder_session.is_some();
        GraniteSpeechStatus {
            loaded,
            model_id: self.model_name.clone(),
            backend: self.backend.to_string(),
            gpu_only: loaded && self.decoder_io_fp16,
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
            self.decoder_io_fp16 = false;
            self.resampler = None;
            println!("[GRANITE] Model unloaded");
        }
    }

    pub fn initialize(
        &mut self,
        model_id: Option<&str>,
        force_cpu: bool,
    ) -> Result<String, String> {
        let models_dir = crate::utils::get_models_dir()?;
        let model_dir = resolve_granite_model_dir(&models_dir, model_id)?;

        let mode_label = if force_cpu {
            " [CPU-only mode]"
        } else if cfg!(target_os = "macos") {
            " [XNNPACK/CPU]"
        } else {
            ""
        };
        println!("[GRANITE] Loading model from: {}{}", model_dir.display(), mode_label);

        let tokenizer_path = model_dir.join("tokenizer.json");
        if !tokenizer_path.exists() {
            return Err(format!("Missing tokenizer.json (expected at {})", tokenizer_path.display()));
        }

        // Release previous ORT sessions before allocating new ones (VRAM + EP state).
        if self.encoder_session.is_some() {
            self.unload();
        }

        // ── Full FP16 bundle (separate download, ~4.6 GB) ─────────────────────
        let enc_fp16 = model_dir.join("audio_encoder_fp16.onnx");
        let emb_fp16 = model_dir.join("embed_tokens_fp16.onnx");
        let dec_fp16 = model_dir.join("decoder_model_merged_fp16.onnx");
        let has_fp16 = granite_fp16_bundle_ready(&model_dir);
        let logical_id = if has_fp16 {
            "granite-speech-1b-fp16".to_string()
        } else {
            "granite-speech-1b".to_string()
        };

        // ── INT4 / INT4+FP16 activation bundles ──────────────────────────────
        let enc_q4f16 = model_dir.join("audio_encoder_q4f16.onnx");
        let emb_q4f16 = model_dir.join("embed_tokens_q4f16.onnx");
        let dec_q4f16 = model_dir.join("decoder_model_merged_q4f16.onnx");
        let encoder_path = model_dir.join("audio_encoder_q4.onnx");
        let embed_path = model_dir.join("embed_tokens_q4.onnx");
        let decoder_path = model_dir.join("decoder_model_merged_q4.onnx");

        let has_q4 = encoder_path.exists() && embed_path.exists() && decoder_path.exists();
        let has_q4f16 = enc_q4f16.exists() && emb_q4f16.exists() && dec_q4f16.exists();

        if !has_fp16 && !has_q4 && !has_q4f16 {
            return Err(
                "No Granite Speech model files found. Download Granite Speech from Settings > Models.".to_string(),
            );
        }

        if has_fp16 && force_cpu {
            return Err(err_granite_fp16_gpu_required(
                "CPU mode was requested, but this model is GPU-only.",
            ));
        }
        if has_fp16 && cfg!(target_os = "macos") {
            return Err(err_granite_fp16_gpu_required(
                "macOS does not load this FP16 GPU bundle.",
            ));
        }

        #[cfg(not(target_os = "windows"))]
        let require_cuda = granite_require_cuda_env() && !force_cpu && !cfg!(target_os = "macos");

        let (backend, encoder, embed, decoder) = if has_fp16 {
            println!("[GRANITE] Using full FP16 ONNX weights (GPU only — no CPU fallback)");
            {
                #[cfg(target_os = "windows")]
                {
                    match self.try_create_sessions_directml(&enc_fp16, &emb_fp16, &dec_fp16) {
                        Ok((enc, emb, dec)) => {
                            println!("[GRANITE] Using FP16 on DirectML (Windows)");
                            (GpuBackend::DirectML, enc, emb, dec)
                        }
                        Err(dml_e) => {
                            println!(
                                "[GRANITE] DirectML failed ({}); trying CUDA for encoder/embed…",
                                dml_e
                            );
                            #[cfg(target_arch = "x86_64")]
                            {
                                match self.try_create_sessions_cuda(&enc_fp16, &emb_fp16, &dec_fp16) {
                                    Ok((enc, emb, dec)) => {
                                        println!(
                                            "[GRANITE] Using FP16: CUDA encoder/embed + CPU decoder (DirectML unavailable)"
                                        );
                                        (GpuBackend::Cuda, enc, emb, dec)
                                    }
                                    Err(cuda_e) => {
                                        return Err(err_granite_fp16_gpu_required(&format!(
                                            "DirectML failed ({dml_e}); CUDA failed ({cuda_e})."
                                        )));
                                    }
                                }
                            }
                            #[cfg(not(target_arch = "x86_64"))]
                            {
                                return Err(err_granite_fp16_gpu_required(&format!(
                                    "DirectML failed ({dml_e}). This Windows build has no CUDA fallback."
                                )));
                            }
                        }
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    match self.try_create_sessions_cuda(&enc_fp16, &emb_fp16, &dec_fp16) {
                        Ok((enc, emb, dec)) => {
                            println!("[GRANITE] Using FP16: CUDA encoder/embed + CPU decoder");
                            (GpuBackend::Cuda, enc, emb, dec)
                        }
                        Err(e) => {
                            if require_cuda {
                                return Err(format!(
                                    "TAURSCRIBE_GRANITE_REQUIRE_CUDA is set but FP16 CUDA failed: {e}"
                                ));
                            }
                            return Err(err_granite_fp16_gpu_required(&format!(
                                "CUDA could not load this model ({e})."
                            )));
                        }
                    }
                }
            }
        } else if force_cpu || cfg!(target_os = "macos") {
            // Prefer q4 on CPU; fall back to q4f16 if only the GPU download is present.
            let (ep, eb, dp) = if has_q4 {
                (&encoder_path, &embed_path, &decoder_path)
            } else {
                (&enc_q4f16, &emb_q4f16, &dec_q4f16)
            };
            let enc = self.create_session_cpu(ep)?;
            let emb = self.create_session_cpu(eb)?;
            let dec = self.create_session_cpu(dp)?;
            (GpuBackend::Cpu, enc, emb, dec)
        } else {
            #[cfg(target_os = "windows")]
            {
                // On x86_64, try CUDA **before** DirectML for q4 / q4f16. DirectML session creation often succeeds
                // but the Granite audio encoder then fails at inference (Reshape / dynamic seq len) on some
                // drivers; CUDA hybrid encoder+embed is the reliable path when an NVIDIA GPU is present.
                // DirectML remains first choice on ARM64 Windows (no CUDA in this build).
                let mut picked: Option<(GpuBackend, Session, Session, Session)> = None;

                #[cfg(target_arch = "x86_64")]
                if picked.is_none() && has_q4f16 {
                    match self.try_create_sessions_cuda(&enc_q4f16, &emb_q4f16, &dec_q4f16) {
                        Ok((e, em, d)) => {
                            println!("[GRANITE] Using q4f16: CUDA encoder/embed + CPU decoder");
                            picked = Some((GpuBackend::Cuda, e, em, d));
                        }
                        Err(e) => println!("[GRANITE] q4f16 CUDA failed: {}", e),
                    }
                }
                if picked.is_none() && has_q4f16 {
                    match self.try_create_sessions_directml(&enc_q4f16, &emb_q4f16, &dec_q4f16) {
                        Ok((e, em, d)) => {
                            println!("[GRANITE] Using q4f16 on DirectML");
                            picked = Some((GpuBackend::DirectML, e, em, d));
                        }
                        Err(e) => println!("[GRANITE] q4f16 DirectML failed: {}", e),
                    }
                }
                #[cfg(target_arch = "x86_64")]
                if picked.is_none() && has_q4 {
                    match self.try_create_sessions_cuda(&encoder_path, &embed_path, &decoder_path) {
                        Ok((e, em, d)) => {
                            println!("[GRANITE] Using INT4: CUDA encoder/embed + CPU decoder");
                            picked = Some((GpuBackend::Cuda, e, em, d));
                        }
                        Err(e) => println!("[GRANITE] INT4 CUDA failed: {}", e),
                    }
                }
                if picked.is_none() && has_q4 {
                    match self.try_create_sessions_directml(&encoder_path, &embed_path, &decoder_path) {
                        Ok((e, em, d)) => {
                            println!("[GRANITE] Using INT4 on DirectML");
                            picked = Some((GpuBackend::DirectML, e, em, d));
                        }
                        Err(e) => println!("[GRANITE] INT4 DirectML failed: {}", e),
                    }
                }

                if let Some(t) = picked {
                    t
                } else if has_q4 {
                    println!("[GRANITE] All GPU paths failed; running INT4 on CPU...");
                    let enc = self.create_session_cpu(&encoder_path)?;
                    let emb = self.create_session_cpu(&embed_path)?;
                    let dec = self.create_session_cpu(&decoder_path)?;
                    (GpuBackend::Cpu, enc, emb, dec)
                } else {
                    println!("[GRANITE] All GPU paths failed; running q4f16 on CPU...");
                    let enc = self.create_session_cpu(&enc_q4f16)?;
                    let emb = self.create_session_cpu(&emb_q4f16)?;
                    let dec = self.create_session_cpu(&dec_q4f16)?;
                    (GpuBackend::Cpu, enc, emb, dec)
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                // Linux: CUDA encoder/embed + CPU decoder, then CPU fallback.
                let gpu_q4f16: Option<(GpuBackend, Session, Session, Session)> = if has_q4f16 {
                    match self.try_create_sessions_cuda(&enc_q4f16, &emb_q4f16, &dec_q4f16) {
                        Ok(r) => {
                            println!("[GRANITE] Using q4f16: CUDA encoder/embed + CPU decoder");
                            Some((GpuBackend::Cuda, r.0, r.1, r.2))
                        }
                        Err(e) => {
                            if require_cuda {
                                return Err(format!(
                                    "TAURSCRIBE_GRANITE_REQUIRE_CUDA is set but q4f16 CUDA failed: {e}"
                                ));
                            }
                            let _ = e;
                            None
                        }
                    }
                } else {
                    None
                };

                if let Some((backend, enc, emb, dec)) = gpu_q4f16 {
                    (backend, enc, emb, dec)
                } else if has_q4 {
                    match self.try_create_sessions_cuda(&encoder_path, &embed_path, &decoder_path) {
                        Ok((enc, emb, dec)) => {
                            println!("[GRANITE] Using INT4: CUDA encoder/embed + CPU decoder");
                            (GpuBackend::Cuda, enc, emb, dec)
                        }
                        Err(e) => {
                            if require_cuda {
                                return Err(format!(
                                    "TAURSCRIBE_GRANITE_REQUIRE_CUDA is set but INT4 CUDA failed: {e}"
                                ));
                            }
                            println!("[GRANITE] CUDA failed ({}), falling back to CPU...", e);
                            let enc = self.create_session_cpu(&encoder_path)?;
                            let emb = self.create_session_cpu(&embed_path)?;
                            let dec = self.create_session_cpu(&decoder_path)?;
                            (GpuBackend::Cpu, enc, emb, dec)
                        }
                    }
                } else {
                    println!("[GRANITE] CUDA unavailable, running q4f16 on CPU");
                    let enc = self.create_session_cpu(&enc_q4f16)?;
                    let emb = self.create_session_cpu(&emb_q4f16)?;
                    let dec = self.create_session_cpu(&dec_q4f16)?;
                    (GpuBackend::Cpu, enc, emb, dec)
                }
            }
        };

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

        self.encoder_session = Some(encoder);
        self.embed_session = Some(embed);
        self.decoder_session = Some(decoder);
        self.tokenizer = Some(tokenizer);
        self.backend = backend.clone();
        self.model_name = Some(logical_id);
        self.decoder_io_fp16 = has_fp16;

        let msg = format!("[GRANITE] Model loaded ({})", backend);
        println!("{}", msg);
        Ok(msg)
    }

    pub fn transcribe_chunk(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<String, String> {
        let audio: std::borrow::Cow<[f32]> = if sample_rate != 16000 {
            std::borrow::Cow::Owned(self.resample(samples, sample_rate)?)
        } else {
            std::borrow::Cow::Borrowed(samples)
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
        let out = strip_whitelisted_sound_captions(text.trim());
        println!(
            "[GRANITE] Transcribed {:.2}s audio in {:.0}ms | Speed: {:.1}x | \"{}\"",
            audio_duration, duration.as_millis(), speedup, out.trim()
        );

        Ok(out)
    }

    // ─────────────────────── Session Creation ─────────────────────────────────

    /// Linux always; Windows **x86_64 only** (CUDA EP not in ARM64 ort build).
    #[cfg(any(
        not(target_os = "windows"),
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    fn try_create_sessions_cuda(
        &self,
        encoder_path: &std::path::Path,
        embed_path: &std::path::Path,
        decoder_path: &std::path::Path,
    ) -> Result<(Session, Session, Session), String> {
        println!("[GRANITE] Attempting CUDA acceleration...");

        // Allocate ONNX initializers (weights) via the CUDA device allocator instead of the
        // CPU arena, so weights land in VRAM rather than a long-lived CPU copy. See ONNX Runtime
        // session option `session.use_device_allocator_for_initializers` (ort:
        // `with_device_allocator_for_initializers`).
        let build_cuda = |path: &std::path::Path, label: &str| -> Result<Session, String> {
            granite_session_builder()?
                .with_device_allocator_for_initializers()
                .map_err(|e| format!("device_allocator_for_initializers: {}", e))?
                .with_execution_providers([ort::execution_providers::CUDAExecutionProvider::default()
                    .build()
                    .error_on_failure()])
                .map_err(|e| format!("CUDA EP: {}", e))?
                .commit_from_file(path)
                .map_err(|e| format!("{} load: {}", label, e))
        };

        let enc = build_cuda(encoder_path, "Encoder")?;
        let emb = build_cuda(embed_path, "Embed")?;
        // ORT's CUDA GroupQueryAttention kernel rejects graphs with `position_ids` and
        // `attention_bias` (Granite decoder export). CPU EP implements the full op.
        let dec = self.create_session_cpu(decoder_path)?;
        println!(
            "[GRANITE] ✓ Encoder + embed on CUDA (device alloc); decoder on CPU (CUDA GQA lacks position_ids/attention_bias)"
        );
        Ok((enc, emb, dec))
    }

    /// DirectML EP on Windows — uses the GPU via DirectX 12 without NVIDIA’s cuDNN DLLs required by the CUDA EP.
    #[cfg(target_os = "windows")]
    fn try_create_sessions_directml(
        &self,
        encoder_path: &std::path::Path,
        embed_path: &std::path::Path,
        decoder_path: &std::path::Path,
    ) -> Result<(Session, Session, Session), String> {
        println!("[GRANITE] Attempting DirectML (GPU via DirectX 12)...");

        let build_dml = |path: &std::path::Path, label: &str| -> Result<Session, String> {
            granite_session_builder()?
                .with_execution_providers([ort::execution_providers::DirectMLExecutionProvider::default()
                    .build()
                    .error_on_failure()])
                .map_err(|e| format!("DirectML EP: {}", e))?
                .commit_from_file(path)
                .map_err(|e| format!("{} load: {}", label, e))
        };

        let enc = build_dml(encoder_path, "Encoder")?;
        let emb = build_dml(embed_path, "Embed")?;
        let dec = build_dml(decoder_path, "Decoder")?;

        println!("[GRANITE] ✓ DirectML sessions created");
        Ok((enc, emb, dec))
    }

    fn create_session_cpu(&self, path: &std::path::Path) -> Result<Session, String> {
        granite_session_builder()?
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

        let d0 = ort_positive_dim(shape[0], "encoder batch")?;
        let d1 = ort_positive_dim(shape[1], "encoder seq")?;
        let d2 = ort_positive_dim(shape[2], "encoder hidden")?;

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

        let audio_seq_len = audio_embeddings.shape()[1];
        if audio_seq_len > MAX_GRANITE_DECODER_SEQ_LEN {
            return Err(format!(
                "Granite audio embedding seq {} exceeds cap {}",
                audio_seq_len, MAX_GRANITE_DECODER_SEQ_LEN
            ));
        }

        // Match HuggingFace `GraniteSpeechProcessor`: repeat `<|audio|>` once per encoder time step
        // before tokenization, then scatter one encoder frame per audio token into `inputs_embeds`
        // (`get_merged_audio_embeddings`). A single `<|audio|>` breaks BPE alignment and makes the
        // decoder predict <|end_of_text|> immediately.
        // Chat template: USER: {{ content }}\n ASSISTANT:
        const AUDIO_MARKER: &str = "<|audio|>";
        let audio_segment: String = std::iter::repeat(AUDIO_MARKER)
            .take(audio_seq_len)
            .collect();
        let prompt = format!(
            "USER: {audio_segment}can you transcribe the speech into a written format?\n ASSISTANT:"
        );
        let encoding = tokenizer
            .encode(prompt, false)
            .map_err(|e| format!("Tokenize error: {}", e))?;
        let prompt_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        println!(
            "[GRANITE] Prompt tokens: {} ({} × <|audio|> in text)",
            prompt_ids.len(),
            audio_seq_len
        );

        let audio_positions: Vec<usize> = prompt_ids
            .iter()
            .enumerate()
            .filter(|(_, &id)| id == AUDIO_TOKEN_INDEX)
            .map(|(i, _)| i)
            .collect();
        if audio_positions.len() != audio_seq_len {
            return Err(format!(
                "Granite prompt: expected {} <|audio|> token ids ({}), got {} — tokenizer/layout mismatch",
                audio_seq_len,
                AUDIO_MARKER,
                audio_positions.len()
            ));
        }
        let audio_start = audio_positions[0];
        for (k, &p) in audio_positions.iter().enumerate() {
            if p != audio_start + k {
                return Err(format!(
                    "Granite prompt: <|audio|> tokens not consecutive at index {}",
                    k
                ));
            }
        }

        // Embed the full prompt (audio slots use the <|audio|> row from embed_tokens; we overwrite)
        let token_tensor = make_tensor_i64(vec![1, prompt_ids.len()], prompt_ids.clone())?;

        let embed_outputs = embed_session
            .run(ort::inputs!["input_ids" => token_tensor])
            .map_err(|e| format!("Embed run: {}", e))?;

        let text_emb_val = embed_outputs.values().next().ok_or("No embed output")?;
        let (text_shape, text_data) = text_emb_val
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Extract text embeddings: {}", e))?;

        let text_seq_len = ort_positive_dim(text_shape[1], "embed seq")?;
        let text_hidden = ort_positive_dim(text_shape[2], "embed hidden")?;
        if text_hidden != HIDDEN_SIZE {
            return Err(format!(
                "Granite embed_tokens hidden dim is {} but decoder expects {}",
                text_hidden, HIDDEN_SIZE
            ));
        }

        let mut combined_embeddings =
            Array3::from_shape_vec((1, text_seq_len, text_hidden), text_data.to_vec())
                .map_err(|e| format!("Text emb reshape: {}", e))?;

        combined_embeddings
            .slice_mut(s![0, audio_start..audio_start + audio_seq_len, ..])
            .assign(&audio_embeddings.slice(s![0, .., ..]));

        let seq_len = combined_embeddings.shape()[1];
        if seq_len > MAX_GRANITE_DECODER_SEQ_LEN {
            return Err(format!(
                "Granite decoder prefill seq {} exceeds cap {} — refusing run (avoids bogus ORT allocations)",
                seq_len, MAX_GRANITE_DECODER_SEQ_LEN
            ));
        }
        println!("[GRANITE] Combined embeddings: 1 × {} × {}", seq_len, HIDDEN_SIZE);

        let embeds_data: Vec<f32> = combined_embeddings.iter().cloned().collect();
        let attn_data: Vec<i64> = vec![1i64; seq_len];

        let mut decoder_inputs: Vec<(String, ort::value::DynValue)> = Vec::new();

        decoder_inputs.push((
            "inputs_embeds".into(),
            make_tensor_f32(vec![1, seq_len, HIDDEN_SIZE], embeds_data)?,
        ));
        decoder_inputs.push(("attention_mask".into(), make_tensor_i64(vec![1, seq_len], attn_data)?));

        // Empty KV: float16 only for the full FP16 decoder ONNX; INT4 paths use float32.
        for layer in 0..NUM_HIDDEN_LAYERS {
            let (pk, pv) = if self.decoder_io_fp16 {
                (
                    make_empty_kv_f16(NUM_KV_HEADS, HEAD_DIM)?,
                    make_empty_kv_f16(NUM_KV_HEADS, HEAD_DIM)?,
                )
            } else {
                (
                    make_empty_kv_f32(NUM_KV_HEADS, HEAD_DIM)?,
                    make_empty_kv_f32(NUM_KV_HEADS, HEAD_DIM)?,
                )
            };
            decoder_inputs.push((format!("past_key_values.{}.key", layer), pk));
            decoder_inputs.push((format!("past_key_values.{}.value", layer), pv));
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

        // Pre-build KV input name strings — avoids format!() inside the hot decode loop.
        let kv_names: Vec<(String, String)> = (0..NUM_HIDDEN_LAYERS)
            .map(|l| (
                format!("past_key_values.{}.key", l),
                format!("past_key_values.{}.value", l),
            ))
            .collect();

        // Pre-fill an attention mask buffer with 1s — slice into it each step
        // instead of allocating and filling a growing Vec every iteration.
        let max_attn_len = kv_cache_seq_len + MAX_NEW_TOKENS + 1;
        let attn_buf = vec![1i64; max_attn_len];
        let mut current_attn_len = kv_cache_seq_len + 1;

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

            let mut decoder_inputs: Vec<(String, ort::value::DynValue)> = Vec::new();

            decoder_inputs.push((
                "inputs_embeds".into(),
                make_tensor_f32(vec![1, 1, HIDDEN_SIZE], emb_data.to_vec())?,
            ));

            decoder_inputs.push(("attention_mask".into(), make_tensor_i64(vec![1, current_attn_len], attn_buf[..current_attn_len].to_vec())?));
            current_attn_len += 1;

            for layer in 0..NUM_HIDDEN_LAYERS {
                let key_idx = layer * 2;
                let val_idx = layer * 2 + 1;

                let key_data = std::mem::take(&mut kv_cache[key_idx]);
                let val_data = std::mem::take(&mut kv_cache[val_idx]);

                if self.decoder_io_fp16 {
                    let key_f16: Vec<f16> = key_data.iter().map(|x| f16::from_f32(*x)).collect();
                    let val_f16: Vec<f16> = val_data.iter().map(|x| f16::from_f32(*x)).collect();
                    decoder_inputs.push((
                        kv_names[layer].0.clone(),
                        make_tensor_f16(vec![1, NUM_KV_HEADS, kv_cache_seq_len, HEAD_DIM], key_f16)?,
                    ));
                    decoder_inputs.push((
                        kv_names[layer].1.clone(),
                        make_tensor_f16(vec![1, NUM_KV_HEADS, kv_cache_seq_len, HEAD_DIM], val_f16)?,
                    ));
                } else {
                    decoder_inputs.push((
                        kv_names[layer].0.clone(),
                        make_tensor_f32(vec![1, NUM_KV_HEADS, kv_cache_seq_len, HEAD_DIM], key_data)?,
                    ));
                    decoder_inputs.push((
                        kv_names[layer].1.clone(),
                        make_tensor_f32(vec![1, NUM_KV_HEADS, kv_cache_seq_len, HEAD_DIM], val_data)?,
                    ));
                }
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

pub(crate) fn resolve_granite_model_dir(models_dir: &Path, model_id: Option<&str>) -> Result<PathBuf, String> {
    let dir = match model_id {
        None => models_dir.join("granite-speech-1b"),
        Some(id) => {
            let pb = PathBuf::from(id);
            if pb.is_absolute() {
                pb
            } else {
                match id {
                    "granite-speech-1b-fp16" => models_dir.join("granite-speech-1b-fp16"),
                    "granite-speech-1b" => models_dir.join("granite-speech-1b"),
                    other => {
                        if other.contains('/') || other.contains('\\') {
                            return Err(format!("Invalid granite model id: {}", other));
                        }
                        models_dir.join(other)
                    }
                }
            }
        }
    };
    if !dir.exists() {
        return Err(format!(
            "Granite Speech model not found at {}. Download it from Settings > Download Manager.",
            dir.display()
        ));
    }
    Ok(dir)
}

/// Logical model id for a resolved on-disk Granite directory (INT4 / q4f16 vs FP16 bundle).
pub(crate) fn granite_logical_model_id_for_dir(model_dir: &Path) -> String {
    if granite_fp16_bundle_ready(model_dir) {
        "granite-speech-1b-fp16".to_string()
    } else {
        "granite-speech-1b".to_string()
    }
}

pub(crate) fn granite_int4_bundle_ready(dir: &Path) -> bool {
    if !dir.is_dir() || !dir.join("tokenizer.json").exists() {
        return false;
    }
    let q4 = dir.join("audio_encoder_q4.onnx").exists()
        && dir.join("embed_tokens_q4.onnx").exists()
        && dir.join("decoder_model_merged_q4.onnx").exists();
    let q4f16 = dir.join("audio_encoder_q4f16.onnx").exists()
        && dir.join("embed_tokens_q4f16.onnx").exists()
        && dir.join("decoder_model_merged_q4f16.onnx").exists();
    q4 || q4f16
}

pub(crate) fn granite_fp16_bundle_ready(dir: &Path) -> bool {
    const FILES: &[&str] = &[
        "audio_encoder_fp16.onnx",
        "audio_encoder_fp16.onnx_data",
        "embed_tokens_fp16.onnx",
        "embed_tokens_fp16.onnx_data",
        "decoder_model_merged_fp16.onnx",
        "decoder_model_merged_fp16.onnx_data",
        "decoder_model_merged_fp16.onnx_data_1",
        "tokenizer.json",
    ];
    dir.is_dir() && FILES.iter().all(|f| dir.join(f).exists())
}
