//! File drag-and-drop transcription (Whisper / Granite 5 / Qwen3).
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
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize, Deserialize)]
pub struct FileTranscriptionProgress {
    pub path: String,
    pub job_id: Option<String>,
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

struct FileJobState {
    flag: Arc<AtomicBool>,
    job_id: Option<String>,
}

static FILE_TRANSCRIBE_CANCEL: OnceLock<Mutex<HashMap<String, FileJobState>>> = OnceLock::new();

fn cancel_flags() -> &'static Mutex<HashMap<String, FileJobState>> {
    FILE_TRANSCRIBE_CANCEL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_cancel_flag(path: &str, job_id: Option<String>) -> Result<Arc<AtomicBool>, String> {
    let flag = Arc::new(AtomicBool::new(false));
    let mut jobs = cancel_flags().lock().unwrap();
    if jobs.contains_key(path) {
        return Err("This file is already being transcribed".into());
    }
    jobs.insert(
        path.to_string(),
        FileJobState {
            flag: Arc::clone(&flag),
            job_id,
        },
    );
    Ok(flag)
}

fn unregister_cancel_flag(path: &str) {
    cancel_flags().lock().unwrap().remove(path);
}

/// Cancel in-progress file transcription for the given file path.
#[tauri::command]
pub async fn cancel_file_transcription(path: String) -> Result<(), String> {
    if let Some(job) = cancel_flags().lock().unwrap().get(&path) {
        job.flag.store(true, Ordering::Relaxed);
        println!("[FILE_TRANSCRIBE] Cancel requested for {}", path);
    }
    Ok(())
}

/// Transcribe an audio file using the currently active ASR engine.
///
/// macOS: wrapped in spawn_blocking because speech inference
/// is synchronous and would block the AppKit main thread in Tauri 2.
#[tauri::command]
pub async fn transcribe_file(
    app: AppHandle,
    state: State<'_, AudioState>,
    path: String,
    expected_engine: Option<String>,
    expected_model_id: Option<String>,
    job_id: Option<String>,
    defer_auto_unload: Option<bool>,
) -> Result<FileTranscriptionResult, String> {
    let model_operation = state.begin_model_operation()?;
    if state.engine_loading.load(Ordering::Acquire) {
        return Err(
            "ASR model is still loading; wait until the engine is ready before transcribing a file"
                .to_string(),
        );
    }

    let cancel = register_cancel_flag(&path, job_id)?;
    let whisper = state.whisper.clone();
    let granite = state.granite.clone();
    let qwen3 = state.qwen3.clone();
    let active_engine = state.active_engine.lock().unwrap().clone();
    let actual_engine = match active_engine {
        ASREngine::Whisper => "whisper",
        ASREngine::Granite => "granite",
        ASREngine::Qwen3 => "qwen3",
    };
    if expected_engine
        .as_deref()
        .is_some_and(|expected| expected != actual_engine)
    {
        unregister_cancel_flag(&path);
        return Err("The active engine changed after this file was queued. Re-run it with the desired model.".into());
    }
    let loaded_model = match active_engine {
        ASREngine::Whisper => state.whisper.lock().unwrap().get_current_model().cloned(),
        ASREngine::Granite => state.granite.lock().unwrap().get_status().model_id,
        ASREngine::Qwen3 => state.qwen3.lock().unwrap().get_status().model_id,
    };
    if expected_model_id
        .as_deref()
        .is_some_and(|expected| Some(expected) != loaded_model.as_deref())
    {
        unregister_cancel_flag(&path);
        return Err("The loaded model changed after this file was queued. Re-run it with the desired model.".into());
    }
    let path_for_task = path.clone();
    let app_for_task = app.clone();

    let res = tauri::async_runtime::spawn_blocking(move || {
        transcribe_file_blocking(
            &app_for_task,
            &path_for_task,
            active_engine,
            whisper,
            granite,
            qwen3,
            cancel,
        )
    })
    .await
    .map_err(|e| format!("transcribe_file task failed: {}", e))
    .and_then(|r| r);

    unregister_cancel_flag(&path);

    state.touch_activity();
    drop(model_operation);

    if !defer_auto_unload.unwrap_or(false) {
        maybe_unload_after_file_batch(&app, &state);
    }

    res
}

fn maybe_unload_after_file_batch(app: &AppHandle, state: &AudioState) {
    if state.auto_unload_seconds.load(Ordering::Relaxed) != 1 {
        return;
    }
    if let Ok(unloaded) = state.unload_all_loaded_asr() {
        if !unloaded.is_empty() {
            crate::memory::trim_process_memory();
            crate::tray::reconcile_model_loaded_tray(app, state);
            let _ = app.emit("model-unloaded", ());
            let _ = app.emit(
                "model-auto-unloaded",
                serde_json::json!({
                    "timeout_seconds": 1,
                    "unloaded_engines": unloaded,
                }),
            );
            let _ = crate::tray::update_tray_icon(app, crate::types::AppState::Ready);
        }
    }
}

#[tauri::command]
pub async fn finish_file_transcription_batch(
    app: AppHandle,
    state: State<'_, AudioState>,
) -> Result<(), String> {
    let state = (*state).clone();
    tauri::async_runtime::spawn_blocking(move || maybe_unload_after_file_batch(&app, &state))
        .await
        .map_err(|e| format!("finish_file_transcription_batch task failed: {e}"))
}

fn emit_progress(app: &AppHandle, path: &str, percent: u8, status: &str, error: Option<String>) {
    let job_id = cancel_flags()
        .lock()
        .unwrap()
        .get(path)
        .and_then(|job| job.job_id.clone());
    let _ = app.emit(
        "file-transcription-progress",
        FileTranscriptionProgress {
            path: path.to_string(),
            job_id,
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
    granite: Arc<Mutex<crate::gguf_asr::GgufAsrManager>>,
    qwen3: Arc<Mutex<crate::gguf_asr::GgufAsrManager>>,
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

    // Decode and transcribe bounded source windows. Keeping a whole imported
    // file (and then a second resampled/speech copy) can exhaust memory.
    let mut parts = Vec::<String>::new();
    let (decoded_frames, sample_rate) = stream_file_windows(
        Path::new(path),
        16_000 * 120,
        || ensure_not_cancelled(app, path, &cancel),
        |window, decoded_so_far, expected_frames| {
            ensure_not_cancelled(app, path, &cancel)?;
            let text = transcribe_file_window(
                app,
                path,
                active_engine,
                &whisper,
                &granite,
                &qwen3,
                &cancel,
                window,
            )?;
            if !text.trim().is_empty() {
                parts.push(text);
            }
            if let Some(total) = expected_frames.filter(|total| *total > 0) {
                let percent = (53.0 + decoded_so_far as f64 / total as f64 * 42.0).min(95.0) as u8;
                emit_progress(app, path, percent, "transcribing", None);
            }
            Ok(())
        },
    )?;
    if sample_rate == 0 {
        return Err("File has invalid sample rate".into());
    }
    ensure_not_cancelled(app, path, &cancel)?;
    let audio_duration_ms = (decoded_frames as f64 / sample_rate as f64 * 1000.0) as i64;
    let text = parts.join(" ");

    let (custom_vocab, _) = crate::context::load_custom_vocabulary_from_settings();
    let cleaned = clean_transcript(&text);
    let final_text = crate::context::apply_custom_vocabulary_casing(&cleaned, &custom_vocab);
    let processing_time_ms = transcribe_start.elapsed().as_millis() as i64;

    emit_progress(app, path, 100, "done", None);

    Ok(FileTranscriptionResult {
        transcript: final_text,
        audio_duration_ms,
        processing_time_ms,
    })
}

/// Keep only one resampled ASR window while decoding, including a final short
/// window. The callback receives source-frame progress for the UI.
fn stream_file_windows<F, C>(
    path: &Path,
    window_samples: usize,
    mut check_cancelled: C,
    mut on_window: F,
) -> Result<(u64, u32), String>
where
    F: FnMut(Vec<f32>, u64, Option<u64>) -> Result<(), String>,
    C: FnMut() -> Result<(), String>,
{
    if window_samples == 0 {
        return Err("Audio window must not be empty".into());
    }
    let mut pending = Vec::<f32>::new();
    let mut resampler: Option<audio_preprocess::StreamingResampler16k> = None;
    let (frames, rate) = crate::audio_decode::decode_audio_mono_stream(
        path,
        |samples, source_rate, decoded_so_far, expected_frames| {
            check_cancelled()?;
            if resampler.is_none() {
                resampler = Some(audio_preprocess::StreamingResampler16k::new(source_rate)?);
            }
            pending.extend_from_slice(&resampler.as_mut().unwrap().push(samples)?);
            while pending.len() >= window_samples {
                check_cancelled()?;
                on_window(
                    pending.drain(..window_samples).collect(),
                    decoded_so_far,
                    expected_frames,
                )?;
            }
            Ok(())
        },
    )?;
    if let Some(mut resampler) = resampler {
        pending.extend_from_slice(&resampler.finish()?);
    }
    if !pending.is_empty() {
        check_cancelled()?;
        on_window(pending, frames, Some(frames))?;
    }
    Ok((frames, rate))
}

#[cfg(test)]
mod vm_feature_tests {
    use super::*;

    fn stereo_wav() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "taurscribe-vm-file-import-{}.wav",
            rand::random::<u64>()
        ));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..156_000 {
            writer.write_sample::<i16>((i % 500) as i16).unwrap();
            writer.write_sample::<i16>((i % 300) as i16).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    #[test]
    fn vm_file_import_streams_bounded_windows_with_final_tail() {
        let path = stereo_wav();
        let mut sizes = Vec::new();
        let mut progress = Vec::new();
        let (frames, rate) = stream_file_windows(
            &path,
            16_000,
            || Ok(()),
            |window, decoded, expected| {
                assert!(window.iter().all(|sample| sample.is_finite()));
                sizes.push(window.len());
                progress.push(decoded);
                assert_eq!(expected, Some(156_000));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!((frames, rate), (156_000, 48_000));
        assert_eq!(sizes, [16_000, 16_000, 16_000, 4_000]);
        assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(progress.last(), Some(&frames));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn vm_file_import_cancellation_stops_before_another_window() {
        let path = stereo_wav();
        let cancelled = AtomicBool::new(false);
        let mut windows = 0;
        let result = stream_file_windows(
            &path,
            16_000,
            || {
                if cancelled.load(Ordering::Relaxed) {
                    Err("Transcription cancelled".into())
                } else {
                    Ok(())
                }
            },
            |_, _, _| {
                windows += 1;
                cancelled.store(true, Ordering::Relaxed);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err(), "Transcription cancelled");
        assert_eq!(windows, 1);
        std::fs::remove_file(path).unwrap();
    }
}

fn transcribe_file_window(
    app: &AppHandle,
    path: &str,
    active_engine: ASREngine,
    whisper: &Arc<Mutex<crate::whisper::WhisperManager>>,
    granite: &Arc<Mutex<crate::gguf_asr::GgufAsrManager>>,
    qwen3: &Arc<Mutex<crate::gguf_asr::GgufAsrManager>>,
    cancel: &Arc<AtomicBool>,
    mut mono: Vec<f32>,
) -> Result<String, String> {
    emit_progress(app, path, 20, "decoding", None);
    // Trim long edge silence before energy VAD.
    audio_preprocess::trim_file_buffer_edges_16k(&mut mono);

    ensure_not_cancelled(app, path, &cancel)?;

    emit_progress(app, path, 30, "transcribing", None);

    // ── Energy VAD: only feed detected speech to ASR (file drop) ───────────────
    // Adaptive RMS thresholding finds speech regions; silent gaps are dropped.
    let mono_samples = mono.len();
    let mut speech_audio =
        crate::vad::assemble_speech_audio(&mono, Some(&cancel)).map_err(|e| {
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
    drop(mono);

    // Universal chain on speech-only buffer (HPF / RNNoise if noisy / level assist / clamp).
    audio_preprocess::preprocess_assembled_speech_16k(&mut speech_audio);

    if speech_audio.is_empty() {
        println!(
            "[FILE_TRANSCRIBE] No speech detected after VAD — skipping ASR ({}s audio)",
            mono_samples as f32 / 16000.0
        );
        return Ok(String::new());
    }

    println!(
        "[FILE_TRANSCRIBE] Assembled {:.1}s of speech from {:.1}s of audio ({} silence dropped)",
        speech_audio.len() as f32 / 16000.0,
        mono_samples as f32 / 16000.0,
        if mono_samples > speech_audio.len() {
            format!(
                "{:.1}s",
                (mono_samples - speech_audio.len()) as f32 / 16000.0
            )
        } else {
            "none".to_string()
        }
    );

    emit_progress(app, path, 50, "transcribing", None);

    let (custom_vocab, context_bias_enabled) =
        crate::context::load_custom_vocabulary_from_settings();
    emit_progress(app, path, 51, "transcribing", None);
    // File jobs run on a worker thread. On macOS, active-window context uses
    // the Accessibility API and must not be queried from this thread. Custom
    // vocabulary remains safe and still provides the intended decoder bias.
    let dynamic_prompt = crate::context::build_dynamic_prompt(&custom_vocab, false);
    if context_bias_enabled {
        println!("[FILE_TRANSCRIBE] Active-window context bias skipped for worker-thread file transcription");
    }
    emit_progress(app, path, 52, "transcribing", None);

    let text = match active_engine {
        // Whisper: chunked so the user can cancel between segments (long files).
        ASREngine::Whisper => {
            const WHISPER_CHUNK_SAMPLES: usize = 16000 * 180; // 3 minutes
            let total_w =
                (speech_audio.len() + WHISPER_CHUNK_SAMPLES - 1).max(1) / WHISPER_CHUNK_SAMPLES;
            let mut parts: Vec<String> = Vec::new();

            for (i, raw_chunk) in speech_audio.chunks(WHISPER_CHUNK_SAMPLES).enumerate() {
                ensure_not_cancelled(app, path, &cancel)?;

                let percent = 50 + ((i as f32 / total_w as f32) * 45.0) as u8;
                emit_progress(app, path, percent, "transcribing", None);

                crate::memory::maybe_log_process_memory_with_sizes(
                    "file transcription whisper chunk",
                    &[
                        ("chunk_index", i + 1),
                        ("total_chunks", total_w),
                        ("chunk_samples", raw_chunk.len()),
                        (
                            "chunk_audio_bytes",
                            raw_chunk.len() * std::mem::size_of::<f32>(),
                        ),
                    ],
                );
                let mut w = whisper.try_lock().map_err(|_| {
                    "Whisper engine is busy loading or processing another request".to_string()
                })?;
                if !w.is_loaded() {
                    emit_progress(app, path, 45, "loading model", None);
                    w.initialize(None, false)?;
                }
                emit_progress(app, path, 53, "transcribing", None);
                let t = w.transcribe_audio_data(raw_chunk, dynamic_prompt.as_deref())?;
                if !t.trim().is_empty() {
                    parts.push(t.trim().to_string());
                }
            }

            parts.join(" ")
        }

        // Chunk-based engines use bounded windows so cancellation stays responsive.
        ASREngine::Granite | ASREngine::Qwen3 => {
            // transcribe.cpp checks a cancel only between runs (Qwen3) or between
            // its internal windows (Granite, much faster), so these windows bound
            // how long a cancel can take.
            const GRANITE_CHUNK_SAMPLES: usize = 16000 * 60;
            const QWEN3_CHUNK_SAMPLES: usize = 16000 * 30;
            let chunk_samples = if matches!(active_engine, ASREngine::Qwen3) {
                QWEN3_CHUNK_SAMPLES
            } else {
                GRANITE_CHUNK_SAMPLES
            };
            let total_chunks = (speech_audio.len() + chunk_samples - 1).max(1) / chunk_samples;
            let mut parts: Vec<String> = Vec::new();

            for (i, raw_chunk) in speech_audio.chunks(chunk_samples).enumerate() {
                ensure_not_cancelled(app, path, &cancel)?;

                let percent = 50 + ((i as f32 / total_chunks as f32) * 45.0) as u8;
                emit_progress(app, path, percent, "transcribing", None);

                crate::memory::maybe_log_process_memory_with_sizes(
                    "file transcription chunk",
                    &[
                        ("chunk_index", i + 1),
                        ("total_chunks", total_chunks),
                        ("chunk_samples", raw_chunk.len()),
                        (
                            "chunk_audio_bytes",
                            raw_chunk.len() * std::mem::size_of::<f32>(),
                        ),
                    ],
                );

                let t = match active_engine {
                    ASREngine::Granite => {
                        let mut g = granite.try_lock().map_err(|_| {
                            "Granite engine is busy loading or processing another request"
                                .to_string()
                        })?;
                        if !g.get_status().loaded {
                            emit_progress(app, path, 45, "loading model", None);
                            g.initialize(None, false)?;
                        }
                        g.transcribe_chunk_cancellable(raw_chunk, 16000, &cancel)
                    }
                    ASREngine::Qwen3 => {
                        let mut q = qwen3.try_lock().map_err(|_| {
                            "Qwen3 engine is busy loading or processing another request".to_string()
                        })?;
                        if !q.get_status().loaded {
                            emit_progress(app, path, 45, "loading model", None);
                            q.initialize(None, false)?;
                        }
                        q.transcribe_chunk_cancellable(raw_chunk, 16000, &cancel)
                    }
                    _ => unreachable!(),
                };
                // A cancel that aborted the run mid-chunk reports "cancelled" like
                // one caught between chunks.
                let t = match t {
                    Err(e) if cancel.load(Ordering::Relaxed) => {
                        ensure_not_cancelled(app, path, &cancel)?;
                        return Err(e);
                    }
                    other => other?,
                };
                // A cancel that arrived during the run still wins: discard the chunk
                // rather than finishing the file.
                ensure_not_cancelled(app, path, &cancel)?;

                if !t.trim().is_empty() {
                    parts.push(t.trim().to_string());
                }
            }

            parts.join(" ")
        }
    };

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_path_jobs_cannot_replace_each_others_progress_identity() {
        let path = format!("/tmp/taurscribe-file-job-{}", rand::random::<u64>());
        let first = register_cancel_flag(&path, Some("first".into())).unwrap();
        assert!(register_cancel_flag(&path, Some("second".into())).is_err());
        assert_eq!(
            cancel_flags()
                .lock()
                .unwrap()
                .get(&path)
                .unwrap()
                .job_id
                .as_deref(),
            Some("first")
        );
        unregister_cancel_flag(&path);

        let second = register_cancel_flag(&path, Some("second".into())).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(
            cancel_flags()
                .lock()
                .unwrap()
                .get(&path)
                .unwrap()
                .job_id
                .as_deref(),
            Some("second")
        );
        unregister_cancel_flag(&path);
    }
}
