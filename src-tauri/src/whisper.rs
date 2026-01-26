use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
}; // Import tools for resampling audio (changing sample rate)
use std::ffi::c_void; // Import raw pointer types for interacting with C code
use std::os::raw::c_char; // Import C-style character types
use whisper_rs::{
    set_log_callback, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
}; // Import the Whisper AI library functions

// Note: We don't embed the model in the binary because it's too big (hundreds of MBs)
// const MODEL_BYTES: &[u8] = ...;

/// GPU Backend type
/// Determines which hardware is powering the AI
#[derive(Debug, Clone)]
pub enum GpuBackend {
    Cuda,   // NVIDIA GPUs (Very Fast)
    Vulkan, // AMD/Intel/Other GPUs (Fast)
    Cpu,    // Processor (Slow fallback)
}

// Allow printing the backend name nicely (e.g. "CUDA" instead of "Cuda")
impl std::fmt::Display for GpuBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuBackend::Cuda => write!(f, "CUDA"),
            GpuBackend::Vulkan => write!(f, "Vulkan"),
            GpuBackend::Cpu => write!(f, "CPU"),
        }
    }
}

/// Information about a Whisper Model
/// Used to display options in the UI
#[derive(Debug, Clone, serde::Serialize)] // Serialize lets us send this to JavaScript
pub struct ModelInfo {
    pub id: String,           // Unique ID, e.g., "tiny.en-q5_1"
    pub display_name: String, // Pretty name, e.g., "Tiny English (Q5_1)"
    pub file_name: String,    // Proper filename, e.g., "ggml-tiny.en-q5_1.bin"
    pub size_mb: f32,         // How big it is in Megabytes
}

/// The Manager that controls the Whisper AI
pub struct WhisperManager {
    context: Option<WhisperContext>, // The loaded AI brain (can be None if not loaded yet)
    last_transcript: String,         // Memorizes what was said previously (context)
    backend: GpuBackend,             // Current hardware being used (CPU/GPU)
    current_model: Option<String>,   // Name of the currently loaded model
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>, // (Sample Rate, Chunk Size, Resampler)
}

// specialized "callback" function to hide confusing logs from the C++ library
// "unsafe" means we are doing dangerous manual memory management (needed for C interop)
unsafe extern "C" fn null_log_callback(_level: i32, _text: *const c_char, _user_data: *mut c_void) {
    // Do nothing. This swallows the logs so they don't clutter our terminal.
}

impl WhisperManager {
    /// Create a new Whisper Manager (Constructor)
    pub fn new() -> Self {
        Self {
            context: None,                  // Start with no model loaded
            last_transcript: String::new(), // Start with empty memory
            backend: GpuBackend::Cpu,       // Assume CPU until we prove otherwise
            current_model: None,            // No model selected yet
            resampler: None,
        }
    }

    /// Helper: Find the folder where models are stored
    fn get_models_dir() -> Result<std::path::PathBuf, String> {
        // Look in 3 places, just in case (current dir, parent, grandparent)
        let possible_paths = [
            "taurscribe-runtime/models",
            "../taurscribe-runtime/models",
            "../../taurscribe-runtime/models",
        ];

        // Loop through each guess
        for path in possible_paths {
            // Check if the path actually exists on the disk
            if let Ok(canonical) = std::fs::canonicalize(path) {
                if canonical.is_dir() {
                    // Check if it's the RIGHT folder by looking for .bin files (models)
                    if let Ok(entries) = std::fs::read_dir(&canonical) {
                        for entry in entries.flatten() {
                            if let Some(name) = entry.file_name().to_str() {
                                // If we find a file starting with "ggml-" and ending in ".bin"...
                                if name.starts_with("ggml-") && name.ends_with(".bin") {
                                    return Ok(canonical); // We found the right place!
                                }
                            }
                        }
                    }
                }
            }
        }

        // If we tried everywhere and failed...
        Err("Could not find models directory containing ggml models".to_string())
    }

    /// List all the models found in the models folder
    pub fn list_available_models() -> Result<Vec<ModelInfo>, String> {
        let models_dir = Self::get_models_dir()?; // Find the directory
        let mut models = Vec::new(); // List to hold our findings

        // Read all files in that directory
        let entries = std::fs::read_dir(&models_dir)
            .map_err(|e| format!("Failed to read models directory: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            if path.is_file() {
                // Get the filename (e.g. "ggml-tiny.bin")
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    // Filter: Must start with ggml-, end with .bin, and NOT be silero (VAD)
                    if file_name.starts_with("ggml-")
                        && file_name.ends_with(".bin")
                        && !file_name.contains("silero")
                    {
                        // Calculate file size in MB
                        let size_bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
                        let size_mb = size_bytes as f32 / (1024.0 * 1024.0);

                        // Extract the "ID" from the filename
                        // e.g. "ggml-tiny.en.bin" -> "tiny.en"
                        let id = file_name
                            .trim_start_matches("ggml-")
                            .trim_end_matches(".bin")
                            .to_string();

                        // Create a formatted nice name
                        let display_name = Self::format_model_name(&id);

                        // Add to our list
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

        // Sort the list by size (smallest models first)
        models.sort_by(|a, b| a.size_mb.partial_cmp(&b.size_mb).unwrap());

        Ok(models)
    }

    /// Helper: Turn a kryptic ID like "tiny.en-q5_1" into "Tiny English (Q5_1)"
    fn format_model_name(id: &str) -> String {
        let mut name = String::new();

        // 1. Determine size
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

        // 2. Determine Language
        if id.contains(".en") {
            name.push_str(" English");
        } else {
            name.push_str(" Multilingual");
        }

        // 3. Determine Quality/Compression (Quantization)
        if id.contains("q5_0") {
            name.push_str(" (Q5_0)");
        } else if id.contains("q5_1") {
            name.push_str(" (Q5_1)");
        } else if id.contains("q8_0") {
            name.push_str(" (Q8_0)");
        }

        // Fallback: if we couldn't parse it, just return the raw ID
        if name.is_empty() {
            return id.to_string();
        }

        name
    }

    /// Get the name of the currently loaded model
    pub fn get_current_model(&self) -> Option<&String> {
        self.current_model.as_ref()
    }

    /// Get which GPU backend we are using
    pub fn get_backend(&self) -> &GpuBackend {
        &self.backend
    }

    /// Wipe the "memory" of the conversation (clear context)
    /// Used when starting a completely new recording session
    pub fn clear_context(&mut self) {
        self.last_transcript.clear();
        println!("[INFO] Context cleared - starting fresh");
    }

    /// Initialize (Load) the Whisper Brain
    /// This loads the model file from disk into memory (and GPU)
    pub fn initialize(&mut self, model_id: Option<&str>) -> Result<String, String> {
        // Disable noisy C++ logs
        unsafe {
            set_log_callback(Some(null_log_callback), std::ptr::null_mut());
        }

        // Find the folder
        let models_dir = Self::get_models_dir()?;

        // Pick the model: Use argument if provided, otherwise default to "tiny.en-q5_1"
        let target_model = model_id.unwrap_or("tiny.en-q5_1");
        let file_name = format!("ggml-{}.bin", target_model);
        let absolute_path = models_dir.join(&file_name);

        // Verify file exists
        if !absolute_path.exists() {
            return Err(format!("Model file not found: {}", absolute_path.display()));
        }

        println!(
            "[INFO] Loading Whisper model from disk: '{}'",
            absolute_path.display()
        );

        // Try to load with GPU acceleration first. If that fails, fallback to CPU.
        let (ctx, backend) = self
            .try_gpu(&absolute_path)
            .or_else(|_| self.try_cpu(&absolute_path))?; // OR_ELSE is the fallback logic

        // Save the loaded state
        self.context = Some(ctx);
        self.backend = backend.clone();
        self.current_model = Some(target_model.to_string());

        let backend_msg = format!("Backend: {}", backend);
        println!("[INFO] {}", backend_msg);
        println!("[INFO] Model loaded: {}", target_model);

        // "Warm Up" the GPU
        println!("[INFO] Warming up GPU...");
        println!("[DEBUG] Creating warmup audio buffer...");
        let warmup_audio = vec![0.0_f32; 16000]; // Create 1 second of silence
        println!("[DEBUG] Starting transcribe_chunk for warmup...");
        match self.transcribe_chunk(&warmup_audio, 16000) {
            Ok(_) => println!("[INFO] GPU warm-up complete"),
            Err(e) => println!("[WARN] Warm-up failed (not critical): {}", e),
        }
        println!("[DEBUG] Initialization sequence finished.");

        Ok(backend_msg)
    }

    /// Helper: Try to initialize with GPU settings
    fn try_gpu(
        &self,
        model_path: &std::path::Path,
    ) -> Result<(WhisperContext, GpuBackend), String> {
        println!("[GPU] Attempting GPU acceleration...");

        // Configure Whisper to use GPU
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);

        // Attempt load
        match WhisperContext::new_with_params(model_path.to_str().unwrap(), params) {
            Ok(ctx) => {
                // Success! But which GPU backend? (CUDA vs Vulkan)
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

    /// Heuristic: Guess which GPU backend is active
    fn detect_gpu_backend(&self) -> GpuBackend {
        // If we can run 'nvidia-smi', the user definitely has NVIDIA drivers
        if self.is_cuda_available() {
            return GpuBackend::Cuda;
        }

        // Otherwise assume Vulkan (AMD/Intel/Apple)
        GpuBackend::Vulkan
    }

    /// Check for NVIDIA drivers
    fn is_cuda_available(&self) -> bool {
        std::process::Command::new("nvidia-smi")
            .output()
            .map(|output| output.status.success()) // True if command ran successfully
            .unwrap_or(false) // False if command failed/not found
    }

    /// Helper: Fallback to slow CPU mode
    fn try_cpu(
        &self,
        model_path: &std::path::Path,
    ) -> Result<(WhisperContext, GpuBackend), String> {
        println!("[GPU] Falling back to CPU...");

        // Default params = CPU only
        let params = WhisperContextParameters::default();

        match WhisperContext::new_with_params(model_path.to_str().unwrap(), params) {
            Ok(ctx) => {
                println!("[SUCCESS] ✓ CPU backend loaded");
                Ok((ctx, GpuBackend::Cpu))
            }
            Err(e) => Err(format!("Failed to load model: {:?}", e)),
        }
    }

    /// 🎤 Real-Time Transcription Function
    /// Takes a small chunk of audio (e.g. 6 seconds) and transcribes it
    pub fn transcribe_chunk(
        &mut self,
        samples: &[f32],        // Raw audio numbers
        input_sample_rate: u32, // e.g. 48000 Hz
    ) -> Result<String, String> {
        // Get access to the loaded brain
        let ctx = self
            .context
            .as_mut()
            .ok_or("Whisper context not initialized")?;

        // 🔧 STEP 1: Resample Audio
        let audio_data = if input_sample_rate != 16000 {
            // Check if we need to (re)create the resampler
            let needs_new = match &self.resampler {
                Some((rate, size, _)) => *rate != input_sample_rate || *size != samples.len(),
                None => true,
            };

            if needs_new {
                let params = SincInterpolationParameters {
                    sinc_len: 256,
                    f_cutoff: 0.95,
                    interpolation: SincInterpolationType::Linear,
                    window: WindowFunction::BlackmanHarris2,
                    oversampling_factor: 128,
                };
                let resampler = SincFixedIn::<f32>::new(
                    16000_f64 / input_sample_rate as f64,
                    2.0,
                    params,
                    samples.len(),
                    1,
                )
                .map_err(|e| format!("Failed to create resampler: {:?}", e))?;
                self.resampler = Some((input_sample_rate, samples.len(), Box::new(resampler)));
            }

            let (_, _, resampler) = self.resampler.as_mut().unwrap();
            let waves_in = vec![samples.to_vec()];
            let waves_out = resampler
                .process(&waves_in, None)
                .map_err(|e| format!("Resampling failed: {:?}", e))?;
            waves_out[0].clone()
        } else {
            samples.to_vec()
        };

        // 🧠 STEP 2: Create a state for this specific transcription task
        let mut state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {:?}", e))?;

        // ⚙️ STEP 3: Configure Transcription Parameters
        // "Greedy" strategy picks the most likely word immediately (fastest)
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        params.set_n_threads(4); // Use 4 CPU threads
        params.set_translate(false); // Don't translate to English, just transcribe
        params.set_language(Some("en")); // Assume English for now
        params.set_print_special(false); // Don't print <SOT>, <EOT>, etc.
        params.set_print_progress(false); // Don't print "10%... 20%..."
        params.set_print_realtime(false);
        params.set_print_timestamps(false); // Don't print timestamps "[00:01.000]"

        // 🧠 STEP 4: Context / Prompting
        // We feed the PREVIOUS text as a "prompt" to the AI.
        // This helps it understand context (e.g. if previous sentence was "The", next is likely "cat")
        if !self.last_transcript.is_empty() {
            params.set_initial_prompt(&self.last_transcript);
        }

        // Start timing the performance
        let start = std::time::Instant::now();

        // 🚀 STEP 5: Run the AI!
        state
            .full(params, &audio_data)
            .map_err(|e| format!("Transcription failed: {:?}", e))?;

        // 📝 STEP 6: Extract the text from the result
        let num_segments = state.full_n_segments();
        let mut transcript = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                transcript.push_str(&segment.to_string());
            }
        }

        let final_text = transcript.trim().to_string();

        // Update our "memory" so next chunk uses this text as context
        // NOTE: We accumulate text throughout the whole recording session
        if !final_text.is_empty() {
            if !self.last_transcript.is_empty() {
                self.last_transcript.push(' '); // Add a space
            }
            self.last_transcript.push_str(&final_text);
        }

        // Print performance stats
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

    /// 📁 Final File Transcription Function
    /// Processes a whole WAV file at once for maximum quality.
    pub fn transcribe_file(&mut self, file_path: &str) -> Result<String, String> {
        println!("[PROCESSING] Transcribing full file: {}", file_path);
        let total_start = std::time::Instant::now();

        let ctx = self
            .context
            .as_mut()
            .ok_or("Whisper context not initialized")?;

        // ===== STEP 1: Read WAV file =====
        let step1_start = std::time::Instant::now();

        // Open the file
        let mut reader = hound::WavReader::open(file_path)
            .map_err(|e| format!("Failed to open WAV file: {}", e))?;

        let spec = reader.spec();
        println!(
            "[INFO] WAV spec: {}Hz, {} channels",
            spec.sample_rate, spec.channels
        );

        // Read all samples into memory
        let sample_count = reader.len() as usize;
        let mut samples: Vec<f32> = Vec::with_capacity(sample_count);

        // Convert audio integers (16-bit) to floats (0.0 - 1.0)
        if spec.sample_format == hound::SampleFormat::Float {
            samples.extend(reader.samples::<f32>().map(|s| s.unwrap_or(0.0)));
        } else {
            samples.extend(
                reader
                    .samples::<i16>()
                    .map(|s| s.unwrap_or(0) as f32 / 32768.0), // Normalize i16 to f32
            );
        }

        let step1_ms = step1_start.elapsed().as_secs_f32() * 1000.0;
        println!("[TIMING] Step 1 (File I/O): {:.0}ms", step1_ms);

        // ===== STEP 2: Convert stereo to mono =====
        // Whisper requires mono (1 channel). If stereo (2 channels), average them.
        let step2_start = std::time::Instant::now();

        let mono_samples = if spec.channels == 2 {
            samples
                .chunks(2)
                .map(|chunk| (chunk[0] + chunk[1]) / 2.0) // (Left + Right) / 2
                .collect::<Vec<f32>>()
        } else {
            samples
        };

        let step2_ms = step2_start.elapsed().as_secs_f32() * 1000.0;
        println!("[TIMING] Step 2 (Stereo→Mono): {:.0}ms", step2_ms);

        // ===== STEP 3: Resample to 16kHz =====
        let step3_start = std::time::Instant::now();

        let audio_data = if spec.sample_rate != 16000 {
            // Setup resampler (same as before)
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                window: WindowFunction::BlackmanHarris2,
                oversampling_factor: 128,
            };

            // Process in "Chunks" to avoid eating too much RAM at once
            let chunk_size = 1024 * 10;
            let mut resampler = SincFixedIn::<f32>::new(
                16000_f64 / spec.sample_rate as f64,
                2.0,
                params,
                chunk_size,
                1,
            )
            .map_err(|e| format!("Failed to create resampler: {:?}", e))?;

            let mut resampled_audio = Vec::new();

            // Pad samples to handle the last partial chunk
            let mut padding = mono_samples.len() % chunk_size;
            if padding > 0 {
                padding = chunk_size - padding;
            }

            let mut padded_samples = mono_samples.clone();
            padded_samples.extend(std::iter::repeat(0.0).take(padding));

            // Resample loop
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

        // ===== STEP 4: Setup AI State =====
        let step4_start = std::time::Instant::now();

        let mut state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {:?}", e))?;

        // Optimize params for BATCH processing (Offline)
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(8); // Use MORE threads (8) since we are not recording live
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_max_len(1); // Optimization: Force model to be concise
        params.set_token_timestamps(false); // Optimization: Skip detailed timing math

        // Note: We do NOT use 'initial_prompt' here. This is a fresh start for the full file.

        let step4_ms = step4_start.elapsed().as_secs_f32() * 1000.0;
        println!("[TIMING] Step 4 (State Setup): {:.0}ms", step4_ms);

        // ===== STEP 5: Run Inference (The Main Event) =====
        let step5_start = std::time::Instant::now();

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

        // ===== STEP 6: Extract Text =====
        let step6_start = std::time::Instant::now();

        let num_segments = state.full_n_segments();
        let mut transcript = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                transcript.push_str(&segment.to_string());
                transcript.push(' '); // Add spaces between segments
            }
        }

        let step6_ms = step6_start.elapsed().as_secs_f32() * 1000.0;
        println!("[TIMING] Step 6 (Extract Text): {:.0}ms", step6_ms);

        // ===== TOTAL Stats =====
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

    /// Optimized: Transcribe raw audio data that is ALREADY loaded
    /// Used when we filter audio with VAD and don't want to re-read from disk
    pub fn transcribe_audio_data(&mut self, audio_data: &[f32]) -> Result<String, String> {
        let ctx = self
            .context
            .as_mut()
            .ok_or("Whisper context not initialized")?;

        println!(
            "[PROCESSING] Transcribing {} samples ({}s)...",
            audio_data.len(),
            audio_data.len() as f32 / 16000.0
        );
        let start = std::time::Instant::now();

        // Create state
        let mut state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create state: {:?}", e))?;

        // Use offline parameters (same as transcribe_file)
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(8);
        params.set_translate(false);
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_max_len(1);
        params.set_token_timestamps(false);

        // Run
        state
            .full(params, audio_data)
            .map_err(|e| format!("Transcription failed: {:?}", e))?;

        // Extract
        let num_segments = state.full_n_segments();
        let mut transcript = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                transcript.push_str(&segment.to_string());
                transcript.push(' ');
            }
        }

        let duration = start.elapsed();
        let audio_duration = audio_data.len() as f32 / 16000.0;
        let speedup = audio_duration / duration.as_secs_f32();

        println!(
            "[PERF] Transcribed sequence in {:.0}ms | Speed: {:.1}x",
            duration.as_millis(),
            speedup
        );

        Ok(transcript.trim().to_string())
    }

    /// Helper: Load and prepare a WAV file for VAD/Whisper
    /// Handles reading, mono conversion, and resampling in one go
    pub fn load_audio(&self, file_path: &str) -> Result<Vec<f32>, String> {
        println!("[I/O] Loading audio file: {}", file_path);

        // Open
        let mut reader = hound::WavReader::open(file_path)
            .map_err(|e| format!("Failed to open WAV file: {}", e))?;
        let spec = reader.spec();

        // Read
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

        // Mono
        let mono_samples = if spec.channels == 2 {
            samples
                .chunks(2)
                .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
                .collect::<Vec<f32>>()
        } else {
            samples
        };

        // Resample
        if spec.sample_rate != 16000 {
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                window: WindowFunction::BlackmanHarris2,
                oversampling_factor: 128,
            };

            let chunk_size = 1024 * 10;
            let mut resampler = SincFixedIn::<f32>::new(
                16000_f64 / spec.sample_rate as f64,
                2.0,
                params,
                chunk_size,
                1,
            )
            .map_err(|e| format!("Failed to create resampler: {:?}", e))?;

            let mut resampled_audio = Vec::new();

            // Padding
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
            Ok(resampled_audio)
        } else {
            Ok(mono_samples)
        }
    }
}
