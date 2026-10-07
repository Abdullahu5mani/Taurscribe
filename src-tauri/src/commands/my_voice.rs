//! "Your voice": a voiceprint of the user, recorded once by reading a passage
//! aloud. In-person meetings are one microphone, so this is how the speaker
//! who is the user gets labelled "You" (calls know it from the mic channel).

use crate::state::AudioState;
use crate::types::CommandResult;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

/// How long the setup records. The passage takes about 20 s to read.
const RECORD_SECONDS: u64 = 22;
/// Least speech a usable voiceprint needs. More than a meeting voiceprint
/// (`MIN_VOICEPRINT_SECONDS`) because this one labels every in-person meeting.
const MIN_SPEECH_SECONDS: f32 = 8.0;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MyVoiceStatus {
    pub enrolled: bool,
    /// Seconds of speech the voiceprint was made from.
    pub speech_seconds: Option<f32>,
    pub created_at: Option<String>,
    /// Voiceprints need the speaker recognition model (Settings → Models).
    pub speaker_model_installed: bool,
}

fn status(conn: &Connection) -> Result<MyVoiceStatus, String> {
    let row: Option<(f32, String)> = conn
        .query_row("SELECT speech_seconds, created_at FROM my_voice WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(MyVoiceStatus {
        enrolled: row.is_some(),
        speech_seconds: row.as_ref().map(|r| r.0),
        created_at: row.map(|r| r.1),
        speaker_model_installed: super::meetings::speaker_model_installed(),
    })
}

/// The saved voiceprint, if the user set one up.
pub fn my_voice_embedding(conn: &Connection) -> Option<Vec<f32>> {
    let json: String = conn.query_row("SELECT embedding_json FROM my_voice WHERE id = 1", [], |r| r.get(0)).ok()?;
    serde_json::from_str::<Vec<f32>>(&json).ok().filter(|e| !e.is_empty())
}

fn save(conn: &Connection, embedding: &[f32], speech_seconds: f32) -> Result<(), String> {
    let json = serde_json::to_string(embedding).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO my_voice (id, embedding_json, speech_seconds, created_at) VALUES (1, ?1, ?2, ?3)",
        params![json, speech_seconds, chrono::Utc::now().to_rfc3339()],
    )
    .map_err(|e| format!("Could not save your voice: {e}"))?;
    Ok(())
}

/// Only the speech in `samples` (pauses dropped), and how many seconds that is.
fn speech_only(samples: &[f32], sample_rate: u32) -> (Vec<f32>, f32) {
    let segments = crate::diarization::detect_channel_speech_segments(samples, sample_rate, 0, 0.015);
    let speech: Vec<f32> = segments
        .iter()
        .flat_map(|s| samples[s.start_sample.min(samples.len())..s.end_sample.min(samples.len())].iter().copied())
        .collect();
    let seconds = speech.len() as f32 / sample_rate.max(1) as f32;
    (speech, seconds)
}

#[tauri::command]
pub async fn get_my_voice() -> Result<MyVoiceStatus, String> {
    tauri::async_runtime::spawn_blocking(|| status(&super::meetings::open_connection()?))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_my_voice() -> Result<MyVoiceStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let conn = super::meetings::open_connection()?;
        conn.execute("DELETE FROM my_voice", []).map_err(|e| e.to_string())?;
        status(&conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Records the user reading the setup passage and saves their voiceprint.
/// Emits `my-voice-progress` ({ elapsed_ms, total_ms, level }) while recording.
#[tauri::command]
pub async fn record_my_voice(app: AppHandle, state: State<'_, AudioState>) -> Result<CommandResult<MyVoiceStatus>, String> {
    if state.recording_handle.lock().unwrap().is_some() {
        return Ok(CommandResult::err("already_recording", "Stop the recording first"));
    }
    if !super::meetings::speaker_model_installed() {
        return Ok(CommandResult::err(
            "speaker_model_missing",
            "Download the speaker recognition model in Settings → Models first",
        ));
    }
    let state = (*state).clone();
    let result = tauri::async_runtime::spawn_blocking(move || record_blocking(&app, &state))
        .await
        .map_err(|e| e.to_string())?;
    Ok(match result {
        Ok(status) => CommandResult::ok(status),
        Err((code, message)) => CommandResult::err(code, message),
    })
}

fn record_blocking(app: &AppHandle, state: &AudioState) -> Result<MyVoiceStatus, (&'static str, String)> {
    use cpal::traits::StreamTrait;
    let mic = super::recording::select_input_device(app, state).map_err(|e| ("mic_unavailable", e))?;
    let channels = mic.config.channels.max(1) as usize;
    let sample_rate = mic.config.sample_rate.0;
    let captured = Arc::new(Mutex::new(Vec::<f32>::with_capacity(sample_rate as usize * RECORD_SECONDS as usize)));
    let level = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let stream = {
        let (captured, level) = (captured.clone(), level.clone());
        crate::audio::build_input_stream_f32(
            &mic.device,
            &mic.config,
            mic.sample_format,
            move |data: &[f32]| {
                let mono: Vec<f32> = data.chunks(channels).map(|f| f.iter().sum::<f32>() / f.len() as f32).collect();
                let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len().max(1) as f32).sqrt();
                level.store(rms.to_bits(), std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut c) = captured.lock() {
                    c.extend_from_slice(&mono);
                }
            },
            |e| eprintln!("[MY_VOICE] Microphone error: {e}"),
        )
        .map_err(|e| ("mic_unavailable", format!("Could not open the microphone: {e}")))?
    };
    stream.play().map_err(|e| ("mic_unavailable", format!("Could not start the microphone: {e}")))?;
    let total_ms = RECORD_SECONDS * 1000;
    let started = std::time::Instant::now();
    while (started.elapsed().as_millis() as u64) < total_ms {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let rms = f32::from_bits(level.load(std::sync::atomic::Ordering::Relaxed));
        let _ = app.emit(
            "my-voice-progress",
            serde_json::json!({ "elapsed_ms": started.elapsed().as_millis() as u64, "total_ms": total_ms, "level": rms }),
        );
    }
    drop(stream);

    let samples = std::mem::take(&mut *captured.lock().unwrap());
    let (speech, seconds) = speech_only(&samples, sample_rate);
    println!("[MY_VOICE] {:.1}s recorded, {:.1}s of speech", samples.len() as f32 / sample_rate as f32, seconds);
    if seconds < MIN_SPEECH_SECONDS {
        return Err((
            "too_little_speech",
            format!("Only {seconds:.0} s of speech was heard. Read the whole passage, close to the microphone."),
        ));
    }
    let embedding = crate::speaker_embedding::get_speaker_engine()
        .lock()
        .map_err(|e| ("voiceprint_failed", e.to_string()))?
        .compute_embedding(&speech, sample_rate)
        .map_err(|e| ("voiceprint_failed", e))?;
    let conn = super::meetings::open_connection().map_err(|e| ("voiceprint_failed", e))?;
    save(&conn, &embedding, seconds).map_err(|e| ("voiceprint_failed", e))?;
    status(&conn).map_err(|e| ("voiceprint_failed", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn the_voiceprint_is_saved_once_and_replaced() {
        let conn = db();
        assert!(!status(&conn).unwrap().enrolled);
        assert!(my_voice_embedding(&conn).is_none());
        save(&conn, &[0.1, 0.2, 0.3], 12.5).unwrap();
        save(&conn, &[0.4, 0.5, 0.6], 15.0).unwrap();
        let st = status(&conn).unwrap();
        assert!(st.enrolled);
        assert_eq!(st.speech_seconds, Some(15.0));
        assert_eq!(my_voice_embedding(&conn), Some(vec![0.4, 0.5, 0.6]));
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM my_voice", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
        // Never mixed into the speaker vault.
        assert!(crate::commands::meetings::load_vault_embeddings_internal(&conn).unwrap().is_empty());
    }

    #[test]
    fn pauses_do_not_count_as_speech() {
        let rate = 16_000;
        let tone = |secs: f32| (0..(secs * rate as f32) as usize).map(|i| 0.3 * (i as f32 * 0.07).sin()).collect::<Vec<_>>();
        let mut samples = tone(3.0);
        samples.extend(vec![0.0; rate as usize * 4]);
        samples.extend(tone(2.0));
        let (speech, seconds) = speech_only(&samples, rate);
        assert!((4.5..=5.5).contains(&seconds), "{seconds}");
        assert_eq!(speech.len() as f32 / rate as f32, seconds);
    }
}
