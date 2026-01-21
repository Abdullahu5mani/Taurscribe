use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{unbounded, Sender};
use std::sync::{Arc, Mutex};
use tauri::State;

mod whisper;
use whisper::WhisperManager;

// Wrapper to make cpal::Stream Send/Sync.
// Safety: We only use this to keep the stream alive and drop it.
#[allow(dead_code)]
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

struct AudioState {
    recording_handle: Mutex<Option<RecordingHandle>>,
    whisper: Arc<Mutex<WhisperManager>>,
    last_recording_path: Mutex<Option<String>>,
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

#[tauri::command]
fn benchmark_test(state: State<AudioState>, file_path: String) -> Result<String, String> {
    use std::time::Instant;

    println!("[BENCHMARK] Starting test on: {}", file_path);

    // Resolve the file path (same logic as model loading)
    let absolute_path = std::fs::canonicalize(&file_path)
        .or_else(|_| std::fs::canonicalize(format!("../{}", file_path)))
        .or_else(|_| std::fs::canonicalize(format!("../../{}", file_path)))
        .map_err(|e| format!("Could not find file at '{}'. Error: {}", file_path, e))?;

    println!("[BENCHMARK] Reading file: {}", absolute_path.display());

    // Read WAV file to get audio duration
    let reader = hound::WavReader::open(&absolute_path)
        .map_err(|e| format!("Failed to open WAV file: {}", e))?;
    let spec = reader.spec();
    let sample_count = reader.len();
    // Fix: For stereo, sample_count includes both channels, so divide by channels
    let audio_duration_secs = sample_count as f32 / spec.sample_rate as f32 / spec.channels as f32;

    println!("[BENCHMARK] Audio duration: {:.2}s", audio_duration_secs);

    let start = Instant::now();
    let transcript = state
        .whisper
        .lock()
        .unwrap()
        .transcribe_file(absolute_path.to_str().unwrap())?;
    let duration = start.elapsed();

    let duration_ms = duration.as_millis();
    let duration_secs = duration.as_secs_f32();
    let realtime_factor = audio_duration_secs / duration_secs;

    println!(
        "[BENCHMARK] Completed in {:.2}s ({}ms) | Speed: {:.1}× realtime",
        duration_secs, duration_ms, realtime_factor
    );
    println!("[BENCHMARK] Transcript length: {} chars", transcript.len());

    Ok(format!(
        "🚀 Benchmark Complete!\n\n\
        ⏱️  Processing Time: {:.2}s ({}ms)\n\
        🎵 Audio Duration: {:.2}s\n\
        ⚡ Speed: {:.1}× realtime\n\
        📝 Transcript: {} characters\n\n\
        Transcript:\n{}",
        duration_secs,
        duration_ms,
        audio_duration_secs,
        realtime_factor,
        transcript.len(),
        transcript
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

    // Clone whisper manager for the thread (Arc allows shared ownership)
    let whisper = state.whisper.clone();

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

                // Extract one 3-second chunk
                let chunk: Vec<f32> = buffer.drain(..chunk_size).collect();

                println!(
                    "[PROCESSING] Transcribing {:.2}s chunk ({} samples)...",
                    chunk.len() as f32 / sample_rate as f32,
                    chunk.len()
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
                "[PROCESSING] Running final high-quality transcription on: {}",
                path
            );
            match state.whisper.lock().unwrap().transcribe_file(&path) {
                Ok(text) => {
                    println!("[FINAL_TRANSCRIPT]\n{}", text);
                    Ok(text) // Return the accurate text to the frontend!
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize Whisper manager
    let mut whisper = WhisperManager::new();
    println!("[INFO] Initializing Whisper transcription engine...");
    match whisper.initialize() {
        Ok(backend_msg) => {
            println!("[SUCCESS] {}", backend_msg);
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize Whisper: {}", e);
            eprintln!("   Transcription will be disabled.");
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AudioState {
            recording_handle: Mutex::new(None),
            whisper: Arc::new(Mutex::new(whisper)),
            last_recording_path: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            start_recording,
            stop_recording,
            get_backend_info,
            benchmark_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
