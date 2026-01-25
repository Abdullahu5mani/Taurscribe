use parakeet_rs::{Nemotron, Parakeet, ParakeetEOU, ParakeetTDT, TimestampMode, Transcriber};
use std::path::PathBuf;

/// GPU Backend Type
#[derive(Debug, Clone, serde::Serialize)]
pub enum GpuBackend {
    Cuda, // NVIDIA GPUs (Very Fast)
    Cpu,  // Processor (Slow fallback)
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Cuda => write!(f, "CUDA"),
            GpuBackend::Cpu => write!(f, "CPU"),
        }
    }
}

/// Information about a Parakeet Model
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParakeetModelInfo {
    pub id: String,
    pub display_name: String,
    pub model_type: String, // "Nemotron" or "CTC"
    pub size_mb: f64,
}

/// Wrapper for different loaded model types
#[allow(dead_code)]
enum LoadedModel {
    Nemotron(Nemotron),
    Ctc(Parakeet),
    Eou(ParakeetEOU),
    Tdt(ParakeetTDT),
}

/// Status Report Struct
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParakeetStatus {
    pub loaded: bool,
    pub model_id: Option<String>,
    pub model_type: Option<String>,
    pub backend: String,
}

/// The Manager that controls the Parakeet ASR
pub struct ParakeetManager {
    model: Option<LoadedModel>,
    model_name: Option<String>,
    backend: GpuBackend,
}

impl ParakeetManager {
    /// Create a new Parakeet Manager (Constructor)
    pub fn new() -> Self {
        ParakeetManager {
            model: None,
            model_name: None,
            backend: GpuBackend::Cpu,
        }
    }

    /// Helper: Find the folder where Parakeet models are stored
    fn get_models_dir() -> Result<PathBuf, String> {
        let possible_paths = [
            "taurscribe-runtime/models",
            "../taurscribe-runtime/models",
            "../../taurscribe-runtime/models",
        ];

        for path in possible_paths {
            if let Ok(canonical) = std::fs::canonicalize(path) {
                if canonical.is_dir() {
                    return Ok(canonical);
                }
            }
        }

        // Fallback to checking exe location if relative paths fail
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let runtime_dir = exe_dir.join("taurscribe-runtime");
                if runtime_dir.exists() {
                    return Ok(runtime_dir.join("models"));
                }
            }
        }

        Err("Could not find taurscribe-runtime/models directory".to_string())
    }

    /// List all the models found in the models folder
    pub fn list_available_models() -> Result<Vec<ParakeetModelInfo>, String> {
        let models_dir = Self::get_models_dir()?;
        let mut models = Vec::new();

        if !models_dir.exists() {
            return Ok(vec![]); // Return empty if dir doesn't exist yet
        }

        // 1. Check for Nemotron (Top level or in subdirs)
        let entries = std::fs::read_dir(&models_dir)
            .map_err(|e| format!("Failed to read models directory: {}", e))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path.file_name().unwrap_or_default().to_string_lossy();

                // Detect Nemotron
                if path.join("encoder.onnx").exists() && path.join("decoder_joint.onnx").exists() {
                    // It could be Nemotron or EOU
                    if path.join("tokenizer.model").exists() {
                        models.push(ParakeetModelInfo {
                            id: format!("nemotron:{}", dir_name),
                            display_name: format!("Nemotron (Streaming) - {}", dir_name),
                            model_type: "Nemotron".to_string(),
                            size_mb: Self::estimate_model_size(&path),
                        });
                    } else if path.join("tokenizer.json").exists() {
                        models.push(ParakeetModelInfo {
                            id: format!("eou:{}", dir_name),
                            display_name: format!("Parakeet EOU - {}", dir_name),
                            model_type: "EOU".to_string(),
                            size_mb: Self::estimate_model_size(&path),
                        });
                    }
                }

                // Detect TDT
                if path.join("encoder.onnx").exists()
                    && path.join("decoder.onnx").exists()
                    && path.join("joint.onnx").exists()
                {
                    models.push(ParakeetModelInfo {
                        id: format!("tdt:{}", dir_name),
                        display_name: format!("Parakeet TDT - {}", dir_name),
                        model_type: "TDT".to_string(),
                        size_mb: Self::estimate_model_size(&path),
                    });
                }

                // Detect Parakeet / CTC models (often in models/parakeet/ctc-en)
                // Check if this dir ITSELF is a CTC model
                if path.join("model.onnx").exists() && path.join("tokenizer.json").exists() {
                    models.push(ParakeetModelInfo {
                        id: format!("ctc:{}", dir_name),
                        display_name: format!("Parakeet CTC - {}", dir_name),
                        model_type: "CTC".to_string(),
                        size_mb: Self::estimate_model_size(&path),
                    });
                }
            }
        }

        // 2. Check explicitly in 'parakeet' subdirectory for both CTC and Nemotron models
        let parakeet_dir = models_dir.join("parakeet");
        if parakeet_dir.exists() {
            let entries = std::fs::read_dir(&parakeet_dir).ok();
            if let Some(entries) = entries {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let dir_name = path.file_name().unwrap_or_default().to_string_lossy();

                        // Check for CTC
                        if path.join("model.onnx").exists() && path.join("tokenizer.json").exists()
                        {
                            models.push(ParakeetModelInfo {
                                id: format!("ctc:parakeet/{}", dir_name),
                                display_name: format!("Parakeet CTC - {}", dir_name),
                                model_type: "CTC".to_string(),
                                size_mb: Self::estimate_model_size(&path),
                            });
                        }

                        // Check for Nemotron
                        if path.join("encoder.onnx").exists()
                            && path.join("decoder_joint.onnx").exists()
                        {
                            if path.join("tokenizer.model").exists() {
                                models.push(ParakeetModelInfo {
                                    id: format!("nemotron:parakeet/{}", dir_name),
                                    display_name: format!("Nemotron - {}", dir_name),
                                    model_type: "Nemotron".to_string(),
                                    size_mb: Self::estimate_model_size(&path),
                                });
                            } else if path.join("tokenizer.json").exists() {
                                models.push(ParakeetModelInfo {
                                    id: format!("eou:parakeet/{}", dir_name),
                                    display_name: format!("Parakeet EOU - {}", dir_name),
                                    model_type: "EOU".to_string(),
                                    size_mb: Self::estimate_model_size(&path),
                                });
                            }
                        }

                        // Check for TDT
                        if path.join("encoder.onnx").exists()
                            && path.join("decoder.onnx").exists()
                            && path.join("joint.onnx").exists()
                        {
                            models.push(ParakeetModelInfo {
                                id: format!("tdt:parakeet/{}", dir_name),
                                display_name: format!("Parakeet TDT - {}", dir_name),
                                model_type: "TDT".to_string(),
                                size_mb: Self::estimate_model_size(&path),
                            });
                        }
                    }
                }
            }
        }

        Ok(models)
    }

    /// Helper: Estimate model size in MB
    fn estimate_model_size(path: &PathBuf) -> f64 {
        let mut total_size = 0u64;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        total_size += metadata.len();
                    }
                }
            }
        }
        total_size as f64 / (1024.0 * 1024.0)
    }

    /// Get the name of the currently loaded model
    #[allow(dead_code)] // Public API - may be called from frontend
    pub fn get_current_model(&self) -> Option<&String> {
        self.model_name.as_ref()
    }

    /// Get which GPU backend we are using
    #[allow(dead_code)] // Public API - may be called from frontend
    pub fn get_backend(&self) -> &GpuBackend {
        &self.backend
    }

    /// Get full status of the engine
    pub fn get_status(&self) -> ParakeetStatus {
        let model_type = self.model.as_ref().map(|m| match m {
            LoadedModel::Nemotron(_) => "Nemotron".to_string(),
            LoadedModel::Ctc(_) => "CTC".to_string(),
            LoadedModel::Eou(_) => "EOU".to_string(),
            LoadedModel::Tdt(_) => "TDT".to_string(),
        });

        ParakeetStatus {
            loaded: self.model.is_some(),
            model_id: self.model_name.clone(),
            model_type,
            backend: self.backend.to_string(),
        }
    }

    /// Initialize (Load) a Model
    pub fn initialize(&mut self, model_id: Option<&str>) -> Result<String, String> {
        let models_dir = Self::get_models_dir()?;

        let available = Self::list_available_models()?;
        if available.is_empty() {
            return Err("No Parakeet/Nemotron models found.".to_string());
        }

        // Default to first available if none specified
        let target_id = model_id.unwrap_or(&available[0].id);

        // Find info for this ID
        let info = available
            .iter()
            .find(|m| m.id == target_id)
            .ok_or_else(|| format!("Model ID '{}' not found in list", target_id))?;

        println!(
            "[PARAKEET] Initializing model: {} ({})",
            info.display_name, info.model_type
        );

        // Construct full path
        // ID format "type:subpath" -> e.g. "ctc:parakeet/ctc-en"
        let subpath = target_id
            .split_once(':')
            .map(|(_, p)| p)
            .unwrap_or(target_id);
        let model_path = models_dir.join(subpath);

        if !model_path.exists() {
            return Err(format!("Model path not found: {}", model_path.display()));
        }

        // Initialize based on type
        let (model, backend) = match info.model_type.as_str() {
            "Nemotron" => {
                let (m, b) = Self::init_nemotron(&model_path)?;
                (LoadedModel::Nemotron(m), b)
            }
            "CTC" => {
                let (m, b) = Self::init_ctc(&model_path)?;
                (LoadedModel::Ctc(m), b)
            }
            "EOU" => {
                let (m, b) = Self::init_eou(&model_path)?;
                (LoadedModel::Eou(m), b)
            }
            "TDT" => {
                let (m, b) = Self::init_tdt(&model_path)?;
                (LoadedModel::Tdt(m), b)
            }
            _ => return Err(format!("Unknown model type: {}", info.model_type)),
        };

        self.model = Some(model);
        self.model_name = Some(target_id.to_string());
        self.backend = backend.clone();

        Ok(format!("Loaded {} ({})", info.display_name, backend))
    }

    fn init_nemotron(path: &PathBuf) -> Result<(Nemotron, GpuBackend), String> {
        // Try GPU
        if let Ok(m) = Self::try_gpu_nemotron(path.to_str().unwrap()) {
            println!("[PARAKEET] Loaded Nemotron with CUDA");
            return Ok((m, GpuBackend::Cuda));
        }
        println!("[PARAKEET] Fallback to CPU for Nemotron");
        let m = Self::try_cpu_nemotron(path.to_str().unwrap())?;
        Ok((m, GpuBackend::Cpu))
    }

    fn init_ctc(path: &PathBuf) -> Result<(Parakeet, GpuBackend), String> {
        // Try GPU
        if let Ok(m) = Self::try_gpu_ctc(path.to_str().unwrap()) {
            println!("[PARAKEET] Loaded CTC with CUDA");
            return Ok((m, GpuBackend::Cuda));
        }
        println!("[PARAKEET] Fallback to CPU for CTC");
        let m = Self::try_cpu_ctc(path.to_str().unwrap())?;
        Ok((m, GpuBackend::Cpu))
    }

    // --- GPU/CPU Loaders ---

    fn try_gpu_nemotron(path: &str) -> Result<Nemotron, String> {
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda);
        Nemotron::from_pretrained(path, Some(config)).map_err(|e| format!("{}", e))
    }

    fn try_cpu_nemotron(path: &str) -> Result<Nemotron, String> {
        Nemotron::from_pretrained(path, None).map_err(|e| format!("{}", e))
    }

    fn try_gpu_ctc(path: &str) -> Result<Parakeet, String> {
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda);
        Parakeet::from_pretrained(path, Some(config)).map_err(|e| format!("{}", e))
    }

    fn try_cpu_ctc(path: &str) -> Result<Parakeet, String> {
        Parakeet::from_pretrained(path, None).map_err(|e| format!("{}", e))
    }

    fn init_eou(path: &PathBuf) -> Result<(ParakeetEOU, GpuBackend), String> {
        if let Ok(m) = Self::try_gpu_eou(path.to_str().unwrap()) {
            return Ok((m, GpuBackend::Cuda));
        }
        let m = Self::try_cpu_eou(path.to_str().unwrap())?;
        Ok((m, GpuBackend::Cpu))
    }

    fn try_gpu_eou(path: &str) -> Result<ParakeetEOU, String> {
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda);
        ParakeetEOU::from_pretrained(path, Some(config)).map_err(|e| format!("{}", e))
    }

    fn try_cpu_eou(path: &str) -> Result<ParakeetEOU, String> {
        ParakeetEOU::from_pretrained(path, None).map_err(|e| format!("{}", e))
    }

    fn init_tdt(path: &PathBuf) -> Result<(ParakeetTDT, GpuBackend), String> {
        if let Ok(m) = Self::try_gpu_tdt(path.to_str().unwrap()) {
            return Ok((m, GpuBackend::Cuda));
        }
        let m = Self::try_cpu_tdt(path.to_str().unwrap())?;
        Ok((m, GpuBackend::Cpu))
    }

    fn try_gpu_tdt(path: &str) -> Result<ParakeetTDT, String> {
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda);
        ParakeetTDT::from_pretrained(path, Some(config)).map_err(|e| format!("{}", e))
    }

    fn try_cpu_tdt(path: &str) -> Result<ParakeetTDT, String> {
        ParakeetTDT::from_pretrained(path, None).map_err(|e| format!("{}", e))
    }

    // --- Transcription ---

    /// Transcribe a chunk of audio
    #[allow(dead_code)]
    pub fn transcribe_chunk(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<String, String> {
        // Resample first
        let audio = if sample_rate != 16000 {
            Self::resample_audio(samples, sample_rate, 16000)?
        } else {
            samples.to_vec()
        };

        if let Some(model) = &mut self.model {
            match model {
                LoadedModel::Nemotron(m) => {
                    let mut transcript = String::new();
                    const CHUNK_SIZE: usize = 8960; // 560ms at 16kHz
                    for chunk in audio.chunks(CHUNK_SIZE) {
                        let mut chunk_vec = chunk.to_vec();
                        if chunk_vec.len() < CHUNK_SIZE {
                            chunk_vec.resize(CHUNK_SIZE, 0.0);
                        }
                        transcript.push_str(&m.transcribe_chunk(&chunk_vec).unwrap_or_default());
                    }
                    println!("[PARAKEET NEMOTRON] {}", transcript.trim());
                    Ok(transcript)
                }
                LoadedModel::Ctc(m) => {
                    let result = m
                        .transcribe_samples(audio.clone(), 16000, 1, Some(TimestampMode::Words))
                        .map_err(|e| format!("CTC Error: {}", e))?;

                    println!("[PARAKEET CTC] {}", result.text.trim());
                    Ok(result.text)
                }
                LoadedModel::Eou(m) => {
                    let mut full_text = String::new();
                    const CHUNK_SIZE: usize = 2560; // 160ms
                    for chunk in audio.chunks(CHUNK_SIZE) {
                        let text = m.transcribe(&chunk.to_vec(), false).unwrap_or_default();
                        full_text.push_str(&text);
                    }
                    println!("[PARAKEET EOU] {}", full_text.trim());
                    Ok(full_text)
                }
                LoadedModel::Tdt(m) => {
                    let result = m
                        .transcribe_samples(audio.clone(), 16000, 1, Some(TimestampMode::Sentences))
                        .map_err(|e| format!("TDT Error: {}", e))?;

                    println!("[PARAKEET TDT] {}", result.text.trim());
                    Ok(result.text)
                }
            }
        } else {
            Err("No model loaded".to_string())
        }
    }

    /// Transcribe a file
    #[allow(dead_code)]
    pub fn transcribe_file(&mut self, file_path: &str) -> Result<String, String> {
        println!("[PARAKEET FILE] Loading: {}", file_path);
        let start_time = std::time::Instant::now();

        let audio = Self::load_audio(file_path)?;
        let load_time = start_time.elapsed();

        println!(
            "[PARAKEET FILE] Audio loaded: {} samples ({:.2}s), took {:.2}ms",
            audio.len(),
            audio.len() as f64 / 16000.0,
            load_time.as_secs_f64() * 1000.0
        );

        let transcribe_start = std::time::Instant::now();
        let transcript = self.transcribe_chunk(&audio, 16000)?;
        let transcribe_time = transcribe_start.elapsed();

        let audio_duration = audio.len() as f64 / 16000.0;
        let speed_factor = audio_duration / transcribe_time.as_secs_f64();

        println!(
            "[PARAKEET FILE] ✅ Transcription complete in {:.2}ms ({:.1}x realtime)",
            transcribe_time.as_secs_f64() * 1000.0,
            speed_factor
        );

        println!("[PARAKEET FINAL] {}", transcript.trim());
        Ok(transcript)
    }

    /// Helper: Resample audio to target sample rate
    fn resample_audio(samples: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>, String> {
        use rubato::{
            Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
            WindowFunction,
        };
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };
        let mut resampler = SincFixedIn::<f32>::new(
            to_rate as f64 / from_rate as f64,
            2.0,
            params,
            samples.len(),
            1,
        )
        .map_err(|e| e.to_string())?;

        let waves = resampler
            .process(&vec![samples.to_vec()], None)
            .map_err(|e| e.to_string())?;
        Ok(waves[0].clone())
    }

    /// Helper: Load and prepare a WAV file
    #[allow(dead_code)] // Used internally by transcribe_file
    #[allow(dead_code)]
    fn load_audio(file_path: &str) -> Result<Vec<f32>, String> {
        let mut reader = hound::WavReader::open(file_path).map_err(|e| e.to_string())?;
        let spec = reader.spec();

        println!(
            "[PARAKEET] WAV specs: {}Hz, {} channels, {} bits",
            spec.sample_rate, spec.channels, spec.bits_per_sample
        );

        let samples: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
            reader.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect()
        } else {
            reader
                .samples::<i16>()
                .map(|s| s.unwrap_or(0) as f32 / 32768.0)
                .collect()
        };

        // Mono
        let mono = if spec.channels == 2 {
            samples.chunks(2).map(|c| (c[0] + c[1]) / 2.0).collect()
        } else {
            samples
        };

        // Resample to 16kHz if needed
        if spec.sample_rate != 16000 {
            Self::resample_audio(&mono, spec.sample_rate, 16000)
        } else {
            Ok(mono)
        }
    }
}
