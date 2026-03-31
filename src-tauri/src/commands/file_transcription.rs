//! File drag-and-drop transcription (Whisper / Parakeet / Cohere).
//!
//! **Speaker diarization (planned):** VAD segments are concatenated into one mono buffer
//! before ASR, so speakers cannot be labeled yet. A future pipeline should keep
//! time-aligned regions, run diarization (embeddings + clustering or a dedicated model),
//! transcribe per speaker segment, and return labels (e.g. `Speaker 1:` / timestamps) in
//! [`FileTranscriptionResult`].

use crate::audio_preprocess;
use crate::state::AudioState;
use crate::types::ASREngine;
use crate::utils::clean_transcript;
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
/// macOS: wrapped in spawn_blocking because Whisper/Parakeet/Cohere inference
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
    let cohere = state.cohere.clone();
    let active_engine = state.active_engine.lock().unwrap().clone();
    let path_for_task = path.clone();

    let join_result = tauri::async_runtime::spawn_blocking(move || {
        transcribe_file_blocking(
            &app,
            &path_for_task,
            active_engine,
            whisper,
            parakeet,
            cohere,
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
    cohere: Arc<Mutex<crate::cohere::CohereManager>>,
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
    let (raw_samples, sample_rate, channels) =
        crate::audio_decode::decode_audio_interleaved_f32(std::path::Path::new(path))?;

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
        mono = audio_preprocess::resample_mono_to_16k(&mono, sample_rate)?;
    }

    // Trim long edge silence before energy VAD.
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);

    // Capture audio duration after resampling (always 16kHz at this point).
    let audio_duration_ms = (mono.len() as f64 / 16000.0 * 1000.0) as i64;

    ensure_not_cancelled(app, path, &cancel)?;

    emit_progress(app, path, 30, "transcribing", None);

    // ── Energy VAD: only feed detected speech to ASR (file drop) ───────────────
    // Adaptive RMS thresholding finds speech regions; silent gaps are dropped.
    let mut speech_audio = crate::vad::assemble_speech_audio(&mono, Some(&cancel)).map_err(|e| {
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

    // Universal chain on speech-only buffer (HPF / RNNoise if noisy / level assist / clamp).
    audio_preprocess::preprocess_assembled_speech_16k(&mut speech_audio);

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

                let chunk = raw_chunk.to_vec();
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

        // Parakeet and Cohere are chunk-based engines - feed in engine-sized windows.
        ASREngine::Parakeet | ASREngine::Cohere => {
            const PARAKEET_CHUNK_SAMPLES: usize = 16000 * 15;
            const COHERE_CHUNK_SAMPLES: usize = 16000 * 35;
            let chunk_samples = if matches!(active_engine, ASREngine::Cohere) {
                COHERE_CHUNK_SAMPLES
            } else {
                PARAKEET_CHUNK_SAMPLES
            };
            let total_chunks = (speech_audio.len() + chunk_samples - 1).max(1) / chunk_samples;
            let mut parts: Vec<String> = Vec::new();

            for (i, raw_chunk) in speech_audio.chunks(chunk_samples).enumerate() {
                ensure_not_cancelled(app, path, &cancel)?;

                let percent = 50 + ((i as f32 / total_chunks as f32) * 45.0) as u8;
                emit_progress(app, path, percent, "transcribing", None);

                let chunk = raw_chunk.to_vec();

                let t = match active_engine {
                    ASREngine::Parakeet => {
                        let mut p = parakeet
                            .lock()
                            .map_err(|_| "Parakeet lock poisoned".to_string())?;
                        p.transcribe_chunk(&chunk, 16000)?
                    }
                    ASREngine::Cohere => {
                        let mut g = cohere
                            .lock()
                            .map_err(|_| "Cohere lock poisoned".to_string())?;
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

