// granite.rs — IBM Granite Speech NAR ONNX manager.
//
// This intentionally exposes Cohere-compatible type/function names through
// cohere.rs so the existing frontend IPC can be reused while the underlying
// model slot is migrated from Cohere Transcribe to Granite Speech NAR.

use ndarray::Array2;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::builder::SessionBuilder;
use ort::session::Session;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use serde::Deserialize;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::utils::strip_whitelisted_sound_captions;

const MODEL_ID_CUDA: &str = "granite-speech-4.1-2b-nar-cuda";
const DEFAULT_MODEL_DIR: &str = MODEL_ID_CUDA;
const MODEL_ID_UNIVERSAL: &str = MODEL_ID_CUDA;
const MODEL_ID_PORTABLE: &str = "granite-speech-4.1-2b-nar-portable";
const BLANK_TOKEN_ID: i64 = 100_257;
const BPE_POOLING_WINDOW: usize = 4;
const PROJECTOR_DOWNSAMPLE_RATE: usize = 5;
const MIN_EDIT_SEQUENCE_LENGTH: usize = 8;
const VOCAB_SIZE: usize = 100_352;
const HIDDEN_SIZE: usize = 2048;
const EXPORT_FRAMES: usize = 800;
const TEXT_EMBEDDING_MULTIPLIER: f32 = 12.0;

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn preload_granite_cuda_dlls() {
    use std::env;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn LoadLibraryW(lp_lib_file_name: *const u16) -> *mut c_void;
        fn GetLastError() -> u32;
    }

    fn load_dll(path: &Path) {
        if !path.exists() {
            println!(
                "[GRANITE] CUDA preload skipped missing DLL: {}",
                path.display()
            );
            return;
        }
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe { LoadLibraryW(wide_path.as_ptr()) };
        if handle.is_null() {
            let error = unsafe { GetLastError() };
            eprintln!(
                "[GRANITE] CUDA preload failed for {}: Win32 error {}",
                path.display(),
                error
            );
        } else {
            println!("[GRANITE] CUDA preload loaded: {}", path.display());
        }
    }

    let cuda_bin = env::var_os("CUDA_PATH")
        .map(PathBuf::from)
        .map(|path| path.join("bin"))
        .unwrap_or_else(|| {
            PathBuf::from(r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.6\bin")
        });
    let cudnn_bin = env::var_os("TAURSCRIBE_CUDNN_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\NVIDIA\CUDNN\v9.20\bin\12.9\x64"));

    for dll in [
        "cudart64_12.dll",
        "cublas64_12.dll",
        "cublasLt64_12.dll",
        "cufft64_11.dll",
    ] {
        load_dll(&cuda_bin.join(dll));
    }
    for dll in [
        "cudnn64_9.dll",
        "cudnn_adv64_9.dll",
        "cudnn_cnn64_9.dll",
        "cudnn_engines_precompiled64_9.dll",
        "cudnn_engines_runtime_compiled64_9.dll",
        "cudnn_graph64_9.dll",
        "cudnn_heuristic64_9.dll",
        "cudnn_ops64_9.dll",
    ] {
        load_dll(&cudnn_bin.join(dll));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraniteBackendRequest {
    Auto,
    Cpu,
    Cuda,
    DirectML,
    CoreML,
    /// Native MLX on Apple silicon — needs the safetensors checkpoint, not the
    /// ONNX bundle.
    Mlx,
}

fn granite_backend_request(force_cpu: bool, model_dir: &Path) -> GraniteBackendRequest {
    if force_cpu {
        return GraniteBackendRequest::Cpu;
    }
    match std::env::var("TAURSCRIBE_GRANITE_BACKEND")
        .or_else(|_| std::env::var("TAURSCRIBE_COHERE_BACKEND"))
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("cpu") => GraniteBackendRequest::Cpu,
        Some("cuda") | Some("gpu") => GraniteBackendRequest::Cuda,
        Some("directml") | Some("dml") => GraniteBackendRequest::DirectML,
        Some("coreml") | Some("apple") => GraniteBackendRequest::CoreML,
        Some("mlx") | Some("metal") => GraniteBackendRequest::Mlx,
        // The portable Windows bundle uses an automatic DirectML -> CPU route.
        // Other platforms currently use the portable CPU path.
        _ if is_portable_granite_dir(model_dir) => portable_default_backend_request(),
        _ => GraniteBackendRequest::Auto,
    }
}

#[cfg(target_os = "windows")]
fn portable_default_backend_request() -> GraniteBackendRequest {
    GraniteBackendRequest::Auto
}

// macOS defaults to CPU on purpose. The CoreML execution provider is wired up
// below and selectable with TAURSCRIBE_GRANITE_BACKEND=coreml, but it measured
// *slower* than multi-threaded CPU on an M3 over a matched 50-utterance
// LibriSpeech subset: mean 11.98s vs 8.50s (RTF 1.715 vs 1.218), identical WER.
// Granite's dynamic editor cannot stay on CoreML, so the hybrid pays a
// CoreML<->CPU transfer on every chunk that outweighs the GPU win on the fixed
// encoder. The real Apple speedup is the MLX port in scripts/granite_mlx.
#[cfg(target_os = "macos")]
fn portable_default_backend_request() -> GraniteBackendRequest {
    GraniteBackendRequest::Cpu
}

/// A Granite bundle carrying `model.safetensors` can run on native MLX, which
/// measured RTF 0.069 against 1.218 for the ONNX CPU path on an M3.
#[cfg(target_os = "macos")]
fn mlx_checkpoint_present(model_dir: &Path) -> bool {
    model_dir.join("model.safetensors").exists() && model_dir.join("config.json").exists()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn portable_default_backend_request() -> GraniteBackendRequest {
    GraniteBackendRequest::Cpu
}

/// Intra-op thread count for Granite CPU sessions. The low-RAM defaults pin
/// every ORT session to one thread, which starves the 2B editor/encoder on
/// multi-core machines (measured 15s -> 3.3s per chunk at 8 threads on a
/// Ryzen 7 8845HS). Threads cost only stack memory, not weight memory.
fn granite_cpu_intra_threads() -> usize {
    if let Some(n) = std::env::var("TAURSCRIBE_GRANITE_CPU_THREADS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        if n >= 1 {
            return n.min(16);
        }
    }
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (logical / 2).clamp(2, 8)
}

fn is_portable_granite_dir(model_dir: &Path) -> bool {
    model_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == MODEL_ID_PORTABLE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraniteOrtMode {
    LowRam,
    Perf,
}

fn granite_ort_mode() -> GraniteOrtMode {
    match std::env::var("TAURSCRIBE_GRANITE_ORT_MODE")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("perf") | Some("performance") | Some("speed") => GraniteOrtMode::Perf,
        _ => GraniteOrtMode::LowRam,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum GpuBackend {
    Cuda,
    DirectML,
    CoreML,
    Mlx,
    Cpu,
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Cuda => write!(f, "CUDA"),
            GpuBackend::DirectML => write!(f, "DirectML"),
            GpuBackend::CoreML => write!(f, "CoreML (Apple GPU/Neural Engine)"),
            GpuBackend::Mlx => write!(f, "MLX (Apple GPU)"),
            GpuBackend::Cpu => write!(f, "CPU"),
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

struct GraniteRuntime {
    encoder: Session,
    projector: Session,
    embed_tokens: Session,
    editor: Session,
}

pub struct CohereManager {
    runtime: Option<GraniteRuntime>,
    #[cfg(target_os = "macos")]
    mlx: Option<crate::granite_mlx::GraniteMlx>,
    tokenizer: Option<tokenizers::Tokenizer>,
    backend: GpuBackend,
    model_name: Option<String>,
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>,
}

impl CohereManager {
    pub fn new() -> Self {
        Self {
            runtime: None,
            #[cfg(target_os = "macos")]
            mlx: None,
            tokenizer: None,
            backend: GpuBackend::Cpu,
            model_name: None,
            resampler: None,
        }
    }

    pub fn get_status(&self) -> CohereStatus {
        CohereStatus {
            #[cfg(target_os = "macos")]
            loaded: self.runtime.is_some() || self.mlx.is_some(),
            #[cfg(not(target_os = "macos"))]
            loaded: self.runtime.is_some(),
            model_id: self.model_name.clone(),
            backend: self.backend.to_string(),
            gpu_only: matches!(
                self.backend,
                GpuBackend::Cuda | GpuBackend::DirectML | GpuBackend::CoreML | GpuBackend::Mlx
            ),
        }
    }

    fn load_tokenizer(&mut self, model_dir: &Path) -> Result<(), String> {
        let path = model_dir.join("tokenizer.json");
        let tokenizer = tokenizers::Tokenizer::from_file(&path)
            .map_err(|e| format!("Load tokenizer {}: {e}", path.display()))?;
        self.tokenizer = Some(tokenizer);
        Ok(())
    }

    pub fn unload(&mut self) {
        #[cfg(target_os = "macos")]
        if self.mlx.take().is_some() {
            println!("[GRANITE] Unloading MLX model...");
            self.tokenizer = None;
            self.model_name = None;
            self.resampler = None;
            crate::memory::trim_process_memory();
        }
        if self.runtime.is_some() {
            println!("[GRANITE] Unloading model...");
            if let Some(runtime) = self.runtime.as_mut() {
                end_granite_profiling(runtime);
            }
            self.runtime = None;
            self.tokenizer = None;
            self.model_name = None;
            self.resampler = None;
            crate::memory::trim_process_memory();
            println!("[GRANITE] Model unloaded");
        }
    }

    pub fn initialize(
        &mut self,
        model_id: Option<&str>,
        force_cpu: bool,
    ) -> Result<String, String> {
        let models_dir = crate::utils::get_models_dir()?;
        let model_dir = resolve_cohere_model_dir(&models_dir, model_id)?;

        // Apple silicon: if the bundle carries the safetensors checkpoint, run it
        // natively on MLX. That path has no ONNX graphs, so it is decided before
        // the ONNX bundle check below.
        #[cfg(target_os = "macos")]
        {
            let explicit = matches!(
                granite_backend_request(force_cpu, &model_dir),
                GraniteBackendRequest::Mlx
            );
            if (explicit || mlx_checkpoint_present(&model_dir)) && !force_cpu {
                if self.runtime.is_some() || self.mlx.is_some() {
                    self.unload();
                }
                let started = std::time::Instant::now();
                match crate::granite_mlx::GraniteMlx::load(&model_dir, mlx_rs::Dtype::Float16) {
                    Ok(engine) => {
                        self.load_tokenizer(&model_dir)?;
                        self.mlx = Some(engine);
                        self.backend = GpuBackend::Mlx;
                        self.model_name = model_dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(str::to_string);
                        println!(
                            "[GRANITE] MLX backend ready in {:.2}s ({})",
                            started.elapsed().as_secs_f32(),
                            model_dir.display()
                        );
                        self.warm_up_if_needed();
                        return Ok(format!("Granite Speech NAR loaded ({})", self.backend));
                    }
                    Err(err) if explicit => return Err(format!("MLX init failed: {err}")),
                    Err(err) => {
                        eprintln!("[GRANITE] MLX init failed; falling back to ONNX. {err}")
                    }
                }
            }
        }

        if !cohere_onnx_bundle_ready(&model_dir) {
            return Err(format!(
                "Granite ONNX bundle not found in {}. Download/install Granite Speech NAR from Settings > Models.",
                model_dir.display()
            ));
        }
        if self.runtime.is_some() {
            self.unload();
        }

        let request = granite_backend_request(force_cpu, &model_dir);
        let ort_mode = granite_ort_mode();
        println!(
            "[GRANITE] initialize: model_dir={} request={:?} ort_mode={:?}",
            model_dir.display(),
            request,
            ort_mode
        );
        crate::memory::maybe_log_process_memory("granite before initialize");

        let graph_paths = GraniteGraphPaths::new(&model_dir);
        let (backend, runtime) = match request {
            // Reachable only when MLX was asked for but is unavailable on this
            // build; macOS returns above once the native engine is up.
            GraniteBackendRequest::Mlx => {
                eprintln!("[GRANITE] MLX backend unavailable here; using multi-threaded CPU.");
                (GpuBackend::Cpu, self.create_runtime_cpu(&graph_paths)?)
            }
            GraniteBackendRequest::Cpu => (GpuBackend::Cpu, self.create_runtime_cpu(&graph_paths)?),
            GraniteBackendRequest::Cuda => {
                (GpuBackend::Cuda, self.create_runtime_cuda(&graph_paths)?)
            }
            GraniteBackendRequest::DirectML => (
                GpuBackend::DirectML,
                self.create_runtime_directml(&graph_paths)?,
            ),
            GraniteBackendRequest::CoreML => match self.create_runtime_coreml(&graph_paths) {
                Ok(rt) => (GpuBackend::CoreML, rt),
                Err(coreml_err) if !force_cpu => {
                    eprintln!(
                        "[GRANITE] CoreML init failed; falling back to multi-threaded CPU. {coreml_err}"
                    );
                    (GpuBackend::Cpu, self.create_runtime_cpu(&graph_paths)?)
                }
                Err(coreml_err) => return Err(coreml_err),
            },
            GraniteBackendRequest::Auto if is_portable_granite_dir(&model_dir) => {
                match self.create_runtime_directml(&graph_paths) {
                    Ok(rt) => {
                        println!(
                            "[GRANITE] Portable DirectML mode active: encoder, projector, token embedder, and editor loaded on DirectML"
                        );
                        (GpuBackend::DirectML, rt)
                    }
                    Err(dml_err) => {
                        eprintln!(
                            "[GRANITE] Portable DirectML init failed; falling back to multi-threaded CPU. {dml_err}"
                        );
                        (GpuBackend::Cpu, self.create_runtime_cpu(&graph_paths)?)
                    }
                }
            }
            // No CUDA on macOS, and CoreML benchmarked slower than CPU for this
            // graph, so an unqualified Auto stays on multi-threaded CPU here.
            #[cfg(target_os = "macos")]
            GraniteBackendRequest::Auto => {
                (GpuBackend::Cpu, self.create_runtime_cpu(&graph_paths)?)
            }
            #[cfg(not(target_os = "macos"))]
            GraniteBackendRequest::Auto => match self.create_runtime_cuda(&graph_paths) {
                Ok(rt) => {
                    println!(
                        "[GRANITE] CUDA mode active: encoder, projector, token embedder, and editor loaded on CUDA"
                    );
                    (GpuBackend::Cuda, rt)
                }
                Err(cuda_err) => {
                    eprintln!("[GRANITE] CUDA init failed; trying DirectML. {cuda_err}");
                    match self.create_runtime_directml(&graph_paths) {
                        Ok(rt) => {
                            println!(
                                "[GRANITE] DirectML mode active: all Granite ONNX graphs loaded on DirectML"
                            );
                            (GpuBackend::DirectML, rt)
                        }
                        Err(dml_err) => {
                            eprintln!(
                                "[GRANITE] DirectML init failed; falling back to CPU. {dml_err}"
                            );
                            (GpuBackend::Cpu, self.create_runtime_cpu(&graph_paths)?)
                        }
                    }
                }
            },
        };

        self.load_tokenizer(&model_dir)?;

        self.runtime = Some(runtime);
        self.backend = backend;
        self.model_name = Some(cohere_logical_model_id_for_dir(&model_dir));
        self.warm_up_if_needed();
        crate::memory::maybe_log_process_memory("granite after initialize");
        Ok(format!("Granite Speech NAR loaded ({})", self.backend))
    }

    pub fn transcribe_chunk(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<String, String> {
        match self.transcribe_chunk_loaded(samples, sample_rate) {
            Ok(text) => Ok(text),
            Err(err)
                if matches!(self.backend, GpuBackend::DirectML | GpuBackend::CoreML | GpuBackend::Mlx)
                    && std::env::var("TAURSCRIBE_GRANITE_DISABLE_DML_FALLBACK")
                        .ok()
                        .as_deref()
                        != Some("1") =>
            {
                eprintln!(
                    "[GRANITE] DirectML inference failed; falling back to CPU and retrying. {err}"
                );
                let model_id = self.model_name.clone();
                self.unload();
                self.initialize(model_id.as_deref(), true)
                    .map_err(|cpu_err| format!("{err}; CPU fallback init failed: {cpu_err}"))?;
                self.transcribe_chunk_loaded(samples, sample_rate)
                    .map_err(|cpu_err| format!("{err}; CPU fallback transcribe failed: {cpu_err}"))
            }
            Err(err) => Err(err),
        }
    }

    fn transcribe_chunk_loaded(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<String, String> {
        let audio: Cow<[f32]> = if sample_rate != 16_000 {
            Cow::Owned(self.resample(samples, sample_rate)?)
        } else {
            Cow::Borrowed(samples)
        };

        let features = crate::granite_features::extract_features(&audio);

        #[cfg(target_os = "macos")]
        if let Some(engine) = self.mlx.as_ref() {
            let frames = features.nrows();
            if frames == 0 {
                return Ok(String::new());
            }
            let tokenizer = self
                .tokenizer
                .as_ref()
                .ok_or("Granite tokenizer not loaded")?;
            // MLX has no fixed-shape bucket, so the true frame count is used.
            let flat: Vec<f32> = features.iter().copied().collect();
            let ids = engine.transcribe_features(&flat, frames)?;
            let text = tokenizer
                .decode(&ids, true)
                .map_err(|e| format!("Granite tokenizer decode: {e}"))?;
            return Ok(strip_whitelisted_sound_captions(&text).trim().to_string());
        }

        let valid_frames = features.nrows().min(EXPORT_FRAMES);
        if valid_frames == 0 {
            return Ok(String::new());
        }

        let runtime = self.runtime.as_mut().ok_or("Granite runtime not loaded")?;
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or("Granite tokenizer not loaded")?;

        let feature_data = pad_features_to_export_bucket(&features, valid_frames)?;
        let input_features = make_tensor_f32(vec![1, EXPORT_FRAMES, 160], feature_data)?;

        let (encoder_token_ids, multilayer) = {
            let enc_outputs = runtime
                .encoder
                .run(ort::inputs!["input_features" => input_features])
                .map_err(|e| format!("Granite encoder run: {e}"))?;

            let (bpe_shape, bpe_logits) = extract_named_f32_ref(&enc_outputs, "bpe_logits")?;
            let pooled_len = ceil_div(valid_frames, BPE_POOLING_WINDOW)
                .min(*bpe_shape.get(1).ok_or("Bad bpe_logits shape")?);
            let encoder_token_ids =
                ctc_collapse_from_logits(bpe_logits, pooled_len, VOCAB_SIZE, BLANK_TOKEN_ID)?;

            let (_, h4) = extract_named_f32_ref(&enc_outputs, "hidden_4")?;
            let (_, h8) = extract_named_f32_ref(&enc_outputs, "hidden_8")?;
            let (_, h12) = extract_named_f32_ref(&enc_outputs, "hidden_12")?;
            let (_, hlast) = extract_named_f32_ref(&enc_outputs, "hidden_last")?;
            let multilayer = concat_encoder_layers(EXPORT_FRAMES, [h4, h8, h12, hlast])?;

            (encoder_token_ids, multilayer)
        };

        let projector_input = make_tensor_f32(vec![1, EXPORT_FRAMES, 4096], multilayer)?;

        let slotted = add_insertion_slots(&encoder_token_ids);
        let text_len = slotted.len();
        let token_tensor = make_tensor_i64(vec![text_len], slotted)?;

        let (audio_tokens, mut editor_input) = {
            let projector_outputs = runtime
                .projector
                .run(ort::inputs!["multilayer_features" => projector_input])
                .map_err(|e| format!("Granite projector run: {e}"))?;
            let (audio_shape, audio_embeds_all) =
                extract_named_f32_ref(&projector_outputs, "audio_embeds")?;
            let available_audio_tokens = *audio_shape.get(1).ok_or("Bad audio_embeds shape")?;
            let audio_tokens =
                (valid_frames / PROJECTOR_DOWNSAMPLE_RATE).min(available_audio_tokens);
            let audio_values = audio_tokens * HIDDEN_SIZE;
            if audio_values > audio_embeds_all.len() {
                return Err(format!(
                    "Granite audio embeddings too short: need {audio_values}, got {}",
                    audio_embeds_all.len()
                ));
            }

            let mut editor_input = Vec::with_capacity((audio_tokens + text_len) * HIDDEN_SIZE);
            editor_input.extend(
                audio_embeds_all[..audio_values]
                    .iter()
                    .map(|value| *value / TEXT_EMBEDDING_MULTIPLIER),
            );
            (audio_tokens, editor_input)
        };

        {
            let embed_outputs = runtime
                .embed_tokens
                .run(ort::inputs!["token_ids" => token_tensor])
                .map_err(|e| format!("Granite token embedding run: {e}"))?;
            let (_text_shape, text_embeds) = extract_named_f32_ref(&embed_outputs, "text_embeds")?;
            let text_values = text_len * HIDDEN_SIZE;
            if text_values > text_embeds.len() {
                return Err(format!(
                    "Granite text embeddings too short: need {text_values}, got {}",
                    text_embeds.len()
                ));
            }
            editor_input.extend_from_slice(&text_embeds[..text_values]);
        }

        let sequence = audio_tokens + text_len;
        let inputs_embeds = make_tensor_f32(vec![1, sequence, HIDDEN_SIZE], editor_input)?;
        let position_ids =
            make_tensor_i64(vec![1, sequence], (0..sequence as i64).collect::<Vec<_>>())?;

        let final_ids = {
            let editor_outputs = runtime
                .editor
                .run(ort::inputs![
                    "inputs_embeds" => inputs_embeds,
                    "position_ids" => position_ids,
                ])
                .map_err(|e| format!("Granite editor run: {e}"))?;
            if let Some((_ids_shape, token_ids)) =
                try_extract_named_i64_ref(&editor_outputs, "token_ids")?
            {
                let text_ids_start = audio_tokens;
                let text_ids_end = text_ids_start + text_len;
                if text_ids_end > token_ids.len() {
                    return Err(format!(
                        "Granite editor token_ids too short: need {text_ids_end}, got {}",
                        token_ids.len()
                    ));
                }
                ctc_collapse_from_ids(&token_ids[text_ids_start..text_ids_end], BLANK_TOKEN_ID)
            } else {
                let (_logit_shape, logits) = extract_named_f32_ref(&editor_outputs, "logits")?;
                let text_logits_start = audio_tokens * VOCAB_SIZE;
                let text_logits_end = text_logits_start + text_len * VOCAB_SIZE;
                if text_logits_end > logits.len() {
                    return Err(format!(
                        "Granite editor logits too short: need {text_logits_end}, got {}",
                        logits.len()
                    ));
                }
                ctc_collapse_from_logits(
                    &logits[text_logits_start..text_logits_end],
                    text_len,
                    VOCAB_SIZE,
                    BLANK_TOKEN_ID,
                )?
            }
        };
        let token_ids: Vec<u32> = final_ids
            .into_iter()
            .filter_map(|id| u32::try_from(id).ok())
            .collect();
        let text = tokenizer
            .decode(&token_ids, true)
            .map_err(|e| format!("Granite tokenizer decode: {e}"))?;
        Ok(strip_whitelisted_sound_captions(&text).trim().to_string())
    }

    fn warm_up_if_needed(&mut self) {
        let label = match self.backend {
            GpuBackend::DirectML => "DirectML",
            // MLX compiles its Metal kernels on first use — ~13s on an M3 — so
            // pay it at load instead of on the user's first dictation.
            GpuBackend::Mlx => "MLX",
            GpuBackend::CoreML => "CoreML",
            GpuBackend::Cuda if granite_ort_mode() == GraniteOrtMode::Perf => "CUDA perf",
            _ => return,
        };
        if std::env::var("TAURSCRIBE_GRANITE_WARMUP").ok().as_deref() == Some("0") {
            return;
        }
        let silence = vec![0.0_f32; 16_000 * 4];
        let start = std::time::Instant::now();
        match self.transcribe_chunk_loaded(&silence, 16_000) {
            Ok(_) => println!(
                "[GRANITE] {label} warmup completed in {:.3}s",
                start.elapsed().as_secs_f32()
            ),
            Err(err) => eprintln!("[GRANITE] {label} warmup failed: {err}"),
        }
    }

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
                16_000.0 / sample_rate as f64,
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

    fn create_runtime_cpu(&self, paths: &GraniteGraphPaths) -> Result<GraniteRuntime, String> {
        Ok(GraniteRuntime {
            encoder: self.create_session_cpu(&paths.encoder)?,
            projector: self.create_session_cpu(&paths.projector)?,
            embed_tokens: self.create_session_cpu(&paths.embed_tokens)?,
            editor: self.create_session_cpu(&paths.editor)?,
        })
    }

    fn create_runtime_directml(&self, paths: &GraniteGraphPaths) -> Result<GraniteRuntime, String> {
        #[cfg(target_os = "windows")]
        {
            // Bundles whose manifest declares `encoder_dml_safe` carry the
            // DML-static encoder (rank-3 attention MatMuls + baked shape
            // chains) and run fully on DirectML. Older bundles keep the
            // CPU-encoder hybrid because their encoder graph crashes the
            // DirectML provider. Env var wins in both directions.
            let encoder_dml_safe = paths.encoder_dml_safe();
            let cpu_encoder = match std::env::var("TAURSCRIBE_GRANITE_DML_CPU_ENCODER")
                .ok()
                .as_deref()
            {
                Some("0") => false,
                Some(_) => true,
                None => !encoder_dml_safe,
            };
            let cpu_editor = std::env::var("TAURSCRIBE_GRANITE_DML_CPU_EDITOR")
                .ok()
                .as_deref()
                == Some("1");
            if cpu_encoder {
                eprintln!(
                    "[GRANITE] DirectML hybrid mode: encoder on CPU, projector/embed_tokens on DirectML"
                );
            } else if encoder_dml_safe {
                eprintln!("[GRANITE] DirectML full mode: bundle declares a DML-safe encoder");
            } else {
                eprintln!(
                    "[GRANITE] DirectML full-encoder mode requested; this may crash on bundles without a DML-safe encoder"
                );
            }
            if cpu_editor {
                let encoder_label = if cpu_encoder { "CPU" } else { "DirectML" };
                eprintln!(
                    "[GRANITE] DirectML hybrid mode: encoder on {encoder_label}, projector/embed_tokens on DirectML, editor on CPU"
                );
            }
            Ok(GraniteRuntime {
                encoder: if cpu_encoder {
                    self.create_session_cpu(&paths.encoder)?
                } else {
                    self.create_session_directml(&paths.encoder)?
                },
                projector: self.create_session_directml(&paths.projector)?,
                embed_tokens: self.create_session_directml(&paths.embed_tokens)?,
                editor: if cpu_editor {
                    self.create_session_cpu(&paths.editor)?
                } else {
                    self.create_session_directml(&paths.editor)?
                },
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = paths;
            Err("DirectML is only available on Windows".to_string())
        }
    }

    fn create_runtime_cuda(&self, paths: &GraniteGraphPaths) -> Result<GraniteRuntime, String> {
        #[cfg(any(
            not(target_os = "windows"),
            all(target_os = "windows", target_arch = "x86_64")
        ))]
        {
            Ok(GraniteRuntime {
                encoder: self.create_session_cuda(&paths.encoder)?,
                projector: self.create_session_cuda(&paths.projector)?,
                embed_tokens: self.create_session_cuda(&paths.embed_tokens)?,
                editor: self.create_session_cuda(&paths.editor)?,
            })
        }
        #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
        {
            let _ = paths;
            Err("CUDA is not available on Windows ARM64".to_string())
        }
    }

    fn create_runtime_coreml(&self, paths: &GraniteGraphPaths) -> Result<GraniteRuntime, String> {
        #[cfg(target_os = "macos")]
        {
            println!(
                "[GRANITE] CoreML hybrid: fixed-shape encoder/projector on Apple GPU/Neural Engine; dynamic token embedder/editor on CPU"
            );
            Ok(GraniteRuntime {
                encoder: self.create_session_coreml(&paths.encoder)?,
                projector: self.create_session_coreml(&paths.projector)?,
                // CoreML's dynamic MLProgram execution plan for the editor can
                // take minutes to compile or stall. Keep the small gather and
                // dynamic editor on ORT CPU; the fixed, compute-heavy audio
                // path remains accelerated on Apple hardware.
                embed_tokens: self.create_session_cpu(&paths.embed_tokens)?,
                editor: self.create_session_cpu(&paths.editor)?,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = paths;
            Err("CoreML is only available on macOS".to_string())
        }
    }

    #[cfg(target_os = "macos")]
    fn create_session_coreml(&self, path: &Path) -> Result<Session, String> {
        use ort::ep::coreml::{ComputeUnits, ModelFormat, SpecializationStrategy};

        let cache_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("coreml-cache");
        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| format!("Create CoreML cache {}: {e}", cache_dir.display()))?;

        let profile_compute_plan = std::env::var("TAURSCRIBE_GRANITE_COREML_PROFILE")
            .ok()
            .as_deref()
            == Some("1");
        let low_precision_gpu = std::env::var("TAURSCRIBE_GRANITE_COREML_FP16_ACCUM")
            .ok()
            .as_deref()
            != Some("0");
        let coreml = ort::ep::CoreML::default()
            .with_model_format(ModelFormat::MLProgram)
            .with_compute_units(ComputeUnits::All)
            .with_static_input_shapes(false)
            .with_specialization_strategy(SpecializationStrategy::FastPrediction)
            .with_low_precision_accumulation_on_gpu(low_precision_gpu)
            .with_profile_compute_plan(profile_compute_plan)
            .with_model_cache_dir(cache_dir.to_string_lossy())
            .build()
            .error_on_failure();

        let builder = Session::builder()
            .map_err(|e| format!("ORT builder: {e}"))?
            .with_execution_providers([coreml])
            .map_err(|e| format!("CoreML EP: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("CoreML opt level: {e}"))?;
        let builder = maybe_enable_granite_profiling(builder, "coreml", path)?;
        let mut builder =
            crate::ort_session::configure_low_ram_session_builder(builder, "granite-coreml")?;
        builder
            .commit_from_file(path)
            .map_err(|e| format!("CoreML session load {}: {e}", path.display()))
    }

    fn create_session_cpu(&self, path: &Path) -> Result<Session, String> {
        let builder = Session::builder()
            .map_err(|e| format!("ORT builder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("CPU opt level: {e}"))?;
        let builder = maybe_enable_granite_profiling(builder, "cpu", path)?;
        let builder =
            crate::ort_session::configure_low_ram_session_builder(builder, "granite-cpu")?;
        // Re-raise the intra-op thread count after the low-RAM defaults: Granite's
        // 2B graphs are matmul-bound and unusable single-threaded on CPU.
        let threads = granite_cpu_intra_threads();
        let mut builder = builder
            .with_intra_threads(threads)
            .map_err(|e| format!("CPU intra threads: {e}"))?;
        println!(
            "[GRANITE] CPU session {} with {} intra-op threads",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("graph"),
            threads
        );
        builder
            .commit_from_file(path)
            .map_err(|e| format!("CPU session load {}: {e}", path.display()))
    }

    #[cfg(target_os = "windows")]
    fn create_session_directml(&self, path: &Path) -> Result<Session, String> {
        let mut dml = ort::ep::DirectML::default();
        if let Ok(raw_device_id) = std::env::var("TAURSCRIBE_GRANITE_DML_DEVICE_ID") {
            let device_id = raw_device_id.trim().parse::<i32>().map_err(|e| {
                format!("DirectML device id must be an integer, got {raw_device_id}: {e}")
            })?;
            dml = dml.with_device_id(device_id);
        }
        let dml_optimization_level =
            if std::env::var("TAURSCRIBE_GRANITE_DML_OPT").ok().as_deref() == Some("all") {
                GraphOptimizationLevel::Level3
            } else {
                GraphOptimizationLevel::Disable
            };
        let mut builder = Session::builder()
            .map_err(|e| format!("ORT builder: {e}"))?
            .with_execution_providers([dml.build()])
            .map_err(|e| format!("DirectML EP: {e}"))?
            .with_optimization_level(dml_optimization_level)
            .map_err(|e| format!("DirectML opt level: {e}"))?;
        if std::env::var("TAURSCRIBE_GRANITE_DML_STATIC_DIMS")
            .ok()
            .as_deref()
            == Some("1")
            && path.file_name().and_then(|name| name.to_str()) == Some("encoder.onnx")
        {
            for (name, size) in [
                ("Addbpe_logits_dim_0", 1),
                ("Addbpe_logits_dim_1", 200),
                ("LayerNormalizationhidden_4_dim_0", 1),
                ("LayerNormalizationhidden_4_dim_1", 800),
                ("Addhidden_8_dim_0", 1),
                ("Addhidden_8_dim_1", 800),
                ("LayerNormalizationhidden_12_dim_0", 1),
                ("LayerNormalizationhidden_12_dim_1", 800),
                ("Casthidden_last_dim_0", 1),
                ("Casthidden_last_dim_1", 800),
            ] {
                builder = builder
                    .with_dimension_override(name, size)
                    .map_err(|e| format!("DirectML dimension override {name}: {e}"))?;
            }
        }
        let builder = maybe_enable_granite_profiling(builder, "directml", path)?;
        let mut builder =
            crate::ort_session::configure_low_ram_session_builder(builder, "granite-directml")?;
        builder
            .commit_from_file(path)
            .map_err(|e| format!("DirectML session load {}: {e}", path.display()))
    }

    #[cfg(any(
        not(target_os = "windows"),
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    fn create_session_cuda(&self, path: &Path) -> Result<Session, String> {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        preload_granite_cuda_dlls();

        let builder = Session::builder()
            .map_err(|e| format!("ORT builder: {e}"))?
            .with_execution_providers([match granite_ort_mode() {
                GraniteOrtMode::LowRam => {
                    crate::ort_session::build_low_ram_cuda_execution_provider()
                        .build()
                        .error_on_failure()
                }
                GraniteOrtMode::Perf => crate::ort_session::build_perf_cuda_execution_provider()
                    .build()
                    .error_on_failure(),
            }])
            .map_err(|e| format!("CUDA EP: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("CUDA opt level: {e}"))?;
        let builder = maybe_enable_granite_profiling(builder, "cuda", path)?;
        let mut builder = match granite_ort_mode() {
            GraniteOrtMode::LowRam => {
                crate::ort_session::configure_low_ram_session_builder(builder, "granite-cuda")?
            }
            GraniteOrtMode::Perf => {
                crate::ort_session::configure_perf_session_builder(builder, "granite-cuda")?
            }
        };
        builder
            .commit_from_file(path)
            .map_err(|e| format!("CUDA session load {}: {e}", path.display()))
    }
}

fn maybe_enable_granite_profiling(
    builder: SessionBuilder,
    backend: &str,
    model_path: &Path,
) -> Result<SessionBuilder, String> {
    let profile_dir = match std::env::var_os("TAURSCRIBE_GRANITE_PROFILE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => return Ok(builder),
    };
    std::fs::create_dir_all(&profile_dir).map_err(|e| {
        format!(
            "Create Granite ORT profile dir {}: {e}",
            profile_dir.display()
        )
    })?;
    let stem = model_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("graph");
    let prefix = profile_dir.join(format!("granite_{backend}_{stem}"));
    builder
        .with_profiling(&prefix)
        .map_err(|e| format!("Enable Granite ORT profiling {}: {e}", prefix.display()))
}

fn end_granite_profiling(runtime: &mut GraniteRuntime) {
    if std::env::var_os("TAURSCRIBE_GRANITE_PROFILE_DIR").is_none() {
        return;
    }
    for (label, session) in [
        ("encoder", &mut runtime.encoder),
        ("projector", &mut runtime.projector),
        ("embed_tokens", &mut runtime.embed_tokens),
        ("editor", &mut runtime.editor),
    ] {
        match session.end_profiling() {
            Ok(path) => println!("[GRANITE] ORT profile {label}: {path}"),
            Err(err) => eprintln!("[GRANITE] ORT profile end failed for {label}: {err}"),
        }
    }
}

struct GraniteGraphPaths {
    encoder: PathBuf,
    projector: PathBuf,
    embed_tokens: PathBuf,
    editor: PathBuf,
    manifest: PathBuf,
}

impl GraniteGraphPaths {
    fn new(dir: &Path) -> Self {
        Self {
            encoder: dir.join("encoder.onnx"),
            projector: dir.join("projector.onnx"),
            embed_tokens: dir.join("embed_tokens.onnx"),
            editor: dir.join("editor.onnx"),
            manifest: dir.join("taurscribe_granite_nar_manifest.json"),
        }
    }

    /// Whether the bundle manifest marks the encoder graph as safe to run
    /// fully on DirectML (see scripts/make_granite_portable_dml.py).
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn encoder_dml_safe(&self) -> bool {
        std::fs::read_to_string(&self.manifest)
            .ok()
            .and_then(|text| serde_json::from_str::<BundleManifest>(&text).ok())
            .is_some_and(|manifest| manifest.encoder_dml_safe)
    }
}

fn ceil_div(a: usize, b: usize) -> usize {
    if a == 0 {
        0
    } else {
        (a - 1) / b + 1
    }
}

fn ctc_collapse_from_logits(
    logits: &[f32],
    timesteps: usize,
    vocab: usize,
    blank_id: i64,
) -> Result<Vec<i64>, String> {
    if logits.len() < timesteps.saturating_mul(vocab) {
        return Err(format!(
            "CTC logits too short: timesteps={timesteps} vocab={vocab} len={}",
            logits.len()
        ));
    }
    let mut out = Vec::new();
    let mut prev: Option<i64> = None;
    for t in 0..timesteps {
        let row = &logits[t * vocab..(t + 1) * vocab];
        let id = row
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i as i64)
            .ok_or("empty CTC row")?;
        if Some(id) != prev && id != blank_id {
            out.push(id);
        }
        prev = Some(id);
    }
    Ok(out)
}

fn ctc_collapse_from_ids(token_ids: &[i64], blank_id: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let mut prev: Option<i64> = None;
    for &id in token_ids {
        if Some(id) != prev && id != blank_id {
            out.push(id);
        }
        prev = Some(id);
    }
    out
}

fn add_insertion_slots(token_ids: &[i64]) -> Vec<i64> {
    let total = (2 * token_ids.len() + 1).max(MIN_EDIT_SEQUENCE_LENGTH);
    let mut out = vec![BLANK_TOKEN_ID; total];
    for (i, &id) in token_ids.iter().enumerate() {
        out[2 * i + 1] = id;
    }
    out
}

fn pad_features_to_export_bucket(
    features: &Array2<f32>,
    valid_frames: usize,
) -> Result<Vec<f32>, String> {
    if features.ncols() != 160 {
        return Err(format!(
            "Granite features must have 160 columns, got {}",
            features.ncols()
        ));
    }
    let mut out = vec![0.0_f32; EXPORT_FRAMES * 160];
    for t in 0..valid_frames {
        for c in 0..160 {
            out[t * 160 + c] = features[[t, c]];
        }
    }
    Ok(out)
}

fn concat_encoder_layers(frames: usize, layers: [&[f32]; 4]) -> Result<Vec<f32>, String> {
    let layer_dim = 1024;
    for (idx, layer) in layers.iter().enumerate() {
        let need = frames * layer_dim;
        if layer.len() < need {
            return Err(format!(
                "hidden layer {idx} too short: need {need}, got {}",
                layer.len()
            ));
        }
    }
    let mut out = Vec::with_capacity(frames * layer_dim * 4);
    for t in 0..frames {
        for layer in layers {
            let start = t * layer_dim;
            out.extend_from_slice(&layer[start..start + layer_dim]);
        }
    }
    Ok(out)
}

fn extract_named_f32_ref<'a>(
    outputs: &'a ort::session::SessionOutputs,
    name: &str,
) -> Result<(Vec<usize>, &'a [f32]), String> {
    let value = outputs
        .get(name)
        .ok_or_else(|| format!("Missing ONNX output: {name}"))?;
    let (shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("Extract {name}: {e}"))?;
    Ok((shape.iter().map(|&d| d as usize).collect(), data))
}

fn try_extract_named_i64_ref<'a>(
    outputs: &'a ort::session::SessionOutputs,
    name: &str,
) -> Result<Option<(Vec<usize>, &'a [i64])>, String> {
    let Some(value) = outputs.get(name) else {
        return Ok(None);
    };
    let (shape, data) = value
        .try_extract_tensor::<i64>()
        .map_err(|e| format!("Extract {name}: {e}"))?;
    Ok(Some((shape.iter().map(|&d| d as usize).collect(), data)))
}

fn make_tensor_f32(shape: Vec<usize>, data: Vec<f32>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {e}"))
}

fn make_tensor_i64(shape: Vec<usize>, data: Vec<i64>) -> Result<ort::value::DynValue, String> {
    ort::value::Value::from_array((shape, data))
        .map(|t| t.into_dyn())
        .map_err(|e| format!("Tensor creation error: {e}"))
}

pub(crate) fn resolve_cohere_model_dir(
    models_dir: &Path,
    model_id: Option<&str>,
) -> Result<PathBuf, String> {
    let dir = match model_id {
        None => models_dir.join(DEFAULT_MODEL_DIR),
        Some(id) => {
            let pb = PathBuf::from(id);
            if pb.is_absolute() {
                pb
            } else {
                match id {
                    "cohere-speech-1b"
                    | "cohere-speech-1b-cpu"
                    | "cohere-speech-1b-fp16"
                    | "cohere-speech-1b-fp16-cuda"
                    | "granite-speech-4.1-2b-nar-cuda"
                    | "granite-speech-4.1-2b-nar"
                    | "granite-speech-4.1-2b-nar-onnx" => models_dir.join(DEFAULT_MODEL_DIR),
                    "granite-speech-4.1-2b-nar-portable" => models_dir.join(MODEL_ID_PORTABLE),
                    other => {
                        if other.contains('/') || other.contains('\\') {
                            return Err(format!("Invalid model id: {other}"));
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

pub(crate) fn cohere_logical_model_id_for_dir(model_dir: &Path) -> String {
    if is_portable_granite_dir(model_dir) {
        MODEL_ID_PORTABLE.to_string()
    } else {
        MODEL_ID_UNIVERSAL.to_string()
    }
}

pub(crate) fn cohere_onnx_bundle_ready(dir: &Path) -> bool {
    dir.is_dir()
        && dir.join("encoder.onnx").exists()
        && dir.join("projector.onnx").exists()
        && dir.join("embed_tokens.onnx").exists()
        && dir.join("editor.onnx").exists()
        && dir.join("tokenizer.json").exists()
        && dir.join("preprocessor_config.json").exists()
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BundleManifest {
    format: Option<String>,
    source_model: Option<String>,
    variant: Option<String>,
    #[serde(default)]
    encoder_dml_safe: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_cpu_always_wins_backend_selection() {
        let portable = Path::new("models").join(MODEL_ID_PORTABLE);
        assert_eq!(
            granite_backend_request(true, &portable),
            GraniteBackendRequest::Cpu
        );
        let cuda = Path::new("models").join(MODEL_ID_CUDA);
        assert_eq!(
            granite_backend_request(true, &cuda),
            GraniteBackendRequest::Cpu
        );
    }

    #[test]
    fn portable_dir_detection_uses_folder_name() {
        assert!(is_portable_granite_dir(
            &Path::new("any").join(MODEL_ID_PORTABLE)
        ));
        assert!(!is_portable_granite_dir(
            &Path::new("any").join(MODEL_ID_CUDA)
        ));
    }

    #[test]
    fn portable_default_uses_platform_acceleration_policy() {
        let portable = Path::new("models").join(MODEL_ID_PORTABLE);
        // macOS deliberately shares the CPU default: CoreML is opt-in only.
        let expected = if cfg!(target_os = "windows") {
            GraniteBackendRequest::Auto
        } else {
            GraniteBackendRequest::Cpu
        };
        assert_eq!(granite_backend_request(false, &portable), expected);
    }

    #[test]
    fn granite_cpu_threads_stay_in_sane_bounds() {
        let threads = granite_cpu_intra_threads();
        assert!((1..=16).contains(&threads), "got {threads}");
    }

    #[test]
    fn bundle_manifest_parses_encoder_dml_safe() {
        let with_flag: BundleManifest = serde_json::from_str(
            r#"{"format":"taurscribe-granite-nar-onnx-bundle","variant":"int4-argmax-dml-static","encoder_dml_safe":true}"#,
        )
        .expect("manifest with flag");
        assert!(with_flag.encoder_dml_safe);

        let without_flag: BundleManifest =
            serde_json::from_str(r#"{"format":"taurscribe-granite-nar-onnx-bundle"}"#)
                .expect("manifest without flag");
        assert!(!without_flag.encoder_dml_safe);
    }

    #[test]
    fn missing_manifest_means_encoder_not_dml_safe() {
        let paths = GraniteGraphPaths::new(Path::new("definitely-missing-granite-dir"));
        assert!(!paths.encoder_dml_safe());
    }
}
