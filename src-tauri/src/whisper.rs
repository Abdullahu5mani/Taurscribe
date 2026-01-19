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

/// Whisper transcription manager
pub struct WhisperManager {
    context: Option<WhisperContext>,
    last_transcript: String,
    backend: GpuBackend,
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
        }
    }

    /// Get the current GPU backend being used
    pub fn get_backend(&self) -> &GpuBackend {
        &self.backend
    }

    /// Initialize the Whisper context (loads the model from DISK with GPU support)
    pub fn initialize(&mut self) -> Result<String, String> {
        // Suppress verbose C++ logs from whisper.cpp
        unsafe {
            set_log_callback(Some(null_log_callback), std::ptr::null_mut());
        }

        // Path to the large model
        // Line 66 in whisper.rs - change to:
        let model_path = "taurscribe-runtime/models/ggml-base.en-q5_0.bin";
        let absolute_path = std::fs::canonicalize(model_path)
            .or_else(|_| std::fs::canonicalize(format!("../{}", model_path)))
            .or_else(|_| std::fs::canonicalize(format!("../../{}", model_path)))
            .map_err(|e| format!("Could not find model at '{}'. Error: {}", model_path, e))?;

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

        let backend_msg = format!("Backend: {}", backend);
        println!("[INFO] {}", backend_msg);

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

        // Update history for next time (keep only the last chunk to avoid infinite growth)
        if !final_text.is_empty() {
            self.last_transcript = final_text.clone();
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

        let ctx = self
            .context
            .as_mut()
            .ok_or("Whisper context not initialized")?;

        // Read the WAV file
        let mut reader = hound::WavReader::open(file_path)
            .map_err(|e| format!("Failed to open WAV file: {}", e))?;

        let spec = reader.spec();
        println!(
            "[INFO] WAV spec: {}Hz, {} channels",
            spec.sample_rate, spec.channels
        );

        // Read all samples and convert to f32
        let samples: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
            reader.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect()
        } else {
            reader
                .samples::<i16>()
                .map(|s| s.unwrap_or(0) as f32 / 32768.0)
                .collect()
        };

        // Convert stereo to mono if needed
        let mono_samples = if spec.channels == 2 {
            samples
                .chunks(2)
                .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
                .collect::<Vec<f32>>()
        } else {
            samples
        };

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

        println!("[INFO] Processing {} samples at 16kHz", audio_data.len());

        // Create state
        let mut state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {:?}", e))?;

        // Set up parameters
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(4);
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // Start timing
        let start = std::time::Instant::now();

        // Run transcription
        state
            .full(params, &audio_data)
            .map_err(|e| format!("Transcription failed: {:?}", e))?;

        // Get full transcript
        let num_segments = state.full_n_segments();

        let mut transcript = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                transcript.push_str(&segment.to_string());
                transcript.push(' ');
            }
        }

        // Calculate performance metrics
        let duration = start.elapsed();
        let audio_duration_sec = audio_data.len() as f32 / 16000.0;
        let speedup = audio_duration_sec / duration.as_secs_f32();

        println!(
            "[PERF] Processed {:.2}s full file in {:.0}ms | Speed: {:.1}x",
            audio_duration_sec,
            duration.as_millis(),
            speedup
        );

        Ok(transcript.trim().to_string())
    }
}
