use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::unbounded;
use tauri::{AppHandle, Emitter, State};

use crate::audio::{RecordingHandle, SendStream};
use crate::state::AudioState;
use crate::types::{ASREngine, TranscriptionChunk};
use crate::utils::{clean_transcript, get_recordings_dir};
use enigo::{Enigo, Keyboard, Settings};

/// COMMAND: START RECORDING
/// This initializes the microphone, files, and processing threads.
#[tauri::command]
pub fn start_recording(app_handle: AppHandle, state: State<AudioState>) -> Result<String, String> {
    // 1. Setup Microphone
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No input device")?;
    let config: cpal::StreamConfig = device
        .default_input_config()
        .map_err(|e| e.to_string())?
        .into();

    // 2. Prepare Output File
    let recordings_dir = get_recordings_dir()?;
    let filename = format!("recording_{}.wav", chrono::Utc::now().timestamp());
    let path = recordings_dir.join(&filename);

    println!("[INFO] Saving recording to: {}", path.display());

    // 3. Reset AI Context (Start fresh for new recording)
    let active_engine = *state.active_engine.lock().unwrap();
    if active_engine == ASREngine::Whisper {
        state.whisper.lock().unwrap().clear_context();
    } else {
        state.parakeet.lock().unwrap().clear_context();
    }

    *state.last_recording_path.lock().unwrap() = Some(path.to_string_lossy().into_owned());
    state.session_transcript.lock().unwrap().clear();

    // 4. Create proper WAV header settings
    let spec = hound::WavSpec {
        channels: config.channels,
        sample_rate: config.sample_rate.0,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;

    // 5. Create COMMUNICATION PIPES (Channels)
    let (file_tx, file_rx) = unbounded::<Vec<f32>>();
    let (whisper_tx, whisper_rx) = unbounded::<Vec<f32>>();

    let file_tx_clone = file_tx.clone();
    let whisper_tx_clone = whisper_tx.clone();

    let sample_rate = config.sample_rate.0;

    // 6. SPAWN THREAD 1: THE FILE SAVER
    let writer_thread = std::thread::spawn(move || {
        let mut writer = writer;
        while let Ok(samples) = file_rx.recv() {
            for sample in samples {
                writer.write_sample(sample).ok();
            }
        }
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
    let app_clone = app_handle.clone();
    let transcriber_thread = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let chunk_size = (sample_rate * 6) as usize;
        let max_buffer_size = chunk_size * 2;
        println!(
            "[INFO] Runtime Transcriber thread started (Engine: {:?})",
            active_engine
        );

        while let Ok(samples) = whisper_rx.recv() {
            if active_engine == ASREngine::Whisper {
                buffer.extend(samples);

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
                            6.0
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
                buffer.extend(samples);

                let parakeet_chunk_size = (sample_rate as f32 * 1.12) as usize;
                let max_buffer_size = parakeet_chunk_size * 2;

                while buffer.len() >= parakeet_chunk_size {
                    if buffer.len() > max_buffer_size {
                        buffer.drain(..parakeet_chunk_size);
                    }

                    let chunk: Vec<f32> = buffer.drain(..parakeet_chunk_size).collect();
                    let start_time = std::time::Instant::now();

                    match parakeet_manager
                        .lock()
                        .unwrap()
                        .transcribe_chunk(&chunk, sample_rate)
                    {
                        Ok(transcript) => {
                            if !transcript.is_empty() {
                                let elapsed = start_time.elapsed().as_millis() as u32;
                                println!(
                                    "[TRANSCRIPT] 🦜 \"{}\" (took {}ms)",
                                    transcript.trim(),
                                    elapsed
                                );
                                let _ = app_clone.emit(
                                    "transcription-chunk",
                                    TranscriptionChunk {
                                        text: transcript.clone(),
                                        processing_time_ms: elapsed,
                                        method: "Parakeet".to_string(),
                                    },
                                );

                                let mut session = session_transcript.lock().unwrap();
                                session.push_str(&transcript);
                            }
                        }
                        Err(e) => eprintln!("[ERROR] Parakeet error: {}", e),
                    }
                }
            }
        }

        println!("[INFO] Recording stopped, processing remaining audio...");
        while buffer.len() >= chunk_size {
            let chunk: Vec<f32> = buffer.drain(..chunk_size).collect();
            if active_engine == ASREngine::Whisper {
                whisper
                    .lock()
                    .unwrap()
                    .transcribe_chunk(&chunk, sample_rate)
                    .ok();
            } else {
                let mut p_manager = parakeet_manager.lock().unwrap();
                if let Ok(transcript) = p_manager.transcribe_chunk(&chunk, sample_rate) {
                    if !transcript.is_empty() {
                        let mut session = session_transcript.lock().unwrap();
                        session.push_str(&transcript);
                        println!("[TRANSCRIPT] 🦜 (Final) \"{}\"", transcript);
                    }
                }
            }
        }

        if !buffer.is_empty() {
            let chunk_duration = buffer.len() as f32 / sample_rate as f32;
            if chunk_duration > 0.1 {
                if active_engine == ASREngine::Whisper {
                    whisper
                        .lock()
                        .unwrap()
                        .transcribe_chunk(&buffer, sample_rate)
                        .ok();
                } else {
                    let mut p_manager = parakeet_manager.lock().unwrap();
                    if let Ok(transcript) = p_manager.transcribe_chunk(&buffer, sample_rate) {
                        if !transcript.is_empty() {
                            let mut session = session_transcript.lock().unwrap();
                            session.push_str(&transcript);
                            println!("[TRANSCRIPT] 🦜 (Final Partial) \"{}\"", transcript);
                        }
                    }
                }
            }
        }

        println!("[INFO] Transcriber thread finished");
    });

    let channels = config.channels as usize;

    // 10. Start the Microphone Stream
    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _: &_| {
                file_tx_clone.send(data.to_vec()).ok();

                let mono_data: Vec<f32> = if channels > 1 {
                    data.chunks(channels)
                        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };

                whisper_tx_clone.send(mono_data).ok();
            },
            move |err| {
                eprintln!("Audio input error: {}", err);
            },
            None,
        )
        .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;

    *state.recording_handle.lock().unwrap() = Some(RecordingHandle {
        stream: SendStream(stream),
        file_tx,
        whisper_tx,
        writer_thread,
        transcriber_thread,
    });

    Ok(format!("Recording started: {}", path.display()))
}

/// COMMAND: Type text with Enigo (called by frontend once after spell/grammar processing).
#[tauri::command]
pub fn type_text(text: String) {
    if text.trim().is_empty() || text.trim() == "[silence]" {
        return;
    }
    println!("[ENIGO] Typing out text: \"{}\"", text.trim());
    let text_to_type = text.trim().to_string();
    std::thread::spawn(move || {
        match Enigo::new(&Settings::default()) {
            Ok(mut enigo) => {
                if let Err(e) = enigo.text(&text_to_type) {
                    eprintln!("[ERROR] Enigo failed to type text: {:?}", e);
                }
            }
            Err(e) => eprintln!("[ERROR] Failed to initialize Enigo: {:?}", e),
        }
    });
}

/// COMMAND: STOP RECORDING
#[tauri::command]
pub fn stop_recording(state: State<AudioState>) -> Result<String, String> {
    let mut handle = state.recording_handle.lock().unwrap();
    if let Some(recording) = handle.take() {
        drop(recording.stream);
        drop(recording.file_tx);
        drop(recording.whisper_tx);

        // Join the threads to ensure they finish processing before we proceed.
        // This is CRITICAL for Parakeet which relies on the transcript built by the thread.
        println!("[INFO] Waiting for worker threads to finish...");
        if let Err(e) = recording.writer_thread.join() {
            eprintln!("[ERROR] Writer thread panicked: {:?}", e);
        }
        if let Err(e) = recording.transcriber_thread.join() {
            eprintln!("[ERROR] Transcriber thread panicked: {:?}", e);
        }
        println!("[INFO] Worker threads finished.");

        let active_engine = *state.active_engine.lock().unwrap();

        if active_engine == ASREngine::Parakeet {
            println!("[PROCESSING] Skipping final pass (Parakeet streaming is sufficient)");
            let transcript = state.session_transcript.lock().unwrap().clone();
            let final_text = if transcript.is_empty() {
                "Recording saved.".to_string()
            } else {
                clean_transcript(&transcript)
            };
            println!("[FINAL_TRANSCRIPT] (Raw)\n{}", final_text);
            return Ok(final_text);
        }

        let path_opt = state.last_recording_path.lock().unwrap().clone();
        if let Some(path) = path_opt {
            println!(
                "[PROCESSING] Running final high-quality transcription with VAD on: {}",
                path
            );

            let whisper = state.whisper.lock().unwrap();
            let audio_data = whisper.load_audio(&path)?;

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

            // Release locks before LLM processing to avoid deadlock
            drop(whisper);
            drop(vad);

            let result = {
                let mut whisper = state.whisper.lock().unwrap();
                whisper.transcribe_audio_data(&clean)
            };

            match result {
                Ok(raw_text) => {
                    println!("[FINAL_TRANSCRIPT] (Raw)\n{}", raw_text);
                    let final_text = clean_transcript(&raw_text);
                    Ok(final_text)
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
