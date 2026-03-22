use crate::state::AudioState;
use crate::types::ASREngine;
use crate::utils::{clean_transcript, normalize_audio};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize, Deserialize)]
pub struct FileTranscriptionProgress {
    pub path: String,
    pub percent: u8,
    pub status: String, // "decoding" | "transcribing" | "done" | "error" | "cancelled"
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct FileTranscriptionResult {
    pub transcript: String,
    /// Duration of the original audio file in milliseconds.
    pub audio_duration_ms: i64,
    /// Wall-clock time taken to transcribe in milliseconds.
    pub processing_time_ms: i64,
}

// ── Cancellation (same pattern as model downloads) ───────────────────────────

static FILE_TRANSCRIBE_CANCEL: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    OnceLock::new();

fn cancel_flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    FILE_TRANSCRIBE_CANCEL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_cancel_flag(path: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    cancel_flags()
        .lock()
        .unwrap()
        .insert(path.to_string(), Arc::clone(&flag));
    flag
}

fn unregister_cancel_flag(path: &str) {
    cancel_flags().lock().unwrap().remove(path);
}

/// Cancel in-progress file transcription for the given file path.
#[tauri::command]
pub async fn cancel_file_transcription(path: String) -> Result<(), String> {
    if let Some(flag) = cancel_flags().lock().unwrap().get(&path) {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Transcribe an audio file using the currently active ASR engine.
///
/// macOS: wrapped in spawn_blocking because Whisper/Parakeet/Granite inference
/// is synchronous and would block the AppKit main thread in Tauri 2.
#[tauri::command]
pub async fn transcribe_file(
    app: AppHandle,
    state: State<'_, AudioState>,
    path: String,
) -> Result<FileTranscriptionResult, String> {
    let cancel = register_cancel_flag(&path);
    let whisper = state.whisper.clone();
    let parakeet = state.parakeet.clone();
    let granite = state.granite_speech.clone();
    let vad = state.vad.clone();
    let active_engine = state.active_engine.lock().unwrap().clone();
    let path_for_task = path.clone();

    let join_result = tauri::async_runtime::spawn_blocking(move || {
        transcribe_file_blocking(
            &app,
            &path_for_task,
            active_engine,
            whisper,
            parakeet,
            granite,
            vad,
            cancel,
        )
    })
    .await;

    unregister_cancel_flag(&path);

    join_result
        .map_err(|e| format!("transcribe_file task failed: {}", e))
        .and_then(|r| r)
}

fn emit_progress(app: &AppHandle, path: &str, percent: u8, status: &str, error: Option<String>) {
    let _ = app.emit(
        "file-transcription-progress",
        FileTranscriptionProgress {
            path: path.to_string(),
            percent,
            status: status.to_string(),
            error,
        },
    );
}

fn ensure_not_cancelled(
    app: &AppHandle,
    path: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        emit_progress(
            app,
            path,
            0,
            "cancelled",
            Some("Cancelled by user".to_string()),
        );
        Err("Transcription cancelled".to_string())
    } else {
        Ok(())
    }
}

fn transcribe_file_blocking(
    app: &AppHandle,
    path: &str,
    active_engine: ASREngine,
    whisper: Arc<Mutex<crate::whisper::WhisperManager>>,
    parakeet: Arc<Mutex<crate::parakeet::ParakeetManager>>,
    granite: Arc<Mutex<crate::granite_speech::GraniteSpeechManager>>,
    vad: Arc<Mutex<crate::vad::VADManager>>,
    cancel: Arc<AtomicBool>,
) -> Result<FileTranscriptionResult, String> {
    let transcribe_start = std::time::Instant::now();
    // Validate extension
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let supported = ["wav", "mp3", "m4a", "aac", "flac", "ogg", "mp4", "mov"];
    if !supported.contains(&ext.as_str()) {
        return Err(format!(
            "Unsupported format: .{ext}. Supported: WAV, MP3, M4A, FLAC, OGG"
        ));
    }

    ensure_not_cancelled(app, path, &cancel)?;

    emit_progress(app, path, 5, "decoding", None);

    // Decode audio file to raw f32 samples
    let (raw_samples, sample_rate, channels) = decode_audio(path)?;

    ensure_not_cancelled(app, path, &cancel)?;

    emit_progress(app, path, 20, "decoding", None);

    // Merge to mono
    let mut mono = if channels > 1 {
        let ch = channels as usize;
        raw_samples
            .chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect::<Vec<f32>>()
    } else {
        raw_samples
    };

    // Resample to 16 kHz (all engines require this)
    if sample_rate != 16000 {
        mono = resample_to_16k(mono, sample_rate)?;
    }

    // Capture audio duration after resampling (always 16kHz at this point).
    let audio_duration_ms = (mono.len() as f64 / 16000.0 * 1000.0) as i64;

    ensure_not_cancelled(app, path, &cancel)?;

    emit_progress(app, path, 30, "transcribing", None);

    // ── VAD: only feed speech to Whisper / Granite (file drop) ────────────────
    // Silero + hysteresis detect where human speech starts/stops; silent regions
    // are never passed to the ASR — only concatenated speech segments from the
    // original mono buffer. No "send whole file" fallback.
    let speech_audio = assemble_speech_audio(&mono, &vad, Some(&cancel)).map_err(|e| {
        if e == "Transcription cancelled" {
            emit_progress(
                app,
                path,
                0,
                "cancelled",
                Some("Cancelled by user".to_string()),
            );
        }
        e
    })?;

    if speech_audio.is_empty() {
        println!(
            "[FILE_TRANSCRIBE] No speech detected after VAD — skipping ASR ({}s audio)",
            mono.len() as f32 / 16000.0
        );
        emit_progress(app, path, 100, "done", None);
        return Ok(FileTranscriptionResult {
            transcript: String::new(),
            audio_duration_ms,
            processing_time_ms: transcribe_start.elapsed().as_millis() as i64,
        });
    }

    println!(
        "[FILE_TRANSCRIBE] Assembled {:.1}s of speech from {:.1}s of audio ({} silence dropped)",
        speech_audio.len() as f32 / 16000.0,
        mono.len() as f32 / 16000.0,
        if mono.len() > speech_audio.len() {
            format!("{:.1}s", (mono.len() - speech_audio.len()) as f32 / 16000.0)
        } else {
            "none".to_string()
        }
    );

    emit_progress(app, path, 50, "transcribing", None);

    let text = match active_engine {
        // Whisper: chunked so the user can cancel between segments (long files).
        ASREngine::Whisper => {
            const WHISPER_CHUNK_SAMPLES: usize = 16000 * 180; // 3 minutes
            let total_w = (speech_audio.len() + WHISPER_CHUNK_SAMPLES - 1).max(1)
                / WHISPER_CHUNK_SAMPLES;
            let mut parts: Vec<String> = Vec::new();

            for (i, raw_chunk) in speech_audio.chunks(WHISPER_CHUNK_SAMPLES).enumerate() {
                ensure_not_cancelled(app, path, &cancel)?;

                let percent = 50 + ((i as f32 / total_w as f32) * 45.0) as u8;
                emit_progress(app, path, percent, "transcribing", None);

                let mut chunk = raw_chunk.to_vec();
                normalize_audio(&mut chunk);
                let mut w = whisper
                    .lock()
                    .map_err(|_| "Whisper lock poisoned".to_string())?;
                let t = w.transcribe_audio_data(&chunk, None)?;
                if !t.trim().is_empty() {
                    parts.push(t.trim().to_string());
                }
            }

            parts.join(" ")
        }

        // Parakeet and Granite are streaming/chunk-based engines - feed in windows.
        ASREngine::Parakeet | ASREngine::GraniteSpeech => {
            const CHUNK_SAMPLES: usize = 16000 * 15;
            let total_chunks = (speech_audio.len() + CHUNK_SAMPLES - 1).max(1) / CHUNK_SAMPLES;
            let mut parts: Vec<String> = Vec::new();

            for (i, raw_chunk) in speech_audio.chunks(CHUNK_SAMPLES).enumerate() {
                ensure_not_cancelled(app, path, &cancel)?;

                let percent = 50 + ((i as f32 / total_chunks as f32) * 45.0) as u8;
                emit_progress(app, path, percent, "transcribing", None);

                let mut chunk = raw_chunk.to_vec();
                normalize_audio(&mut chunk);

                let t = match active_engine {
                    ASREngine::Parakeet => {
                        let mut p = parakeet
                            .lock()
                            .map_err(|_| "Parakeet lock poisoned".to_string())?;
                        p.transcribe_chunk(&chunk, 16000)?
                    }
                    ASREngine::GraniteSpeech => {
                        let mut g = granite
                            .lock()
                            .map_err(|_| "Granite lock poisoned".to_string())?;
                        g.transcribe_chunk(&chunk, 16000)?
                    }
                    _ => unreachable!(),
                };

                if !t.trim().is_empty() {
                    parts.push(t.trim().to_string());
                }
            }

            parts.join(" ")
        }
    };

    let final_text = clean_transcript(&text);
    let processing_time_ms = transcribe_start.elapsed().as_millis() as i64;

    emit_progress(app, path, 100, "done", None);

    Ok(FileTranscriptionResult {
        transcript: final_text,
        audio_duration_ms,
        processing_time_ms,
    })
}

/// Decode an audio file to interleaved f32 samples using symphonia.
/// Returns (samples, sample_rate, channel_count).
fn decode_audio(path: &str) -> Result<(Vec<f32>, u32, u32), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error as SymphError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file =
        std::fs::File::open(path).map_err(|e| format!("Cannot open file: {}", e))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
    {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Cannot probe audio format: {}", e))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("No audio track found in file")?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("File has unknown sample rate")?;
    // Prefer codec_params channel count; will be corrected from the first decoded
    // packet's spec if codec_params reports None (e.g. some MP3/M4A containers).
    let hint_channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u32)
        .unwrap_or(0); // 0 = unknown; resolved below

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Cannot create audio decoder: {}", e))?;

    let mut all_samples: Vec<f32> = Vec::new();
    let mut actual_channels: u32 = hint_channels;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                // Resolve channel count from the first decoded frame if not known.
                if actual_channels == 0 {
                    actual_channels = spec.channels.count() as u32;
                }
                let capacity = decoded.capacity() as u64;
                if capacity == 0 {
                    continue;
                }
                let mut buf = SampleBuffer::<f32>::new(capacity, spec);
                buf.copy_interleaved_ref(decoded);
                all_samples.extend_from_slice(buf.samples());
            }
            Err(SymphError::IoError(_)) => continue,
            Err(SymphError::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    if all_samples.is_empty() {
        return Err("Audio file is empty or could not be decoded".to_string());
    }

    // Final fallback: if we still couldn't determine channels, assume mono.
    if actual_channels == 0 {
        actual_channels = 1;
    }

    Ok((all_samples, sample_rate, actual_channels))
}

/// Resample mono audio from `from_rate` to 16 kHz using the same rubato
/// SincFixedIn parameters used throughout the rest of the app.
fn resample_to_16k(samples: Vec<f32>, from_rate: u32) -> Result<Vec<f32>, String> {
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    let params = SincInterpolationParameters {
        sinc_len: 64,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        window: WindowFunction::BlackmanHarris2,
        oversampling_factor: 32,
    };

    const CHUNK: usize = 1024 * 10;

    let mut resampler = SincFixedIn::<f32>::new(
        16000.0 / from_rate as f64,
        2.0,
        params,
        CHUNK,
        1,
    )
    .map_err(|e| format!("Resampler init failed: {:?}", e))?;

    let pad = samples.len() % CHUNK;
    let mut padded = samples;
    if pad > 0 {
        padded.extend(std::iter::repeat(0.0_f32).take(CHUNK - pad));
    }

    let mut resampled = Vec::new();
    for chunk in padded.chunks(CHUNK) {
        let waves_in = vec![chunk.to_vec()];
        if let Ok(waves_out) = resampler.process(&waves_in, None) {
            resampled.extend_from_slice(&waves_out[0]);
        }
    }

    Ok(resampled)
}

/// Run VAD on the full audio, collect speech-only segments, and concatenate
/// them into a single buffer for the ASR. **Silent sections are omitted** — they
/// are never passed to Whisper/Granite.
///
/// Steps:
/// 1. Normalise a probe copy for reliable VAD (level-independent detection).
/// 2. Silero hysteresis: segment **starts** when speech prob rises above `ONSET`,
///    **ends** when prob stays below `OFFSET` for `PADDING_MS` (speech boundaries).
/// 3. If Silero is too permissive (coverage >90%), fall back to energy segmentation.
/// 4. Energy fallback: same rule — only active frames become segments; **never**
///    return the full file when no speech is found (empty vec instead).
/// 5. Slices come from the *original* mono (not the probe).
fn assemble_speech_audio(
    mono: &[f32],
    vad: &Arc<Mutex<crate::vad::VADManager>>,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Vec<f32>, String> {
    const SAMPLE_RATE: f32 = 16000.0;
    /// How long prob must stay below `offset` before closing a segment (ms).
    const VAD_HANGOVER_MS: usize = 300;
    /// Multi-pass Silero: a single strict onset (e.g. 0.42) often yields **0 segments**
    /// on real files because speech probability sits in ~0.25–0.40. We try progressively
    /// more permissive (onset, offset) pairs before energy fallback.
    const SILERO_PASSES: &[(f32, f32)] = &[
        (0.35, 0.15),
        (0.28, 0.12),
        (0.22, 0.10),
    ];

    if let Some(c) = cancel {
        if c.load(Ordering::Relaxed) {
            return Err("Transcription cancelled".to_string());
        }
    }

    // Feed raw (un-normalized) audio to Silero VAD.
    // normalize_audio() boosts the entire file including silent sections, which raises
    // the noise floor and makes silence look like speech to Silero. Silero is a neural
    // network trained on real speech at natural levels — it does not need normalization.

    let mut timestamps: Vec<(f32, f32)> = Vec::new();
    match vad.lock() {
        Ok(mut v) => {
            for (pi, &(onset, offset)) in SILERO_PASSES.iter().enumerate() {
                let ts = v
                    .get_speech_timestamps_hysteresis(mono, VAD_HANGOVER_MS, onset, offset)
                    .unwrap_or_default();
                if ts.is_empty() {
                    println!(
                        "[FILE_TRANSCRIBE] Silero pass {} (onset={:.2}, offset={:.2}): 0 segments",
                        pi + 1,
                        onset,
                        offset
                    );
                    continue;
                }

                let silero_samples: usize = ts.iter().map(|(s, e)| {
                    let start = (*s * SAMPLE_RATE) as usize;
                    let end = (*e * SAMPLE_RATE) as usize;
                    end.saturating_sub(start)
                }).sum();

                let coverage = silero_samples as f32 / mono.len().max(1) as f32;
                if coverage <= 0.90 {
                    println!(
                        "[FILE_TRANSCRIBE] Silero VAD (pass {} onset={:.2} offset={:.2}): {} segment(s), {:.1}s speech / {:.1}s total ({:.0}% coverage)",
                        pi + 1,
                        onset,
                        offset,
                        ts.len(),
                        silero_samples as f32 / SAMPLE_RATE,
                        mono.len() as f32 / SAMPLE_RATE,
                        coverage * 100.0
                    );
                    timestamps = ts;
                    break;
                }

                println!(
                    "[FILE_TRANSCRIBE] Silero pass {} coverage {:.0}% (too uniform) — trying next pass / energy",
                    pi + 1,
                    coverage * 100.0
                );
            }
        }
        Err(_) => {
            println!("[FILE_TRANSCRIBE] VAD lock error - falling back to energy-based segmentation");
        }
    }

    // Assemble from Silero — we only store `timestamps` when coverage ≤ 90%.
    if !timestamps.is_empty() {
        let mut assembled: Vec<f32> = Vec::new();
        for (start_sec, end_sec) in &timestamps {
            let start = ((*start_sec * SAMPLE_RATE) as usize).min(mono.len());
            let end = ((*end_sec * SAMPLE_RATE) as usize).min(mono.len());
            if end > start {
                assembled.extend_from_slice(&mono[start..end]);
            }
        }
        if !assembled.is_empty() {
            return Ok(assembled);
        }
    } else {
        println!("[FILE_TRANSCRIBE] Silero had no acceptable split — energy-based segmentation");
    }

    // ── Energy-based segmentation fallback ───────────────────────────────────
    // Per-frame RMS vs **noise floor** (low percentiles), not global mean RMS.
    // Long files with loud speech + long quiet stretches used to set mean RMS high while
    // silence frames still sat above (mean*8%) — one giant "speech" segment. Using p10–p20
    // of frame energies estimates room noise; speech must clear that by a margin.
    const ENERGY_FRAME_SAMPLES: usize = 800; // 50ms at 16kHz
    // ~800 ms of consecutive silence before closing a segment (16 × 50ms).
    // Increased from 8 to 16: natural inter-sentence pauses can be 500–700ms and should
    // NOT split a sentence into two segments.
    const MIN_SILENCE_FRAMES: usize = 16;
    // Pre/post padding kept around each active segment (2 frames = 100ms)
    const PAD_FRAMES: usize = 2;

    let frames: Vec<f32> = mono
        .chunks(ENERGY_FRAME_SAMPLES)
        .map(|f| (f.iter().map(|&x| x * x).sum::<f32>() / f.len() as f32).sqrt())
        .collect();

    if frames.is_empty() {
        return Ok(Vec::new());
    }

    let mut sorted = frames.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let p10 = sorted[(n * 10 / 100).min(n.saturating_sub(1))];
    let p25 = sorted[(n * 25 / 100).min(n.saturating_sub(1))];
    let p50 = sorted[(n * 50 / 100).min(n.saturating_sub(1))];
    let p75 = sorted[(n * 75 / 100).min(n.saturating_sub(1))];

    // Noise floor ≈ quietest 10–25% of frames (room tone / true silence).
    // Threshold must sit clearly above the noise floor but below speech.
    // Old formula (p50 * 0.18) produced ~8% of mean RMS — far too low, nearly every
    // frame was "speech". New formula aims for ~50-60% of median to properly split
    // silence (below p50) from speech (above p50).
    let noise_floor = p10.max(p25 * 0.85);
    let energy_threshold = (noise_floor * 20.0) // Well above room noise
        .max(p50 * 0.55)   // ~55% of median: silence sits below, speech above
        .max(p25 * 2.0)    // At least 2× the 25th-percentile frame energy
        .max(0.005_f32)    // Absolute floor (was 0.0014, far too low for normalized audio)
        .min(0.08_f32);

    println!(
        "[FILE_TRANSCRIBE] Energy fallback: frames={}, p10={:.5} p25={:.5} p50={:.5} p75={:.5} noise_floor={:.5} threshold={:.5}",
        frames.len(),
        p10,
        p25,
        p50,
        p75,
        noise_floor,
        energy_threshold
    );

    // Mark each frame active/silent, then group into segments with hysteresis:
    // open on first active frame, close after MIN_SILENCE_FRAMES consecutive silent frames.
    let mut segments: Vec<(usize, usize)> = Vec::new(); // in frame indices
    let mut seg_start: Option<usize> = None;
    let mut silence_run = 0usize;

    for (i, &rms) in frames.iter().enumerate() {
        if i % 2048 == 0 {
            if let Some(c) = cancel {
                if c.load(Ordering::Relaxed) {
                    return Err("Transcription cancelled".to_string());
                }
            }
        }
        if rms >= energy_threshold {
            if seg_start.is_none() {
                seg_start = Some(i);
            }
            silence_run = 0;
        } else if let Some(start) = seg_start {
            silence_run += 1;
            if silence_run >= MIN_SILENCE_FRAMES {
                let seg_end = i - silence_run + 1; // exclusive frame index at silence start
                segments.push((start, seg_end));
                seg_start = None;
                silence_run = 0;
            }
        }
    }
    if let Some(start) = seg_start {
        segments.push((start, frames.len()));
    }

    if segments.is_empty() {
        println!(
            "[FILE_TRANSCRIBE] Energy fallback found no active speech — not sending silence to ASR"
        );
        return Ok(Vec::new());
    }

    println!(
        "[FILE_TRANSCRIBE] Energy fallback: {} segment(s)",
        segments.len()
    );

    let mut assembled = Vec::new();
    let nseg = segments.len();
    const LOG_EACH: usize = 8;
    for (i, (fs, fe)) in segments.iter().enumerate() {
        if i % 32 == 0 {
            if let Some(c) = cancel {
                if c.load(Ordering::Relaxed) {
                    return Err("Transcription cancelled".to_string());
                }
            }
        }
        // Add PAD_FRAMES of context around each segment
        let sample_start = fs.saturating_sub(PAD_FRAMES) * ENERGY_FRAME_SAMPLES;
        let sample_end = ((fe + PAD_FRAMES) * ENERGY_FRAME_SAMPLES).min(mono.len());
        let log_line = nseg <= LOG_EACH
            || i < LOG_EACH / 2
            || i >= nseg.saturating_sub(LOG_EACH / 2);
        if log_line {
            println!(
                "  Energy segment {}: {:.2}s - {:.2}s",
                i + 1,
                sample_start as f32 / SAMPLE_RATE,
                sample_end as f32 / SAMPLE_RATE
            );
        } else if i == LOG_EACH / 2 {
            println!("  Energy segment ... ({} segments omitted) ...", nseg.saturating_sub(LOG_EACH));
        }
        assembled.extend_from_slice(&mono[sample_start..sample_end]);
    }

    if assembled.is_empty() {
        println!("[FILE_TRANSCRIBE] Assembled buffer empty after energy fallback");
        return Ok(Vec::new());
    }

    Ok(assembled)
}
