use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::bounded;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter, State};

use crate::audio::{RecordingHandle, SendStream};
use crate::audio_preprocess;
use crate::denoise::Denoiser;
use crate::state::AudioState;
use crate::types::{ASREngine, CommandResult};
use crate::utils::{clean_transcript, get_recordings_dir, strip_whitelisted_sound_captions};

/// Granite's final pass transcribes the saved recording in one go up to this
/// length, and in overlapping windows beyond it.
const FINAL_PASS_FULL_MAX_SAMPLES: usize = 16_000 * 240; // 4 minutes
const FINAL_PASS_CHUNK_SAMPLES: usize = 16_000 * 180; // 3 minutes
const FINAL_PASS_OVERLAP_SAMPLES: usize = 16_000 * 2;

/// Live chunk length for each engine: Qwen3 needs longer context per decode.
fn live_chunk_samples(engine: ASREngine, sample_rate: u32) -> usize {
    match engine {
        ASREngine::Qwen3 => (sample_rate * 15) as usize,
        ASREngine::Whisper | ASREngine::Granite => (sample_rate * 6) as usize,
    }
}

/// Name shown in logs and on live transcription chunks.
fn engine_label(engine: ASREngine) -> &'static str {
    match engine {
        ASREngine::Whisper => "Whisper",
        ASREngine::Granite => "Granite",
        ASREngine::Qwen3 => "Qwen3",
    }
}

fn pop_audio_chunk(buffer: &mut VecDeque<f32>, chunk_size: usize, scratch: &mut Vec<f32>) {
    scratch.clear();
    if scratch.capacity() < chunk_size {
        scratch.reserve(chunk_size - scratch.capacity());
    }
    scratch.extend(buffer.drain(..chunk_size));
}

fn discard_audio_front(buffer: &mut VecDeque<f32>, samples: usize) {
    let drop_samples = samples.min(buffer.len());
    buffer.drain(..drop_samples).for_each(drop);
}

fn load_recording_for_final_pass(path: &str) -> Result<Vec<f32>, String> {
    let (mut mono, sample_rate) = crate::audio_decode::decode_audio_mono_f32(Path::new(path))?;

    if sample_rate != 16000 {
        let resampled = audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
        drop(mono);
        mono = resampled;
    }

    // Give the model a clean trailing boundary without asking VAD to remove
    // anything.
    mono.extend(std::iter::repeat(0.0_f32).take(16000 * 400 / 1000));
    audio_preprocess::preprocess_assembled_speech_16k(&mut mono);

    if mono.is_empty() {
        Err("Saved recording contained no audio samples".to_string())
    } else {
        Ok(mono)
    }
}

fn normalize_merge_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn append_transcript_with_word_overlap(base: &mut String, next: &str) {
    let next = next.trim();
    if next.is_empty() {
        return;
    }
    if base.trim().is_empty() {
        base.push_str(next);
        return;
    }

    let base_words: Vec<&str> = base.split_whitespace().collect();
    let next_words: Vec<&str> = next.split_whitespace().collect();
    let max_overlap = base_words.len().min(next_words.len()).min(12);
    let mut overlap = 0usize;

    for n in (1..=max_overlap).rev() {
        let base_tail = &base_words[base_words.len() - n..];
        let next_head = &next_words[..n];
        if base_tail
            .iter()
            .zip(next_head.iter())
            .all(|(a, b)| normalize_merge_word(a) == normalize_merge_word(b))
        {
            overlap = n;
            break;
        }
    }

    let remainder = next_words[overlap..].join(" ");
    if !remainder.is_empty() {
        if !base.ends_with(char::is_whitespace) {
            base.push(' ');
        }
        base.push_str(&remainder);
    }
}

/// Transcribes a whole saved recording (Granite's final pass), in overlapping
/// windows when it is long, merging the words the windows share.
fn transcribe_final_pass(
    manager: &Arc<std::sync::Mutex<crate::gguf_asr::GgufAsrManager>>,
    audio: Vec<f32>,
) -> Result<String, String> {
    println!(
        "[FINAL_PASS] Transcribing saved recording ({:.2}s)...",
        audio.len() as f32 / 16000.0
    );
    let mut manager = manager.lock().map_err(|_| "ASR lock poisoned".to_string())?;

    if audio.len() <= FINAL_PASS_FULL_MAX_SAMPLES {
        return manager.transcribe_chunk(&audio, 16000, None);
    }

    let mut merged = String::new();
    let mut start = 0usize;
    while start < audio.len() {
        let end = (start + FINAL_PASS_CHUNK_SAMPLES).min(audio.len());
        let text = manager.transcribe_chunk(&audio[start..end], 16000, None)?;
        append_transcript_with_word_overlap(&mut merged, &text);
        if end == audio.len() {
            break;
        }
        start = end.saturating_sub(FINAL_PASS_OVERLAP_SAMPLES);
    }
    Ok(merged)
}

/// Tells the UI whether a recording is running. Recordings can be started or
/// stopped outside the UI (control server, tray), and the UI's own flag would
/// otherwise go stale and keep showing RECORDING.
fn emit_recording_state(app: &AppHandle, state: &AudioState) {
    let is_recording = state.recording_handle.lock().map(|h| h.is_some()).unwrap_or(false);
    let _ = app.emit(
        "recording-state",
        serde_json::json!({
            "is_recording": is_recording,
            "is_dual_channel": is_recording && state.last_recording_is_dual_channel.load(Ordering::SeqCst),
        }),
    );
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
    audio_source: Option<String>,
) -> Result<CommandResult<String>, String> {
    let _model_operation = state.begin_model_operation()?;
    // Guard: reject if already recording (e.g. spam hotkey)
    if state.recording_handle.lock().unwrap().is_some() {
        return Ok(CommandResult::err("already_recording", "Already recording"));
    }

    if let Some(key) = crate::meeting_continuation::call_key(
        state.meeting_detector.get_status().active_meetings.first(),
    ) {
        crate::meeting_continuation::claim_for_recording(&key);
    }

    // Clone the whole state — every field is Arc<…> so this is just ref-count bumps.
    let state = (*state).clone();
    let app_for_event = app_handle.clone();
    let state_for_event = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        start_recording_blocking(app_handle, state, denoise, audio_source)
    })
    .await;
    if !matches!(&result, Ok(Ok(_))) {
        crate::meeting_continuation::release_recording_claim();
    }
    result.map(|result| match result {
        Ok(message) => {
            emit_recording_state(&app_for_event, &state_for_event);
            CommandResult::ok(message)
        }
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
    audio_source: Option<String>,
) -> Result<String, String> {
    let denoise_enabled = denoise.unwrap_or(true);
    state.recording_paused.store(false, Ordering::Relaxed);

    let is_dual_channel = audio_source
        .as_deref()
        .map(|s| s == "dual_channel")
        .unwrap_or_else(|| *state.audio_source_mode.lock().unwrap() == "dual_channel");

    #[cfg(target_os = "linux")]
    if is_dual_channel {
        return Err("Dual-channel meeting audio capture is not supported on Linux".into());
    }

    state.last_recording_is_dual_channel.store(is_dual_channel, Ordering::SeqCst);

    // 1. Setup Audio Config & Device
    let (config_channels, config_sample_rate, cpal_device, cpal_config) = if is_dual_channel {
        println!("[INFO] Dual-Channel System Loopback & Mic Recorder selected (stereo 48 kHz)");
        (2u16, 48000u32, None, None)
    } else {
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

        #[cfg(target_os = "linux")]
        {
            // On Linux, detect if PipeWire / PulseAudio is active
            let is_pipewire = std::env::var("PIPEWIRE_REMOTE").is_ok()
                || std::env::var("XDG_RUNTIME_DIR")
                    .map(|p| {
                        std::path::Path::new(&p).join("pipewire-0").exists()
                            || std::path::Path::new(&p).join("pulse/native").exists()
                    })
                    .unwrap_or(false);

            // If no preferred device, or if preferred device is a raw hardware "hw:X,Y" PCM while PipeWire is active,
            // prioritize the virtual ALSA/PipeWire PCM ("default" or "pipewire") to eliminate EBUSY device lock contention.
            let prefer_virtual = device_opt.is_none()
                || (is_pipewire
                    && preferred
                        .as_deref()
                        .map(|s| s.starts_with("hw:") || s.contains("hw:"))
                        .unwrap_or(false));

            if prefer_virtual {
                if let Ok(devices) = host.input_devices() {
                    let dev_list: Vec<_> = devices.collect();
                    if let Some(d) = dev_list.into_iter().find(|d| {
                        if let Ok(name) = d.name() {
                            name == "default"
                                || name.to_lowercase().contains("pipewire")
                                || name == "pulse"
                        } else {
                            false
                        }
                    }) {
                        println!("[INFO] Linux PipeWire audio: Selected virtual PCM '{}' to eliminate EBUSY device lock contention", d.name().unwrap_or_default());
                        device_opt = Some(d);
                    }
                }
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

        let cfg: cpal::StreamConfig = device
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

        (cfg.channels, cfg.sample_rate.0, Some(device), Some(cfg))
    };

    // 2. Prepare Output File
    let recordings_dir = get_recordings_dir()?;
    let prefix = if is_dual_channel { "meeting" } else { "recording" };
    let filename = format!("{}_{}.wav", prefix, chrono::Utc::now().timestamp());
    let path = recordings_dir.join(&filename);

    println!("[INFO] Saving recording to: {}", path.display());

    // 3. Reset AI Context (Start fresh for new recording)
    let active_engine = *state.active_engine.lock().unwrap();
    match active_engine {
        ASREngine::Whisper => state.whisper.lock().unwrap().clear_context(),
        ASREngine::Granite => state.granite.lock().unwrap().clear_context(),
        ASREngine::Qwen3 => state.qwen3.lock().unwrap().clear_context(),
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
        channels: config_channels,
        sample_rate: config_sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;

    // 5. Create COMMUNICATION PIPES (Channels)
    // Bounded: prevents unbounded memory growth if file writer or transcriber falls behind.
    // Audio callback uses try_send so it never blocks the real-time capture thread.
    //
    // A slow encode pass (Qwen3 on a 15-second chunk, Whisper on CPU) can take several
    // seconds. At 48 kHz / 1024-sample callbacks (~21 ms each) a 32-message bound fills
    // in ~672 ms and then try_send silently drops audio — the "whole last sentence
    // disappeared" symptom. 512 messages ≈ 10.7 s of headroom.
    let (file_tx, file_rx) = bounded::<Vec<f32>>(256); // ~5s headroom at 48kHz/1024
    let (whisper_tx, whisper_rx) = bounded::<Vec<f32>>(512);

    let file_tx_clone = file_tx.clone();
    let whisper_tx_clone = whisper_tx.clone();
    let transcriber_dropped_callbacks = Arc::new(AtomicU64::new(0));
    let transcriber_dropped_samples = Arc::new(AtomicU64::new(0));
    let transcriber_dropped_callbacks_writer = transcriber_dropped_callbacks.clone();
    let transcriber_dropped_samples_writer = transcriber_dropped_samples.clone();

    let sample_rate = config_sample_rate;

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
    let active_engine = *state.active_engine.lock().unwrap();
    // Granite / Qwen3 manager (None for Whisper).
    let gguf = state.gguf_manager(active_engine);
    let engine_label = engine_label(active_engine);
    let vad = state.vad.clone();
    let session_transcript = state.session_transcript.clone();
    let denoiser_arc = state.denoiser.clone();
    let recording_handle_arc = state.recording_handle.clone();
    let denoise_enabled_thread = denoise_enabled;
    let transcriber_dropped_callbacks_reader = transcriber_dropped_callbacks.clone();
    let transcriber_dropped_samples_reader = transcriber_dropped_samples.clone();

    /// VAD-gated transcription — shared logic for Whisper, Granite and Qwen3.
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
                    let text = if matches!(method, "Whisper") {
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
                    {
                        let mut st = session_transcript.lock().unwrap();
                        if !st.is_empty() {
                            st.push(' ');
                        }
                        st.push_str(text.trim());
                    }
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
        // Apply P-core affinity, elevated priority, and disable EcoQoS on Windows hybrid CPUs
        crate::platform_tuning::apply_thread_performance_affinity();

        let mut buffer: VecDeque<f32> = VecDeque::new();
        let chunk_size = live_chunk_samples(active_engine, sample_rate);
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

            buffer.extend(samples);
            while buffer.len() >= chunk_size {
                if buffer.len() > max_buffer_size {
                    println!("[WARNING] Buffer full, dropping old audio to catch up");
                    discard_audio_front(&mut buffer, chunk_size);
                }
                pop_audio_chunk(&mut buffer, chunk_size, &mut chunk);
                if let Some(gguf) = &gguf {
                    let mut manager = gguf.lock().unwrap();
                    let mut transcribe = |c: &[f32], sr| manager.transcribe_chunk(c, sr, None);
                    vad_gated_transcribe(
                        &mut chunk,
                        sample_rate,
                        &vad,
                        &mut transcribe,
                        engine_label,
                        "🔊",
                        &app_clone,
                        &session_transcript,
                        denoise_enabled_thread,
                        &denoiser_arc,
                    );
                } else {
                    crate::memory::maybe_log_process_memory_with_sizes(
                        "recording whisper live chunk start",
                        &[
                            ("buffer_len_samples", buffer.len()),
                            ("chunk_samples", chunk.len()),
                            ("chunk_audio_bytes", chunk.len() * std::mem::size_of::<f32>()),
                        ],
                    );
                    let mut wm = whisper.lock().unwrap();
                    let mut transcribe =
                        |c: &[f32], sr| wm.transcribe_chunk(c, sr).map_err(|e| e.to_string());
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

        // Flush full-sized chunks from the tail buffer, at the live chunk size.
        while buffer.len() >= chunk_size {
            pop_audio_chunk(&mut buffer, chunk_size, &mut chunk);
            if let Some(gguf) = &gguf {
                let mut manager = gguf.lock().unwrap();
                let mut transcribe = |c: &[f32], sr| manager.transcribe_chunk(c, sr, None);
                vad_gated_transcribe(
                    &mut chunk,
                    sample_rate,
                    &vad,
                    &mut transcribe,
                    engine_label,
                    "🔊",
                    &app_clone,
                    &session_transcript,
                    denoise_enabled_thread,
                    &denoiser_arc,
                );
            } else {
                crate::memory::maybe_log_process_memory_with_sizes(
                    "recording whisper final flush chunk",
                    &[
                        ("remaining_buffer_samples", buffer.len()),
                        ("chunk_samples", chunk.len()),
                    ],
                );
                let mut wm = whisper.lock().unwrap();
                let mut t = |c: &[f32], sr| wm.transcribe_chunk(c, sr).map_err(|e| e.to_string());
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
        }

        // Flush the sub-chunk tail (< chunk_size but > 0.1s)
        // For short tails (< 3s, e.g. a single word), bypass VAD entirely —
        // VAD is designed for filtering silence in long streams, not for
        // gating short utterances where every sample matters.
        if !buffer.is_empty() && buffer.len() as f32 / sample_rate as f32 > 0.1 {
            let mut tail: Vec<f32> = buffer.drain(..).collect();
            let tail_secs = tail.len() as f32 / sample_rate as f32;
            let use_vad = tail_secs >= 3.0;
            match active_engine {
                ASREngine::Whisper => {
                    let mut wm = whisper.lock().unwrap();
                    if use_vad {
                        let mut t =
                            |c: &[f32], sr| wm.transcribe_chunk(c, sr).map_err(|e| e.to_string());
                        vad_gated_transcribe(
                            &mut tail,
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
                            &tail,
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
                                let mut st = session_transcript.lock().unwrap();
                                if !st.is_empty() {
                                    st.push(' ');
                                }
                                st.push_str(text.trim());
                            }
                        }
                    }
                }
                ASREngine::Granite | ASREngine::Qwen3 => {
                    let gguf = gguf.as_ref().expect("GGUF engine");
                    let mut manager = gguf.lock().unwrap();
                    let mut transcribe = |c: &[f32], sr| manager.transcribe_chunk(c, sr, None);
                    vad_gated_transcribe(
                        &mut tail,
                        sample_rate,
                        &vad,
                        &mut transcribe,
                        engine_label,
                        "🔊",
                        &app_clone,
                        &session_transcript,
                        denoise_enabled_thread,
                        &denoiser_arc,
                    );
                }
            }
        }

        let dropped_callbacks = transcriber_dropped_callbacks_reader.load(Ordering::Relaxed);
        if dropped_callbacks > 0 {
            let dropped_samples = transcriber_dropped_samples_reader.load(Ordering::Relaxed);
            println!(
                "[AUDIO_DROP] Transcriber queue dropped {} callback(s), {} sample(s) total",
                dropped_callbacks, dropped_samples
            );
        }

        println!("[INFO] Transcriber thread finished");
    });

    let channels = config_channels as usize;

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
            crate::overlay::push_level(&app_for_level, level);
        }
    });

    if is_dual_channel {
        use crate::audio_dual_channel::DualChannelTarget;
        let dual_stop = Arc::new(AtomicBool::new(false));

        // Tap only the detected meeting's app, so other apps' sound (notifications,
        // music, anything else playing) stays out of the callers channel. Without a
        // detected meeting (or for the simulator's synthetic pid) fall back to the
        // whole system mix.
        let meeting_pid = state
            .meeting_detector
            .get_status()
            .active_meetings
            .first()
            .map(|m| m.pid)
            .filter(|pid| *pid > 0 && *pid != 99999);
        let target = meeting_pid.map(DualChannelTarget::Process).unwrap_or(DualChannelTarget::System);
        println!("[INFO] Dual-channel callers track: {:?}", target);

        let dc_handle = match crate::audio_dual_channel::start_dual_channel_capture(
            target,
            48000,
            file_tx_clone.clone(),
            whisper_tx_clone.clone(),
            app_handle.clone(),
            dual_stop.clone(),
        ) {
            Ok(handle) => handle,
            Err(e) if target != DualChannelTarget::System => {
                eprintln!("[WARN] Meeting-process capture failed ({}); falling back to system audio", e);
                crate::audio_dual_channel::start_dual_channel_capture(
                    DualChannelTarget::System,
                    48000,
                    file_tx_clone,
                    whisper_tx_clone,
                    app_handle.clone(),
                    dual_stop.clone(),
                )?
            }
            Err(e) => return Err(e),
        };

        *recording_handle_arc.lock().unwrap() = Some(RecordingHandle {
            stream: None,
            file_tx,
            whisper_tx,
            writer_thread,
            transcriber_thread,
            level_stop,
            level_thread,
            is_dual_channel: true,
            dual_channel_stop: Some(dc_handle.stop_signal),
            dual_channel_thread: Some(dc_handle.capture_thread),
        });

        println!("[INFO] Dual-channel recording started: {}", path.display());
        return Ok(format!("Recording started: {}", path.display()));
    }

    let device = cpal_device.ok_or("No input device available for standard recording")?;
    let config = cpal_config.ok_or("No input audio configuration available")?;

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

                let mono_len = mono_data.len();
                if whisper_tx_clone.try_send(mono_data).is_err() {
                    let dropped_callbacks =
                        transcriber_dropped_callbacks_writer.fetch_add(1, Ordering::Relaxed) + 1;
                    transcriber_dropped_samples_writer
                        .fetch_add(mono_len as u64, Ordering::Relaxed);
                    if dropped_callbacks == 1 || dropped_callbacks % 100 == 0 {
                        eprintln!(
                            "[AUDIO_DROP] Transcriber queue dropped {} callback(s); latest={} samples",
                            dropped_callbacks, mono_len
                        );
                    }
                }
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
        stream: Some(SendStream(stream)),
        file_tx,
        whisper_tx,
        writer_thread,
        transcriber_thread,
        level_stop,
        level_thread,
        is_dual_channel: false,
        dual_channel_stop: None,
        dual_channel_thread: None,
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
        is_dual_channel: _,
        dual_channel_stop,
        dual_channel_thread,
    } = recording;

    if tail_capture_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(tail_capture_ms));
    }

    if let Some(stream) = stream {
        let _ = stream.0.pause();
        drop(stream);
    }
    if let Some(dc_stop) = dual_channel_stop {
        dc_stop.store(true, Ordering::Relaxed);
    }
    if let Some(thread) = dual_channel_thread {
        if let Err(e) = thread.join() {
            eprintln!("[ERROR] Dual-channel capture thread panicked: {:?}", e);
        }
    }
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

    if let Some(ref stream) = handle.stream {
        stream
            .0
            .pause()
            .map_err(|e| format!("Failed to pause recording: {}", e))?;
    }
    state.recording_paused.store(true, Ordering::Relaxed);
    Ok(CommandResult::ok("Recording paused".to_string()))
}

#[tauri::command]
pub fn resume_recording(state: State<'_, AudioState>) -> Result<CommandResult<String>, String> {
    let guard = state.recording_handle.lock().unwrap();
    let Some(handle) = guard.as_ref() else {
        return Ok(CommandResult::err("not_recording", "Not recording"));
    };

    if let Some(ref stream) = handle.stream {
        stream
            .0
            .play()
            .map_err(|e| format!("Failed to resume recording: {}", e))?;
    }
    state.recording_paused.store(false, Ordering::Relaxed);
    Ok(CommandResult::ok("Recording resumed".to_string()))
}

#[tauri::command]
pub async fn cancel_recording(state: State<'_, AudioState>) -> Result<CommandResult<()>, String> {
    let _model_operation = state.begin_model_operation()?;
    *state.denoiser.lock().unwrap() = None;
    state.recording_paused.store(false, Ordering::Relaxed);

    let Some(recording) = state.recording_handle.lock().unwrap().take() else {
        return Ok(CommandResult::err("not_recording", "Not recording"));
    };
    let last_recording_path = state.last_recording_path.lock().unwrap().clone();
    let session_transcript = state.session_transcript.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        teardown_recording(recording, 0);
        session_transcript.lock().unwrap().clear();
        if let Some(path) = last_recording_path {
            let _ = std::fs::remove_file(path);
        }
        Ok::<CommandResult<()>, String>(CommandResult::ok(()))
    })
    .await;
    crate::meeting_continuation::release_recording_claim();
    result.map_err(|e| format!("cancel_recording task failed: {}", e))?
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

/// Injects transcription text into the active window.
/// On Linux, executes the multi-tier Wayland/X11 injection strategy.
/// On macOS/Windows, delegates to the platform-specific clipboard paste routine.
#[allow(dead_code)]
pub fn inject_transcription(
    text: &str,
) -> Result<crate::text_injection::TextInjectionBackend, String> {
    crate::text_injection::inject_text_or_paste(text)
}

/// Linux: insert through the Wayland/X11 text-injection chain.
#[cfg(target_os = "linux")]
fn clipboard_paste(text: &str) -> Result<(), String> {
    let backend = inject_transcription(text)?;
    println!(
        "[INSERT] Linux text injection succeeded using backend {:?}",
        backend
    );
    Ok(())
}

/// Clipboard + simulated paste keystroke (Cmd+V on macOS, Ctrl+V elsewhere).
/// Saves and restores the previous clipboard content.
#[cfg(not(target_os = "linux"))]
fn clipboard_paste(text: &str) -> Result<(), String> {
    let _guard = crate::text_injection::CLIPBOARD_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

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

/// Builds the per-turn transcriber for meeting diarization from the loaded engine.
/// Each turn is transcribed on its own, without the streaming context of the live
/// session, so earlier speech cannot bias it.
fn meeting_turn_transcriber<'a>(
    active_engine: ASREngine,
    whisper_arc: &'a Arc<std::sync::Mutex<crate::whisper::WhisperManager>>,
    gguf: Option<&'a Arc<std::sync::Mutex<crate::gguf_asr::GgufAsrManager>>>,
) -> Option<Box<dyn FnMut(&[f32], u32) -> Option<String> + 'a>> {
    let to_16k = |samples: &[f32], rate: u32| -> Option<Vec<f32>> {
        if rate == 16000 {
            Some(samples.to_vec())
        } else {
            audio_preprocess::resample_mono_to_16k(samples, rate).ok()
        }
    };
    match (active_engine, gguf) {
        (ASREngine::Whisper, _) | (_, None) => Some(Box::new(move |samples: &[f32], rate: u32| {
            let audio = to_16k(samples, rate)?;
            let text = whisper_arc.lock().ok()?.transcribe_audio_data(&audio, None).ok()?;
            Some(clean_transcript(&text))
        })),
        (_, Some(gguf)) => Some(Box::new(move |samples: &[f32], rate: u32| {
            let audio = to_16k(samples, rate)?;
            let text = gguf.lock().ok()?.transcribe_chunk(&audio, 16000, None).ok()?;
            Some(clean_transcript(&text))
        })),
    }
}

/// Length of a WAV in milliseconds (0 when unreadable).
fn wav_duration_ms(path: &std::path::Path) -> u64 {
    hound::WavReader::open(path)
        .map(|r| r.duration() as u64 * 1000 / r.spec().sample_rate.max(1) as u64)
        .unwrap_or(0)
}

fn process_and_save_meeting_if_applicable(
    is_meeting: bool,
    last_recording_path: Option<&str>,
    final_text: &str,
    meeting_info: Option<crate::meeting_detector::MeetingInfo>,
    active_engine: ASREngine,
    whisper_arc: &Arc<std::sync::Mutex<crate::whisper::WhisperManager>>,
    gguf: Option<&Arc<std::sync::Mutex<crate::gguf_asr::GgufAsrManager>>>,
) -> Result<Option<i64>, String> {
    // meeting_info is None for a dual-channel recording with no detected meeting;
    // it is saved under the "Direct Audio" label below.
    if !is_meeting {
        return Ok(None);
    }
    let src_path_str = last_recording_path.ok_or("Meeting recording path is missing")?;
    let src_path = std::path::Path::new(src_path_str);
    if !src_path.exists() {
        return Err(format!("Meeting recording is missing: {}", src_path.display()));
    }

    let meetings_dir = crate::commands::meetings::get_meetings_dir()?;
    let meeting_timestamp = chrono::Utc::now().timestamp_millis();
    let dest_filename = format!("meeting_{}.wav", meeting_timestamp);
    let dest_path = meetings_dir.join("audio").join(dest_filename);

    // Recording the same call again soon after stopping continues that meeting:
    // join the kept recording and this one, and re-process the whole call.
    let call_key = crate::meeting_continuation::call_key(meeting_info.as_ref());
    let src_ms = wav_duration_ms(src_path);
    let mut continued = None;
    if let Some(p) = call_key.as_deref().and_then(|k| crate::meeting_continuation::take_match(k, src_ms)) {
        match crate::meeting_continuation::join_wavs(&p.wav, src_path, &dest_path) {
            Ok(()) => {
                println!("[MEETING] Continuing meeting #{} ({} ms + {} ms)", p.meeting_id, p.duration_ms, src_ms);
                continued = Some(p);
            }
            Err(e) => eprintln!("[WARN] Could not continue meeting #{}: {e}; saving a new one", p.meeting_id),
        }
    }
    if continued.is_none() {
        if let Err(e) = std::fs::copy(src_path, &dest_path) {
            return Err(format!("Failed to preserve meeting audio to {}: {}", dest_path.display(), e));
        }
    }

    let snippets_dir = meetings_dir.join("snippets");
    let mut turn_transcriber = meeting_turn_transcriber(active_engine, whisper_arc, gguf);
    let per_turn = turn_transcriber.is_some();
    let mut turns = crate::diarization::diarize_meeting_recording_with(
        &dest_path,
        final_text,
        &snippets_dir,
        meeting_timestamp,
        turn_transcriber.as_mut().map(|t| t.as_mut() as crate::diarization::TurnTranscriber<'_>),
    );
    // With per-turn transcription the turns are the complete record (the live
    // transcript can have gaps where its queue overflowed), so save their text.
    let turns_text = turns.iter().map(|t| t.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
    let app_data = dirs::data_local_dir().ok_or("Local data directory is unavailable")?;
    let db_path = app_data.join("Taurscribe").join("transcript_history.db");
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("Could not open meeting database: {e}"))?;
    crate::commands::meetings::ensure_meetings_schema(&conn)?;

    // A continued meeting: names from the first part carry over, and without
    // per-turn text the transcripts of both parts are joined.
    let joined_text;
    let final_text: &str = match &continued {
        Some(p) => {
            crate::meeting_continuation::carry_names(
                &mut turns,
                &crate::commands::meetings::stored_turns(&conn, p.meeting_id),
                p.duration_ms,
            );
            let old_text: String = conn
                .query_row("SELECT transcript_raw FROM meetings WHERE id = ?1", [p.meeting_id], |r| r.get(0))
                .unwrap_or_default();
            joined_text = if per_turn && !turns_text.is_empty() {
                turns_text.clone()
            } else {
                format!("{} {}", old_text.trim(), final_text.trim()).trim().to_string()
            };
            &joined_text
        }
        None if per_turn && !turns_text.is_empty() => &turns_text,
        None => final_text,
    };
    let duration_ms = if let Ok(reader) = hound::WavReader::open(&dest_path) {
        let spec = reader.spec();
        let total_samples = reader.duration();
        ((total_samples as f64 / spec.sample_rate as f64) * 1000.0) as i64
    } else {
        turns.last().map(|t| t.end_ms as i64).unwrap_or(1000)
    };

    let (title, platform, app_name, url) = if let Some(info) = meeting_info {
        (
            info.title,
            info.platform,
            info.app_name,
            info.url,
        )
    } else {
        (
            format!("Meeting on {}", chrono::Local::now().format("%b %-d, %Y")),
            "Direct Audio".to_string(),
            "Taurscribe".to_string(),
            String::new(),
        )
    };

    let saved = match &continued {
        Some(p) => crate::commands::meetings::replace_meeting_content(
            &conn,
            p.meeting_id,
            duration_ms,
            Some(&dest_path.to_string_lossy()),
            final_text,
            &turns,
        )
        .map(|()| p.meeting_id),
        None => crate::commands::meetings::insert_completed_meeting(
            &conn,
            &title,
            &platform,
            &app_name,
            &url,
            duration_ms,
            Some(&dest_path.to_string_lossy()),
            final_text,
            &turns,
        ),
    };
    match saved {
        Ok(id) => {
            // Keep the full stereo recording for a while so recording this call
            // again continues this meeting (before the WAV is swapped for playback).
            if let Some(key) = call_key.as_deref() {
                crate::meeting_continuation::remember(id, key, &dest_path, duration_ms.max(0) as u64);
            }
            // The meeting points at the WAV until the compressed copy is fully
            // written and the path update succeeds.
            if let Err(e) = crate::commands::meetings::convert_meeting_audio(&conn, id, &dest_path) {
                eprintln!("[WARN] Keeping raw meeting WAV for playback: {}", e);
            }
            println!("[MEETING] Successfully persisted meeting #{}: '{}' (duration: {}ms, {} turns)", id, title, duration_ms, turns.len());
            Ok(Some(id))
        }
        Err(e) => {
            Err(format!("Could not save meeting; recording retained at {}: {}", dest_path.display(), e))
        }
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
    gguf: Option<Arc<std::sync::Mutex<crate::gguf_asr::GgufAsrManager>>>,
    vad_arc: Arc<std::sync::Mutex<crate::vad::VADManager>>,
    is_meeting: bool,
    meeting_info: Option<crate::meeting_detector::MeetingInfo>,
) -> Result<(String, Option<i64>), String> {
    // Brief tail capture for OS audio scheduling; silence padding in the
    // transcriber thread handles the actual word-boundary safety margin.
    teardown_recording(recording, 80);

    // Ensure final inference pass runs on P-cores with elevated priority on Windows
    crate::platform_tuning::apply_thread_performance_affinity();

    // Granite re-transcribes the whole saved recording: one pass over the full
    // audio beats the stitched live chunks and costs well under a second.
    if active_engine == ASREngine::Granite {
        match (gguf.as_ref(), last_recording_path.as_ref()) {
            (Some(granite), Some(path)) => {
                match load_recording_for_final_pass(path).and_then(|audio| transcribe_final_pass(granite, audio)) {
                    Ok(raw_text) => {
                        let cleaned = clean_transcript(&raw_text);
                        let (custom_vocab, _) = crate::context::load_custom_vocabulary_from_settings();
                        let final_text = crate::context::apply_custom_vocabulary_casing(&cleaned, &custom_vocab);
                        println!("[FINAL_TRANSCRIPT] (Granite final)\n{}", final_text);
                        let meeting_id = process_and_save_meeting_if_applicable(
                            is_meeting,
                            Some(path),
                            &final_text,
                            meeting_info,
                            active_engine,
                            &whisper_arc,
                            gguf.as_ref(),
                        )?;
                        let _ = std::fs::remove_file(path);
                        return Ok((final_text, meeting_id));
                    }
                    Err(e) => eprintln!("[ERROR] Granite final pass failed; using the live transcript: {}", e),
                }
            }
            _ => eprintln!("[ERROR] Granite final pass skipped: no saved recording"),
        }
    }

    // Engines that transcribe while recording (their tail is flushed into the
    // session transcript at teardown) need no final pass. The final pass below is
    // Whisper's: running it for Qwen3 failed with "Whisper context not initialized".
    if matches!(active_engine, ASREngine::Granite | ASREngine::Qwen3) {
        let engine_name = engine_label(active_engine);
        println!(
            "[PROCESSING] Skipping final pass ({} streaming is sufficient)",
            engine_name
        );
        let transcript = session_transcript.lock().unwrap().clone();
        let (custom_vocab, _) = crate::context::load_custom_vocabulary_from_settings();
        let final_text = if transcript.trim().is_empty() {
            String::new()
        } else {
            let cleaned = clean_transcript(&transcript);
            crate::context::apply_custom_vocabulary_casing(&cleaned, &custom_vocab)
        };
        println!("[FINAL_TRANSCRIPT] (Raw)\n{}", final_text);
        let meeting_id = process_and_save_meeting_if_applicable(
            is_meeting,
            last_recording_path.as_deref(),
            &final_text,
            meeting_info,
            active_engine,
            &whisper_arc,
            gguf.as_ref(),
        )?;
        if let Some(path) = last_recording_path.as_ref() {
            let _ = std::fs::remove_file(path);
        }
        return Ok((final_text, meeting_id));
    }

    if let Some(path) = last_recording_path {
        println!(
            "[PROCESSING] Running final high-quality transcription with VAD on: {}",
            path
        );

        // Build dynamic decoder prompt combining user custom vocabulary and active window context
        let (custom_vocab, context_bias_enabled) =
            crate::context::load_custom_vocabulary_from_settings();
        let prompt = crate::context::build_dynamic_prompt(&custom_vocab, context_bias_enabled);
        if let Some(ref p) = prompt {
            println!(
                "[CONTEXT] Dynamic decoder prompt ({} chars): \"{}\"",
                p.len(),
                p
            );
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
            whisper.transcribe_audio_data(&clean, prompt.as_deref())
        };

        match result {
            Ok(raw_text) => {
                println!("[FINAL_TRANSCRIPT] (Raw)\n{}", raw_text);
                let cleaned = clean_transcript(&raw_text);
                let final_text =
                    crate::context::apply_custom_vocabulary_casing(&cleaned, &custom_vocab);
                let meeting_id = process_and_save_meeting_if_applicable(
                    is_meeting,
                    Some(&path),
                    &final_text,
                    meeting_info,
                    active_engine,
                    &whisper_arc,
                    gguf.as_ref(),
                )?;
                let _ = std::fs::remove_file(&path);
                Ok((final_text, meeting_id))
            }
            Err(e) => {
                eprintln!("[ERROR] Final transcription failed: {}", e);
                let live = session_transcript.lock().unwrap().clone();
                if live.trim().is_empty() {
                    return Err(format!("Final transcription failed: {e}; recording retained at {path}"));
                }
                let fallback = clean_transcript(&live);
                let (custom_vocab, _) = crate::context::load_custom_vocabulary_from_settings();
                let fallback = crate::context::apply_custom_vocabulary_casing(&fallback, &custom_vocab);
                let meeting_id = process_and_save_meeting_if_applicable(
                    is_meeting,
                    Some(&path),
                    &fallback,
                    meeting_info,
                    active_engine,
                    &whisper_arc,
                    gguf.as_ref(),
                )?;
                if is_meeting && meeting_id.is_some() {
                    let _ = std::fs::remove_file(&path);
                }
                Ok((fallback, meeting_id))
            }
        }
    } else {
        Ok(("Recording saved.".to_string(), None))
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
pub async fn stop_recording(
    state: State<'_, AudioState>,
    app: AppHandle,
) -> Result<CommandResult<String>, String> {
    let model_operation = state.begin_model_operation()?;
    // --- Quick state access (non-blocking, just mutex snapshots) ---
    *state.denoiser.lock().unwrap() = None;
    state.recording_paused.store(false, Ordering::Relaxed);

    let Some(recording) = state.recording_handle.lock().unwrap().take() else {
        return Ok(CommandResult::err("not_recording", "Not recording"));
    };
    emit_recording_state(&app, &state);

    let active_engine = *state.active_engine.lock().unwrap();
    let session_transcript = state.session_transcript.clone();
    let last_recording_path = state.last_recording_path.lock().unwrap().clone();
    let whisper_arc = state.whisper.clone();
    let gguf = state.gguf_manager(active_engine);
    let vad_arc = state.vad.clone();

    let is_dual_channel = state.last_recording_is_dual_channel.load(Ordering::SeqCst);
    let active_meeting = state.meeting_detector.get_status().active_meetings.into_iter().next();
    // A dual-channel recording is a meeting recording even when no meeting app was
    // detected: it is saved to the meetings area (as "Direct Audio") instead of
    // being treated as dictation.
    let is_meeting = active_meeting.is_some() || is_dual_channel;

    // Transcription + diarization can take a while; let the meetings view show it.
    if is_meeting {
        let payload = match active_meeting.as_ref() {
            Some(m) => serde_json::json!({ "title": m.title, "platform": m.platform, "app_name": m.app_name }),
            None => serde_json::json!({ "title": "Direct Audio recording", "platform": "Direct Audio", "app_name": "Taurscribe" }),
        };
        let _ = app.emit("meeting-processing-started", payload);
    }

    // --- Heavy work: dispatched off the main thread via spawn_blocking so the
    //     macOS AppKit event loop stays responsive (thread joins, VAD, Whisper). ---
    let task = tauri::async_runtime::spawn_blocking(move || {
        stop_recording_blocking(
            recording,
            active_engine,
            session_transcript,
            last_recording_path,
            whisper_arc,
            gguf,
            vad_arc,
            is_meeting,
            active_meeting,
        )
    })
    .await;
    crate::meeting_continuation::release_recording_claim();
    let outcome = task.unwrap_or_else(|e| Err(format!("stop_recording task failed: {}", e)));

    state.touch_activity();
    drop(model_operation);

    // If configured to unload immediately after each transcription, free VRAM now.
    if state.auto_unload_seconds.load(Ordering::Relaxed) == 1 {
        if let Ok(unloaded) = state.unload_all_loaded_asr() {
            if !unloaded.is_empty() {
                crate::memory::trim_process_memory();
                crate::tray::reconcile_model_loaded_tray(&app, &state);
                let _ = app.emit("model-unloaded", ());
                let _ = app.emit(
                    "model-auto-unloaded",
                    serde_json::json!({
                        "timeout_seconds": 1,
                        "unloaded_engines": unloaded,
                    }),
                );
                let _ = crate::tray::update_tray_icon(&app, crate::types::AppState::Ready);
            }
        }
    }

    if is_meeting {
        // Always paired with meeting-processing-started, including when nothing was
        // saved (empty transcript) or processing failed, so the indicator clears.
        let payload = match &outcome {
            Ok((_, id)) => serde_json::json!({ "meeting_id": id, "error": null }),
            Err(message) => serde_json::json!({ "meeting_id": null, "error": message }),
        };
        let _ = app.emit("meeting-processing-finished", payload);
    }

    match outcome {
        Ok((transcript, maybe_meeting_id)) => {
            if let Some(meeting_id) = maybe_meeting_id {
                let _ = app.emit(
                    "meeting-processing-complete",
                    serde_json::json!({
                        "meeting_id": meeting_id,
                        "transcript": transcript
                    }),
                );
            }
            Ok(CommandResult::ok(transcript))
        }
        Err(message) => Ok(CommandResult::err("recording_stop_failed", message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dual_channel_tail_is_drained_before_writer_exits() {
        let (file_tx, file_rx) = crossbeam_channel::unbounded::<Vec<f32>>();
        let (whisper_tx, whisper_rx) = crossbeam_channel::unbounded::<Vec<f32>>();
        let written = Arc::new(std::sync::Mutex::new(Vec::<f32>::new()));
        let writer_output = written.clone();
        let writer_thread = std::thread::spawn(move || {
            while let Ok(frames) = file_rx.recv() {
                writer_output.lock().unwrap().extend(frames);
            }
        });
        let transcriber_thread = std::thread::spawn(move || {
            while whisper_rx.recv().is_ok() {}
        });
        let stop = Arc::new(AtomicBool::new(false));
        let capture_stop = stop.clone();
        let capture_tx = file_tx.clone();
        let capture_thread = std::thread::spawn(move || {
            while !capture_stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            capture_tx.send(vec![0.25, -0.25]).unwrap();
        });
        let level_stop = Arc::new(AtomicBool::new(false));
        let level_thread = std::thread::spawn(|| {});
        teardown_recording(RecordingHandle {
            stream: None,
            file_tx,
            whisper_tx,
            writer_thread,
            transcriber_thread,
            level_stop,
            level_thread,
            is_dual_channel: true,
            dual_channel_stop: Some(stop),
            dual_channel_thread: Some(capture_thread),
        }, 0);
        assert_eq!(*written.lock().unwrap(), vec![0.25, -0.25]);
    }

    #[test]
    fn append_transcript_with_word_overlap_removes_duplicate_boundary_words() {
        let mut transcript = "this is a final chunk boundary".to_string();

        append_transcript_with_word_overlap(
            &mut transcript,
            "chunk, boundary with punctuation handled",
        );

        assert_eq!(
            transcript,
            "this is a final chunk boundary with punctuation handled"
        );
    }

    #[test]
    fn append_transcript_with_word_overlap_keeps_non_overlapping_text() {
        let mut transcript = "first sentence".to_string();

        append_transcript_with_word_overlap(&mut transcript, "second sentence");

        assert_eq!(transcript, "first sentence second sentence");
    }

    #[test]
    fn live_chunks_are_longer_for_qwen3() {
        assert_eq!(live_chunk_samples(ASREngine::Qwen3, 16_000), 16_000 * 15);
        assert_eq!(live_chunk_samples(ASREngine::Granite, 16_000), 16_000 * 6);
        assert_eq!(live_chunk_samples(ASREngine::Whisper, 48_000), 48_000 * 6);
    }
}
