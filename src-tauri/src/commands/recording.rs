use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::bounded;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter, State};

use crate::audio::{RecordingHandle, SendStream};
use crate::audio_preprocess;
use crate::context::get_active_context;
use crate::denoise::Denoiser;
use crate::state::AudioState;
use crate::types::{ASREngine, CommandResult, TranscriptionChunk};
use crate::utils::{clean_transcript, get_recordings_dir, strip_whitelisted_sound_captions};

/// Live Parakeet chunk length in seconds. Very short windows (~1s) hurt accuracy on
/// streaming CTC; ~4s trades a bit of latency for much better context (see NeMo
/// streaming / buffered-chunk guidance).
const PARAKEET_LIVE_CHUNK_SECS: f32 = 4.0;

#[inline]
fn parakeet_min_samples(sample_rate: u32) -> usize {
    (sample_rate as f32 * PARAKEET_LIVE_CHUNK_SECS) as usize
}

/// Universal preprocess → 16 kHz, then pad to `PARAKEET_LIVE_CHUNK_SECS` at 16 kHz.
fn parakeet_preprocess_for_transcribe(
    buf: &[f32],
    sample_rate: u32,
    user_denoise: bool,
    denoiser_arc: &Arc<Mutex<Option<Denoiser>>>,
) -> Vec<f32> {
    let mut guard = denoiser_arc.lock().unwrap();
    let mut pcm16 = audio_preprocess::preprocess_live_transcribe_chunk(
        buf,
        sample_rate,
        user_denoise,
        guard.as_mut(),
    );
    drop(guard);
    let min_len = parakeet_min_samples(16000);
    if pcm16.len() < min_len {
        pcm16.resize(min_len, 0.0);
    }
    pcm16
}

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
) -> Result<CommandResult<String>, String> {
    // Guard: reject if already recording (e.g. spam hotkey)
    if state.recording_handle.lock().unwrap().is_some() {
        return Ok(CommandResult::err("already_recording", "Already recording"));
    }

    // Clone the whole state — every field is Arc<…> so this is just ref-count bumps.
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        start_recording_blocking(app_handle, state, denoise)
    })
    .await
    .map(|result| match result {
        Ok(message) => CommandResult::ok(message),
        Err(message) => {
            let lower = message.to_lowercase();
            let code = if lower.contains("microphone permission denied") {
                "mic_permission_denied"
            } else if lower.contains("no input device found")
                || lower.contains("no microphone found")
            {
                "no_input_device"
            } else if lower.contains("already recording") {
                "already_recording"
            } else {
                "recording_start_failed"
            };
            CommandResult::err(code, message)
        }
    })
    .map_err(|e| format!("start_recording task failed: {}", e))
}

/// The blocking core of start_recording, run inside spawn_blocking.
/// Receives a cloned AudioState (cheap — all fields are Arc) instead of
/// 13 individually-cloned Arc parameters.
fn start_recording_blocking(
    app_handle: AppHandle,
    state: AudioState,
    denoise: Option<bool>,
) -> Result<String, String> {
    let denoise_enabled = denoise.unwrap_or(true);
    state.recording_paused.store(false, Ordering::Relaxed);

    // 1. Setup Microphone
    let host = cpal::default_host();
    let preferred = state.selected_input_device.lock().unwrap().clone();

    let mut device_opt = None;
    let mut fallback_triggered = false;

    if let Some(ref name) = preferred {
        device_opt = host
            .input_devices()
            .ok()
            .and_then(|mut iter| iter.find(|d| d.name().ok().as_deref() == Some(name.as_str())));

        if device_opt.is_none() {
            println!(
                "[WARNING] Preferred input device '{}' not found, falling back to default",
                name
            );
            fallback_triggered = true;
        }
    }

    if device_opt.is_none() {
        device_opt = host.default_input_device();
    }

    let device =
        device_opt.ok_or("No input device found. Check that a microphone is connected.")?;
    let device_name = device
        .name()
        .unwrap_or_else(|_| "Unknown Device".to_string());

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
    let active_engine = *state.active_engine.lock().unwrap();
    match active_engine {
        ASREngine::Whisper => state.whisper.lock().unwrap().clear_context(),
        ASREngine::Parakeet => state.parakeet.lock().unwrap().clear_context(),
        ASREngine::Cohere => { /* Cohere is stateless per chunk */ }
    }
    // Reset Silero VAD LSTM state so prior session context doesn't bleed in
    state.vad.lock().unwrap().reset_state();

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
    // Bounded: prevents unbounded memory growth if file writer or transcriber falls behind.
    // Audio callback uses try_send so it never blocks the real-time capture thread.
    let (file_tx, file_rx) = bounded::<Vec<f32>>(256); // ~5s headroom at 48kHz/1024
    let (whisper_tx, whisper_rx) = bounded::<Vec<f32>>(32); // transcriber has its own accumulator

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

    // Pull shared references out of state for the transcriber thread
    let whisper = state.whisper.clone();
    let parakeet_manager = state.parakeet.clone();
    let cohere = state.cohere.clone();
    let vad = state.vad.clone();
    let active_engine = *state.active_engine.lock().unwrap();
    let session_transcript = state.session_transcript.clone();
    let denoiser_arc = state.denoiser.clone();
    let recording_handle_arc = state.recording_handle.clone();
    let denoise_enabled_thread = denoise_enabled;

    /// VAD-gated transcription — shared logic for Whisper and Cohere.
    /// Both managers expose the same `transcribe_chunk(&[f32], u32) -> Result<String, _>` API,
    /// so the entire accumulate → normalize → VAD-check → transcribe → emit pipeline
    /// lives here once instead of being copy-pasted per engine.
    ///
    /// Returns the transcript text if speech was detected and transcription succeeded,
    /// or `None` when the chunk was silence or the transcription was empty.
    #[allow(clippy::too_many_arguments)]
    fn vad_gated_transcribe(
        chunk: &mut Vec<f32>,
        sample_rate: u32,
        vad: &std::sync::Arc<std::sync::Mutex<crate::vad::VADManager>>,
        transcribe: &mut impl FnMut(&[f32], u32) -> Result<String, String>,
        method: &str,
        emoji: &str,
        app: &AppHandle,
        session_transcript: &std::sync::Arc<std::sync::Mutex<String>>,
        user_denoise: bool,
        denoiser_arc: &Arc<Mutex<Option<Denoiser>>>,
    ) -> bool {
        let mut denoise_guard = denoiser_arc.lock().unwrap();
        let pcm16 = audio_preprocess::preprocess_live_transcribe_chunk(
            chunk.as_slice(),
            sample_rate,
            user_denoise,
            denoise_guard.as_mut(),
        );
        drop(denoise_guard);

        if pcm16.is_empty() {
            return false;
        }

        // Scan the full chunk frame-by-frame and take the peak speech probability.
        // Evaluating only the first 32 ms (one Silero frame) of a 6-second chunk is
        // unreliable: the LSTM needs several warmup frames from a cold state, and speech
        // can begin anywhere in the window. Threshold 0.25 matches assemble_speech_audio's
        // second Silero pass (onset=0.28) — Silero returns 0.25–0.40 for clean speech.
        let is_speech = vad.lock().unwrap().max_speech_prob(&pcm16, usize::MAX);

        if is_speech > 0.25 {
            println!(
                "[PROCESSING] {} Speech ({:.0}%) - {} transcribing {:.2}s chunk...",
                emoji,
                is_speech * 100.0,
                method,
                pcm16.len() as f32 / 16000.0,
            );
            let start = std::time::Instant::now();
            match transcribe(&pcm16, 16000) {
                Ok(text) if !text.trim().is_empty() => {
                    let text = if matches!(method, "Whisper" | "Cohere") {
                        strip_whitelisted_sound_captions(&text)
                    } else {
                        text
                    };
                    if text.trim().is_empty() {
                        return false;
                    }
                    let elapsed = start.elapsed().as_millis() as u32;
                    println!(
                        "[TRANSCRIPT] {} \"{}\" (took {}ms)",
                        emoji,
                        text.trim(),
                        elapsed
                    );
                    let _ = app.emit(
                        "transcription-chunk",
                        crate::types::TranscriptionChunk {
                            text: text.clone(),
                            processing_time_ms: elapsed,
                            method: method.to_string(),
                        },
                    );
                    session_transcript.lock().unwrap().push_str(&text);
                    true
                }
                Ok(_) => false,
                Err(e) => {
                    eprintln!("[ERROR] {} transcription error: {}", method, e);
                    false
                }
            }
        } else {
            println!(
                "[VAD] 🔇 Silence ({:.0}%) - Skipping {} chunk",
                (1.0 - is_speech) * 100.0,
                method,
            );
            false
        }
    }

    // 7. SPAWN THREAD 2: THE REAL-TIME TRANSCRIBER
    let app_clone = app_handle.clone();
    let transcriber_thread = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let chunk_size = match active_engine {
            ASREngine::Cohere => (sample_rate * 15) as usize,
            _ => (sample_rate * 6) as usize,
        };
        let max_buffer_size = chunk_size * 2;
        // Pre-allocated scratch buffer reused each iteration to avoid per-chunk Vec allocation
        let mut chunk = Vec::with_capacity(chunk_size);
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

            match active_engine {
                ASREngine::Whisper | ASREngine::Cohere => {
                    buffer.extend(samples);
                    while buffer.len() >= chunk_size {
                        if buffer.len() > max_buffer_size {
                            println!("[WARNING] Buffer full, dropping old audio to catch up");
                            buffer.drain(..chunk_size);
                        }
                        chunk.clear();
                        chunk.extend_from_slice(&buffer[..chunk_size]);
                        buffer.drain(..chunk_size);
                        if active_engine == ASREngine::Whisper {
                            crate::memory::maybe_log_process_memory_with_sizes(
                                "recording whisper live chunk start",
                                &[
                                    ("buffer_len_samples", buffer.len()),
                                    ("chunk_samples", chunk.len()),
                                    (
                                        "chunk_audio_bytes",
                                        chunk.len() * std::mem::size_of::<f32>(),
                                    ),
                                ],
                            );
                            let mut wm = whisper.lock().unwrap();
                            let mut transcribe = |c: &[f32], sr| {
                                wm.transcribe_chunk(c, sr).map_err(|e| e.to_string())
                            };
                            vad_gated_transcribe(
                                &mut chunk,
                                sample_rate,
                                &vad,
                                &mut transcribe,
                                "Whisper",
                                "🎙️",
                                &app_clone,
                                &session_transcript,
                                denoise_enabled_thread,
                                &denoiser_arc,
                            );
                        } else {
                            crate::memory::maybe_log_process_memory_with_sizes(
                                "recording cohere live chunk start",
                                &[
                                    ("buffer_len_samples", buffer.len()),
                                    ("chunk_samples", chunk.len()),
                                    (
                                        "chunk_audio_bytes",
                                        chunk.len() * std::mem::size_of::<f32>(),
                                    ),
                                ],
                            );
                            let mut gs = cohere.lock().unwrap();
                            let mut transcribe = |c: &[f32], sr| {
                                gs.transcribe_chunk(c, sr).map_err(|e| e.to_string())
                            };
                            vad_gated_transcribe(
                                &mut chunk,
                                sample_rate,
                                &vad,
                                &mut transcribe,
                                "Cohere",
                                "🪨",
                                &app_clone,
                                &session_transcript,
                                denoise_enabled_thread,
                                &denoiser_arc,
                            );
                        }
                    }
                }
                ASREngine::Parakeet => {
                    buffer.extend(samples);
                    let parakeet_chunk_size =
                        (sample_rate as f32 * PARAKEET_LIVE_CHUNK_SECS) as usize;
                    let max_buffer_size = parakeet_chunk_size * 2;
                    while buffer.len() >= parakeet_chunk_size {
                        if buffer.len() > max_buffer_size {
                            buffer.drain(..parakeet_chunk_size);
                        }
                        chunk.clear();
                        chunk.extend_from_slice(&buffer[..parakeet_chunk_size]);
                        buffer.drain(..parakeet_chunk_size);
                        crate::memory::maybe_log_process_memory_with_sizes(
                            "recording parakeet live chunk before preprocess",
                            &[
                                ("buffer_len_samples", buffer.len()),
                                ("chunk_samples", chunk.len()),
                                (
                                    "chunk_audio_bytes",
                                    chunk.len() * std::mem::size_of::<f32>(),
                                ),
                            ],
                        );
                        let buf16 = parakeet_preprocess_for_transcribe(
                            &chunk,
                            sample_rate,
                            denoise_enabled_thread,
                            &denoiser_arc,
                        );
                        crate::memory::maybe_log_process_memory_with_sizes(
                            "recording parakeet live chunk after preprocess",
                            &[
                                ("chunk_samples", chunk.len()),
                                ("buf16_samples", buf16.len()),
                                (
                                    "buf16_audio_bytes",
                                    buf16.len() * std::mem::size_of::<f32>(),
                                ),
                            ],
                        );
                        let start_time = std::time::Instant::now();
                        match parakeet_manager
                            .lock()
                            .unwrap()
                            .transcribe_chunk(&buf16, 16000)
                        {
                            Ok(transcript) if !transcript.is_empty() => {
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
                                session_transcript.lock().unwrap().push_str(&transcript);
                            }
                            Ok(_) => {}
                            Err(e) => eprintln!("[ERROR] Parakeet error: {}", e),
                        }
                    }
                }
            }
        }

        println!("[INFO] Recording stopped, processing remaining audio...");
        // Drain any remaining samples from the channel into the buffer
        while let Ok(samples) = whisper_rx.try_recv() {
            buffer.extend(samples);
        }

        // Pad 400ms of silence so trailing words aren't clipped by the
        // transcription engine. This is better than keeping the mic open
        // longer because it adds zero background noise.
        let silence_samples = (sample_rate as usize) * 400 / 1000;
        buffer.extend(std::iter::repeat(0.0_f32).take(silence_samples));

        // Flush full-sized chunks from the tail buffer
        while buffer.len() >= chunk_size {
            chunk.clear();
            chunk.extend_from_slice(&buffer[..chunk_size]);
            buffer.drain(..chunk_size);
            match active_engine {
                ASREngine::Whisper => {
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "recording whisper final flush chunk",
                        &[
                            ("remaining_buffer_samples", buffer.len()),
                            ("chunk_samples", chunk.len()),
                        ],
                    );
                    let mut wm = whisper.lock().unwrap();
                    let mut t =
                        |c: &[f32], sr| wm.transcribe_chunk(c, sr).map_err(|e| e.to_string());
                    vad_gated_transcribe(
                        &mut chunk,
                        sample_rate,
                        &vad,
                        &mut t,
                        "Whisper",
                        "🎙️",
                        &app_clone,
                        &session_transcript,
                        denoise_enabled_thread,
                        &denoiser_arc,
                    );
                }
                ASREngine::Cohere => {
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "recording cohere final flush chunk",
                        &[
                            ("remaining_buffer_samples", buffer.len()),
                            ("chunk_samples", chunk.len()),
                        ],
                    );
                    let mut gs = cohere.lock().unwrap();
                    let mut t =
                        |c: &[f32], sr| gs.transcribe_chunk(c, sr).map_err(|e| e.to_string());
                    vad_gated_transcribe(
                        &mut chunk,
                        sample_rate,
                        &vad,
                        &mut t,
                        "Cohere",
                        "🪨",
                        &app_clone,
                        &session_transcript,
                        denoise_enabled_thread,
                        &denoiser_arc,
                    );
                }
                ASREngine::Parakeet => {
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "recording parakeet final flush chunk before preprocess",
                        &[
                            ("remaining_buffer_samples", buffer.len()),
                            ("chunk_samples", chunk.len()),
                        ],
                    );
                    let buf16 = parakeet_preprocess_for_transcribe(
                        &chunk,
                        sample_rate,
                        denoise_enabled_thread,
                        &denoiser_arc,
                    );
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "recording parakeet final flush chunk after preprocess",
                        &[
                            ("buf16_samples", buf16.len()),
                            (
                                "buf16_audio_bytes",
                                buf16.len() * std::mem::size_of::<f32>(),
                            ),
                        ],
                    );
                    if let Ok(transcript) = parakeet_manager
                        .lock()
                        .unwrap()
                        .transcribe_chunk(&buf16, 16000)
                    {
                        if !transcript.is_empty() {
                            session_transcript.lock().unwrap().push_str(&transcript);
                            println!("[TRANSCRIPT] 🦜 (Final) \"{}\"", transcript.trim());
                        }
                    }
                }
            }
        }

        // Flush the sub-chunk tail (< chunk_size but > 0.1s)
        // For short tails (< 3s, e.g. a single word), bypass VAD entirely —
        // VAD is designed for filtering silence in long streams, not for
        // gating short utterances where every sample matters.
        if !buffer.is_empty() && buffer.len() as f32 / sample_rate as f32 > 0.1 {
            let tail_secs = buffer.len() as f32 / sample_rate as f32;
            let use_vad = tail_secs >= 3.0;
            match active_engine {
                ASREngine::Whisper => {
                    let mut wm = whisper.lock().unwrap();
                    if use_vad {
                        let mut t =
                            |c: &[f32], sr| wm.transcribe_chunk(c, sr).map_err(|e| e.to_string());
                        vad_gated_transcribe(
                            &mut buffer,
                            sample_rate,
                            &vad,
                            &mut t,
                            "Whisper",
                            "🎙️",
                            &app_clone,
                            &session_transcript,
                            denoise_enabled_thread,
                            &denoiser_arc,
                        );
                    } else {
                        println!(
                            "[PROCESSING] 🎙️ Short tail ({:.2}s) — bypassing VAD for Whisper",
                            tail_secs
                        );
                        let mut dg = denoiser_arc.lock().unwrap();
                        let pcm16 = audio_preprocess::preprocess_live_transcribe_chunk(
                            &buffer,
                            sample_rate,
                            denoise_enabled_thread,
                            dg.as_mut(),
                        );
                        drop(dg);
                        if let Ok(text) = wm.transcribe_chunk(&pcm16, 16000) {
                            let text = strip_whitelisted_sound_captions(&text);
                            if !text.trim().is_empty() {
                                println!("[TRANSCRIPT] 🎙️ (Tail) \"{}\"", text.trim());
                                let _ = app_clone.emit(
                                    "transcription-chunk",
                                    crate::types::TranscriptionChunk {
                                        text: text.clone(),
                                        processing_time_ms: 0,
                                        method: "Whisper".to_string(),
                                    },
                                );
                                session_transcript.lock().unwrap().push_str(&text);
                            }
                        }
                    }
                }
                ASREngine::Cohere => {
                    let mut gs = cohere.lock().unwrap();
                    if use_vad {
                        let mut t =
                            |c: &[f32], sr| gs.transcribe_chunk(c, sr).map_err(|e| e.to_string());
                        vad_gated_transcribe(
                            &mut buffer,
                            sample_rate,
                            &vad,
                            &mut t,
                            "Cohere",
                            "🪨",
                            &app_clone,
                            &session_transcript,
                            denoise_enabled_thread,
                            &denoiser_arc,
                        );
                    } else {
                        println!(
                            "[PROCESSING] 🪨 Short tail ({:.2}s) — bypassing VAD for Cohere",
                            tail_secs
                        );
                        let mut dg = denoiser_arc.lock().unwrap();
                        let pcm16 = audio_preprocess::preprocess_live_transcribe_chunk(
                            &buffer,
                            sample_rate,
                            denoise_enabled_thread,
                            dg.as_mut(),
                        );
                        drop(dg);
                        if let Ok(text) = gs.transcribe_chunk(&pcm16, 16000) {
                            let text = strip_whitelisted_sound_captions(&text);
                            if !text.trim().is_empty() {
                                println!("[TRANSCRIPT] 🪨 (Tail) \"{}\"", text.trim());
                                let _ = app_clone.emit(
                                    "transcription-chunk",
                                    crate::types::TranscriptionChunk {
                                        text: text.clone(),
                                        processing_time_ms: 0,
                                        method: "Cohere".to_string(),
                                    },
                                );
                                session_transcript.lock().unwrap().push_str(&text);
                            }
                        }
                    }
                }
                ASREngine::Parakeet => {
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "recording parakeet tail before preprocess",
                        &[("tail_buffer_samples", buffer.len())],
                    );
                    let buf16 = parakeet_preprocess_for_transcribe(
                        &buffer,
                        sample_rate,
                        denoise_enabled_thread,
                        &denoiser_arc,
                    );
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "recording parakeet tail after preprocess",
                        &[
                            ("tail_buffer_samples", buffer.len()),
                            ("buf16_samples", buf16.len()),
                            (
                                "buf16_audio_bytes",
                                buf16.len() * std::mem::size_of::<f32>(),
                            ),
                        ],
                    );
                    if let Ok(transcript) = parakeet_manager
                        .lock()
                        .unwrap()
                        .transcribe_chunk(&buf16, 16000)
                    {
                        if !transcript.is_empty() {
                            session_transcript.lock().unwrap().push_str(&transcript);
                            println!("[TRANSCRIPT] 🦜 (Final Partial) \"{}\"", transcript.trim());
                        }
                    }
                }
            }
        }

        println!("[INFO] Transcriber thread finished");
    });

    let channels = config.channels as usize;

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
                file_tx_clone.try_send(data.to_vec()).ok();

                let mono_data: Vec<f32> = if channels > 1 {
                    data.chunks(channels)
                        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };

                // RNNoise + universal chain run in the transcriber thread (48 kHz → 16 kHz order).

                // Store audio level in atomic for the emitter thread to pick up.
                // Only compute every ~5 callbacks to avoid unnecessary work.
                let cnt = level_counter_clone.fetch_add(1, Ordering::Relaxed);
                if cnt % 5 == 0 && !data.is_empty() {
                    let rms = (data.iter().map(|&s| s * s).sum::<f32>() / data.len() as f32).sqrt();
                    let level = (rms / 0.015_f32).min(1.0_f32).sqrt();
                    audio_level_writer.store(level.to_bits(), Ordering::Relaxed);
                }

                whisper_tx_clone.try_send(mono_data).ok();
            },
            move |err| {
                eprintln!("[ERROR] Audio input stream error: {}", err);
                let _ = app_for_error.emit(
                    "audio-disconnected",
                    serde_json::json!({
                        "code": "audio_device_disconnected",
                        "message": err.to_string(),
                    }),
                );
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

fn teardown_recording(recording: RecordingHandle, tail_capture_ms: u64) {
    use cpal::traits::StreamTrait;

    let RecordingHandle {
        stream,
        file_tx,
        whisper_tx,
        writer_thread,
        transcriber_thread,
        level_stop,
        level_thread,
        ..
    } = recording;

    if tail_capture_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(tail_capture_ms));
    }

    let _ = stream.0.pause();
    drop(stream);
    drop(file_tx);
    drop(whisper_tx);

    level_stop.store(true, Ordering::Relaxed);
    if let Err(e) = level_thread.join() {
        eprintln!("[ERROR] Level thread panicked: {:?}", e);
    }

    println!("[INFO] Waiting for worker threads to finish...");
    if let Err(e) = writer_thread.join() {
        eprintln!("[ERROR] Writer thread panicked: {:?}", e);
    }
    if let Err(e) = transcriber_thread.join() {
        eprintln!("[ERROR] Transcriber thread panicked: {:?}", e);
    }
    println!("[INFO] Worker threads finished.");
}

#[tauri::command]
pub fn pause_recording(state: State<'_, AudioState>) -> Result<CommandResult<String>, String> {
    let guard = state.recording_handle.lock().unwrap();
    let Some(handle) = guard.as_ref() else {
        return Ok(CommandResult::err("not_recording", "Not recording"));
    };

    handle
        .stream
        .0
        .pause()
        .map_err(|e| format!("Failed to pause recording: {}", e))?;
    state.recording_paused.store(true, Ordering::Relaxed);
    Ok(CommandResult::ok("Recording paused".to_string()))
}

#[tauri::command]
pub fn resume_recording(state: State<'_, AudioState>) -> Result<CommandResult<String>, String> {
    let guard = state.recording_handle.lock().unwrap();
    let Some(handle) = guard.as_ref() else {
        return Ok(CommandResult::err("not_recording", "Not recording"));
    };

    handle
        .stream
        .0
        .play()
        .map_err(|e| format!("Failed to resume recording: {}", e))?;
    state.recording_paused.store(false, Ordering::Relaxed);
    Ok(CommandResult::ok("Recording resumed".to_string()))
}

#[tauri::command]
pub async fn cancel_recording(state: State<'_, AudioState>) -> Result<CommandResult<()>, String> {
    *state.denoiser.lock().unwrap() = None;
    state.recording_paused.store(false, Ordering::Relaxed);

    let Some(recording) = state.recording_handle.lock().unwrap().take() else {
        return Ok(CommandResult::err("not_recording", "Not recording"));
    };
    let last_recording_path = state.last_recording_path.lock().unwrap().clone();
    let session_transcript = state.session_transcript.clone();

    tauri::async_runtime::spawn_blocking(move || {
        teardown_recording(recording, 0);
        session_transcript.lock().unwrap().clear();
        if let Some(path) = last_recording_path {
            let _ = std::fs::remove_file(path);
        }
        Ok::<CommandResult<()>, String>(CommandResult::ok(()))
    })
    .await
    .map_err(|e| format!("cancel_recording task failed: {}", e))?
}

/// COMMAND: Insert text into the focused application.
/// macOS:         AXUIElement (kAXSelectedTextAttribute) — inserts at cursor, no clipboard touch
///                → fallback: clipboard + Cmd+V
/// Windows/Linux: clipboard save → set text → Ctrl+V → restore clipboard
/// Returns Err with a short error code on failure so the frontend can show
/// a "couldn't paste" indicator without silently dropping the transcript.
#[tauri::command]
pub async fn type_text(text: String) -> Result<CommandResult<()>, String> {
    if text.trim().is_empty() || text.trim() == "[silence]" {
        return Ok(CommandResult::ok(()));
    }
    let text_to_type = text.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || insert_text(&text_to_type))
        .await
        .map(|result| match result {
            Ok(()) => CommandResult::ok(()),
            Err(message) => {
                let code = match message.as_str() {
                    "secure_input" => "paste_blocked_secure_input",
                    "console" => "paste_blocked_console",
                    _ => "paste_failed",
                };
                CommandResult::err(code, message)
            }
        })
        .map_err(|e| format!("thread_panic:{e:?}"))
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

/// Returns true when the frontmost application is a browser, terminal, or Electron
/// app whose text fields don't expose AXSelectedText. In these apps ax_insert()
/// always fails, wasting ~260ms on retries before falling back to clipboard paste.
/// Skip straight to Cmd+V for speed and reliability.
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
            extern "C" {
                fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
            }
            const RTLD_DEFAULT: *mut c_void = std::ptr::null_mut::<c_void>().wrapping_sub(2);
            let sym = dlsym(
                RTLD_DEFAULT,
                CStr::from_bytes_with_nul_unchecked(b"objc_msgSend\0").as_ptr(),
            );
            if sym.is_null() {
                return false;
            }
            std::mem::transmute(sym)
        };

        let ws_cls = objc_getClass(CStr::from_bytes_with_nul_unchecked(b"NSWorkspace\0").as_ptr());
        if ws_cls.is_null() {
            return false;
        }
        let shared_sel =
            sel_registerName(CStr::from_bytes_with_nul_unchecked(b"sharedWorkspace\0").as_ptr());
        let ws = msg_send(ws_cls, shared_sel);
        if ws.is_null() {
            return false;
        }

        let front_sel = sel_registerName(
            CStr::from_bytes_with_nul_unchecked(b"frontmostApplication\0").as_ptr(),
        );
        let app = msg_send(ws, front_sel);
        if app.is_null() {
            return false;
        }

        let bundle_sel =
            sel_registerName(CStr::from_bytes_with_nul_unchecked(b"bundleIdentifier\0").as_ptr());
        let bundle_id = msg_send(app, bundle_sel);
        if bundle_id.is_null() {
            return false;
        }

        let utf8_sel =
            sel_registerName(CStr::from_bytes_with_nul_unchecked(b"UTF8String\0").as_ptr());
        let cstr_ptr = msg_send(bundle_id, utf8_sel) as *const std::ffi::c_char;
        if cstr_ptr.is_null() {
            return false;
        }

        let bid = CStr::from_ptr(cstr_ptr).to_string_lossy();
        let bid_lower = bid.to_lowercase();
        println!("[INSERT] Frontmost app bundle ID: {}", bid);

        const PREFER_CLIPBOARD_BUNDLES: &[&str] = &[
            // ── Browsers (web content does not expose AXSelectedText) ───────
            "com.google.chrome",
            "org.mozilla.firefox",
            "com.apple.safari",
            "company.thebrowser.browser", // Arc
            "com.brave.browser",
            "com.operasoftware.opera",
            "com.vivaldi.vivaldi",
            "com.microsoft.edgemac", // Edge
            "org.chromium.chromium",
            "app.zen-browser",    // Zen
            "com.kagi.kagimacOS", // Orion
            "com.naver.whale",    // Whale
            // Google Meet has no standalone macOS app — covered by browsers above
            // ── Terminals (AXSelectedText write is unsupported) ──────────────
            "com.apple.terminal",
            "com.googlecode.iterm2",
            "com.github.wez.wezterm",
            "org.alacritty",
            "net.kovidgoyal.kitty",
            // ── Electron / web-rendered apps ─────────────────────────────────
            "com.microsoft.vscode",      // VS Code
            "com.tinyspeck.slackmacgap", // Slack
            "com.hnc.discord",           // Discord
            "notion.id",                 // Notion
            "md.obsidian",               // Obsidian
            "net.whatsapp.whatsapp",     // WhatsApp
            "com.evernote.evernote",     // Evernote
            "abnerworks.typora",         // Typora
            "com.todesktop",             // Cursor + other ToDesktop Electron apps
            "com.github.atom",           // Atom
            "org.zotero.zotero",         // Zotero
            "com.superhuman",            // Superhuman
            "com.goodnotesapp",          // GoodNotes
            // ── Custom rendering engines ──────────────────────────────────────
            "com.sublimetext", // Sublime Text (Skia renderer, no AX text)
            // ── Communication & productivity ──────────────────────────────────
            "com.apple.mail",      // Apple Mail
            "com.apple.mobilesms", // Apple Messages
            "us.zoom.xos",         // Zoom
            "com.raycast.macos",   // Raycast
            // ── Writing & note-taking apps ────────────────────────────────────
            "net.shinyfrog.bear",    // Bear
            "com.ulyssesapp.mac",    // Ulysses
            "com.apple.notes",       // Apple Notes
            "com.apple.iwork.pages", // Apple Pages
            // ── Microsoft Office ──────────────────────────────────────────────
            "com.microsoft.word",    // Word
            "com.microsoft.excel",   // Excel
            "com.microsoft.outlook", // Outlook
            // ── Other productivity ────────────────────────────────────────────
            "com.ideasoncanvas",  // MindNode
            "com.adobe.indesign", // Adobe InDesign
        ];

        PREFER_CLIPBOARD_BUNDLES
            .iter()
            .any(|b| bid_lower.starts_with(b))
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

    enum SavedClipboard {
        Text(String),
        Image(arboard::ImageData<'static>),
        Nothing,
    }
    let previous = if let Ok(t) = clipboard.get_text() {
        SavedClipboard::Text(t)
    } else if let Ok(img) = clipboard.get_image() {
        SavedClipboard::Image(arboard::ImageData {
            width: img.width,
            height: img.height,
            bytes: std::borrow::Cow::Owned(img.bytes.into_owned()),
        })
    } else {
        SavedClipboard::Nothing
    };

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
        simulate_cmd_v_cgevent()?;
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
    match previous {
        SavedClipboard::Text(t) => {
            let _ = clipboard.set_text(t);
        }
        SavedClipboard::Image(img) => {
            let _ = clipboard.set_image(img);
        }
        SavedClipboard::Nothing => {}
    }
    Ok(())
}

/// macOS fix: Simulate Cmd+V using CGEvent instead of enigo.
/// Enigo's key simulation calls HIToolbox TSMGetInputSourceProperty which
/// requires the main dispatch queue and crashes from background threads.
/// CGEvent's CGEventPost has no such restriction and is thread-safe.
#[cfg(target_os = "macos")]
fn simulate_cmd_v_cgevent() -> Result<(), String> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // kVK_ANSI_V = 0x09
    const VK_V: CGKeyCode = 0x09;

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|_| {
        eprintln!("[INSERT] CGEventSource creation failed");
        "cgevent_source".to_string()
    })?;

    let key_down = CGEvent::new_keyboard_event(source.clone(), VK_V, true).map_err(|_| {
        eprintln!("[INSERT] Failed to create CGEvent for Cmd+V");
        "cgevent_create".to_string()
    })?;
    let key_up = CGEvent::new_keyboard_event(source, VK_V, false)
        .map_err(|_| "cgevent_create".to_string())?;

    key_down.set_flags(CGEventFlags::CGEventFlagCommand);
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);
    // AnnotatedSession delivers the event after the window server has
    // assigned it a target process and annotated it with process/window
    // info. This is the level at which Carbon HIToolbox (used by Word,
    // Excel, Outlook) intercepts keyboard shortcuts — posting at HID
    // bypasses that layer and those apps silently ignore the event.
    key_down.post(core_graphics::event::CGEventTapLocation::AnnotatedSession);
    key_up.post(core_graphics::event::CGEventTapLocation::AnnotatedSession);
    Ok(())
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
        let matching =
            IOServiceMatching(CStr::from_bytes_with_nul_unchecked(b"IOHIDSystem\0").as_ptr());
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
    // Brief tail capture for OS audio scheduling; silence padding in the
    // transcriber thread handles the actual word-boundary safety margin.
    teardown_recording(recording, 80);

    if active_engine == ASREngine::Parakeet || active_engine == ASREngine::Cohere {
        let engine_name = if active_engine == ASREngine::Parakeet {
            "Parakeet"
        } else {
            "Cohere"
        };
        println!(
            "[PROCESSING] Skipping final pass ({} streaming is sufficient)",
            engine_name
        );
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

        // Pad 400ms of silence so trailing words aren't clipped by VAD or Whisper
        audio_data.extend(std::iter::repeat(0.0_f32).take(16000 * 400 / 1000));

        // Universal preprocess on the saved 16 kHz WAV (same chain as file speech assembly).
        audio_preprocess::preprocess_assembled_speech_16k(&mut audio_data);

        println!("[PROCESSING] Applying VAD filtering for Whisper...");
        let mut vad = vad_arc.lock().unwrap();
        // For short recordings (< 4s, likely a single word or phrase), use a
        // more permissive VAD threshold and wider padding so short utterances
        // aren't accidentally filtered out.
        let audio_duration_s = audio_data.len() as f32 / 16000.0;
        let (vad_padding, vad_threshold) = if audio_duration_s < 4.0 {
            println!(
                "[VAD] Short recording ({:.1}s) — using permissive threshold",
                audio_duration_s
            );
            (800_usize, 0.2_f32)
        } else {
            (500_usize, 0.35_f32)
        };
        let timestamps = vad.get_speech_timestamps_hysteresis(
            &audio_data,
            vad_padding,
            vad_threshold,
            vad_threshold * 0.5,
        )?;

        let mut clean = Vec::with_capacity(audio_data.len());
        if timestamps.is_empty() {
            // VAD found nothing — let Whisper decide rather than hard-failing
            println!("[VAD] No speech segments found, passing full audio to Whisper as fallback");
            clean.extend_from_slice(&audio_data);
        }
        for (start, end) in timestamps {
            let s = (start * 16000.0) as usize;
            let e = (end * 16000.0) as usize;
            clean.extend_from_slice(&audio_data[s.min(audio_data.len())..e.min(audio_data.len())]);
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
                Err(format!("Final transcription failed: {}", e))
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
pub async fn stop_recording(state: State<'_, AudioState>) -> Result<CommandResult<String>, String> {
    // --- Quick state access (non-blocking, just mutex snapshots) ---
    *state.denoiser.lock().unwrap() = None;
    state.recording_paused.store(false, Ordering::Relaxed);

    let Some(recording) = state.recording_handle.lock().unwrap().take() else {
        return Ok(CommandResult::err("not_recording", "Not recording"));
    };

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
    .map(|result| match result {
        Ok(transcript) => CommandResult::ok(transcript),
        Err(message) => CommandResult::err("recording_stop_failed", message),
    })
    .map_err(|e| format!("stop_recording task failed: {}", e))
}
