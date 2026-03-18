use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::unbounded;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, State};

use crate::audio::{RecordingHandle, SendStream};
use crate::context::get_active_context;
use crate::denoise::Denoiser;
use crate::state::AudioState;
use crate::types::{ASREngine, TranscriptionChunk};
use crate::utils::{clean_transcript, get_recordings_dir, normalize_audio};

/// COMMAND: START RECORDING
/// This initializes the microphone, files, and processing threads.
///
/// macOS fix: Made async with spawn_blocking because Tauri 2 dispatches
/// synchronous `#[tauri::command]` handlers on the main (AppKit) thread.
/// cpal device enumeration and stream creation block that thread, freezing
/// the entire window. Async commands run on the tokio runtime instead.
#[tauri::command]
pub async fn start_recording(
    app_handle: AppHandle,
    state: State<'_, AudioState>,
    denoise: Option<bool>,
) -> Result<String, String> {
    // Guard: reject if already recording (e.g. spam hotkey)
    {
        let handle = state.recording_handle.lock().unwrap();
        if handle.is_some() {
            return Err("Already recording".to_string());
        }
    }

    // macOS fix: Clone all Arc state so it can be moved into spawn_blocking.
    // Fields were changed from Mutex<T> to Arc<Mutex<T>> in state.rs to
    // allow cloning into the background closure.
    let recording_handle_arc = state.recording_handle.clone();
    let selected_input_device_arc = state.selected_input_device.clone();
    let active_engine_arc = state.active_engine.clone();
    let whisper_arc = state.whisper.clone();
    let parakeet_arc = state.parakeet.clone();
    let vad_arc = state.vad.clone();
    let last_recording_path_arc = state.last_recording_path.clone();
    let session_transcript_arc = state.session_transcript.clone();
    let denoiser_state_arc = state.denoiser.clone();
    let granite_speech_arc = state.granite_speech.clone();

    tauri::async_runtime::spawn_blocking(move || {
        start_recording_blocking(
            app_handle,
            denoise,
            recording_handle_arc,
            selected_input_device_arc,
            active_engine_arc,
            whisper_arc,
            parakeet_arc,
            vad_arc,
            last_recording_path_arc,
            session_transcript_arc,
            denoiser_state_arc,
            granite_speech_arc,
        )
    })
    .await
    .map_err(|e| format!("start_recording task failed: {}", e))?
}

/// The blocking core of start_recording, run inside spawn_blocking.
#[allow(clippy::too_many_arguments)]
fn start_recording_blocking(
    app_handle: AppHandle,
    denoise: Option<bool>,
    recording_handle_arc: Arc<std::sync::Mutex<Option<RecordingHandle>>>,
    selected_input_device_arc: Arc<std::sync::Mutex<Option<String>>>,
    active_engine_arc: Arc<std::sync::Mutex<ASREngine>>,
    whisper_arc: Arc<std::sync::Mutex<crate::whisper::WhisperManager>>,
    parakeet_arc: Arc<std::sync::Mutex<crate::parakeet::ParakeetManager>>,
    vad_arc: Arc<std::sync::Mutex<crate::vad::VADManager>>,
    last_recording_path_arc: Arc<std::sync::Mutex<Option<String>>>,
    session_transcript_arc: Arc<std::sync::Mutex<String>>,
    denoiser_state_arc: Arc<std::sync::Mutex<Option<Denoiser>>>,
    granite_speech_arc: Arc<std::sync::Mutex<crate::granite_speech::GraniteSpeechManager>>,
) -> Result<String, String> {
    let denoise_enabled = denoise.unwrap_or(false);
    // 1. Setup Microphone
    let host = cpal::default_host();
    let preferred = selected_input_device_arc.lock().unwrap().clone();
    
    let mut device_opt = None;
    let mut fallback_triggered = false;
    
    if let Some(ref name) = preferred {
        device_opt = host.input_devices()
            .ok()
            .and_then(|mut iter| iter.find(|d| d.name().ok().as_deref() == Some(name.as_str())));
            
        if device_opt.is_none() {
            println!("[WARNING] Preferred input device '{}' not found, falling back to default", name);
            fallback_triggered = true;
        }
    }
    
    if device_opt.is_none() {
        device_opt = host.default_input_device();
    }
    
    let device = device_opt.ok_or("No input device found. Check that a microphone is connected.")?;
    let device_name = device.name().unwrap_or_else(|_| "Unknown Device".to_string());
    
    println!("[INFO] Using input device: {}", device_name);
    
    if fallback_triggered {
        let _ = app_handle.emit("audio-fallback", device_name);
    }

    let config: cpal::StreamConfig = device
        .default_input_config()
        .or_else(|e| {
            println!("[WARNING] default_input_config failed: {}, falling back to iterating supported configs", e);
            device.supported_input_configs()
                .map_err(|_err| cpal::DefaultStreamConfigError::DeviceNotAvailable)?
                .find(|c| c.sample_format() == cpal::SampleFormat::F32 || c.sample_format() == cpal::SampleFormat::I16)
                .map(|c| c.with_max_sample_rate())
                .ok_or(cpal::DefaultStreamConfigError::StreamTypeNotSupported)
        })
        .map_err(|e| {
            // macOS: permission denial often surfaces as a vague
            // CoreAudio error during config or stream creation.
            let msg = e.to_string();
            if msg.contains("permission") || msg.contains("denied") || msg.contains("not supported") {
                "Microphone permission denied. Grant access in System Settings → Privacy & Security → Microphone.".to_string()
            } else {
                format!("Failed to get audio config: {}", msg)
            }
        })?
        .into();

    // 2. Prepare Output File
    let recordings_dir = get_recordings_dir()?;
    let filename = format!("recording_{}.wav", chrono::Utc::now().timestamp());
    let path = recordings_dir.join(&filename);

    println!("[INFO] Saving recording to: {}", path.display());

    // 3. Reset AI Context (Start fresh for new recording)
    let active_engine = *active_engine_arc.lock().unwrap();
    match active_engine {
        ASREngine::Whisper => whisper_arc.lock().unwrap().clear_context(),
        ASREngine::Parakeet => parakeet_arc.lock().unwrap().clear_context(),
        ASREngine::GraniteSpeech => { /* Granite Speech is stateless per chunk */ }
    }
    // Reset Silero VAD LSTM state so prior session context doesn't bleed in
    vad_arc.lock().unwrap().reset_state();

    *last_recording_path_arc.lock().unwrap() = Some(path.to_string_lossy().into_owned());
    session_transcript_arc.lock().unwrap().clear();

    // Create a fresh denoiser for this session (RNNoise GRU state must not leak across sessions)
    if denoise_enabled {
        *denoiser_state_arc.lock().unwrap() = Some(Denoiser::new());
        println!("[INFO] RNNoise denoiser enabled for this session");
    } else {
        *denoiser_state_arc.lock().unwrap() = None;
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

    let level_stop = Arc::new(AtomicBool::new(false));
    let level_stop_clone1 = level_stop.clone();
    let level_stop_clone2 = level_stop.clone();
    let level_stop_clone3 = level_stop.clone();

    // 6. SPAWN THREAD 1: THE FILE SAVER
    let writer_thread = std::thread::spawn(move || {
        let mut writer = writer;
        loop {
            match file_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(samples) => {
                    for sample in samples {
                        writer.write_sample(sample).ok();
                    }
                    // macOS fix: CoreAudio may keep the audio callback alive
                    // briefly after Stream::drop() when called from a non-main
                    // thread, so the channel stays open and we never hit the
                    // Timeout branch. Check the stop signal here too.
                    if level_stop_clone1.load(Ordering::Relaxed) {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if level_stop_clone1.load(Ordering::Relaxed) {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }

        // Drain any remaining
        while let Ok(samples) = file_rx.try_recv() {
            for sample in samples {
                writer.write_sample(sample).ok();
            }
        }
        writer.finalize().ok();
        println!("WAV file saved.");
    });

    // Get shared references to our AI tools
    let whisper = whisper_arc.clone();
    let parakeet_manager = parakeet_arc.clone();
    let granite_speech = granite_speech_arc.clone();
    let vad = vad_arc.clone();
    let active_engine = *active_engine_arc.lock().unwrap();
    let session_transcript = session_transcript_arc.clone();

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

        while !level_stop_clone2.load(Ordering::Relaxed) {
            let samples = match whisper_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(s) => s,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };
            if active_engine == ASREngine::Whisper {
                buffer.extend(samples);

                while buffer.len() >= chunk_size {
                    if buffer.len() > max_buffer_size {
                        println!("[WARNING] Buffer full, dropping old audio to catch up");
                        buffer.drain(..chunk_size);
                    }
                    let mut chunk: Vec<f32> = buffer.drain(..chunk_size).collect();
                    normalize_audio(&mut chunk);
                    // VAD expects 16kHz — downsample from device rate before checking
                    let vad_chunk: Vec<f32> = if sample_rate != 16000 {
                        let ratio = sample_rate as f64 / 16000.0;
                        let out_len = (chunk.len() as f64 / ratio) as usize;
                        (0..out_len)
                            .map(|i| chunk[(i as f64 * ratio) as usize])
                            .collect()
                    } else {
                        chunk.clone()
                    };
                    let is_speech = vad.lock().unwrap().is_speech(&vad_chunk).unwrap_or(0.5);

                    if is_speech > 0.35 {
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
            } else if active_engine == ASREngine::GraniteSpeech {
                // Granite Speech: VAD-gated, chunk-based (same pattern as Whisper)
                buffer.extend(samples);

                while buffer.len() >= chunk_size {
                    if buffer.len() > max_buffer_size {
                        println!("[WARNING] Buffer full, dropping old audio to catch up");
                        buffer.drain(..chunk_size);
                    }
                    let mut chunk: Vec<f32> = buffer.drain(..chunk_size).collect();
                    normalize_audio(&mut chunk);
                    let vad_chunk: Vec<f32> = if sample_rate != 16000 {
                        let ratio = sample_rate as f64 / 16000.0;
                        let out_len = (chunk.len() as f64 / ratio) as usize;
                        (0..out_len)
                            .map(|i| chunk[(i as f64 * ratio) as usize])
                            .collect()
                    } else {
                        chunk.clone()
                    };
                    let is_speech = vad.lock().unwrap().is_speech(&vad_chunk).unwrap_or(0.5);

                    if is_speech > 0.35 {
                        println!(
                            "[PROCESSING] 🪨 Speech ({:.0}%) - Granite transcribing {:.2}s chunk...",
                            is_speech * 100.0,
                            6.0
                        );
                        let start_time = std::time::Instant::now();
                        match granite_speech
                            .lock()
                            .unwrap()
                            .transcribe_chunk(&chunk, sample_rate)
                        {
                            Ok(transcript) => {
                                if !transcript.trim().is_empty() {
                                    let elapsed = start_time.elapsed().as_millis() as u32;
                                    println!(
                                        "[TRANSCRIPT] 🪨 \"{}\" (took {}ms)",
                                        transcript, elapsed
                                    );
                                    let _ = app_clone.emit(
                                        "transcription-chunk",
                                        TranscriptionChunk {
                                            text: transcript.clone(),
                                            processing_time_ms: elapsed,
                                            method: "GraniteSpeech".to_string(),
                                        },
                                    );
                                    let mut session = session_transcript.lock().unwrap();
                                    session.push_str(&transcript);
                                }
                            }
                            Err(e) => eprintln!("[ERROR] Granite Speech error: {}", e),
                        }
                    } else {
                        println!(
                            "[VAD] 🔇 Silence ({:.0}%) - Skipping Granite chunk",
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

                    let mut chunk: Vec<f32> = buffer.drain(..parakeet_chunk_size).collect();
                    normalize_audio(&mut chunk);
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
        // Fast drain any remaining audio chunks from the queue
        while let Ok(samples) = whisper_rx.try_recv() {
            if active_engine == ASREngine::Whisper {
                buffer.extend(samples);
            } else {
                buffer.extend(samples);
            }
        }

        while buffer.len() >= chunk_size {
            let chunk: Vec<f32> = buffer.drain(..chunk_size).collect();
            if active_engine == ASREngine::Whisper {
                whisper
                    .lock()
                    .unwrap()
                    .transcribe_chunk(&chunk, sample_rate)
                    .ok();
            } else if active_engine == ASREngine::GraniteSpeech {
                let mut gs = granite_speech.lock().unwrap();
                match gs.transcribe_chunk(&chunk, sample_rate) {
                    Ok(transcript) if !transcript.is_empty() => {
                        let mut session = session_transcript.lock().unwrap();
                        session.push_str(&transcript);
                        println!("[TRANSCRIPT] 🪨 (Final) \"{}\"", transcript);
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("[ERROR] Granite Speech error (final): {}", e),
                }
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
                } else if active_engine == ASREngine::GraniteSpeech {
                    let mut gs = granite_speech.lock().unwrap();
                    match gs.transcribe_chunk(&buffer, sample_rate) {
                        Ok(transcript) if !transcript.is_empty() => {
                            let mut session = session_transcript.lock().unwrap();
                            session.push_str(&transcript);
                            println!("[TRANSCRIPT] 🪨 (Final Partial) \"{}\"", transcript);
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("[ERROR] Granite Speech error (final partial): {}", e),
                    }
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
    let denoiser_arc = denoiser_state_arc.clone();

    // Audio level metering: the cpal callback writes a float (as AtomicU32 bits)
    // and a dedicated thread reads it every 50ms to emit the Tauri event.
    // We do NOT call emit() from inside the cpal callback because on Windows
    // the WASAPI callback runs on a COM apartment thread where Tauri IPC fails.
    let audio_level = Arc::new(AtomicU32::new(0u32));
    let audio_level_writer = audio_level.clone();
    let level_counter = Arc::new(AtomicU32::new(0));
    let level_counter_clone = level_counter.clone();

    let app_for_level = app_handle.clone();

    let level_thread = std::thread::spawn(move || {
        while !level_stop_clone3.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let bits = audio_level.load(Ordering::Relaxed);
            let level = f32::from_bits(bits);
            let _ = app_for_level.emit("audio-level", level);
        }
    });

    let app_for_error = app_handle.clone();
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

                // Store audio level in atomic for the emitter thread to pick up.
                // Only compute every ~5 callbacks to avoid unnecessary work.
                let cnt = level_counter_clone.fetch_add(1, Ordering::Relaxed);
                if cnt % 5 == 0 && !data.is_empty() {
                    let rms = (data.iter().map(|&s| s * s).sum::<f32>() / data.len() as f32).sqrt();
                    let level = (rms / 0.015_f32).min(1.0_f32).sqrt();
                    audio_level_writer.store(level.to_bits(), Ordering::Relaxed);
                }

                whisper_tx_clone.send(transcriber_data).ok();
            },
            move |err| {
                eprintln!("[ERROR] Audio input stream error: {}", err);
                let _ = app_for_error.emit("audio-disconnected", err.to_string());
            },
            None,
        )
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("permission") || msg.contains("denied") {
                "Microphone permission denied. Grant access in System Settings → Privacy & Security → Microphone.".to_string()
            } else {
                format!("Failed to open audio stream: {}", msg)
            }
        })?;

    stream.play().map_err(|e| {
        let msg = e.to_string();
        if msg.contains("permission") || msg.contains("denied") {
            "Microphone permission denied. Grant access in System Settings → Privacy & Security → Microphone.".to_string()
        } else {
            format!("Failed to start audio stream: {}", msg)
        }
    })?;

    *recording_handle_arc.lock().unwrap() = Some(RecordingHandle {
        stream: SendStream(stream),
        file_tx,
        whisper_tx,
        writer_thread,
        transcriber_thread,
        level_stop,
        level_thread,
        sample_rate,
    });

    Ok(format!("Recording started: {}", path.display()))
}

/// COMMAND: Insert text into the focused application.
/// macOS:         AXUIElement (kAXSelectedTextAttribute) — inserts at cursor, no clipboard touch
///                → fallback: clipboard + Cmd+V
/// Windows/Linux: clipboard save → set text → Ctrl+V → restore clipboard
/// Returns Err with a short error code on failure so the frontend can show
/// a "couldn't paste" indicator without silently dropping the transcript.
#[tauri::command]
pub async fn type_text(text: String) -> Result<(), String> {
    if text.trim().is_empty() || text.trim() == "[silence]" {
        return Ok(());
    }
    let text_to_type = text.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || insert_text(&text_to_type))
        .await
        .map_err(|e| format!("thread_panic:{e:?}"))?
}

fn insert_text(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Bail early if the OS has locked keyboard injection (e.g. a password
        // field has focus). CGEventPost silently does nothing while this lock
        // is held — detecting it lets us surface a real error to the user.
        if is_secure_input_active() {
            eprintln!("[INSERT] Secure input is active — aborting keyboard injection");
            return Err("secure_input".to_string());
        }

        if should_prefer_clipboard_paste() {
            println!("[INSERT] Browser/web app detected — using clipboard+Cmd+V directly");
            return clipboard_paste(text);
        }

        // macOS fix: After the hotkey is released, the OS needs a moment to
        // settle focus back to the target app's text field. Without this
        // delay, AXFocusedUIElement often returns null or a stale element.
        std::thread::sleep(std::time::Duration::from_millis(50));

        for attempt in 0..3 {
            if ax_insert(text) {
                println!("[INSERT] AXUIElement succeeded (attempt {})", attempt + 1);
                return Ok(());
            }
            if attempt < 2 {
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        }
        eprintln!("[INSERT] AXUIElement failed after 3 attempts, falling back to clipboard+Cmd+V");
    }
    clipboard_paste(text)
}

/// Returns true when the frontmost application is a web browser or Electron app
/// whose web content fields don't expose AXSelectedText. In these apps,
/// ax_insert() always fails, wasting ~260ms on retries before falling back
/// to clipboard paste anyway. Skip straight to Cmd+V for speed and reliability.
#[cfg(target_os = "macos")]
fn should_prefer_clipboard_paste() -> bool {
    use std::ffi::{c_void, CStr};

    type MsgSendFn = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;

    extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut c_void;
    }

    // Obtain objc_msgSend via dlsym to avoid clashing extern declarations
    // with the different signature in misc.rs.
    unsafe {
        let msg_send: MsgSendFn = {
            extern "C" { fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void; }
            const RTLD_DEFAULT: *mut c_void = std::ptr::null_mut::<c_void>().wrapping_sub(2);
            let sym = dlsym(RTLD_DEFAULT, CStr::from_bytes_with_nul_unchecked(b"objc_msgSend\0").as_ptr());
            if sym.is_null() { return false; }
            std::mem::transmute(sym)
        };

        let ws_cls = objc_getClass(CStr::from_bytes_with_nul_unchecked(b"NSWorkspace\0").as_ptr());
        if ws_cls.is_null() { return false; }
        let shared_sel = sel_registerName(CStr::from_bytes_with_nul_unchecked(b"sharedWorkspace\0").as_ptr());
        let ws = msg_send(ws_cls, shared_sel);
        if ws.is_null() { return false; }

        let front_sel = sel_registerName(CStr::from_bytes_with_nul_unchecked(b"frontmostApplication\0").as_ptr());
        let app = msg_send(ws, front_sel);
        if app.is_null() { return false; }

        let bundle_sel = sel_registerName(CStr::from_bytes_with_nul_unchecked(b"bundleIdentifier\0").as_ptr());
        let bundle_id = msg_send(app, bundle_sel);
        if bundle_id.is_null() { return false; }

        let utf8_sel = sel_registerName(CStr::from_bytes_with_nul_unchecked(b"UTF8String\0").as_ptr());
        let cstr_ptr = msg_send(bundle_id, utf8_sel) as *const std::ffi::c_char;
        if cstr_ptr.is_null() { return false; }

        let bid = CStr::from_ptr(cstr_ptr).to_string_lossy();
        let bid_lower = bid.to_lowercase();
        println!("[INSERT] Frontmost app bundle ID: {}", bid);

        const BROWSER_BUNDLES: &[&str] = &[
            "com.google.chrome",
            "org.mozilla.firefox",
            "com.apple.safari",
            "company.thebrowser.browser",   // Arc
            "com.brave.browser",
            "com.operasoftware.opera",
            "com.vivaldi.vivaldi",
            "com.microsoft.edgemac",
            "org.chromium.chromium",
        ];

        BROWSER_BUNDLES.iter().any(|b| bid_lower.starts_with(b))
    }
}

/// Clipboard + simulated paste keystroke (Cmd+V on macOS, Ctrl+V elsewhere).
/// Saves and restores the previous clipboard content.
fn clipboard_paste(text: &str) -> Result<(), String> {
    use arboard::Clipboard;

    // Windows: classic cmd.exe console windows use a different paste path
    // (right-click context menu or Win+V). They do not process Ctrl+V from
    // synthetic SendInput events, so detect them before touching the clipboard.
    #[cfg(target_os = "windows")]
    if let Some(reason) = get_foreground_window_issue() {
        return Err(reason);
    }

    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[INSERT] Clipboard init failed: {}", e);
            return Err(format!("clipboard_init:{e}"));
        }
    };

    let previous = clipboard.get_text().ok();

    if let Err(e) = clipboard.set_text(text) {
        eprintln!("[INSERT] Failed to set clipboard: {}", e);
        return Err(format!("clipboard_set:{e}"));
    }

    // Give the pasteboard server (pbs) time to propagate the write to other
    // processes. 10 ms was too tight for heavy apps (Word, Excel, Outlook)
    // that validate the pasteboard change count before reading on Cmd+V.
    std::thread::sleep(std::time::Duration::from_millis(50));

    #[cfg(target_os = "macos")]
    {
        // macOS fix: Use CGEvent directly instead of enigo. Enigo internally
        // calls TSMGetInputSourceProperty (via HIToolbox) which asserts it
        // runs on the main dispatch queue. Since type_text spawns a
        // std::thread (background thread), that assertion fails with
        // EXC_BREAKPOINT (SIGTRAP), crashing the app. CGEvent's
        // CGEventPost works safely from any thread.
        simulate_cmd_v_cgevent();
    }

    #[cfg(not(target_os = "macos"))]
    {
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};
        let mut enigo = match Enigo::new(&Settings::default()) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[INSERT] Enigo init failed: {:?}", e);
                return Err(format!("enigo_init:{e:?}"));
            }
        };
        // Small gap between modifier down and V so the target app's message
        // pump sees them as distinct WM_KEYDOWN events. Zero-gap synthetic
        // sequences can be coalesced or dropped by apps like Word/LibreOffice.
        let _ = enigo.key(Key::Control, Direction::Press);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = enigo.key(Key::Unicode('v'), Direction::Click);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _ = enigo.key(Key::Control, Direction::Release);
    }

    // Wait for the target app to finish reading the clipboard before restoring.
    // 150 ms was too short for heavy apps (Word, LibreOffice) that process
    // paste asynchronously through their own undo/format pipeline.
    std::thread::sleep(std::time::Duration::from_millis(300));
    if let Some(prev) = previous {
        let _ = clipboard.set_text(prev);
    }
    Ok(())
}

/// macOS fix: Simulate Cmd+V using CGEvent instead of enigo.
/// Enigo's key simulation calls HIToolbox TSMGetInputSourceProperty which
/// requires the main dispatch queue and crashes from background threads.
/// CGEvent's CGEventPost has no such restriction and is thread-safe.
#[cfg(target_os = "macos")]
fn simulate_cmd_v_cgevent() {
    use core_graphics::event::{CGEvent, CGEventFlags, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // kVK_ANSI_V = 0x09
    const VK_V: CGKeyCode = 0x09;

    let source = match CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("[INSERT] CGEventSource creation failed");
            return;
        }
    };

    if let (Ok(key_down), Ok(key_up)) = (
        CGEvent::new_keyboard_event(source.clone(), VK_V, true),
        CGEvent::new_keyboard_event(source, VK_V, false),
    ) {
        key_down.set_flags(CGEventFlags::CGEventFlagCommand);
        key_up.set_flags(CGEventFlags::CGEventFlagCommand);
        // AnnotatedSession delivers the event after the window server has
        // assigned it a target process and annotated it with process/window
        // info. This is the level at which Carbon HIToolbox (used by Word,
        // Excel, Outlook) intercepts keyboard shortcuts — posting at HID
        // bypasses that layer and those apps silently ignore the event.
        key_down.post(core_graphics::event::CGEventTapLocation::AnnotatedSession);
        key_up.post(core_graphics::event::CGEventTapLocation::AnnotatedSession);
    } else {
        eprintln!("[INSERT] Failed to create CGEvent for Cmd+V");
    }
}

/// macOS only: Insert text at the cursor via the Accessibility API.
/// Uses kAXSelectedTextAttribute — replaces the current selection or
/// inserts at the caret if nothing is selected. Avoids clipboard entirely.
/// Requires Accessibility permission in System Settings → Privacy & Security.
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

/// macOS: Returns true when any process has activated Secure Input — an IOKit
/// flag set when a password field (or Terminal "Secure Keyboard Entry") has
/// focus. While active, CGEventPost keyboard injection is silently blocked
/// system-wide by the OS kernel; there is no way to paste into any app.
/// We check before attempting so we can return a real error code rather than
/// silently succeeding with no text inserted.
#[cfg(target_os = "macos")]
fn is_secure_input_active() -> bool {
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;
    use std::ffi::{c_void, CStr};

    type IOService = u32;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOServiceMatching(name: *const std::ffi::c_char) -> *mut c_void;
        // IOServiceGetMatchingService takes ownership of (and releases) `matching`.
        fn IOServiceGetMatchingService(masterPort: u32, matching: *mut c_void) -> IOService;
        fn IORegistryEntryCreateCFProperty(
            entry: IOService,
            key: CFTypeRef,
            allocator: *const c_void,
            options: u32,
        ) -> CFTypeRef;
        fn IOObjectRelease(object: IOService) -> i32;
    }

    extern "C" {
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> bool;
    }

    unsafe {
        let matching = IOServiceMatching(
            CStr::from_bytes_with_nul_unchecked(b"IOHIDSystem\0").as_ptr(),
        );
        if matching.is_null() {
            return false;
        }
        // kIOMasterPortDefault = 0; matching ref is consumed by this call.
        let service = IOServiceGetMatchingService(0, matching);
        if service == 0 {
            return false;
        }
        let key = CFString::new("HIDSecureEventInputIsActive");
        let prop =
            IORegistryEntryCreateCFProperty(service, key.as_CFTypeRef(), std::ptr::null(), 0);
        IOObjectRelease(service);
        if prop.is_null() {
            return false;
        }
        let result = CFGetTypeID(prop) == CFBooleanGetTypeID() && CFBooleanGetValue(prop);
        CFRelease(prop);
        result
    }
}

/// Windows: checks the foreground window's class name before attempting paste.
/// Classic cmd.exe uses "ConsoleWindowClass" and does not process Ctrl+V from
/// synthetic SendInput — it expects right-click → Paste or Win+V. Windows
/// Terminal ("CASCADIA_HOSTING_WINDOW_CLASS") does handle Ctrl+V correctly.
#[cfg(target_os = "windows")]
fn get_foreground_window_issue() -> Option<String> {
    unsafe {
        extern "system" {
            fn GetForegroundWindow() -> *mut std::ffi::c_void;
            fn GetClassNameW(
                hWnd: *mut std::ffi::c_void,
                lpClassName: *mut u16,
                nMaxCount: i32,
            ) -> i32;
        }

        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }

        let mut class_buf = [0u16; 256];
        let len = GetClassNameW(hwnd, class_buf.as_mut_ptr(), 256);
        if len <= 0 {
            return None;
        }

        let class_name = String::from_utf16_lossy(&class_buf[..len as usize]);
        if class_name == "ConsoleWindowClass" {
            return Some("console".to_string());
        }

        None
    }
}

/// macOS fix: Extracted the heavy blocking core of stop_recording into a
/// separate function so it can be dispatched via spawn_blocking. This keeps
/// the macOS AppKit main thread free during thread joins, VAD processing,
/// and Whisper inference which would otherwise freeze the window.
fn stop_recording_blocking(
    recording: crate::audio::RecordingHandle,
    active_engine: ASREngine,
    session_transcript: Arc<std::sync::Mutex<String>>,
    last_recording_path: Option<String>,
    whisper_arc: Arc<std::sync::Mutex<crate::whisper::WhisperManager>>,
    vad_arc: Arc<std::sync::Mutex<crate::vad::VADManager>>,
) -> Result<String, String> {
    use cpal::traits::StreamTrait;

    // Tail capture: keep mic open briefly to capture the last syllable
    // (e.g. "LLM design?" — "design?" often gets cut when releasing the hotkey)
    std::thread::sleep(std::time::Duration::from_millis(220));

    // macOS fix: Explicitly pause the audio unit before dropping the stream.
    // On macOS, CoreAudio may keep the render callback alive after
    // Stream::drop() if called from a non-main thread. pause() calls
    // AudioOutputUnitStop() synchronously, ensuring the callback stops
    // before the stream is dropped and preventing use-after-free.
    let _ = recording.stream.0.pause();
    drop(recording.stream);
    drop(recording.file_tx);
    drop(recording.whisper_tx);

    // Signal the audio-level emitter thread to exit and join it.
    recording.level_stop.store(true, Ordering::Relaxed);
    if let Err(e) = recording.level_thread.join() {
        eprintln!("[ERROR] Level thread panicked: {:?}", e);
    }

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

    if active_engine == ASREngine::Parakeet || active_engine == ASREngine::GraniteSpeech {
        let engine_name = if active_engine == ASREngine::Parakeet { "Parakeet" } else { "Granite Speech" };
        println!("[PROCESSING] Skipping final pass ({} streaming is sufficient)", engine_name);
        let transcript = session_transcript.lock().unwrap().clone();
        let final_text = if transcript.trim().is_empty() {
            String::new()
        } else {
            clean_transcript(&transcript)
        };
        println!("[FINAL_TRANSCRIPT] (Raw)\n{}", final_text);
        if let Some(path) = last_recording_path.as_ref() {
            let _ = std::fs::remove_file(path);
        }
        return Ok(final_text);
    }

    if let Some(path) = last_recording_path {
        println!(
            "[PROCESSING] Running final high-quality transcription with VAD on: {}",
            path
        );

        // Snapshot active-app context BEFORE acquiring any locks
        let app_context = get_active_context();
        if let Some(ref ctx) = app_context {
            println!("[CONTEXT] Active window: \"{}\"", ctx);
        }

        let whisper = whisper_arc.lock().unwrap();
        let mut audio_data = whisper.load_audio(&path)?;

        // Normalize the full recording before VAD + final transcription
        normalize_audio(&mut audio_data);

        println!("[PROCESSING] Applying VAD filtering for Whisper...");
        let mut vad = vad_arc.lock().unwrap();
        let timestamps = vad.get_speech_timestamps(&audio_data, 500)?;

        let mut clean = Vec::with_capacity(audio_data.len());
        if timestamps.is_empty() {
            // VAD found nothing — let Whisper decide rather than hard-failing
            println!(
                "[VAD] No speech segments found, passing full audio to Whisper as fallback"
            );
            clean.extend_from_slice(&audio_data);
        }
        for (start, end) in timestamps {
            let s = (start * 16000.0) as usize;
            let e = (end * 16000.0) as usize;
            clean.extend_from_slice(
                &audio_data[s.min(audio_data.len())..e.min(audio_data.len())],
            );
        }

        // Release locks before transcription to avoid deadlock
        drop(whisper);
        drop(vad);

        let result = {
            let mut whisper = whisper_arc.lock().unwrap();
            whisper.transcribe_audio_data(&clean, app_context.as_deref())
        };

        let _ = std::fs::remove_file(&path);

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
}

/// COMMAND: STOP RECORDING
///
/// On macOS this must be async because Tauri 2 runs synchronous commands on the
/// main (AppKit) thread — blocking it with thread joins + Whisper inference
/// freezes the entire window. Async commands are dispatched to the tokio runtime
/// instead, keeping the UI responsive.
///
/// On Windows/Linux synchronous commands already run on a thread pool so the
/// original blocking behaviour is fine, but async is harmless there too.
#[tauri::command]
pub async fn stop_recording(state: State<'_, AudioState>) -> Result<String, String> {
    // --- Quick state access (non-blocking, just mutex snapshots) ---
    *state.denoiser.lock().unwrap() = None;

    let recording = state.recording_handle.lock().unwrap().take()
        .ok_or_else(|| "Not recording".to_string())?;

    let active_engine = *state.active_engine.lock().unwrap();
    let session_transcript = state.session_transcript.clone();
    let last_recording_path = state.last_recording_path.lock().unwrap().clone();
    let whisper_arc = state.whisper.clone();
    let vad_arc = state.vad.clone();

    // --- Heavy work: dispatched off the main thread via spawn_blocking so the
    //     macOS AppKit event loop stays responsive (thread joins, VAD, Whisper). ---
    tauri::async_runtime::spawn_blocking(move || {
        stop_recording_blocking(
            recording,
            active_engine,
            session_transcript,
            last_recording_path,
            whisper_arc,
            vad_arc,
        )
    })
    .await
    .map_err(|e| format!("stop_recording task failed: {}", e))?
}
