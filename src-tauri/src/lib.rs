use cpal::traits::{DeviceTrait, HostTrait, StreamTrait}; // cpal is the library for microphone access
use crossbeam_channel::{unbounded, Sender}; // Channels act like pipes to send data between threads
use std::sync::{Arc, Mutex}; // Arc = Shared Ownership, Mutex = Exclusive Access (Thread Safety)
use tauri::tray::TrayIconBuilder; // Used to create the system tray icon (in the taskbar)
use tauri::{AppHandle, Emitter, Manager, State}; // Core Tauri types for app management

mod whisper; // Import the code from whisper.rs
use whisper::WhisperManager;

mod vad; // Import the code from vad.rs
use vad::VADManager;

mod parakeet; // Import the code from parakeet.rs (NVIDIA Parakeet ASR alternative)
use parakeet::ParakeetManager;

/// Defines the possible states of our application
/// This helps us decide which icon to show in the tray
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    Ready,      // Green: Waiting for user input
    Recording,  // Red: Mic is active, recording audio
    Processing, // Yellow: Computing/Transcribing
}

/// The possible ASR engines we support
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum ASREngine {
    Whisper,
    Parakeet,
}

/// Structured payload for live transcription chunks
#[derive(Debug, Clone, serde::Serialize)]
pub struct TranscriptionChunk {
    pub text: String,
    pub processing_time_ms: u32,
    pub method: String,
}

// Macros to load icon images into the executable at compile time.
// This is faster and safer than loading from disk at runtime.
macro_rules! tray_icon_green {
    () => {
        tauri::include_image!("icons/emoji-green_circle.ico")
    };
}
macro_rules! tray_icon_red {
    () => {
        tauri::include_image!("icons/emoji-red_circle.ico")
    };
}
macro_rules! tray_icon_yellow {
    () => {
        tauri::include_image!("icons/emoji-yellow_circle.ico")
    };
}

// Wrapper struct to make the Audio Stream "moveable" between threads.
// By default, raw pointers/streams aren't thread-safe.
// We implement Send and Sync manually (unsafe) to tell Rust "Check constraints are met".
#[allow(dead_code)]
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {} // Can be moved to another thread
unsafe impl Sync for SendStream {} // Can be accessed from multiple threads

/// Simple Post-Processing to clean up raw ASR artifacts
fn clean_transcript(text: &str) -> String {
    let mut cleaned = text.trim().to_string();

    // Fix floating punctuation
    cleaned = cleaned.replace(" ,", ",");
    cleaned = cleaned.replace(" .", ".");
    cleaned = cleaned.replace(" ?", "?");
    cleaned = cleaned.replace(" !", "!");

    // Fix percent signs
    cleaned = cleaned.replace(" %", "%");

    // Fix double spaces
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    // Capitalize first letter
    if let Some(first) = cleaned.chars().next() {
        if first.is_lowercase() {
            let mut c = cleaned.chars();
            cleaned = match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            };
        }
    }

    cleaned
}

/// The Global "Brain" of the application.
/// This struct holds all the data that needs to live as long as the app runs.
struct AudioState {
    // Holds the active recording stream. If None, we are not recording.
    // Use Mutex because we need to change it (start/stop) safely.
    recording_handle: Mutex<Option<RecordingHandle>>,

    // The Whisper AI engine. Wrapped in Arc<Mutex<>> so it can be shared and used by multiple threads.
    whisper: Arc<Mutex<WhisperManager>>,

    // The Parakeet AI engine (alternative to Whisper). Also shared across threads.
    parakeet: Arc<Mutex<ParakeetManager>>,

    // The Voice Activity Detector. Also shared.
    vad: Arc<Mutex<VADManager>>,

    // Remembers where we saved the last WAV file so we can process it when recording stops.
    last_recording_path: Mutex<Option<String>>,

    // Keeps track of whether we are Ready, Recording, or Processing.
    current_app_state: Mutex<AppState>,

    // Which ASR engine is currently active?
    active_engine: Mutex<ASREngine>,

    // Accumulates the full transcript during a recording session (for Parakeet streaming reuse)
    session_transcript: Arc<Mutex<String>>,
}

/// Keeps track of the tools needed while recording involves.
struct RecordingHandle {
    stream: SendStream,           // The actual connection to the microphone hardware
    file_tx: Sender<Vec<f32>>,    // Pipe to send audio to the "File Writer" thread
    whisper_tx: Sender<Vec<f32>>, // Pipe to send audio to the "Whisper AI" thread
}

// Simple test command to see if Rust is working
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Ask the backend what hardware is running the AI (CPU vs GPU)
#[tauri::command]
fn get_backend_info(state: State<AudioState>) -> Result<String, String> {
    let whisper = state.whisper.lock().unwrap(); // Lock the AI to read it
    Ok(format!("{}", whisper.get_backend())) // Return the backend name
}

/// List all available AI models found in the models folder
#[tauri::command]
fn list_models() -> Result<Vec<whisper::ModelInfo>, String> {
    whisper::WhisperManager::list_available_models() // Call static function in whisper.rs
}

/// Ask which model is currently loaded
#[tauri::command]
fn get_current_model(state: State<AudioState>) -> Result<Option<String>, String> {
    let whisper = state.whisper.lock().unwrap(); // Lock access
    Ok(whisper.get_current_model().cloned()) // Return a copy of the name
}

/// Test command: List Parakeet models
#[tauri::command]
fn list_parakeet_models() -> Result<Vec<parakeet::ParakeetModelInfo>, String> {
    parakeet::ParakeetManager::list_available_models()
}

/// Test command: Initialize Parakeet
#[tauri::command]
fn init_parakeet(state: State<AudioState>, model_id: Option<String>) -> Result<String, String> {
    let mut parakeet = state.parakeet.lock().unwrap();
    let result = parakeet.initialize(model_id.as_deref())?;

    // Auto-switch to parakeet if initialized
    *state.active_engine.lock().unwrap() = ASREngine::Parakeet;

    Ok(result)
}

/// Change the active ASR engine
#[tauri::command]
fn set_active_engine(state: State<AudioState>, engine: String) -> Result<String, String> {
    let new_engine = match engine.to_lowercase().as_str() {
        "whisper" => ASREngine::Whisper,
        "parakeet" => ASREngine::Parakeet,
        _ => return Err(format!("Unknown engine: {}", engine)),
    };

    *state.active_engine.lock().unwrap() = new_engine;
    println!("[ENGINE] Active engine switched to: {:?}", new_engine);
    Ok(format!("Engine switched to {:?}", new_engine))
}

/// Ask which engine is active
#[tauri::command]
fn get_active_engine(state: State<AudioState>) -> Result<ASREngine, String> {
    Ok(*state.active_engine.lock().unwrap())
}

/// Ask for Parakeet status (Model, Type, Backend)
#[tauri::command]
fn get_parakeet_status(state: State<AudioState>) -> Result<parakeet::ParakeetStatus, String> {
    let parakeet = state.parakeet.lock().unwrap();
    Ok(parakeet.get_status())
}

/// Command to swap the AI model (e.g. from Tiny to Large)
#[tauri::command]
fn switch_model(state: State<AudioState>, model_id: String) -> Result<String, String> {
    // 1. Safety Check: Don't switch models while recording!
    let handle = state.recording_handle.lock().unwrap();
    if handle.is_some() {
        return Err("Cannot switch models while recording".to_string());
    }
    drop(handle); // We are done checking, release the lock so others can use it.

    println!("[INFO] Switching to model: {}", model_id);

    // 2. Initialize the new model
    let mut whisper = state.whisper.lock().unwrap();
    whisper.initialize(Some(&model_id)) // This might take a few seconds
}

/// Update the system tray icon manually from the frontend
#[tauri::command]
fn set_tray_state(
    app: AppHandle,
    state: State<AudioState>,
    new_state: String,
) -> Result<(), String> {
    // Convert string command ("ready") to Enum (AppState::Ready)
    let app_state = match new_state.as_str() {
        "ready" => AppState::Ready,
        "recording" => AppState::Recording,
        "processing" => AppState::Processing,
        _ => return Err(format!("Unknown state: {}", new_state)), // Error on typos
    };

    // Update our internal memory of the state
    *state.current_app_state.lock().unwrap() = app_state;

    // Actually change the visual icon
    update_tray_icon(&app, app_state)?;

    Ok(())
}

/// Helper function to physically change the tray icon
fn update_tray_icon(app: &AppHandle, state: AppState) -> Result<(), String> {
    // Pick the right image macro based on state
    let icon = match state {
        AppState::Ready => tray_icon_green!(),
        AppState::Recording => tray_icon_red!(),
        AppState::Processing => tray_icon_yellow!(),
    };

    // Pick the right hover text
    let tooltip = match state {
        AppState::Ready => "Taurscribe - Ready",
        AppState::Recording => "Taurscribe - Recording...",
        AppState::Processing => "Taurscribe - Processing...",
    };

    // Find the tray item by ID and apply changes
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_icon(Some(icon))
            .map_err(|e| format!("Failed to set tray icon: {}", e))?;
        tray.set_tooltip(Some(tooltip))
            .map_err(|e| format!("Failed to set tooltip: {}", e))?;

        println!("[TRAY] State changed to: {:?}", state);
    }

    Ok(())
}

// Structure to describe a sample audio file
#[derive(serde::Serialize)]
struct SampleFile {
    name: String, // e.g. "Space.wav"
    path: String, // Full path on disk
}

/// List default sample files for testing
#[tauri::command]
fn list_sample_files() -> Result<Vec<SampleFile>, String> {
    let mut files = Vec::new();

    // Look for samples folder in common locations
    let possible_paths = [
        "taurscribe-runtime/samples",
        "../taurscribe-runtime/samples",
        "../../taurscribe-runtime/samples",
    ];

    let mut target_dir = std::path::PathBuf::new();
    let mut found = false;

    // Same search logic as searching for models...
    for path in possible_paths {
        if let Ok(p) = std::fs::canonicalize(path) {
            if p.is_dir() {
                // Check if folder is valid by looking for ANY .wav file
                if let Ok(entries) = std::fs::read_dir(&p) {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.to_lowercase().ends_with(".wav") {
                                target_dir = p;
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if found {
                    break;
                }
            }
        }
    }

    if !found {
        return Ok(vec![]); // Return empty list if no samples found (don't crash)
    }

    // Read all files in the found directory
    let entries = std::fs::read_dir(target_dir)
        .map_err(|e| format!("Failed to read samples directory: {}", e))?;

    for entry in entries {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_file() {
                // Only process .wav files
                if let Some(ext) = path.extension() {
                    if ext.to_string_lossy().to_lowercase() == "wav" {
                        if let Some(name) = path.file_name() {
                            files.push(SampleFile {
                                name: name.to_string_lossy().to_string(),
                                path: path.to_string_lossy().to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    // Sort alphabetically
    files.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(files)
}

/// RUN A PERFORMANCE TEST
/// This simulates a recording session using a pre-recorded file.
/// It compares "Simple" transcription vs "VAD-Optimized" transcription.
#[tauri::command]
fn benchmark_test(state: State<AudioState>, file_path: String) -> Result<String, String> {
    use std::time::Instant; // For timing execution

    println!("[BENCHMARK] Starting REALISTIC benchmark on: {}", file_path);
    println!("[BENCHMARK] Simulating actual recording workflow...");

    // Find the file on disk
    let absolute_path = std::fs::canonicalize(&file_path)
        .or_else(|_| std::fs::canonicalize(format!("../{}", file_path)))
        .or_else(|_| std::fs::canonicalize(format!("../../{}", file_path)))
        .map_err(|e| format!("Could not find file at '{}'. Error: {}", file_path, e))?;

    // ===== STEP 1: Load Audio Data =====
    println!("[BENCHMARK] Step 1: Loading WAV file...");
    let mut reader = hound::WavReader::open(&absolute_path)
        .map_err(|e| format!("Failed to open WAV file: {}", e))?;
    let spec = reader.spec();
    let sample_count = reader.len();

    // Calculate how long the audio is in seconds (Samples / Rate / Channels)
    let audio_duration_secs = sample_count as f32 / spec.sample_rate as f32 / spec.channels as f32;

    println!(
        "[BENCHMARK] Audio: {:.2}s, {}Hz, {} channels",
        audio_duration_secs, spec.sample_rate, spec.channels
    );

    // Read raw data into a Vector
    let mut samples: Vec<f32> = Vec::with_capacity(sample_count as usize);
    if spec.sample_format == hound::SampleFormat::Float {
        samples.extend(reader.samples::<f32>().map(|s| s.unwrap_or(0.0)));
    } else {
        // Convert integer samples to float (-1.0 to 1.0)
        samples.extend(
            reader
                .samples::<i16>()
                .map(|s| s.unwrap_or(0) as f32 / 32768.0),
        );
    }

    // Convert to Mono (AI needs 1 channel)
    let mono_samples = if spec.channels == 2 {
        samples
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
            .collect::<Vec<f32>>()
    } else {
        samples
    };

    // ===== PREPARE SIMULATION =====
    let sample_rate = spec.sample_rate;
    let chunk_duration_secs = 6; // Simulate 6-second buffers (same as real app)
    let chunk_size = (sample_rate * chunk_duration_secs) as usize;
    let num_chunks = (mono_samples.len() + chunk_size - 1) / chunk_size; // Ceiling division

    println!(
        "[BENCHMARK] Processing {} chunks of {}s each...",
        num_chunks, chunk_duration_secs
    );

    // --- TEST 1: WHISPER (Naive & Optimized) ---
    state.whisper.lock().unwrap().clear_context();
    let start_whisper_naive = Instant::now();
    for chunk in mono_samples.chunks(chunk_size) {
        state
            .whisper
            .lock()
            .unwrap()
            .transcribe_chunk(chunk, sample_rate)
            .ok();
    }
    state
        .whisper
        .lock()
        .unwrap()
        .transcribe_file(absolute_path.to_str().unwrap())
        .ok();
    let time_whisper_naive = start_whisper_naive.elapsed();

    state.whisper.lock().unwrap().clear_context();
    let start_whisper_vad = Instant::now();
    let mut chunks_skipped = 0;
    for chunk in mono_samples.chunks(chunk_size) {
        let is_speech = state.vad.lock().unwrap().is_speech(chunk).unwrap_or(0.6);
        if is_speech > 0.5 {
            state
                .whisper
                .lock()
                .unwrap()
                .transcribe_chunk(chunk, sample_rate)
                .ok();
        } else {
            chunks_skipped += 1;
        }
    }
    // Final Pass with VAD
    {
        let mut whisper = state.whisper.lock().unwrap();
        let audio_data = whisper.load_audio(absolute_path.to_str().unwrap()).unwrap();
        let mut vad = state.vad.lock().unwrap();
        let timestamps = vad.get_speech_timestamps(&audio_data, 500).unwrap();
        let mut clean = Vec::new();
        for (s, e) in timestamps {
            let start = (s * 16000.0) as usize;
            let end = (e * 16000.0) as usize;
            clean.extend_from_slice(
                &audio_data[start.min(audio_data.len())..end.min(audio_data.len())],
            );
        }
        if !clean.is_empty() {
            whisper.transcribe_audio_data(&clean).ok();
        }
    }
    let time_whisper_vad = start_whisper_vad.elapsed();

    // --- TEST 2: PARAKEET (Streaming Simulation) ---
    // Parakeet uses 1.12s chunks for high accuracy (NVIDIA Spec [70, 13])
    // The benchmark simulates the exact "Streaming" workflow used in production.
    let parakeet_chunk_size = (sample_rate as f32 * 1.12) as usize;
    let parakeet_manager = state.parakeet.clone();

    let start_parakeet = Instant::now();
    for chunk in mono_samples.chunks(parakeet_chunk_size) {
        parakeet_manager
            .lock()
            .unwrap()
            .transcribe_chunk(chunk, sample_rate)
            .ok();
    }
    // Note: We do NOT run a final pass for Parakeet in production, so we don't benchmark it here.
    let time_parakeet = start_parakeet.elapsed();

    // --- CALCULATE RESULTS ---
    let factor_whisper = audio_duration_secs / time_whisper_vad.as_secs_f32();
    let factor_parakeet = audio_duration_secs / time_parakeet.as_secs_f32();

    let winner = if time_whisper_vad < time_parakeet {
        "Whisper AI"
    } else {
        "NVIDIA Parakeet"
    };

    Ok(format!(
        "📊 EXTENSIVE CUDA BENCHMARK RESULTS\n\
        ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
        🎙️ WHISPER AI:\n\
        - Baseline (No VAD): {:.2}s\n\
        - Optimized (With VAD): {:.2}s\n\
        - Speed Factor: {:.1}x Real-time\n\n\
        🦜 NVIDIA PARAKEET:\n\
        - Streaming (No VAD): {:.2}s\n\
        - Speed Factor: {:.1}x Real-time\n\
        ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
        🏆 WINNER: {} is faster on your system!\n\
        📉 Resource Usage: Whisper skipped {}/{} chunks",
        time_whisper_naive.as_secs_f32(),
        time_whisper_vad.as_secs_f32(),
        factor_whisper,
        time_parakeet.as_secs_f32(),
        factor_parakeet,
        winner,
        chunks_skipped,
        num_chunks
    ))
}

/// Helper: Find or create the directory to save recordings
fn get_recordings_dir() -> Result<std::path::PathBuf, String> {
    // Get the standard AppData folder (C:\Users\Name\AppData\Local)
    let app_data = dirs::data_local_dir().ok_or("Could not find AppData directory")?;

    // Append our specific folder: ...\Taurscribe\temp
    let recordings_dir = app_data.join("Taurscribe").join("temp");

    // Create folder if it doesn't exist
    std::fs::create_dir_all(&recordings_dir)
        .map_err(|e| format!("Failed to create recordings directory: {}", e))?;

    Ok(recordings_dir)
}

/// COMMAND: START RECORDING
/// This initializes the microphone, files, and processing threads.
#[tauri::command]
fn start_recording(
    app_handle: tauri::AppHandle,
    state: State<AudioState>,
) -> Result<String, String> {
    // 1. Setup Microphone
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No input device")?;
    let config: cpal::StreamConfig = device
        .default_input_config()
        .map_err(|e| e.to_string())?
        .into();

    // 2. Prepare Output File
    let recordings_dir = get_recordings_dir()?;
    // Name file with timestamp so it's unique
    let filename = format!("recording_{}.wav", chrono::Utc::now().timestamp());
    let path = recordings_dir.join(&filename);

    println!("[INFO] Saving recording to: {}", path.display());

    // 3. Reset AI Context (Start fresh for new recording)
    let active_engine = *state.active_engine.lock().unwrap();
    if active_engine == ASREngine::Whisper {
        state.whisper.lock().unwrap().clear_context();
    } else {
        // For Parakeet, we also want to start fresh
        state.parakeet.lock().unwrap().clear_context();
    }

    // Remember this path for when we stop later
    *state.last_recording_path.lock().unwrap() = Some(path.to_string_lossy().into_owned());

    // Reset session transcript for the new recording
    state.session_transcript.lock().unwrap().clear();

    // 4. Create proper WAV header settings (48kHz, 32-bit float, etc.)
    let spec = hound::WavSpec {
        channels: config.channels,
        sample_rate: config.sample_rate.0,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    // Initialize the FileWriter
    let writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;

    // 5. Create COMMUNICATION PIPES (Channels)
    // We split the audio into two copies: one for saving, one for AI.
    let (file_tx, file_rx) = unbounded::<Vec<f32>>(); // Pipe 1 -> File Thread
    let (whisper_tx, whisper_rx) = unbounded::<Vec<f32>>(); // Pipe 2 -> Whisper Thread

    // We need clones of the "transmitter" ends to give to the microphone callback
    let file_tx_clone = file_tx.clone();
    let whisper_tx_clone = whisper_tx.clone();

    let sample_rate = config.sample_rate.0;

    // 6. SPAWN THREAD 1: THE FILE SAVER
    // Responsible for saving audio to disk safely without lagging the UI.
    std::thread::spawn(move || {
        let mut writer = writer;

        // Loop: Wait for data -> Write to disk
        while let Ok(samples) = file_rx.recv() {
            for sample in samples {
                writer.write_sample(sample).ok();
            }
        }
        // When loop breaks (channel closed), save and close file
        writer.finalize().ok();
        println!("WAV file saved.");
    });

    // Get shared references to our AI tools
    let whisper = state.whisper.clone();
    let parakeet_manager = state.parakeet.clone();
    let vad = state.vad.clone();
    let active_engine = *state.active_engine.lock().unwrap();
    let session_transcript = state.session_transcript.clone();

    // 7. SPAWN THREAD 2: THE REAL-TIME TRANSCRIBER
    // Responsible for doing "Live Preview" transcription.
    let app_clone = app_handle.clone();
    std::thread::spawn(move || {
        let mut buffer = Vec::new(); // Holds incoming audio (mainly for Whisper)

        // Chunk size for Whisper (6s)
        let chunk_size = (sample_rate * 6) as usize;
        let max_buffer_size = chunk_size * 2;

        println!(
            "[INFO] Runtime Transcriber thread started (Engine: {:?})",
            active_engine
        );
        let engine_icon = if active_engine == ASREngine::Whisper {
            "🎙️"
        } else {
            "🦜"
        };

        // Loop: Receive audio chunks from Mic
        while let Ok(samples) = whisper_rx.recv() {
            if active_engine == ASREngine::Whisper {
                buffer.extend(samples);

                // Whisper logic: Process in 6s chunks
                while buffer.len() >= chunk_size {
                    if buffer.len() > max_buffer_size {
                        println!("[WARNING] Buffer full, dropping old audio to catch up");
                        buffer.drain(..chunk_size);
                    }
                    let chunk: Vec<f32> = buffer.drain(..chunk_size).collect();
                    let is_speech = vad.lock().unwrap().is_speech(&chunk).unwrap_or(0.5);

                    if is_speech > 0.5 {
                        println!(
                            "[PROCESSING] 🎙️ Speech ({:.0}%) - Transcribing {:.2}s chunk...",
                            is_speech * 100.0,
                            6.0 // Whisper uses 6s chunks
                        );
                        let start_time = std::time::Instant::now();
                        match whisper
                            .lock()
                            .unwrap()
                            .transcribe_chunk(&chunk, sample_rate)
                        {
                            Ok(transcript) => {
                                if !transcript.trim().is_empty() {
                                    let elapsed = start_time.elapsed().as_millis() as u32;
                                    println!(
                                        "[TRANSCRIPT] \"{}\" (took {}ms)",
                                        transcript, elapsed
                                    );
                                    let _ = app_clone.emit(
                                        "transcription-chunk",
                                        TranscriptionChunk {
                                            text: transcript,
                                            processing_time_ms: elapsed,
                                            method: "Whisper".to_string(),
                                        },
                                    );
                                }
                            }
                            Err(e) => eprintln!("[ERROR] Whisper error: {}", e),
                        }
                    } else {
                        println!(
                            "[VAD] 🔇 Silence ({:.0}%) - Skipping Whisper chunk",
                            (1.0 - is_speech) * 100.0
                        );
                    }
                }
            } else {
                // Parakeet logic: Process in ~1.12s chunks (NVIDIA Spec [70, 13] for Best Accuracy)
                // We reuse the same buffer logic as Whisper but with the optimized chunk size
                buffer.extend(samples);

                // 1.12s chunk size = 17920 samples at 16kHz
                // This corresponds to NVIDIA's "High Accuracy" operating point.
                let parakeet_chunk_size = (sample_rate as f32 * 1.12) as usize;
                let max_buffer_size = parakeet_chunk_size * 2;

                while buffer.len() >= parakeet_chunk_size {
                    if buffer.len() > max_buffer_size {
                        buffer.drain(..parakeet_chunk_size); // Drop if falling behind
                    }

                    let chunk: Vec<f32> = buffer.drain(..parakeet_chunk_size).collect();
                    let start_time = std::time::Instant::now();

                    // Optional: You could enable VAD here if desired, but Parakeet is fast enough to just run.
                    match parakeet_manager
                        .lock()
                        .unwrap()
                        .transcribe_chunk(&chunk, sample_rate)
                    {
                        Ok(transcript) => {
                            if !transcript.trim().is_empty() {
                                let elapsed = start_time.elapsed().as_millis() as u32;
                                println!(
                                    "[TRANSCRIPT] 🦜 \"{}\" (took {}ms)",
                                    transcript.trim(),
                                    elapsed
                                );
                                let _ = app_clone.emit(
                                    "transcription-chunk",
                                    TranscriptionChunk {
                                        text: transcript.trim().to_string(),
                                        processing_time_ms: elapsed,
                                        method: "Parakeet".to_string(),
                                    },
                                );

                                // Append to session transcript
                                let mut session = session_transcript.lock().unwrap();
                                if !session.is_empty() {
                                    session.push(' ');
                                }
                                session.push_str(transcript.trim());
                            }
                        }
                        Err(e) => eprintln!("[ERROR] Parakeet error: {}", e),
                    }
                }
            }
        }

        // 9. CLEANUP: Processing leftover audio after stop
        println!("[INFO] Recording stopped, processing remaining audio...");
        while buffer.len() >= chunk_size {
            // ... (Process remaining full chunks) ...
            let chunk: Vec<f32> = buffer.drain(..chunk_size).collect();
            // Same logic as above...
            if active_engine == ASREngine::Whisper {
                whisper
                    .lock()
                    .unwrap()
                    .transcribe_chunk(&chunk, sample_rate)
                    .ok();
            } else {
                let mut p_manager = parakeet_manager.lock().unwrap();
                if let Ok(transcript) = p_manager.transcribe_chunk(&chunk, sample_rate) {
                    if !transcript.trim().is_empty() {
                        let mut session = session_transcript.lock().unwrap();
                        if !session.is_empty() {
                            session.push(' ');
                        }
                        session.push_str(transcript.trim());
                        println!("[TRANSCRIPT] 🦜 (Final) \"{}\"", transcript.trim());
                    }
                }
            }
        }

        // Process the very last partial chunk
        if !buffer.is_empty() {
            let chunk_duration = buffer.len() as f32 / sample_rate as f32;
            if chunk_duration > 0.1 {
                // Lower threshold to capture end of words
                if active_engine == ASREngine::Whisper {
                    whisper
                        .lock()
                        .unwrap()
                        .transcribe_chunk(&buffer, sample_rate)
                        .ok();
                } else {
                    let mut p_manager = parakeet_manager.lock().unwrap();
                    if let Ok(transcript) = p_manager.transcribe_chunk(&buffer, sample_rate) {
                        if !transcript.trim().is_empty() {
                            let mut session = session_transcript.lock().unwrap();
                            if !session.is_empty() {
                                session.push(' ');
                            }
                            session.push_str(transcript.trim());
                            println!("[TRANSCRIPT] 🦜 (Final Partial) \"{}\"", transcript.trim());
                        }
                    }
                }
            }
        }

        println!("[INFO] Transcriber thread finished");
    });

    let channels = config.channels as usize;

    // 10. Start the Microphone Stream
    // This connects to the OS audio system
    let stream = device
        .build_input_stream(
            &config,
            // CALLBACK: Runs ~100 times per second with new audio data
            move |data: &[f32], _: &_| {
                // 1. Send exact copy to file writer
                file_tx_clone.send(data.to_vec()).ok();

                // 2. Mix to MONO for Whisper
                // (Average left and right channels)
                let mono_data: Vec<f32> = if channels > 1 {
                    data.chunks(channels)
                        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };

                // Send mono copy to Whisper thread
                whisper_tx_clone.send(mono_data).ok();
            },
            move |err| {
                eprintln!("Audio input error: {}", err);
            },
            None,
        )
        .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;

    // Save the active stream handle so we can stop it later
    *state.recording_handle.lock().unwrap() = Some(RecordingHandle {
        stream: SendStream(stream),
        file_tx,
        whisper_tx,
    });

    Ok(format!("Recording started: {}", path.display()))
}

/// COMMAND: STOP RECORDING
#[tauri::command]
fn stop_recording(state: State<AudioState>) -> Result<String, String> {
    // 1. Get the handle and "take" it (sets the global state to None)
    let mut handle = state.recording_handle.lock().unwrap();
    if let Some(recording) = handle.take() {
        // Dropping these objects triggers the "hang up"
        drop(recording.stream); // Stop Mic
        drop(recording.file_tx); // Tell File Thread to finish
        drop(recording.whisper_tx); // Tell Whisper Thread to finish

        // Wait a small bit for file to close properly
        std::thread::sleep(std::time::Duration::from_millis(500));

        // 2. Run FINAL "Professional" Transcription
        let active_engine = *state.active_engine.lock().unwrap();

        // Parakeet is fast enough in streaming mode, so we SKIP the final pass to keep it snappy.
        // We return the accumulated text we gathered during the real-time stream.
        if active_engine == ASREngine::Parakeet {
            println!("[PROCESSING] Skipping final pass (Parakeet streaming is sufficient)");
            let transcript = state.session_transcript.lock().unwrap().clone();
            let final_text = if transcript.is_empty() {
                "Recording saved.".to_string()
            } else {
                clean_transcript(&transcript)
            };
            println!("[FINAL_TRANSCRIPT] (Cleaned)\n{}", final_text);
            return Ok(final_text);
        }

        let path_opt = state.last_recording_path.lock().unwrap().clone();
        if let Some(path) = path_opt {
            println!(
                "[PROCESSING] Running final high-quality transcription with VAD on: {}",
                path
            );

            // Access Audio State
            let mut whisper = state.whisper.lock().unwrap();
            let audio_data = whisper.load_audio(&path)?;

            // Whisper: Use VAD to cut out silence (prevents hallucinations)
            println!("[PROCESSING] Applying VAD filtering for Whisper...");
            let mut vad = state.vad.lock().unwrap();
            let timestamps = vad.get_speech_timestamps(&audio_data, 500)?;

            if timestamps.is_empty() {
                return Ok("[silence]".to_string());
            }

            let mut clean = Vec::with_capacity(audio_data.len());
            for (start, end) in timestamps {
                let s = (start * 16000.0) as usize;
                let e = (end * 16000.0) as usize;
                clean.extend_from_slice(
                    &audio_data[s.min(audio_data.len())..e.min(audio_data.len())],
                );
            }

            // Step D: Transcribe the audio (Whisper Only)
            let result = whisper.transcribe_audio_data(&clean);

            match result {
                Ok(text) => {
                    println!("[FINAL_TRANSCRIPT]\n{}", text);
                    Ok(text)
                }
                Err(e) => {
                    eprintln!("[ERROR] Final transcription failed: {}", e);
                    Ok(format!("Recording saved, but transcription failed: {}", e))
                }
            }
        } else {
            Ok("Recording saved.".to_string())
        }
    } else {
        Err("Not recording".to_string())
    }
}

/// BACKGROUND: Listen for Ctrl+Win global hotkeys
fn start_hotkey_listener(app_handle: AppHandle) {
    use rdev::{listen, Event, EventType, Key};
    use std::sync::atomic::{AtomicBool, Ordering};

    // Shared "flags" to remember if keys are pressed
    let ctrl_held = Arc::new(AtomicBool::new(false));
    let meta_held = Arc::new(AtomicBool::new(false)); // Meta = Windows Key
    let recording_active = Arc::new(AtomicBool::new(false));

    // Clones for the closure
    let ctrl_held_clone = ctrl_held.clone();
    let meta_held_clone = meta_held.clone();
    let recording_active_clone = recording_active.clone();
    let app_handle_clone = app_handle.clone();

    // The callback runs for EVERY key press on the system
    let callback = move |event: Event| {
        match event.event_type {
            EventType::KeyPress(key) => {
                match key {
                    Key::ControlLeft | Key::ControlRight => {
                        ctrl_held_clone.store(true, Ordering::SeqCst);
                    }
                    Key::MetaLeft | Key::MetaRight => {
                        meta_held_clone.store(true, Ordering::SeqCst);
                    }
                    _ => {}
                }

                // CHECK: Are BOTH keys pressed? And are we NOT recording?
                if ctrl_held_clone.load(Ordering::SeqCst)
                    && meta_held_clone.load(Ordering::SeqCst)
                    && !recording_active_clone.load(Ordering::SeqCst)
                {
                    recording_active_clone.store(true, Ordering::SeqCst);
                    println!("[HOTKEY] Ctrl+Win pressed - Starting recording");

                    // Send signal to frontend to simulate button click
                    let _ = app_handle_clone.emit("hotkey-start-recording", ());
                }
            }
            EventType::KeyRelease(key) => {
                match key {
                    Key::ControlLeft | Key::ControlRight => {
                        ctrl_held_clone.store(false, Ordering::SeqCst);
                    }
                    Key::MetaLeft | Key::MetaRight => {
                        meta_held_clone.store(false, Ordering::SeqCst);
                    }
                    _ => {}
                }

                // If keys released, STOP recording
                if recording_active_clone.load(Ordering::SeqCst)
                    && (!ctrl_held_clone.load(Ordering::SeqCst)
                        || !meta_held_clone.load(Ordering::SeqCst))
                {
                    recording_active_clone.store(false, Ordering::SeqCst);
                    println!("[HOTKEY] Ctrl+Win released - Stopping recording");

                    let _ = app_handle_clone.emit("hotkey-stop-recording", ());
                }
            }
            _ => {}
        }
    };

    // Start the listener (this blocks the thread, so it must be in a spawn)
    if let Err(error) = listen(callback) {
        eprintln!("[ERROR] Hotkey listener error: {:?}", error);
    }
}

/// MAIN ENTRY POINT
/// This is where the app starts!
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 1. Initialize Whisper AI
    // We run this in a separate thread with a LARGE stack size (8MB).
    // This fixes the "STATUS_STACK_BUFFER_OVERRUN" (0xc0000409) crash on Windows/CUDA.
    println!("[INFO] Initializing Whisper transcription engine...");
    let whisper = WhisperManager::new();

    let (whisper, init_result) = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024) // 8 MiB Stack
        .spawn(move || {
            let mut whisper = whisper;
            let res = whisper.initialize(None);
            (whisper, res)
        })
        .expect("Failed to spawn whisper init thread")
        .join()
        .expect("Whisper init thread panicked");

    match init_result {
        Ok(backend_msg) => {
            println!("[SUCCESS] {}", backend_msg);
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize Whisper: {}", e);
            eprintln!("   Transcription will be disabled.");
        }
    }

    // 2. Initialize VAD
    println!("[INFO] Initializing Voice Activity Detection...");
    let vad = VADManager::new().unwrap_or_else(|e| {
        eprintln!("[ERROR] Failed to initialize VAD: {}", e);
        panic!("VAD initialization failed");
    });
    println!("[SUCCESS] VAD initialized successfully");

    // 3. Initialize Parakeet & Load Model
    println!("[INFO] Initializing Parakeet ASR manager...");
    let mut parakeet = ParakeetManager::new();

    // Try to auto-load Nemotron execution first, then fallback to whatever is found
    println!("[INFO] Attempting to auto-load Parakeet model...");
    match parakeet.initialize(Some("nemotron:nemotron")) {
        Ok(msg) => println!("[SUCCESS] {}", msg),
        Err(_) => {
            // Fallback to first available (e.g. CTC)
            match parakeet.initialize(None) {
                Ok(msg) => println!("[SUCCESS] Fallback load: {}", msg),
                Err(e) => eprintln!("[WARN] No Parakeet models loaded: {}", e),
            }
        }
    }

    // 4. Build the Tauri App
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build()) // Persistence
        // Inject our global state so commands can access it
        .manage(AudioState {
            recording_handle: Mutex::new(None),
            whisper: Arc::new(Mutex::new(whisper)),
            parakeet: Arc::new(Mutex::new(parakeet)),
            vad: Arc::new(Mutex::new(vad)),
            last_recording_path: Mutex::new(None),
            current_app_state: Mutex::new(AppState::Ready),
            active_engine: Mutex::new(ASREngine::Whisper),
            session_transcript: Arc::new(Mutex::new(String::new())),
        })
        .setup(|app| {
            // Setup System Tray
            use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
            let show_item = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

            let icon = tray_icon_green!();

            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .tooltip("Taurscribe - Ready")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    use tauri::tray::TrayIconEvent;
                    if let TrayIconEvent::Click { .. } = event {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            println!("[INFO] System tray icon created");

            // Start Hotkey Listener in Background Thread
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                start_hotkey_listener(app_handle);
            });

            println!("[INFO] Global hotkey listener started (Ctrl+Win to record)");
            Ok(())
        })
        .on_window_event(|window, event| {
            // If user clicks "X", hide to tray instead of killing app
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
                println!("[INFO] Window minimized to tray");
            }
        })
        // Register all our commands so JavaScript can call them
        .invoke_handler(tauri::generate_handler![
            greet,
            start_recording,
            stop_recording,
            get_backend_info,
            benchmark_test,
            list_sample_files,
            list_models,
            get_current_model,
            switch_model,
            set_tray_state,
            list_parakeet_models,
            init_parakeet,
            get_parakeet_status,
            set_active_engine,
            get_active_engine
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
