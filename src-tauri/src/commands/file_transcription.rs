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
    let active_engine = state.active_engine.lock().unwrap().clone();

    tauri::async_runtime::spawn_blocking(move || {
        transcribe_file_blocking(&app, &path, active_engine, whisper, parakeet, granite)
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

    // Normalize to -20 dBFS target
    normalize_audio(&mut mono);

    emit_progress(app, path, 30, "transcribing", None);

    // Transcribe in 30-second chunks so we can emit progress and handle long files
    const CHUNK_SAMPLES: usize = 16000 * 30;
    let total_chunks = (mono.len() + CHUNK_SAMPLES - 1).max(1) / CHUNK_SAMPLES;
    let mut parts: Vec<String> = Vec::new();

    for (i, chunk) in mono.chunks(CHUNK_SAMPLES).enumerate() {
        // Progress: 30% → 95% across all chunks
        let percent = 30 + ((i as f32 / total_chunks as f32) * 65.0) as u8;
        emit_progress(app, path, percent, "transcribing", None);

        let text = match active_engine {
            ASREngine::Whisper => {
                let mut w = whisper
                    .lock()
                    .map_err(|_| "Whisper lock poisoned".to_string())?;
                w.transcribe_audio_data(chunk, None)?
            }
            ASREngine::Parakeet => {
                let mut p = parakeet
                    .lock()
                    .map_err(|_| "Parakeet lock poisoned".to_string())?;
                p.transcribe_chunk(chunk, 16000)?
            }
            ASREngine::GraniteSpeech => {
                let mut g = granite
                    .lock()
                    .map_err(|_| "Granite lock poisoned".to_string())?;
                g.transcribe_chunk(chunk, 16000)?
            }
        };

        if !text.trim().is_empty() {
            parts.push(text.trim().to_string());
        }
    }

    let raw = parts.join(" ");
    let final_text = clean_transcript(&raw);

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
