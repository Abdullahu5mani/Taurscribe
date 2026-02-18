use parakeet_rs::{Nemotron, Parakeet, ParakeetEOU, ParakeetTDT, TimestampMode, Transcriber};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::path::PathBuf;

use crate::parakeet_loaders::{init_ctc, init_eou, init_nemotron, init_tdt};

/// GPU Backend Type
#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)] // Cuda/DirectML used only on non-macOS builds
pub enum GpuBackend {
    Cuda,     // NVIDIA GPUs (Very Fast)
    DirectML, // Windows GPUs/NPUs (ARM64/AMD/Intel)
    Cpu,      // Processor (Slow fallback)
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

/// Information about a Parakeet Model
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParakeetModelInfo {
    pub id: String,
    pub display_name: String,
    pub model_type: String, // "Nemotron" | "CTC" | "EOU" | "TDT"
    pub size_mb: f64,
}

/// Wrapper for different loaded model types
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
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>, // (Sample Rate, Input Size, Resampler)
}

impl ParakeetManager {
    /// Create a new Parakeet Manager (Constructor)
    pub fn new() -> Self {
        ParakeetManager {
            model: None,
            model_name: None,
            backend: GpuBackend::Cpu,
            resampler: None,
        }
    }

    /// Helper: Find the folder where Parakeet models are stored
    fn get_models_dir() -> Result<PathBuf, String> {
        crate::utils::get_models_dir()
    }

    /// List all the models found in the models folder
    pub fn list_available_models() -> Result<Vec<ParakeetModelInfo>, String> {
        let models_dir = Self::get_models_dir()?;
        let mut models = Vec::new();

        if !models_dir.exists() {
            return Ok(vec![]);
        }

        let entries = std::fs::read_dir(&models_dir)
            .map_err(|e| format!("Failed to read models directory: {}", e))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = path.file_name().unwrap_or_default().to_string_lossy();

            // Detect Nemotron / EOU (both have encoder.onnx + decoder_joint.onnx)
            if path.join("encoder.onnx").exists() && path.join("decoder_joint.onnx").exists() {
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

            // Detect TDT (separate encoder / decoder / joint files)
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

            // Detect CTC
            if path.join("model.onnx").exists() && path.join("tokenizer.json").exists() {
                models.push(ParakeetModelInfo {
                    id: format!("ctc:{}", dir_name),
                    display_name: format!("Parakeet CTC - {}", dir_name),
                    model_type: "CTC".to_string(),
                    size_mb: Self::estimate_model_size(&path),
                });
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

    /// Unload the model to free memory
    pub fn unload(&mut self) {
        if self.model.is_some() {
            println!("[INFO] Unloading Parakeet model...");
            self.model = None;
            self.model_name = None;
            self.resampler = None;
            println!("[SUCCESS] Parakeet model unloaded");
        }
    }

    /// Initialize (Load) a Model
    pub fn initialize(&mut self, model_id: Option<&str>) -> Result<String, String> {
        let models_dir = Self::get_models_dir()?;

        let available = Self::list_available_models()?;
        if available.is_empty() {
            return Err("No Parakeet/Nemotron models found.".to_string());
        }

        let target_id = model_id.unwrap_or(&available[0].id);

        let info = available
            .iter()
            .find(|m| m.id == target_id)
            .ok_or_else(|| format!("Model ID '{}' not found in list", target_id))?;

        println!(
            "[PARAKEET] Initializing model: {} ({})",
            info.display_name, info.model_type
        );

        let subpath = target_id
            .split_once(':')
            .map(|(_, p)| p)
            .unwrap_or(target_id);
        let model_path = models_dir.join(subpath);

        if !model_path.exists() {
            return Err(format!("Model path not found: {}", model_path.display()));
        }

        let (model, backend): (LoadedModel, GpuBackend) = match info.model_type.as_str() {
            "Nemotron" => {
                let (m, b) = init_nemotron(&model_path)?;
                (LoadedModel::Nemotron(m), b)
            }
            "CTC" => {
                let (m, b) = init_ctc(&model_path)?;
                (LoadedModel::Ctc(m), b)
            }
            "EOU" => {
                let (m, b) = init_eou(&model_path)?;
                (LoadedModel::Eou(m), b)
            }
            "TDT" => {
                let (m, b) = init_tdt(&model_path)?;
                (LoadedModel::Tdt(m), b)
            }
            _ => return Err(format!("Unknown model type: {}", info.model_type)),
        };

        self.model = Some(model);
        self.model_name = Some(target_id.to_string());
        self.backend = backend.clone();

        Ok(format!("Loaded {} ({})", info.display_name, backend))
    }

    /// Clear the internal context/state of the model (reset for new recording)
    pub fn clear_context(&mut self) {
        if let Some(model) = &mut self.model {
            if let LoadedModel::Nemotron(m) = model {
                m.reset();
            }
        }
    }

    /// Transcribe a chunk of audio
    pub fn transcribe_chunk(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<String, String> {
        // 1. Resample to 16 kHz if needed
        let audio = if sample_rate != 16000 {
            let needs_new_resampler = self
                .resampler
                .as_ref()
                .map_or(true, |(r, s, _)| *r != sample_rate || *s != samples.len());

            if needs_new_resampler {
                let params = SincInterpolationParameters {
                    sinc_len: 256,
                    f_cutoff: 0.95,
                    interpolation: SincInterpolationType::Linear,
                    oversampling_factor: 256,
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
            waves[0].clone()
        } else {
            samples.to_vec()
        };

        // 2. Transcribe
        if let Some(model) = &mut self.model {
            match model {
                LoadedModel::Nemotron(m) => {
                    let mut transcript = String::new();
                    const CHUNK_SIZE: usize = 8960; // 560 ms at 16 kHz
                    for chunk in audio.chunks(CHUNK_SIZE) {
                        let mut chunk_vec = chunk.to_vec();
                        if chunk_vec.len() < CHUNK_SIZE {
                            chunk_vec.resize(CHUNK_SIZE, 0.0);
                        }
                        transcript.push_str(&m.transcribe_chunk(&chunk_vec).unwrap_or_default());
                    }
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
                    const CHUNK_SIZE: usize = 2560; // 160 ms
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
}
