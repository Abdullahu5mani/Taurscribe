use crate::state::AudioState;
use crate::types::ASREngine;
use crate::utils::{clean_transcript, normalize_audio};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize, Deserialize)]
pub struct FileTranscriptionProgress {
    pub path: String,
    pub percent: u8,
    pub status: String, // "decoding" | "transcribing" | "done" | "error"
    pub error: Option<String>,
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
) -> Result<String, String> {
    let whisper = state.whisper.clone();
    let parakeet = state.parakeet.clone();
    let granite = state.granite_speech.clone();
    let vad = state.vad.clone();
    let active_engine = state.active_engine.lock().unwrap().clone();

    tauri::async_runtime::spawn_blocking(move || {
        transcribe_file_blocking(&app, &path, active_engine, whisper, parakeet, granite, vad)
    })
    .await
    .map_err(|e| format!("transcribe_file task failed: {}", e))?
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

fn transcribe_file_blocking(
    app: &AppHandle,
    path: &str,
    active_engine: ASREngine,
    whisper: Arc<Mutex<crate::whisper::WhisperManager>>,
    parakeet: Arc<Mutex<crate::parakeet::ParakeetManager>>,
    granite: Arc<Mutex<crate::granite_speech::GraniteSpeechManager>>,
    vad: Arc<Mutex<crate::vad::VADManager>>,
) -> Result<String, String> {
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

    emit_progress(app, path, 5, "decoding", None);

    // Decode audio file to raw f32 samples
    let (raw_samples, sample_rate, channels) = decode_audio(path)?;

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

    emit_progress(app, path, 30, "transcribing", None);

    // ── VAD: strip silence, assemble one clean speech buffer ─────────────────
    // Run VAD on a normalised probe copy so speech detection isn't biased by
    // overall recording level, then slice the *original* mono samples for the
    // actual transcription (normalization happens after assembly).
    let speech_audio = assemble_speech_audio(&mono, &vad);
    println!(
        "[FILE_TRANSCRIBE] Assembled {:.1}s of speech from {:.1}s of audio ({} silence removed)",
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
        // Whisper: single shot over the full VAD-filtered buffer - no manual chunking.
        // Whisper is designed for long-form audio and handles its own internal segmentation.
        ASREngine::Whisper => {
            let mut audio = speech_audio;
            normalize_audio(&mut audio);
            let mut w = whisper
                .lock()
                .map_err(|_| "Whisper lock poisoned".to_string())?;
            w.transcribe_audio_data(&audio, None)?
        }

        // Parakeet and Granite are streaming/chunk-based engines - feed in windows.
        ASREngine::Parakeet | ASREngine::GraniteSpeech => {
            const CHUNK_SAMPLES: usize = 16000 * 15;
            let total_chunks = (speech_audio.len() + CHUNK_SAMPLES - 1).max(1) / CHUNK_SAMPLES;
            let mut parts: Vec<String> = Vec::new();

            for (i, raw_chunk) in speech_audio.chunks(CHUNK_SAMPLES).enumerate() {
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

    emit_progress(app, path, 100, "done", None);

    Ok(final_text)
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
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u32)
        .unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Cannot create audio decoder: {}", e))?;

    let mut all_samples: Vec<f32> = Vec::new();

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

    Ok((all_samples, sample_rate, channels))
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
/// them into a single contiguous buffer ready for transcription.
///
/// Steps:
/// 1. Normalise a probe copy for reliable VAD detection.
/// 2. Run Silero with hysteresis thresholds (onset=0.40, offset=0.15) to locate speech.
/// 3. If Silero cuts less than 10% of the audio (i.e. was ineffective against background
///    noise), fall back to energy-based segmentation with 50ms frames.
/// 4. Concatenate only speech regions from the *original* (un-normalised) mono.
fn assemble_speech_audio(
    mono: &[f32],
    vad: &Arc<Mutex<crate::vad::VADManager>>,
) -> Vec<f32> {
    const SAMPLE_RATE: f32 = 16000.0;
    // Hysteresis thresholds: onset is high enough to ignore background noise,
    // offset is low enough that real speech trailing off doesn't cut the segment.
    const VAD_ONSET: f32 = 0.40;
    const VAD_OFFSET: f32 = 0.15;

    // Normalised probe - Silero works best on audio at a consistent level.
    let mut probe = mono.to_vec();
    normalize_audio(&mut probe);

    let timestamps = match vad.lock() {
        Ok(mut v) => v
            .get_speech_timestamps_hysteresis(&probe, 300, VAD_ONSET, VAD_OFFSET)
            .unwrap_or_default(),
        Err(_) => {
            println!("[FILE_TRANSCRIBE] VAD lock error - falling back to energy-based segmentation");
            Vec::new()
        }
    };

    // Assemble from Silero segments if it found something AND actually cut a meaningful
    // chunk. If Silero segments cover >90% of the audio it means the thresholds weren't
    // selective enough (e.g. constant background noise kept it active), so we fall through
    // to the energy-based approach which uses a relative per-file threshold.
    if !timestamps.is_empty() {
        let silero_samples: usize = timestamps.iter().map(|(s, e)| {
            let start = (*s * SAMPLE_RATE) as usize;
            let end = (*e * SAMPLE_RATE) as usize;
            end.saturating_sub(start)
        }).sum();

        let coverage = silero_samples as f32 / mono.len() as f32;
        if coverage <= 0.90 {
            println!(
                "[FILE_TRANSCRIBE] Silero VAD: {} segment(s), {:.1}s speech from {:.1}s total ({:.0}% coverage)",
                timestamps.len(),
                silero_samples as f32 / SAMPLE_RATE,
                mono.len() as f32 / SAMPLE_RATE,
                coverage * 100.0
            );
            let mut assembled: Vec<f32> = Vec::new();
            for (start_sec, end_sec) in &timestamps {
                let start = ((*start_sec * SAMPLE_RATE) as usize).min(mono.len());
                let end = ((*end_sec * SAMPLE_RATE) as usize).min(mono.len());
                if end > start {
                    assembled.extend_from_slice(&mono[start..end]);
                }
            }
            if !assembled.is_empty() {
                return assembled;
            }
        } else {
            println!(
                "[FILE_TRANSCRIBE] Silero VAD coverage {:.0}% - likely background noise; switching to energy segmentation",
                coverage * 100.0
            );
        }
    } else {
        println!("[FILE_TRANSCRIBE] Silero VAD found no segments - running energy-based segmentation");
    }

    // ── Energy-based segmentation fallback ───────────────────────────────────
    // Use 50ms frames (800 samples at 16kHz) for fine-grained detection.
    // Threshold is relative: 8% of the file's mean RMS, so it adapts to the
    // overall recording level instead of using a hard absolute value.
    const ENERGY_FRAME_SAMPLES: usize = 800; // 50ms at 16kHz
    // Minimum consecutive silent frames before we close a segment (~300ms)
    const MIN_SILENCE_FRAMES: usize = 6;
    // Pre/post padding kept around each active segment (2 frames = 100ms)
    const PAD_FRAMES: usize = 2;

    let mean_sq = mono.iter().map(|&x| x * x).sum::<f32>() / mono.len() as f32;
    let mean_rms = mean_sq.sqrt();
    // 8% of mean RMS; clamped between a floor (very quiet files) and a ceiling (loud noisy ones)
    let energy_threshold = (mean_rms * 0.08).max(0.0008).min(0.04);

    println!(
        "[FILE_TRANSCRIBE] Energy fallback: mean_rms={:.5}, threshold={:.5}",
        mean_rms, energy_threshold
    );

    let frames: Vec<f32> = mono
        .chunks(ENERGY_FRAME_SAMPLES)
        .map(|f| (f.iter().map(|&x| x * x).sum::<f32>() / f.len() as f32).sqrt())
        .collect();

    // Mark each frame active/silent, then group into segments with hysteresis:
    // open on first active frame, close after MIN_SILENCE_FRAMES consecutive silent frames.
    let mut segments: Vec<(usize, usize)> = Vec::new(); // in frame indices
    let mut seg_start: Option<usize> = None;
    let mut silence_run = 0usize;

    for (i, &rms) in frames.iter().enumerate() {
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
        println!("[FILE_TRANSCRIBE] Energy fallback found no active regions - using full audio");
        return mono.to_vec();
    }

    println!(
        "[FILE_TRANSCRIBE] Energy fallback: {} segment(s)",
        segments.len()
    );

    let mut assembled = Vec::new();
    for (i, (fs, fe)) in segments.iter().enumerate() {
        // Add PAD_FRAMES of context around each segment
        let sample_start = fs.saturating_sub(PAD_FRAMES) * ENERGY_FRAME_SAMPLES;
        let sample_end = ((fe + PAD_FRAMES) * ENERGY_FRAME_SAMPLES).min(mono.len());
        println!(
            "  Energy segment {}: {:.2}s - {:.2}s",
            i + 1,
            sample_start as f32 / SAMPLE_RATE,
            sample_end as f32 / SAMPLE_RATE
        );
        assembled.extend_from_slice(&mono[sample_start..sample_end]);
    }

    if assembled.is_empty() {
        println!("[FILE_TRANSCRIBE] Assembled buffer empty after energy fallback - using full audio");
        return mono.to_vec();
    }

    assembled
}
