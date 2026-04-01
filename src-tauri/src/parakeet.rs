use parakeet_rs::{Nemotron, Parakeet, ParakeetEOU, ParakeetTDT, TimestampMode, Transcriber};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::path::PathBuf;

use crate::parakeet_loaders::{init_ctc, init_eou, init_nemotron, init_tdt, ParakeetLoadPath};
use crate::parakeet_runtime::LoadedParakeetRuntime;

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
pub(crate) enum LoadedModel {
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
    pub load_path: String,
}

/// The Manager that controls the Parakeet ASR
pub struct ParakeetManager {
    runtime: Option<LoadedParakeetRuntime>,
    model_name: Option<String>,
    backend: GpuBackend,
    load_path: ParakeetLoadPath,
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>, // (Sample Rate, Input Size, Resampler)
    next_runtime_generation: u64,
}

impl ParakeetManager {
    /// Create a new Parakeet Manager (Constructor)
    pub fn new() -> Self {
        ParakeetManager {
            runtime: None,
            model_name: None,
            backend: GpuBackend::Cpu,
            load_path: ParakeetLoadPath::FallbackGpu,
            resampler: None,
            next_runtime_generation: 1,
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
        let model_type = self.runtime.as_ref().map(|slot| match &slot.model {
            LoadedModel::Nemotron(_) => "Nemotron".to_string(),
            LoadedModel::Ctc(_) => "CTC".to_string(),
            LoadedModel::Eou(_) => "EOU".to_string(),
            LoadedModel::Tdt(_) => "TDT".to_string(),
        });

        ParakeetStatus {
            loaded: self.runtime.is_some(),
            model_id: self.model_name.clone(),
            model_type,
            backend: self.backend.to_string(),
            load_path: self.load_path.to_string(),
        }
    }

    /// Unload the model to free memory
    pub fn unload(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            println!("[INFO] Unloading Parakeet model...");
            let resampler_buffer_len = self
                .resampler
                .as_ref()
                .map(|(_, size, _)| *size)
                .unwrap_or(0);
            crate::memory::maybe_log_process_memory_with_sizes(
                "parakeet before unload",
                &[
                    ("resampler_input_samples", resampler_buffer_len),
                    (
                        "model_name_chars",
                        self.model_name.as_ref().map(|s| s.len()).unwrap_or(0),
                    ),
                ],
            );
            self.model_name = None;
            self.load_path = ParakeetLoadPath::FallbackGpu;
            self.resampler = None;
            println!(
                "[PARAKEET] Dropping runtime generation {}",
                runtime.generation
            );
            drop(runtime);
            crate::memory::maybe_log_process_memory_with_sizes(
                "parakeet after runtime teardown",
                &[(
                    "next_runtime_generation",
                    self.next_runtime_generation as usize,
                )],
            );
            crate::memory::trim_process_memory();
            crate::memory::maybe_log_process_memory_with_sizes(
                "parakeet after unload",
                &[(
                    "next_runtime_generation",
                    self.next_runtime_generation as usize,
                )],
            );
            println!("[SUCCESS] Parakeet model unloaded");
        }
    }

    /// Initialize (Load) a Model
    pub fn initialize(
        &mut self,
        model_id: Option<&str>,
        force_cpu: bool,
    ) -> Result<String, String> {
        self.initialize_with_load_path(model_id, force_cpu, ParakeetLoadPath::FallbackGpu)
    }

    pub fn initialize_with_load_path(
        &mut self,
        model_id: Option<&str>,
        force_cpu: bool,
        load_path: ParakeetLoadPath,
    ) -> Result<String, String> {
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
            "[PARAKEET] Initializing model: {} ({}){} [load_path={}]",
            info.display_name,
            info.model_type,
            if force_cpu { " [CPU-only mode]" } else { "" },
            load_path,
        );
        crate::memory::maybe_log_process_memory("parakeet before initialize");

        // Explicit unload so ONNX sessions are dropped before new ones are created (clear logs +
        // predictable VRAM release when switching Parakeet models or reloading).
        if self.runtime.is_some() {
            println!("[PARAKEET] Unloading previous Parakeet model before loading new weights...");
            self.unload();
        }

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
                let (m, b) = init_nemotron(&model_path, force_cpu, load_path)?;
                (LoadedModel::Nemotron(m), b)
            }
            "CTC" => {
                let (m, b) = init_ctc(&model_path, force_cpu, load_path)?;
                (LoadedModel::Ctc(m), b)
            }
            "EOU" => {
                let (m, b) = init_eou(&model_path, force_cpu, load_path)?;
                (LoadedModel::Eou(m), b)
            }
            "TDT" => {
                let (m, b) = init_tdt(&model_path, force_cpu, load_path)?;
                (LoadedModel::Tdt(m), b)
            }
            _ => return Err(format!("Unknown model type: {}", info.model_type)),
        };

        let generation = self.next_runtime_generation;
        self.next_runtime_generation += 1;
        self.runtime = Some(LoadedParakeetRuntime { generation, model });
        self.model_name = Some(target_id.to_string());
        self.backend = backend.clone();
        self.load_path = load_path;
        println!("[PARAKEET] Runtime generation {} loaded", generation);

        crate::parakeet_loaders::log_parakeet_backend_resolution(
            &info.model_type,
            &backend,
            force_cpu,
            load_path,
        );
        crate::memory::maybe_log_process_memory("parakeet after initialize");

        Ok(format!("Loaded {} ({})", info.display_name, backend))
    }

    /// Clear the internal context/state of the model (reset for new recording)
    pub fn clear_context(&mut self) {
        if let Some(slot) = &mut self.runtime {
            if let LoadedModel::Nemotron(m) = &mut slot.model {
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
        crate::memory::maybe_log_process_memory_with_sizes(
            "parakeet before transcribe_chunk",
            &[
                ("input_samples", samples.len()),
                (
                    "input_audio_bytes",
                    samples.len() * std::mem::size_of::<f32>(),
                ),
                (
                    "runtime_generation",
                    self.next_runtime_generation.saturating_sub(1) as usize,
                ),
            ],
        );
        // 1. Resample to 16 kHz if needed
        let audio = if sample_rate != 16000 {
            let needs_new_resampler = self
                .resampler
                .as_ref()
                .map_or(true, |(r, s, _)| *r != sample_rate || *s != samples.len());

            if needs_new_resampler {
                // sinc_len 64 + oversampling 32 are more than sufficient for 16kHz
                // speech and are ~5x faster than the audiophile-grade 256/256 defaults.
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
            waves[0].clone()
        } else {
            samples.to_vec()
        };
        crate::memory::maybe_log_process_memory_with_sizes(
            "parakeet after resample",
            &[
                ("input_samples", samples.len()),
                ("resampled_samples", audio.len()),
                (
                    "resampled_audio_bytes",
                    audio.len() * std::mem::size_of::<f32>(),
                ),
            ],
        );

        // 2. Transcribe
        if let Some(slot) = &mut self.runtime {
            let result = match &mut slot.model {
                LoadedModel::Nemotron(m) => {
                    let mut transcript = String::new();
                    const CHUNK_SIZE: usize = 8960; // 560 ms at 16 kHz
                    let total_subchunks = audio.chunks(CHUNK_SIZE).len();
                    for (idx, chunk) in audio.chunks(CHUNK_SIZE).enumerate() {
                        crate::memory::maybe_log_process_memory_with_sizes(
                            &format!(
                                "parakeet nemotron subchunk {}/{} start",
                                idx + 1,
                                total_subchunks
                            ),
                            &[
                                ("subchunk_samples", chunk.len()),
                                (
                                    "subchunk_audio_bytes",
                                    chunk.len() * std::mem::size_of::<f32>(),
                                ),
                                ("transcript_chars_so_far", transcript.len()),
                            ],
                        );
                        let mut chunk_vec = chunk.to_vec();
                        if chunk_vec.len() < CHUNK_SIZE {
                            chunk_vec.resize(CHUNK_SIZE, 0.0);
                        }
                        transcript.push_str(&m.transcribe_chunk(&chunk_vec).unwrap_or_default());
                        crate::memory::maybe_log_process_memory_with_sizes(
                            &format!(
                                "parakeet nemotron subchunk {}/{} end",
                                idx + 1,
                                total_subchunks
                            ),
                            &[
                                ("padded_subchunk_samples", chunk_vec.len()),
                                ("transcript_chars_so_far", transcript.len()),
                            ],
                        );
                    }
                    Ok(transcript)
                }
                LoadedModel::Ctc(m) => {
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "parakeet ctc before model run",
                        &[
                            ("audio_samples", audio.len()),
                            ("audio_bytes", audio.len() * std::mem::size_of::<f32>()),
                        ],
                    );
                    let result = m
                        .transcribe_samples(audio.clone(), 16000, 1, Some(TimestampMode::Words))
                        .map_err(|e| format!("CTC Error: {}", e))?;
                    println!("[PARAKEET CTC] {}", result.text.trim());
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "parakeet ctc after model run",
                        &[
                            ("audio_samples", audio.len()),
                            ("transcript_chars", result.text.len()),
                        ],
                    );
                    Ok(result.text)
                }
                LoadedModel::Eou(m) => {
                    let mut full_text = String::new();
                    const CHUNK_SIZE: usize = 2560; // 160 ms
                    let total_subchunks = audio.chunks(CHUNK_SIZE).len();
                    for (idx, chunk) in audio.chunks(CHUNK_SIZE).enumerate() {
                        crate::memory::maybe_log_process_memory_with_sizes(
                            &format!(
                                "parakeet eou subchunk {}/{} start",
                                idx + 1,
                                total_subchunks
                            ),
                            &[
                                ("subchunk_samples", chunk.len()),
                                ("transcript_chars_so_far", full_text.len()),
                            ],
                        );
                        let text = m.transcribe(&chunk.to_vec(), false).unwrap_or_default();
                        full_text.push_str(&text);
                        crate::memory::maybe_log_process_memory_with_sizes(
                            &format!("parakeet eou subchunk {}/{} end", idx + 1, total_subchunks),
                            &[
                                ("subchunk_samples", chunk.len()),
                                ("transcript_chars_so_far", full_text.len()),
                            ],
                        );
                    }
                    println!("[PARAKEET EOU] {}", full_text.trim());
                    Ok(full_text)
                }
                LoadedModel::Tdt(m) => {
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "parakeet tdt before model run",
                        &[
                            ("audio_samples", audio.len()),
                            ("audio_bytes", audio.len() * std::mem::size_of::<f32>()),
                        ],
                    );
                    let result = m
                        .transcribe_samples(audio.clone(), 16000, 1, Some(TimestampMode::Sentences))
                        .map_err(|e| format!("TDT Error: {}", e))?;
                    println!("[PARAKEET TDT] {}", result.text.trim());
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "parakeet tdt after model run",
                        &[
                            ("audio_samples", audio.len()),
                            ("transcript_chars", result.text.len()),
                        ],
                    );
                    Ok(result.text)
                }
            };
            if let Ok(ref transcript) = result {
                crate::memory::maybe_log_process_memory_with_sizes(
                    "parakeet after transcribe_chunk",
                    &[
                        ("resampled_samples", audio.len()),
                        ("transcript_chars", transcript.len()),
                        ("runtime_generation", slot.generation as usize),
                    ],
                );
            } else {
                crate::memory::maybe_log_process_memory_with_sizes(
                    "parakeet after transcribe_chunk error",
                    &[
                        ("resampled_samples", audio.len()),
                        ("runtime_generation", slot.generation as usize),
                    ],
                );
            }
            result
        } else {
            Err("No model loaded".to_string())
        }
    }
}
