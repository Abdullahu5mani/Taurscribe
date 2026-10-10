use crate::{
    recording_import::{self, RecordingSource},
    state::AudioState,
    types::ASREngine,
};
use std::path::Path;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

#[derive(serde::Serialize)]
pub struct RecordingInspection {
    recordings: Vec<RecordingSource>,
    errors: Vec<String>,
}

#[tauri::command]
pub async fn inspect_meeting_recordings(paths: Vec<String>) -> Result<RecordingInspection, String> {
    if paths.is_empty() || paths.len() > 100 {
        return Err("Choose between 1 and 100 recordings".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut result = RecordingInspection {
            recordings: vec![],
            errors: vec![],
        };
        for path in paths {
            match recording_import::inspect(Path::new(&path)) {
                Ok(source) => result.recordings.push(source),
                Err(error) => result.errors.push(error),
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn import_meeting_recordings(
    app: AppHandle,
    state: State<'_, AudioState>,
    paths: Vec<String>,
    title: String,
    expected_speakers: Option<usize>,
    model_id: Option<String>,
    use_gpu: Option<bool>,
) -> Result<i64, String> {
    validate_speaker_hint(expected_speakers)?;
    if title.trim().is_empty() {
        return Err("Give the meeting a title".into());
    }
    if paths.is_empty() || paths.len() > 100 {
        return Err("Choose between 1 and 100 recordings".into());
    }
    if crate::storage::move_in_progress() {
        return Err("Wait for the storage folder move to finish".into());
    }
    let transition = state.begin_recording_transition()?;
    let models = state.begin_exclusive_model_operation()?;
    if state.engine_loading.load(Ordering::Acquire) {
        return Err("Wait for the speech model to finish loading".into());
    }
    let engine = *state.active_engine.lock().map_err(|e| e.to_string())?;
    let whisper = state.whisper.clone();
    let gguf = state.gguf_manager(engine);
    let activity = state.last_activity_timestamp.clone();
    let title = title.trim().to_string();
    let _ = app.emit(
        "meeting-processing-started",
        serde_json::json!({"title": title, "platform": "Imported recording"}),
    );
    let completion_app = app.clone();
    // The worker owns both gates even if its caller disappears or switches views.
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        let _models = models;
        let result = (|| {
            load_import_engine(engine, &whisper, gguf.as_ref(), model_id.as_deref(), !use_gpu.unwrap_or(true))?;
            let dir = super::meetings::get_meetings_dir()?;
            let wav = recording_import::unique_audio_path(&dir.join("audio"));
            let sources = recording_import::join(&paths, &wav)?;
            let retained = |error: String| format!("{error}. Combined recording retained at {}. Your original files are unchanged.", wav.display());
            let stamp = chrono::Utc::now().timestamp_millis();
            let mut transcribe = super::recording::meeting_turn_transcriber(engine, &whisper, gguf.as_ref()).ok_or_else(|| retained("Speech engine is unavailable".into()))?;
            // Imported stereo is mixed audio, not our mic/remote channel layout.
            // Use the room pipeline: Nemotron + saved voice matching + per-turn ASR.
            let turns = crate::diarization::diarize_meeting_recording_with_hint(&wav, "", &dir.join("snippets"), stamp, Some(transcribe.as_mut()), true, expected_speakers).map_err(retained)?;
            let transcript = turns.iter().map(|t| t.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
            if transcript.is_empty() { return Err(retained("No speech was transcribed; the meeting was not saved".into())); }
            let duration = hound::WavReader::open(&wav).map_err(|e| retained(e.to_string()))?.duration() as i64 * 1000 / 48_000;
            let conn = super::meetings::open_connection().map_err(retained)?;
            let id = super::meetings::insert_meeting_with_sources(&conn, &title, "Imported recording", "Taurscribe", "", duration, Some(&wav.to_string_lossy()), &transcript, &turns, &sources).map_err(retained)?;
            if let Err(error) = super::meetings::convert_meeting_audio(&conn, id, &wav) { eprintln!("[IMPORT] Keeping playable WAV: {error}"); }
            Ok(id)
        })();
        activity.store(chrono::Utc::now().timestamp() as u64, Ordering::Relaxed);
        let (id, error) = match &result { Ok(id) => (Some(*id), None), Err(error) => (None, Some(error.as_str())) };
        let _ = app.emit("meeting-processing-finished", serde_json::json!({"meeting_id": id, "error": error}));
        if let Ok(id) = result.as_ref() { let _ = app.emit("meeting-processing-complete", serde_json::json!({"meeting_id": id})); }
        result
    }).await.map_err(|e| format!("Meeting import task failed: {e}"))?
}

fn validate_speaker_hint(count: Option<usize>) -> Result<(), String> {
    if count.is_some_and(|n| !(1..=32).contains(&n)) { return Err("Choose about 1–32 people, or leave the hint blank".into()); }
    Ok(())
}

fn load_import_engine(engine: ASREngine, whisper: &std::sync::Arc<std::sync::Mutex<crate::whisper::WhisperManager>>, gguf: Option<&std::sync::Arc<std::sync::Mutex<crate::gguf_asr::GgufAsrManager>>>, model_id: Option<&str>, force_cpu: bool) -> Result<(), String> {
    match engine {
        ASREngine::Whisper => {
            let mut model = whisper.lock().map_err(|e| e.to_string())?;
            if super::file_transcription::file_model_needs_load(model_id, model.get_current_model().map(String::as_str))? { model.initialize(model_id, force_cpu)?; }
        }
        _ => {
            let mut model = gguf.ok_or("Speech engine unavailable")?.lock().map_err(|e| e.to_string())?;
            if super::file_transcription::file_model_needs_load(model_id, model.get_status().model_id.as_deref())? { model.initialize(model_id, force_cpu)?; }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn reseparate_meeting_speakers(app: AppHandle, state: State<'_, AudioState>, meeting_id: i64, expected_speakers: Option<usize>, model_id: Option<String>, use_gpu: Option<bool>) -> Result<(), String> {
    validate_speaker_hint(expected_speakers)?;
    if crate::storage::move_in_progress() { return Err("Wait for the storage folder move to finish".into()); }
    let transition = state.begin_recording_transition()?;
    let models = state.begin_exclusive_model_operation()?;
    let engine = *state.active_engine.lock().map_err(|e| e.to_string())?;
    let whisper = state.whisper.clone(); let gguf = state.gguf_manager(engine);
    let activity = state.last_activity_timestamp.clone();
    let _ = app.emit("meeting-processing-started", serde_json::json!({"title":"Re-separating speakers", "platform":"Saved meeting"}));
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition; let _models = models;
        let result = (|| {
            let conn = super::meetings::open_connection()?;
            let revision = super::meetings::turn_revision(&conn, meeting_id)?;
            let (audio, platform): (Option<String>, String) = conn.query_row("SELECT audio_path, platform FROM meetings WHERE id=?1", rusqlite::params![meeting_id], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| e.to_string())?;
            let audio = audio.filter(|p| Path::new(p).exists()).ok_or("Meeting recording is missing; existing transcript is unchanged")?;
            load_import_engine(engine, &whisper, gguf.as_ref(), model_id.as_deref(), !use_gpu.unwrap_or(true))?;
            let dir = super::meetings::get_meetings_dir()?;
            let wav = recording_import::unique_audio_path(&dir.join("audio"));
            struct TemporaryWav(std::path::PathBuf);
            impl Drop for TemporaryWav { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }
            let _wav = TemporaryWav(wav.clone());
            // Preserve call channel identity; never downmix mic and remote audio.
            let (samples, rate, channels) = crate::audio_decode::decode_audio_interleaved_strict(Path::new(&audio))?;
            if channels == 0 || channels > u16::MAX as u32 { return Err("Invalid meeting audio channel count".into()); }
            let spec = hound::WavSpec { channels: channels as u16, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
            let mut writer = hound::WavWriter::create(&wav, spec).map_err(|e| e.to_string())?;
            for sample in samples { writer.write_sample(sample).map_err(|e| e.to_string())?; }
            writer.finalize().map_err(|e| e.to_string())?;
            let mut transcribe = super::recording::meeting_turn_transcriber(engine, &whisper, gguf.as_ref()).ok_or("Speech engine unavailable")?;
            let stamp = chrono::Utc::now().timestamp_millis();
            let room = platform == "In person" || platform == "Imported recording";
            let turns = crate::diarization::diarize_meeting_recording_with_hint(&wav, "", &dir.join("snippets"), stamp, Some(transcribe.as_mut()), room, expected_speakers)?;
            if turns.is_empty() || turns.iter().all(|t| t.text.trim().is_empty()) { return Err("No speech was recovered; existing transcript is unchanged".into()); }
            super::meetings::replace_separated_turns(&conn, meeting_id, &turns, &revision)
        })();
        activity.store(chrono::Utc::now().timestamp() as u64, Ordering::Relaxed);
        let _ = app.emit("meeting-processing-finished", serde_json::json!({"meeting_id":meeting_id,"error":result.as_ref().err()}));
        result
    }).await.map_err(|e| e.to_string())?
}
