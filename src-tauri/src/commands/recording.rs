use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::unbounded;
use tauri::{AppHandle, Emitter, State};

use crate::audio::{RecordingHandle, SendStream};
use crate::denoise::Denoiser;
use crate::state::AudioState;
use crate::types::{ASREngine, TranscriptionChunk};
use crate::utils::{clean_transcript, get_recordings_dir};

/// COMMAND: START RECORDING
/// This initializes the microphone, files, and processing threads.
#[tauri::command]
pub fn start_recording(
    app_handle: AppHandle,
    state: State<AudioState>,
    denoise: Option<bool>,
) -> Result<String, String> {
    let denoise_enabled = denoise.unwrap_or(false);
    // 1. Setup Microphone
    let host = cpal::default_host();
    let preferred = state.selected_input_device.lock().unwrap().clone();
    let device = if let Some(ref name) = preferred {
        host.input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().ok().as_deref() == Some(name))
            .ok_or_else(|| format!("Input device '{}' not found", name))?
    } else {
        host.default_input_device().ok_or("No input device")?
    };
    println!(
        "[INFO] Using input device: {}",
        device.name().unwrap_or_default()
    );
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

    // Create a fresh denoiser for this session (RNNoise GRU state must not leak across sessions)
    if denoise_enabled {
        *state.denoiser.lock().unwrap() = Some(Denoiser::new());
        println!("[INFO] RNNoise denoiser enabled for this session");
    } else {
        *state.denoiser.lock().unwrap() = None;
    }

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

    // Pre-fill the transcriber channel with ~0.5s of silence so the ASR model
    // has a clean lead-in and doesn't clip the first spoken syllable.
    let lead_in_samples = (sample_rate as f32 * 0.5) as usize;
    whisper_tx.send(vec![0.0f32; lead_in_samples]).ok();
    println!(
        "[INFO] 🔇 Injected {} lead-in silence samples (~0.5s) to prevent head clipping",
        lead_in_samples
    );

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
    let denoiser_arc = state.denoiser.clone();

    // 10. Start the Microphone Stream
    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _: &_| {
                // File writer always gets raw (unprocessed) audio
                file_tx_clone.send(data.to_vec()).ok();

                let mono_data: Vec<f32> = if channels > 1 {
                    data.chunks(channels)
                        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };

                // Denoise on the transcriber path only (file writer keeps original)
                let transcriber_data = if let Ok(mut guard) = denoiser_arc.try_lock() {
                    if let Some(ref mut denoiser) = *guard {
                        denoiser.process(&mono_data)
                    } else {
                        mono_data
                    }
                } else {
                    mono_data
                };

                whisper_tx_clone.send(transcriber_data).ok();
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
        sample_rate,
    });

    Ok(format!("Recording started: {}", path.display()))
}

/// COMMAND: Insert text into the focused application.
/// macOS:         AXUIElement (kAXSelectedTextAttribute) — inserts at cursor, no clipboard touch
///                → fallback: clipboard + Cmd+V
/// Windows/Linux: clipboard save → set text → Ctrl+V → restore clipboard
#[tauri::command]
pub fn type_text(text: String) {
    if text.trim().is_empty() || text.trim() == "[silence]" {
        return;
    }
    let text_to_type = text.trim().to_string();
    std::thread::spawn(move || {
        insert_text(&text_to_type);
    });
}

fn insert_text(text: &str) {
    #[cfg(target_os = "macos")]
    {
        if ax_insert(text) {
            println!("[INSERT] AXUIElement succeeded");
            return;
        }
        eprintln!("[INSERT] AXUIElement failed, falling back to clipboard+Cmd+V");
    }
    clipboard_paste(text);
}

/// Clipboard + simulated paste keystroke (Cmd+V on macOS, Ctrl+V elsewhere).
/// Saves and restores the previous clipboard content.
fn clipboard_paste(text: &str) {
    use arboard::Clipboard;
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};

    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[INSERT] Clipboard init failed: {}", e);
            return;
        }
    };

    let previous = clipboard.get_text().ok();

    if let Err(e) = clipboard.set_text(text) {
        eprintln!("[INSERT] Failed to set clipboard: {}", e);
        return;
    }

    // arboard's set_text is synchronous; a short yield lets the OS finalise the write
    std::thread::sleep(std::time::Duration::from_millis(10));

    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[INSERT] Enigo init failed: {:?}", e);
            return;
        }
    };

    #[cfg(target_os = "macos")]
    {
        let _ = enigo.key(Key::Meta, Direction::Press);
        let _ = enigo.key(Key::Unicode('v'), Direction::Click);
        let _ = enigo.key(Key::Meta, Direction::Release);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = enigo.key(Key::Control, Direction::Press);
        let _ = enigo.key(Key::Unicode('v'), Direction::Click);
        let _ = enigo.key(Key::Control, Direction::Release);
    }

    // Wait for the target app to read the clipboard before we restore it
    std::thread::sleep(std::time::Duration::from_millis(150));
    if let Some(prev) = previous {
        let _ = clipboard.set_text(prev);
    }
}

/// macOS only: insert text at the cursor via the Accessibility API.
/// Equivalent to kAXSelectedTextAttribute — replaces the current selection
/// or inserts at the caret if nothing is selected. No clipboard involved.
#[cfg(target_os = "macos")]
fn ax_insert(text: &str) -> bool {
    use accessibility_sys::{
        kAXErrorSuccess, AXUIElementCopyAttributeValue, AXUIElementCreateSystemWide,
        AXUIElementSetAttributeValue,
    };
    use core_foundation::{
        base::{CFRelease, CFTypeRef, TCFType},
        string::CFString,
    };

    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return false;
        }

        let cf_focused_attr = CFString::new("AXFocusedUIElement");
        let mut focused: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            system,
            cf_focused_attr.as_CFTypeRef() as *const _,
            &mut focused,
        );

        // Release the system-wide element — we no longer need it
        CFRelease(system as CFTypeRef);

        if err != kAXErrorSuccess || focused.is_null() {
            return false;
        }

        let cf_text = CFString::new(text);
        let cf_selected_attr = CFString::new("AXSelectedText");
        let err = AXUIElementSetAttributeValue(
            focused as *mut std::ffi::c_void as accessibility_sys::AXUIElementRef,
            cf_selected_attr.as_CFTypeRef() as *const _,
            cf_text.as_CFTypeRef(),
        );

        CFRelease(focused);

        err == kAXErrorSuccess
    }
}

/// COMMAND: STOP RECORDING
#[tauri::command]
pub fn stop_recording(state: State<AudioState>) -> Result<String, String> {
    let mut handle = state.recording_handle.lock().unwrap();
    if let Some(recording) = handle.take() {
        // Stop the microphone first so no new audio arrives
        drop(recording.stream);

        // Drop the file channel immediately so the WAV writer finalizes
        // with clean, unmodified audio (no artificial silence padding).
        drop(recording.file_tx);

        // Inject ~1 second of silence into the TRANSCRIBER channel only,
        // so it can flush any buffered audio without the speaker's last
        // words being clipped. The saved WAV stays clean.
        let silence_samples = recording.sample_rate as usize; // 1 second
        let silence = vec![0.0f32; silence_samples];
        println!(
            "[INFO] 🔇 Injecting {} silence samples (~1s) into transcriber to prevent tail clipping",
            silence_samples
        );
        recording.whisper_tx.send(silence).ok();

        // Now release denoiser state (GRU context must not leak across sessions)
        println!("[DENOISE] 🧹 Releasing denoiser state (end of session)");
        *state.denoiser.lock().unwrap() = None;

        // Drop transcriber channel so the worker threads see EOF and finish
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
