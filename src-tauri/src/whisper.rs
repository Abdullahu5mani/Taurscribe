use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::ffi::c_void;
use std::os::raw::c_char;
use whisper_rs::{
    set_log_callback, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

// NO embedded model (too large for compiler!)
// const MODEL_BYTES: &[u8] = ...;

/// GPU Backend type
#[derive(Debug, Clone)]
pub enum GpuBackend {
    Cuda,
    Vulkan,
    Cpu,
}

impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Cuda => write!(f, "CUDA"),
            GpuBackend::Vulkan => write!(f, "Vulkan"),
            GpuBackend::Cpu => write!(f, "CPU"),
        }
    }
}

/// Available Whisper models with their display names and file paths
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub id: String,           // e.g., "tiny.en-q5_1"
    pub display_name: String, // e.g., "Tiny English (Q5_1)"
    pub file_name: String,    // e.g., "ggml-tiny.en-q5_1.bin"
    pub size_mb: f32,         // Approximate size in MB
}

/// Whisper transcription manager
pub struct WhisperManager {
    context: Option<WhisperContext>,
    last_transcript: String,
    backend: GpuBackend,
    current_model: Option<String>, // Currently loaded model ID
}

// C-compatible callback to suppress logs
unsafe extern "C" fn null_log_callback(_level: i32, _text: *const c_char, _user_data: *mut c_void) {
    // Do nothing - silences logging
}

impl WhisperManager {
    /// Create a new Whisper manager
    pub fn new() -> Self {
        Self {
            context: None,
            last_transcript: String::new(),
            backend: GpuBackend::Cpu, // Default to CPU until initialized
            current_model: None,
        }
    }

    /// Get the models directory path
    fn get_models_dir() -> Result<std::path::PathBuf, String> {
        // Try multiple possible locations
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

        Err("Could not find models directory".to_string())
    }

    /// List all available Whisper models
    pub fn list_available_models() -> Result<Vec<ModelInfo>, String> {
        let models_dir = Self::get_models_dir()?;
        let mut models = Vec::new();

        let entries = std::fs::read_dir(&models_dir)
            .map_err(|e| format!("Failed to read models directory: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    // Skip non-whisper models (like silero VAD)
                    if file_name.starts_with("ggml-")
                        && file_name.ends_with(".bin")
                        && !file_name.contains("silero")
                    {
                        let size_bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
                        let size_mb = size_bytes as f32 / (1024.0 * 1024.0);

                        // Parse model info from filename
                        // e.g., "ggml-tiny.en-q5_1.bin" -> id="tiny.en-q5_1", name="Tiny English (Q5_1)"
                        let id = file_name
                            .trim_start_matches("ggml-")
                            .trim_end_matches(".bin")
                            .to_string();

                        let display_name = Self::format_model_name(&id);

                        models.push(ModelInfo {
                            id,
                            display_name,
                            file_name: file_name.to_string(),
                            size_mb,
                        });
                    }
                }
            }
        }

        // Sort by size (smallest first)
        models.sort_by(|a, b| a.size_mb.partial_cmp(&b.size_mb).unwrap());

        Ok(models)
    }

    /// Format a model ID into a human-readable name
    fn format_model_name(id: &str) -> String {
        let mut name = String::new();

        // Parse size (tiny, base, small, medium, large)
        if id.contains("tiny") {
            name.push_str("Tiny");
        } else if id.contains("base") {
            name.push_str("Base");
        } else if id.contains("small") {
            name.push_str("Small");
        } else if id.contains("medium") {
            name.push_str("Medium");
        } else if id.contains("large-v3-turbo") {
            name.push_str("Large V3 Turbo");
        } else if id.contains("large-v3") {
            name.push_str("Large V3");
        } else if id.contains("large") {
            name.push_str("Large");
        }

        // Add language indicator
        if id.contains(".en") {
            name.push_str(" English");
        } else {
            name.push_str(" Multilingual");
        }

        // Add quantization info
        if id.contains("q5_0") {
            name.push_str(" (Q5_0)");
        } else if id.contains("q5_1") {
            name.push_str(" (Q5_1)");
        } else if id.contains("q8_0") {
            name.push_str(" (Q8_0)");
        }

        if name.is_empty() {
            return id.to_string(); // Fallback to raw ID
        }

        name
    }

    /// Get the currently loaded model name
    pub fn get_current_model(&self) -> Option<&String> {
        self.current_model.as_ref()
    }

    /// Get the current GPU backend being used
    pub fn get_backend(&self) -> &GpuBackend {
        &self.backend
    }

    /// Clear the transcript history (call when starting a new recording)
    pub fn clear_context(&mut self) {
        self.last_transcript.clear();
        println!("[INFO] Context cleared - starting fresh");
    }

    /// Initialize the Whisper context (loads the model from DISK with GPU support)
    /// If model_id is None, uses the default model (tiny.en-q5_1)
    pub fn initialize(&mut self, model_id: Option<&str>) -> Result<String, String> {
        // Suppress verbose C++ logs from whisper.cpp
        unsafe {
            set_log_callback(Some(null_log_callback), std::ptr::null_mut());
        }

        // Get models directory
        let models_dir = Self::get_models_dir()?;

        // Determine which model to load
        let target_model = model_id.unwrap_or("tiny.en-q5_1");
        let file_name = format!("ggml-{}.bin", target_model);
        let absolute_path = models_dir.join(&file_name);

        if !absolute_path.exists() {
            return Err(format!("Model file not found: {}", absolute_path.display()));
        }

        println!(
            "[INFO] Loading Whisper model from disk: '{}'",
            absolute_path.display()
        );

        // Try GPU first, fallback to CPU
        let (ctx, backend) = self
            .try_gpu(&absolute_path)
            .or_else(|_| self.try_cpu(&absolute_path))?;

        self.context = Some(ctx);
        self.backend = backend.clone();
        self.current_model = Some(target_model.to_string());

        let backend_msg = format!("Backend: {}", backend);
        println!("[INFO] {}", backend_msg);
        println!("[INFO] Model loaded: {}", target_model);

        // Warm-up pass: Run a dummy transcription to compile GPU kernels
        // This eliminates the "cold start" on the first real chunk
        println!("[INFO] Warming up GPU...");
        let warmup_audio = vec![0.0_f32; 16000]; // 1 second of silence at 16kHz
        match self.transcribe_chunk(&warmup_audio, 16000) {
            Ok(_) => println!("[INFO] GPU warm-up complete"),
            Err(e) => println!("[WARN] Warm-up failed (not critical): {}", e),
        }

        Ok(backend_msg)
    }

    /// Attempt to load model with GPU acceleration
    fn try_gpu(
        &self,
        model_path: &std::path::Path,
    ) -> Result<(WhisperContext, GpuBackend), String> {
        println!("[GPU] Attempting GPU acceleration...");

        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);

        match WhisperContext::new_with_params(model_path.to_str().unwrap(), params) {
            Ok(ctx) => {
                // Detect which GPU backend is actually being used
                // whisper.cpp tries CUDA first, then Vulkan
                let backend = self.detect_gpu_backend();
                println!("[SUCCESS] ✓ GPU acceleration enabled ({})", backend);
                Ok((ctx, backend))
            }
            Err(e) => {
                println!("[GPU] ✗ GPU failed: {:?}", e);
                Err(format!("GPU failed: {:?}", e))
            }
        }
    }

    /// Detect which GPU backend is being used
    /// Since whisper.cpp tries CUDA first, then Vulkan, we check in that order
    fn detect_gpu_backend(&self) -> GpuBackend {
        // Check if CUDA is available (nvidia-smi exists = NVIDIA GPU present)
        if self.is_cuda_available() {
            return GpuBackend::Cuda;
        }

        // If no NVIDIA GPU, assume Vulkan (AMD/Intel GPU or universal fallback)
        // whisper.cpp will use Vulkan if compiled with it
        GpuBackend::Vulkan
    }

    /// Check if CUDA is available on the system
    fn is_cuda_available(&self) -> bool {
        // Check for nvidia-smi (NVIDIA GPU present)
        std::process::Command::new("nvidia-smi")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Fallback to CPU if GPU fails
    fn try_cpu(
        &self,
        model_path: &std::path::Path,
    ) -> Result<(WhisperContext, GpuBackend), String> {
        println!("[GPU] Falling back to CPU...");

        let params = WhisperContextParameters::default();

        match WhisperContext::new_with_params(model_path.to_str().unwrap(), params) {
            Ok(ctx) => {
                println!("[SUCCESS] ✓ CPU backend loaded");
                Ok((ctx, GpuBackend::Cpu))
            }
            Err(e) => Err(format!("Failed to load model: {:?}", e)),
        }
    }

    /// Transcribe a 3-second audio chunk
    ///
    /// # Arguments
    /// * `samples` - Audio samples (f32)
    /// * `sample_rate` - Input sample rate (e.g. 48000, 44100)
    pub fn transcribe_chunk(
        &mut self,
        samples: &[f32],
        input_sample_rate: u32,
    ) -> Result<String, String> {
        let ctx = self
            .context
            .as_mut()
            .ok_or("Whisper context not initialized")?;

        // Convert samples if needed (Whisper expects 16kHz)
        let audio_data = if input_sample_rate != 16000 {
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                window: WindowFunction::BlackmanHarris2,
                oversampling_factor: 128,
            };

            let mut resampler = SincFixedIn::<f32>::new(
                16000_f64 / input_sample_rate as f64, // ratio
                2.0,                                  // max_resample_ratio_relative
                params,
                samples.len(), // input chunk size
                1,             // channels
            )
            .map_err(|e| format!("Failed to create resampler: {:?}", e))?;

            // rubato expects a Vec<Vec<f32>> (channels)
            let waves_in = vec![samples.to_vec()];
            let waves_out = resampler
                .process(&waves_in, None)
                .map_err(|e| format!("Resampling failed: {:?}", e))?;

            waves_out[0].clone()
        } else {
            samples.to_vec()
        };

        // Create state for this transcription
        let mut state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {:?}", e))?;

        // Set up parameters for transcription
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Configure parameters
        params.set_n_threads(4);
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // FEEDBACK HISTORY: Use the previous transcript as context
        if !self.last_transcript.is_empty() {
            params.set_initial_prompt(&self.last_transcript);
        }

        // Start timing
        let start = std::time::Instant::now();

        // Run transcription
        state
            .full(params, &audio_data)
            .map_err(|e| format!("Transcription failed: {:?}", e))?;

        // Get the transcribed text
        let num_segments = state.full_n_segments();

        let mut transcript = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                // Convert segment to string directly
                transcript.push_str(&segment.to_string());
            }
        }

        let final_text = transcript.trim().to_string();

        // Update history for next time (CUMULATIVE: append all chunks)
        if !final_text.is_empty() {
            // Append new chunk to existing transcript
            if !self.last_transcript.is_empty() {
                self.last_transcript.push(' '); // Add space between chunks
            }
            self.last_transcript.push_str(&final_text);
        }

        // Calculate performance metrics
        let duration = start.elapsed();
        let audio_duration_sec = audio_data.len() as f32 / 16000.0;
        let speedup = audio_duration_sec / duration.as_secs_f32();

        println!(
            "[PERF] Processed {:.2}s audio in {:.0}ms | Speed: {:.1}x",
            audio_duration_sec,
            duration.as_millis(),
            speedup
        );

        Ok(final_text)
    }

    /// Transcribe a full WAV file
    pub fn transcribe_file(&mut self, file_path: &str) -> Result<String, String> {
        println!("[PROCESSING] Transcribing full file: {}", file_path);
        let total_start = std::time::Instant::now();

        let ctx = self
            .context
            .as_mut()
            .ok_or("Whisper context not initialized")?;

        // ===== STEP 1: Read WAV file =====
        let step1_start = std::time::Instant::now();

        // Read the WAV file
        let mut reader = hound::WavReader::open(file_path)
            .map_err(|e| format!("Failed to open WAV file: {}", e))?;

        let spec = reader.spec();
        println!(
            "[INFO] WAV spec: {}Hz, {} channels",
            spec.sample_rate, spec.channels
        );

        // Read all samples and convert to f32 (optimized with pre-allocation)
        let sample_count = reader.len() as usize;
        let mut samples: Vec<f32> = Vec::with_capacity(sample_count);

        if spec.sample_format == hound::SampleFormat::Float {
            samples.extend(reader.samples::<f32>().map(|s| s.unwrap_or(0.0)));
        } else {
            samples.extend(
                reader
                    .samples::<i16>()
                    .map(|s| s.unwrap_or(0) as f32 / 32768.0),
            );
        }

        let step1_ms = step1_start.elapsed().as_secs_f32() * 1000.0;
        println!("[TIMING] Step 1 (File I/O): {:.0}ms", step1_ms);

        // ===== STEP 2: Convert stereo to mono =====
        let step2_start = std::time::Instant::now();

        // Convert stereo to mono if needed
        let mono_samples = if spec.channels == 2 {
            samples
                .chunks(2)
                .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
                .collect::<Vec<f32>>()
        } else {
            samples
        };

        let step2_ms = step2_start.elapsed().as_secs_f32() * 1000.0;
        println!("[TIMING] Step 2 (Stereo→Mono): {:.0}ms", step2_ms);

        // ===== STEP 3: Resample to 16kHz =====
        let step3_start = std::time::Instant::now();

        // Downsample to 16kHz if needed (using rubato)
        let audio_data = if spec.sample_rate != 16000 {
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                window: WindowFunction::BlackmanHarris2,
                oversampling_factor: 128,
            };

            // Process in chunks of 1024 samples to avoid memory issues with large files
            let chunk_size = 1024 * 10;
            let mut resampler = SincFixedIn::<f32>::new(
                16000_f64 / spec.sample_rate as f64, // ratio
                2.0,                                 // max_resample_ratio_relative
                params,
                chunk_size, // input chunk size
                1,          // channels
            )
            .map_err(|e| format!("Failed to create resampler: {:?}", e))?;

            let mut resampled_audio = Vec::new();

            // Pad samples to multiple of chunk_size
            let mut padding = mono_samples.len() % chunk_size;
            if padding > 0 {
                padding = chunk_size - padding;
            }

            let mut padded_samples = mono_samples.clone();
            padded_samples.extend(std::iter::repeat(0.0).take(padding));

            for chunk in padded_samples.chunks(chunk_size) {
                let waves_in = vec![chunk.to_vec()];
                if let Ok(waves_out) = resampler.process(&waves_in, None) {
                    resampled_audio.extend(&waves_out[0]);
                }
            }

            resampled_audio
        } else {
            mono_samples
        };

        let step3_ms = step3_start.elapsed().as_secs_f32() * 1000.0;
        println!(
            "[TIMING] Step 3 (Resampling {}kHz→16kHz): {:.0}ms",
            spec.sample_rate / 1000,
            step3_ms
        );

        println!("[INFO] Processing {} samples at 16kHz", audio_data.len());

        // ===== STEP 4: Create Whisper state =====
        let step4_start = std::time::Instant::now();

        // Create state
        let mut state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {:?}", e))?;

        // Set up parameters (optimized for speed)
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(8); // Increased from 4 (use more CPU for encoding)
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_max_len(1); // Speed: Don't generate extra tokens
        params.set_token_timestamps(false); // Speed: Skip token-level timing

        let step4_ms = step4_start.elapsed().as_secs_f32() * 1000.0;
        println!("[TIMING] Step 4 (State Setup): {:.0}ms", step4_ms);

        // ===== STEP 5: Whisper inference =====
        let step5_start = std::time::Instant::now();

        // Run transcription
        state
            .full(params, &audio_data)
            .map_err(|e| format!("Transcription failed: {:?}", e))?;

        let step5_ms = step5_start.elapsed().as_secs_f32() * 1000.0;
        let audio_duration_sec = audio_data.len() as f32 / 16000.0;
        let inference_speedup = audio_duration_sec / (step5_ms / 1000.0);
        println!(
            "[TIMING] Step 5 (Whisper AI): {:.0}ms | {:.1}x realtime",
            step5_ms, inference_speedup
        );

        // ===== STEP 6: Extract segments =====
        let step6_start = std::time::Instant::now();

        // Get full transcript
        let num_segments = state.full_n_segments();

        let mut transcript = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                transcript.push_str(&segment.to_string());
                transcript.push(' ');
            }
        }

        let step6_ms = step6_start.elapsed().as_secs_f32() * 1000.0;
        println!("[TIMING] Step 6 (Extract Text): {:.0}ms", step6_ms);

        // ===== TOTAL TIME SUMMARY =====
        let total_ms = total_start.elapsed().as_secs_f32() * 1000.0;
        let total_speedup = audio_duration_sec / (total_ms / 1000.0);

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!(
            "[PERF] Processed {:.2}s audio in {:.0}ms total | Speed: {:.1}x",
            audio_duration_sec, total_ms, total_speedup
        );
        println!("[BREAKDOWN] I/O:{:.0}ms + Stereo:{:.0}ms + Resample:{:.0}ms + Setup:{:.0}ms + AI:{:.0}ms + Extract:{:.0}ms",
            step1_ms, step2_ms, step3_ms, step4_ms, step5_ms, step6_ms);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        Ok(transcript.trim().to_string())
    }
}
