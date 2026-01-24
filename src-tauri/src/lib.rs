use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{unbounded, Sender};
use std::sync::{Arc, Mutex};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};

mod whisper;
use whisper::WhisperManager;

mod vad;
use vad::VADManager;

/// App states for tray icon colors
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    Ready,      // Green - ready to record
    Recording,  // Red - currently recording
    Processing, // Yellow - processing/transcribing
}

// Embed tray icons at compile time using Tauri's include_image macro
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

// Wrapper to make cpal::Stream Send/Sync.
// Safety: We only use this to keep the stream alive and drop it.
#[allow(dead_code)]
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

struct AudioState {
    recording_handle: Mutex<Option<RecordingHandle>>,
    whisper: Arc<Mutex<WhisperManager>>,
    vad: Arc<Mutex<VADManager>>, // Voice Activity Detection
    last_recording_path: Mutex<Option<String>>,
    current_app_state: Mutex<AppState>,
}

struct RecordingHandle {
    stream: SendStream,
    file_tx: Sender<Vec<f32>>,
    whisper_tx: Sender<Vec<f32>>,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_backend_info(state: State<AudioState>) -> Result<String, String> {
    let whisper = state.whisper.lock().unwrap();
    Ok(format!("{}", whisper.get_backend()))
}

/// List all available Whisper models
#[tauri::command]
fn list_models() -> Result<Vec<whisper::ModelInfo>, String> {
    whisper::WhisperManager::list_available_models()
}

/// Get the currently loaded model
#[tauri::command]
fn get_current_model(state: State<AudioState>) -> Result<Option<String>, String> {
    let whisper = state.whisper.lock().unwrap();
    Ok(whisper.get_current_model().cloned())
}

/// Switch to a different Whisper model
#[tauri::command]
fn switch_model(state: State<AudioState>, model_id: String) -> Result<String, String> {
    // Check if recording
    let handle = state.recording_handle.lock().unwrap();
    if handle.is_some() {
        return Err("Cannot switch models while recording".to_string());
    }
    drop(handle); // Release lock early

    println!("[INFO] Switching to model: {}", model_id);

    let mut whisper = state.whisper.lock().unwrap();
    whisper.initialize(Some(&model_id))
}

/// Update the tray icon based on app state
#[tauri::command]
fn set_tray_state(
    app: AppHandle,
    state: State<AudioState>,
    new_state: String,
) -> Result<(), String> {
    let app_state = match new_state.as_str() {
        "ready" => AppState::Ready,
        "recording" => AppState::Recording,
        "processing" => AppState::Processing,
        _ => return Err(format!("Unknown state: {}", new_state)),
    };

    // Update stored state
    *state.current_app_state.lock().unwrap() = app_state;

    // Update tray icon
    update_tray_icon(&app, app_state)?;

    Ok(())
}

/// Update the tray icon to match the current app state
fn update_tray_icon(app: &AppHandle, state: AppState) -> Result<(), String> {
    let icon = match state {
        AppState::Ready => tray_icon_green!(),
        AppState::Recording => tray_icon_red!(),
        AppState::Processing => tray_icon_yellow!(),
    };

    let tooltip = match state {
        AppState::Ready => "Taurscribe - Ready",
        AppState::Recording => "Taurscribe - Recording...",
        AppState::Processing => "Taurscribe - Processing...",
    };

    // Get the tray icon
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_icon(Some(icon))
            .map_err(|e| format!("Failed to set tray icon: {}", e))?;
        tray.set_tooltip(Some(tooltip))
            .map_err(|e| format!("Failed to set tooltip: {}", e))?;

        println!("[TRAY] State changed to: {:?}", state);
    }

    Ok(())
}

#[derive(serde::Serialize)]
struct SampleFile {
    name: String,
    path: String,
}

#[tauri::command]
fn list_sample_files() -> Result<Vec<SampleFile>, String> {
    let mut files = Vec::new();

    // Check usual locations for samples directory
    let possible_paths = [
        "taurscribe-runtime/samples",
        "../taurscribe-runtime/samples",
        "../../taurscribe-runtime/samples",
    ];

    let mut target_dir = std::path::PathBuf::new();
    let mut found = false;

    for path in possible_paths {
        if let Ok(p) = std::fs::canonicalize(path) {
            if p.is_dir() {
                // Check if this directory actually contains any .wav files
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
        return Ok(vec![]); // Return empty if not found, don't error
    }

    let entries = std::fs::read_dir(target_dir)
        .map_err(|e| format!("Failed to read samples directory: {}", e))?;

    for entry in entries {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    // Only include .wav files
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

    // Sort by name
    files.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(files)
}

#[tauri::command]
fn benchmark_test(state: State<AudioState>, file_path: String) -> Result<String, String> {
    use std::time::Instant;

    println!("[BENCHMARK] Starting REALISTIC benchmark on: {}", file_path);
    println!("[BENCHMARK] Simulating actual recording workflow...");

    // Resolve the file path
    let absolute_path = std::fs::canonicalize(&file_path)
        .or_else(|_| std::fs::canonicalize(format!("../{}", file_path)))
        .or_else(|_| std::fs::canonicalize(format!("../../{}", file_path)))
        .map_err(|e| format!("Could not find file at '{}'. Error: {}", file_path, e))?;

    // ===== STEP 1: Load and prepare audio =====
    println!("[BENCHMARK] Step 1: Loading WAV file...");
    let mut reader = hound::WavReader::open(&absolute_path)
        .map_err(|e| format!("Failed to open WAV file: {}", e))?;
    let spec = reader.spec();
    let sample_count = reader.len();
    let audio_duration_secs = sample_count as f32 / spec.sample_rate as f32 / spec.channels as f32;

    println!(
        "[BENCHMARK] Audio: {:.2}s, {}Hz, {} channels",
        audio_duration_secs, spec.sample_rate, spec.channels
    );

    // Read all samples
    let mut samples: Vec<f32> = Vec::with_capacity(sample_count as usize);
    if spec.sample_format == hound::SampleFormat::Float {
        samples.extend(reader.samples::<f32>().map(|s| s.unwrap_or(0.0)));
    } else {
        samples.extend(
            reader
                .samples::<i16>()
                .map(|s| s.unwrap_or(0) as f32 / 32768.0),
        );
    }

    // Convert stereo to mono if needed
    let mono_samples = if spec.channels == 2 {
        samples
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
            .collect::<Vec<f32>>()
    } else {
        samples
    };

    // ===== BENCHMARK CONFIGURATION =====
    let sample_rate = spec.sample_rate;
    let chunk_duration_secs = 6;
    let chunk_size = (sample_rate * chunk_duration_secs) as usize;
    let num_chunks = (mono_samples.len() + chunk_size - 1) / chunk_size;

    println!(
        "[BENCHMARK] Processing {} chunks of {}s each...",
        num_chunks, chunk_duration_secs
    );

    // ===== RUN A vs B BENCHMARK =====

    // --- RUN 1: WITHOUT VAD (Baseline) ---
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🏃 RUN 1: Baseline (NO VAD)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    state.whisper.lock().unwrap().clear_context();
    let start_novad = Instant::now();

    // Real-time phase (No VAD)
    for chunk in mono_samples.chunks(chunk_size) {
        // Always transcribe (wasteful)
        state
            .whisper
            .lock()
            .unwrap()
            .transcribe_chunk(chunk, sample_rate)
            .ok();
    }

    // Final transcription (Full File - No filtering)
    state
        .whisper
        .lock()
        .unwrap()
        .transcribe_file(absolute_path.to_str().unwrap())?;

    let time_novad = start_novad.elapsed();

    // --- RUN 2: WITH VAD (Optimized) ---
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🚀 RUN 2: Optimized (WITH VAD)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    state.whisper.lock().unwrap().clear_context();
    let start_withvad = Instant::now();

    // Real-time phase (With VAD)
    let mut chunks_processed = 0;
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
            chunks_processed += 1;
        } else {
            chunks_skipped += 1; // Skip
        }
    }

    // Final transcription (With VAD Filtering)
    {
        let mut whisper = state.whisper.lock().unwrap();
        let audio_data = whisper.load_audio(absolute_path.to_str().unwrap()).unwrap();
        let mut vad = state.vad.lock().unwrap();
        let timestamps = vad.get_speech_timestamps(&audio_data, 500).unwrap();

        // Construct Clean Audio
        let mut clean_audio = Vec::with_capacity(audio_data.len());
        for (start, end) in timestamps {
            let start_idx = (start * 16000.0) as usize;
            let end_idx = (end * 16000.0) as usize;
            let start_idx = start_idx.min(audio_data.len());
            let end_idx = end_idx.min(audio_data.len());
            if start_idx < end_idx {
                clean_audio.extend_from_slice(&audio_data[start_idx..end_idx]);
            }
        }

        if !clean_audio.is_empty() {
            whisper.transcribe_audio_data(&clean_audio).ok();
        }
    }

    let time_withvad = start_withvad.elapsed();

    // --- RESULTS SUMMARY ---
    let speedup_pct = ((time_novad.as_secs_f32() - time_withvad.as_secs_f32())
        / time_novad.as_secs_f32())
        * 100.0;
    let factor_gain = time_novad.as_secs_f32() / time_withvad.as_secs_f32();

    println!("\n📊 BENCHMARK COMPARISON RESULTS");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("⏱️  Baseline (No VAD):  {:.2}s", time_novad.as_secs_f32());
    println!(
        "🚀 Optimized (With VAD): {:.2}s",
        time_withvad.as_secs_f32()
    );
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(
        "⚡ IMPROVEMENT: {:.0}% Faster ({:.1}x Speedup)",
        speedup_pct, factor_gain
    );
    println!(
        "📉 Resource Usage: Skipped {}/{} realtime chunks",
        chunks_skipped,
        chunks_processed + chunks_skipped
    );
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    Ok(format!(
        "📊 VAD PERFORMANCE BENCHMARK\n\n\
        ⏱️  Baseline (No VAD):  {:.2}s\n\
        🚀 Optimized (With VAD): {:.2}s\n\n\
        ⚡ IMPROVEMENT: {:.0}% Faster ({:.1}x Speedup)\n\
        📉 Skipped: {} chunks (silence)",
        time_novad.as_secs_f32(),
        time_withvad.as_secs_f32(),
        speedup_pct,
        factor_gain,
        chunks_skipped
    ))
}

/// Helper function to get the AppData directory for recordings
/// Creates the directory structure if it doesn't exist
fn get_recordings_dir() -> Result<std::path::PathBuf, String> {
    // Get AppData\Local directory (cross-platform way)
    let app_data = dirs::data_local_dir().ok_or("Could not find AppData directory")?;

    // Create our app folder: AppData\Local\Taurscribe\temp
    let recordings_dir = app_data.join("Taurscribe").join("temp");

    // Create the directory if it doesn't exist
    std::fs::create_dir_all(&recordings_dir)
        .map_err(|e| format!("Failed to create recordings directory: {}", e))?;

    Ok(recordings_dir)
}

#[tauri::command]
fn start_recording(state: State<AudioState>) -> Result<String, String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No input device")?;
    let config: cpal::StreamConfig = device
        .default_input_config()
        .map_err(|e| e.to_string())?
        .into();

    // Get the AppData directory for storing recordings
    let recordings_dir = get_recordings_dir()?;

    let filename = format!("recording_{}.wav", chrono::Utc::now().timestamp());
    let path = recordings_dir.join(&filename);

    println!("[INFO] Saving recording to: {}", path.display());

    // Clear previous transcript context (start fresh for new recording)
    state.whisper.lock().unwrap().clear_context();

    // Save path for final high-quality transcription
    *state.last_recording_path.lock().unwrap() = Some(path.to_string_lossy().into_owned());

    // Create the "recipe" for our WAV file
    // This tells the file writer how to format the audio data
    let spec = hound::WavSpec {
        channels: config.channels, // How many audio channels (1=mono, 2=stereo)
        sample_rate: config.sample_rate.0, // Samples per second (e.g., 44100 = CD quality)
        bits_per_sample: 32,       // Precision of each sample (32-bit = very precise)
        sample_format: hound::SampleFormat::Float, // Use decimal numbers (0.5, -0.3, etc.)
    };

    // Create the actual WAV file writer using our "recipe"
    // If creation fails, convert the error to a string and return it (the ? does this)
    let writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;

    // Create TWO "conveyor belts" (channels) for parallel processing
    // Channel 1: For file writing
    let (file_tx, file_rx) = unbounded::<Vec<f32>>();

    // Channel 2: For Whisper transcription simulation
    let (whisper_tx, whisper_rx) = unbounded::<Vec<f32>>();

    // Clone the transmitters for use in the audio callback
    let file_tx_clone = file_tx.clone();
    let whisper_tx_clone = whisper_tx.clone();

    let sample_rate = config.sample_rate.0;

    // ============================================
    // THREAD 1: File Writer (existing logic)
    // ============================================
    std::thread::spawn(move || {
        let mut writer = writer;

        while let Ok(samples) = file_rx.recv() {
            for sample in samples {
                writer.write_sample(sample).ok();
            }
        }
        writer.finalize().ok();
        println!("WAV file saved.");
    });

    // Clone whisper manager and VAD for the thread (Arc allows shared ownership)
    let whisper = state.whisper.clone();
    let vad = state.vad.clone();

    // ============================================
    // THREAD 2: Whisper Processor (REAL TRANSCRIPTION)
    // ============================================
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        // Increase context to 6 seconds for better real-time accuracy
        // 3s is too short and cuts sentences, causing hallucinations like "Our evidence is a key"
        let chunk_size = (sample_rate * 6) as usize;
        let max_buffer_size = chunk_size * 2; // Hold 12 seconds total

        println!("[INFO] Whisper thread started");

        // Keep receiving audio chunks until the channel closes
        while let Ok(samples) = whisper_rx.recv() {
            buffer.extend(samples);

            // Process chunks when we have enough audio
            while buffer.len() >= chunk_size {
                // Don't let buffer grow beyond max size (2 chunks)
                if buffer.len() > max_buffer_size {
                    println!("[WARNING] Buffer exceeded max size, dropping old audio");
                    buffer.drain(..chunk_size);
                }

                // Extract one 6-second chunk
                let chunk: Vec<f32> = buffer.drain(..chunk_size).collect();

                // ===== VAD CHECK: Is this chunk speech or silence? =====
                let is_speech = vad.lock().unwrap().is_speech(&chunk).unwrap_or(0.5); // Default to 0.5 if VAD fails

                let chunk_duration = chunk.len() as f32 / sample_rate as f32;

                if is_speech > 0.5 {
                    // Speech detected - transcribe it
                    println!(
                        "[PROCESSING] 🎤 Speech ({:.0}%) - Transcribing {:.2}s chunk...",
                        is_speech * 100.0,
                        chunk_duration
                    );

                    // REAL TRANSCRIPTION!
                    match whisper
                        .lock()
                        .unwrap()
                        .transcribe_chunk(&chunk, sample_rate)
                    {
                        Ok(transcript) => {
                            if transcript.is_empty() {
                                println!("[TRANSCRIPT] [silence]");
                            } else {
                                println!("[TRANSCRIPT] \"{}\"", transcript);
                            }
                        }
                        Err(e) => {
                            eprintln!("[ERROR] Transcription error: {}", e);
                        }
                    }
                } else {
                    // Silence - skip transcription (save GPU/CPU)
                    println!(
                        "[VAD] 🔇 Silence ({:.0}%) - Skipping {:.2}s chunk",
                        (1.0 - is_speech) * 100.0,
                        chunk_duration
                    );
                }
            }
        }

        // Channel closed - process any remaining audio
        println!("[INFO] Recording stopped, processing remaining audio...");
        while buffer.len() >= chunk_size {
            let chunk: Vec<f32> = buffer.drain(..chunk_size).collect();
            println!(
                "[PROCESSING] Transcribing catch-up chunk ({:.2}s)...",
                chunk.len() as f32 / sample_rate as f32
            );

            match whisper
                .lock()
                .unwrap()
                .transcribe_chunk(&chunk, sample_rate)
            {
                Ok(transcript) => {
                    if !transcript.is_empty() {
                        println!("[TRANSCRIPT] \"{}\"", transcript);
                    }
                }
                Err(e) => eprintln!("[ERROR] Error: {}", e),
            }
        }

        // Process final partial chunk if exists
        if !buffer.is_empty() {
            let chunk_duration = buffer.len() as f32 / sample_rate as f32;

            // Skip very short chunks (< 1 second) to avoid hallucinations
            if chunk_duration < 1.0 {
                println!(
                    "[SKIP] Final chunk too short ({:.2}s) - likely silence or noise",
                    chunk_duration
                );
            } else {
                println!(
                    "[PROCESSING] Transcribing final chunk ({:.2}s)...",
                    chunk_duration
                );

                match whisper
                    .lock()
                    .unwrap()
                    .transcribe_chunk(&buffer, sample_rate)
                {
                    Ok(transcript) => {
                        if !transcript.is_empty() {
                            println!("[TRANSCRIPT] \"{}\"", transcript);
                        }
                    }
                    Err(e) => eprintln!("[ERROR] Error: {}", e),
                }
            }
        }

        println!("[INFO] Whisper thread finished");
    });

    let channels = config.channels as usize;

    // Build the audio input stream (like setting up a security camera)
    let stream = device
        .build_input_stream(
            &config, // [1] Use these microphone settings
            // [2] DATA CALLBACK: This function runs every time new audio arrives
            // Think: "When the camera detects motion, do THIS"
            move |data: &[f32], _: &_| {
                // data = borrowed slice of audio samples (numbers from -1.0 to 1.0)
                // _ = ignore extra info we don't need

                // 1. Send raw data (potentially stereo) to file writer
                // The WAV writer knows the channel count from spec, so this is fine
                file_tx_clone.send(data.to_vec()).ok();

                // 2. Mix to MONO for Whisper (crucial!)
                // If we send stereo (L,R,L,R) to Whisper, it thinks it's (T1,T2,T3,T4)
                // which sounds like 2x speed chipmunks -> severe hallucinations
                let mono_data: Vec<f32> = if channels > 1 {
                    data.chunks(channels)
                        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };

                whisper_tx_clone.send(mono_data).ok();
            },
            // [3] ERROR CALLBACK: This runs if the microphone has a problem
            move |err| {
                eprintln!("Audio input error: {}", err);
            },
            None, // [4] No special options needed
        )
        .map_err(|e| e.to_string())?; // Convert any setup errors to strings

    stream.play().map_err(|e| e.to_string())?;

    *state.recording_handle.lock().unwrap() = Some(RecordingHandle {
        stream: SendStream(stream),
        file_tx,
        whisper_tx,
    });

    Ok(format!("Recording started: {}", path.display()))
}

#[tauri::command]
fn stop_recording(state: State<AudioState>) -> Result<String, String> {
    let mut handle = state.recording_handle.lock().unwrap();
    if let Some(recording) = handle.take() {
        drop(recording.stream); // Stop capturing audio
        drop(recording.file_tx); // Close file channel → Writer thread finishes
        drop(recording.whisper_tx); // Close whisper channel → Whisper thread catches up

        // Give the file writer thread a moment to finish writing and closing the file
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Now run the "Professional Grade" transcription on the full file
        let path_opt = state.last_recording_path.lock().unwrap().clone();
        if let Some(path) = path_opt {
            println!(
                "[PROCESSING] Running final high-quality transcription with VAD on: {}",
                path
            );

            // 1. Lock Whisper to load audio (Step 1-3)
            let mut whisper = state.whisper.lock().unwrap();
            let audio_data = whisper.load_audio(&path)?;

            // 2. VAD: Find speech segments with 500ms padding
            // We release whisper lock temporarily if needed, but here we hold it, which is fine as recording stopped
            let mut vad = state.vad.lock().unwrap();
            let timestamps = vad.get_speech_timestamps(&audio_data, 500)?; // 500ms padding
            drop(vad); // Done with VAD

            if timestamps.is_empty() {
                println!("[FINAL_TRANSCRIPT] [silence]");
                return Ok("[silence]".to_string());
            }

            // 3. Construct Clean Audio (remove "dead air")
            let mut clean_audio = Vec::with_capacity(audio_data.len());
            for (start, end) in timestamps {
                let start_idx = (start * 16000.0) as usize;
                let end_idx = (end * 16000.0) as usize;

                // Bounds check
                let start_idx = start_idx.min(audio_data.len());
                let end_idx = end_idx.min(audio_data.len());

                if start_idx < end_idx {
                    clean_audio.extend_from_slice(&audio_data[start_idx..end_idx]);
                }
            }

            println!(
                "[VAD] Filtered audio from {:.2}s to {:.2}s ({:.0}% reduction)",
                audio_data.len() as f32 / 16000.0,
                clean_audio.len() as f32 / 16000.0,
                (1.0 - clean_audio.len() as f32 / audio_data.len().max(1) as f32) * 100.0
            );

            // 4. Transcribe filtered audio
            match whisper.transcribe_audio_data(&clean_audio) {
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

/// Global hotkey listener for push-to-talk recording
/// Listens for Ctrl+Win key combination
fn start_hotkey_listener(app_handle: AppHandle) {
    use rdev::{listen, Event, EventType, Key};
    use std::sync::atomic::{AtomicBool, Ordering};

    // Track which keys are currently held
    let ctrl_held = Arc::new(AtomicBool::new(false));
    let meta_held = Arc::new(AtomicBool::new(false)); // Meta = Windows key
    let recording_active = Arc::new(AtomicBool::new(false));

    let ctrl_held_clone = ctrl_held.clone();
    let meta_held_clone = meta_held.clone();
    let recording_active_clone = recording_active.clone();
    let app_handle_clone = app_handle.clone();

    // This callback is called for every keyboard event
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

                // Check if both Ctrl and Win are held
                if ctrl_held_clone.load(Ordering::SeqCst)
                    && meta_held_clone.load(Ordering::SeqCst)
                    && !recording_active_clone.load(Ordering::SeqCst)
                {
                    recording_active_clone.store(true, Ordering::SeqCst);
                    println!("[HOTKEY] Ctrl+Win pressed - Starting recording");

                    // Emit event to frontend to start recording
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

                // If either key is released and we were recording, stop
                if recording_active_clone.load(Ordering::SeqCst)
                    && (!ctrl_held_clone.load(Ordering::SeqCst)
                        || !meta_held_clone.load(Ordering::SeqCst))
                {
                    recording_active_clone.store(false, Ordering::SeqCst);
                    println!("[HOTKEY] Ctrl+Win released - Stopping recording");

                    // Emit event to frontend to stop recording
                    let _ = app_handle_clone.emit("hotkey-stop-recording", ());
                }
            }
            _ => {}
        }
    };

    // Start listening (this blocks, which is why we run it in a separate thread)
    if let Err(error) = listen(callback) {
        eprintln!("[ERROR] Hotkey listener error: {:?}", error);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize Whisper manager
    let mut whisper = WhisperManager::new();
    println!("[INFO] Initializing Whisper transcription engine...");
    match whisper.initialize(None) {
        // Use default model
        Ok(backend_msg) => {
            println!("[SUCCESS] {}", backend_msg);
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize Whisper: {}", e);
            eprintln!("   Transcription will be disabled.");
        }
    }

    // Initialize VAD (Voice Activity Detection)
    println!("[INFO] Initializing Voice Activity Detection...");
    let vad = VADManager::new().unwrap_or_else(|e| {
        eprintln!("[ERROR] Failed to initialize VAD: {}", e);
        eprintln!("   VAD features will be disabled.");
        panic!("VAD initialization failed");
    });
    println!("[SUCCESS] VAD initialized successfully");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AudioState {
            recording_handle: Mutex::new(None),
            whisper: Arc::new(Mutex::new(whisper)),
            vad: Arc::new(Mutex::new(vad)),
            last_recording_path: Mutex::new(None),
            current_app_state: Mutex::new(AppState::Ready),
        })
        .setup(|app| {
            use tauri::menu::{Menu, MenuItem};

            // Create tray context menu
            let show_item = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            // Create the system tray icon with embedded green icon
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
                    "quit" => {
                        println!("[INFO] Quitting application from tray");
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    use tauri::tray::TrayIconEvent;
                    match event {
                        TrayIconEvent::Click { .. } => {
                            // Show the main window when tray icon is clicked
                            if let Some(window) = tray.app_handle().get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            println!("[INFO] System tray icon created");

            // Start the global hotkey listener in a background thread
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                start_hotkey_listener(app_handle);
            });

            println!("[INFO] Global hotkey listener started (Ctrl+Win to record)");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Intercept close event - hide to tray instead of quitting
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide the window instead of closing
                let _ = window.hide();
                // Prevent the default close behavior
                api.prevent_close();
                println!("[INFO] Window minimized to tray");
            }
        })
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
            set_tray_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
