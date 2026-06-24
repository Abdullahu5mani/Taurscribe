// granite.rs — IBM Granite Speech NAR ONNX manager.
//
// This intentionally exposes Cohere-compatible type/function names through
// cohere.rs so the existing frontend IPC can be reused while the underlying
// model slot is migrated from Cohere Transcribe to Granite Speech NAR.

use ndarray::Array2;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use serde::Deserialize;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::utils::strip_whitelisted_sound_captions;

const DEFAULT_MODEL_DIR: &str = "granite-speech-4.1-2b-nar";
const MODEL_ID_UNIVERSAL: &str = "granite-speech-4.1-2b-nar";
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
            println!("[GRANITE] CUDA preload skipped missing DLL: {}", path.display());
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
}

fn granite_backend_request(force_cpu: bool) -> GraniteBackendRequest {
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
        _ => GraniteBackendRequest::Auto,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum GpuBackend {
    Cuda,
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
    tokenizer: Option<tokenizers::Tokenizer>,
    backend: GpuBackend,
    model_name: Option<String>,
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>,
}

impl CohereManager {
    pub fn new() -> Self {
        Self {
            runtime: None,
            tokenizer: None,
            backend: GpuBackend::Cpu,
            model_name: None,
            resampler: None,
        }
    }

    pub fn get_status(&self) -> CohereStatus {
        CohereStatus {
            loaded: self.runtime.is_some(),
            model_id: self.model_name.clone(),
            backend: self.backend.to_string(),
            gpu_only: matches!(self.backend, GpuBackend::Cuda | GpuBackend::DirectML),
        }
    }

    pub fn unload(&mut self) {
        if self.runtime.is_some() {
            println!("[GRANITE] Unloading model...");
            self.runtime = None;
            self.tokenizer = None;
            self.model_name = None;
            self.resampler = None;
            crate::memory::trim_process_memory();
            println!("[GRANITE] Model unloaded");
        }
    }

    pub fn initialize(&mut self, model_id: Option<&str>, force_cpu: bool) -> Result<String, String> {
        let models_dir = crate::utils::get_models_dir()?;
        let model_dir = resolve_cohere_model_dir(&models_dir, model_id)?;
        if !cohere_onnx_bundle_ready(&model_dir) {
            return Err(format!(
                "Granite ONNX bundle not found in {}. Download/install Granite Speech NAR from Settings > Models.",
                model_dir.display()
            ));
        }
        if self.runtime.is_some() {
            self.unload();
        }

        let request = granite_backend_request(force_cpu);
        println!(
            "[GRANITE] initialize: model_dir={} request={:?}",
            model_dir.display(),
            request
        );
        crate::memory::maybe_log_process_memory("granite before initialize");

        let graph_paths = GraniteGraphPaths::new(&model_dir);
        let (backend, runtime) = match request {
            GraniteBackendRequest::Cpu => (GpuBackend::Cpu, self.create_runtime_cpu(&graph_paths)?),
            GraniteBackendRequest::Cuda => (GpuBackend::Cuda, self.create_runtime_cuda(&graph_paths)?),
            GraniteBackendRequest::DirectML => {
                (GpuBackend::DirectML, self.create_runtime_directml(&graph_paths)?)
            }
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

        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Load tokenizer {}: {e}", tokenizer_path.display()))?;

        self.runtime = Some(runtime);
        self.tokenizer = Some(tokenizer);
        self.backend = backend;
        self.model_name = Some(cohere_logical_model_id_for_dir(&model_dir));
        crate::memory::maybe_log_process_memory("granite after initialize");
        Ok(format!(
            "Granite Speech NAR loaded ({})",
            self.backend
        ))
    }

    pub fn transcribe_chunk(&mut self, samples: &[f32], sample_rate: u32) -> Result<String, String> {
        match self.transcribe_chunk_loaded(samples, sample_rate) {
            Ok(text) => Ok(text),
            Err(err) if matches!(self.backend, GpuBackend::DirectML) => {
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
        let valid_frames = features.nrows().min(EXPORT_FRAMES);
        if valid_frames == 0 {
            return Ok(String::new());
        }

        let runtime = self.runtime.as_mut().ok_or("Granite runtime not loaded")?;
        let tokenizer = self.tokenizer.as_ref().ok_or("Granite tokenizer not loaded")?;

        let feature_data = pad_features_to_export_bucket(&features, valid_frames)?;
        let input_features = make_tensor_f32(vec![1, EXPORT_FRAMES, 160], feature_data)?;

        let enc_outputs = runtime
            .encoder
            .run(ort::inputs!["input_features" => input_features])
            .map_err(|e| format!("Granite encoder run: {e}"))?;

        let (bpe_shape, bpe_logits) = extract_named_f32(&enc_outputs, "bpe_logits")?;
        let pooled_len = ceil_div(valid_frames, BPE_POOLING_WINDOW)
            .min(*bpe_shape.get(1).ok_or("Bad bpe_logits shape")?);
        let encoder_token_ids = ctc_collapse_from_logits(
            &bpe_logits,
            pooled_len,
            VOCAB_SIZE,
            BLANK_TOKEN_ID,
        )?;

        let (_, h4) = extract_named_f32(&enc_outputs, "hidden_4")?;
        let (_, h8) = extract_named_f32(&enc_outputs, "hidden_8")?;
        let (_, h12) = extract_named_f32(&enc_outputs, "hidden_12")?;
        let (_, hlast) = extract_named_f32(&enc_outputs, "hidden_last")?;
        let multilayer = concat_encoder_layers(EXPORT_FRAMES, [&h4, &h8, &h12, &hlast])?;

        let projector_input = make_tensor_f32(vec![1, EXPORT_FRAMES, 4096], multilayer)?;
        let projector_outputs = runtime
            .projector
            .run(ort::inputs!["multilayer_features" => projector_input])
            .map_err(|e| format!("Granite projector run: {e}"))?;
        let (audio_shape, mut audio_embeds_all) =
            extract_named_f32(&projector_outputs, "audio_embeds")?;
        for value in &mut audio_embeds_all {
            *value /= TEXT_EMBEDDING_MULTIPLIER;
        }
        let available_audio_tokens = *audio_shape.get(1).ok_or("Bad audio_embeds shape")?;
        let audio_tokens = (valid_frames / PROJECTOR_DOWNSAMPLE_RATE).min(available_audio_tokens);

        let slotted = add_insertion_slots(&encoder_token_ids);
        let text_len = slotted.len();
        let token_tensor = make_tensor_i64(vec![text_len], slotted)?;
        let embed_outputs = runtime
            .embed_tokens
            .run(ort::inputs!["token_ids" => token_tensor])
            .map_err(|e| format!("Granite token embedding run: {e}"))?;
        let (_text_shape, text_embeds) = extract_named_f32(&embed_outputs, "text_embeds")?;

        let mut editor_input = Vec::with_capacity((audio_tokens + text_len) * HIDDEN_SIZE);
        editor_input.extend_from_slice(&audio_embeds_all[..audio_tokens * HIDDEN_SIZE]);
        editor_input.extend_from_slice(&text_embeds[..text_len * HIDDEN_SIZE]);
        let sequence = audio_tokens + text_len;
        let inputs_embeds = make_tensor_f32(vec![1, sequence, HIDDEN_SIZE], editor_input)?;
        let position_ids = make_tensor_i64(
            vec![1, sequence],
            (0..sequence as i64).collect::<Vec<_>>(),
        )?;

        let editor_outputs = runtime
            .editor
            .run(ort::inputs![
                "inputs_embeds" => inputs_embeds,
                "position_ids" => position_ids,
            ])
            .map_err(|e| format!("Granite editor run: {e}"))?;
        let (_logit_shape, logits) = extract_named_f32(&editor_outputs, "logits")?;
        let text_logits_start = audio_tokens * VOCAB_SIZE;
        let text_logits_end = text_logits_start + text_len * VOCAB_SIZE;
        if text_logits_end > logits.len() {
            return Err(format!(
                "Granite editor logits too short: need {text_logits_end}, got {}",
                logits.len()
            ));
        }
        let final_ids = ctc_collapse_from_logits(
            &logits[text_logits_start..text_logits_end],
            text_len,
            VOCAB_SIZE,
            BLANK_TOKEN_ID,
        )?;
        let token_ids: Vec<u32> = final_ids
            .into_iter()
            .filter_map(|id| u32::try_from(id).ok())
            .collect();
        let text = tokenizer
            .decode(&token_ids, true)
            .map_err(|e| format!("Granite tokenizer decode: {e}"))?;
        Ok(strip_whitelisted_sound_captions(&text).trim().to_string())
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
            Ok(GraniteRuntime {
                encoder: self.create_session_directml(&paths.encoder)?,
                projector: self.create_session_directml(&paths.projector)?,
                embed_tokens: self.create_session_directml(&paths.embed_tokens)?,
                editor: self.create_session_directml(&paths.editor)?,
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

    fn create_session_cpu(&self, path: &Path) -> Result<Session, String> {
        let builder = Session::builder()
            .map_err(|e| format!("ORT builder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("CPU opt level: {e}"))?;
        let mut builder =
            crate::ort_session::configure_low_ram_session_builder(builder, "granite-cpu")?;
        builder
            .commit_from_file(path)
            .map_err(|e| format!("CPU session load {}: {e}", path.display()))
    }

    #[cfg(target_os = "windows")]
    fn create_session_directml(&self, path: &Path) -> Result<Session, String> {
        let builder = Session::builder()
            .map_err(|e| format!("ORT builder: {e}"))?
            .with_execution_providers([ort::ep::DirectML::default().build()])
            .map_err(|e| format!("DirectML EP: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Disable)
            .map_err(|e| format!("DirectML opt level: {e}"))?;
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
            .with_execution_providers([crate::ort_session::build_low_ram_cuda_execution_provider()
                .build()
                .error_on_failure()])
            .map_err(|e| format!("CUDA EP: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("CUDA opt level: {e}"))?;
        let mut builder =
            crate::ort_session::configure_low_ram_session_builder(builder, "granite-cuda")?;
        builder
            .commit_from_file(path)
            .map_err(|e| format!("CUDA session load {}: {e}", path.display()))
    }
}

struct GraniteGraphPaths {
    encoder: PathBuf,
    projector: PathBuf,
    embed_tokens: PathBuf,
    editor: PathBuf,
}

impl GraniteGraphPaths {
    fn new(dir: &Path) -> Self {
        Self {
            encoder: dir.join("encoder.onnx"),
            projector: dir.join("projector.onnx"),
            embed_tokens: dir.join("embed_tokens.onnx"),
            editor: dir.join("editor.onnx"),
        }
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

fn add_insertion_slots(token_ids: &[i64]) -> Vec<i64> {
    let total = (2 * token_ids.len() + 1).max(MIN_EDIT_SEQUENCE_LENGTH);
    let mut out = vec![BLANK_TOKEN_ID; total];
    for (i, &id) in token_ids.iter().enumerate() {
        out[2 * i + 1] = id;
    }
    out
}

fn pad_features_to_export_bucket(features: &Array2<f32>, valid_frames: usize) -> Result<Vec<f32>, String> {
    if features.ncols() != 160 {
        return Err(format!("Granite features must have 160 columns, got {}", features.ncols()));
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

fn extract_named_f32(
    outputs: &ort::session::SessionOutputs,
    name: &str,
) -> Result<(Vec<usize>, Vec<f32>), String> {
    let value = outputs
        .get(name)
        .ok_or_else(|| format!("Missing ONNX output: {name}"))?;
    let (shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("Extract {name}: {e}"))?;
    Ok((shape.iter().map(|&d| d as usize).collect(), data.to_vec()))
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
                    | "granite-speech-4.1-2b-nar"
                    | "granite-speech-4.1-2b-nar-onnx" => models_dir.join(DEFAULT_MODEL_DIR),
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

pub(crate) fn cohere_logical_model_id_for_dir(_model_dir: &Path) -> String {
    MODEL_ID_UNIVERSAL.to_string()
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
}
