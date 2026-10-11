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
    if paths.is_empty() {
        return Err("Choose at least one recording".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        inspect_recording_paths(&paths)
    })
    .await
    .map_err(|e| e.to_string())
}

fn inspect_recording_paths(paths: &[String]) -> RecordingInspection {
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
    if paths.is_empty() {
        return Err("Choose at least one recording".into());
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
        let progress = |stage: &str| { let _ = app.emit("meeting-processing-progress", serde_json::json!({"stage": stage})); };
        let result = (|| {
            progress("Loading the selected speech model");
            load_import_engine(engine, &whisper, gguf.as_ref(), model_id.as_deref(), !use_gpu.unwrap_or(true))?;
            let actual_model = import_model_id(engine, &whisper, gguf.as_ref());
            let dir = super::meetings::get_meetings_dir()?;
            let wav = recording_import::unique_audio_path(&dir.join("audio"));
            progress("Converting and joining recordings");
            let sources = recording_import::join(&paths, &wav)?;
            let retained = |error: String| format!("{error}. Combined recording retained at {}. Your original files are unchanged.", wav.display());
            let stamp = chrono::Utc::now().timestamp_millis();
            let mut transcribe = super::recording::meeting_turn_transcriber(engine, &whisper, gguf.as_ref()).ok_or_else(|| retained("Speech engine is unavailable".into()))?;
            // Imported stereo is mixed audio, not our mic/remote channel layout.
            // Use the room pipeline: Nemotron + saved voice matching + per-turn ASR.
            progress("Identifying speakers and transcribing speech");
            let turns = crate::diarization::diarize_meeting_recording_with_hint(&wav, "", &dir.join("snippets"), stamp, Some(transcribe.as_mut()), true, expected_speakers).map_err(retained)?;
            let transcript = turns.iter().map(|t| t.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
            if transcript.is_empty() { return Err(retained("No speech was transcribed; the meeting was not saved".into())); }
            let duration = hound::WavReader::open(&wav).map_err(|e| retained(e.to_string()))?.duration() as i64 * 1000 / 48_000;
            progress("Saving the meeting and preparing playback");
            let conn = super::meetings::open_connection().map_err(retained)?;
            let id = super::meetings::insert_meeting_with_sources(&conn, &title, "Imported recording", "Taurscribe", "", duration, Some(&wav.to_string_lossy()), &transcript, &turns, &sources).map_err(retained)?;
            super::meetings::save_transcription_model(&conn, id, actual_model.as_deref()).map_err(retained)?;
            if let Err(error) = super::meetings::convert_meeting_audio(&conn, id, &wav) { eprintln!("[IMPORT] Keeping playable WAV: {error}"); }
            Ok(id)
        })();
        activity.store(chrono::Utc::now().timestamp() as u64, Ordering::Relaxed);
        let (id, error) = match &result { Ok(id) => (Some(*id), None), Err(error) => (None, Some(error.as_str())) };
        let _ = app.emit("meeting-processing-finished", serde_json::json!({"meeting_id": id, "error": error}));
        if let Ok(id) = result.as_ref() { let _ = app.emit("meeting-processing-complete", serde_json::json!({"meeting_id": id, "imported": true})); }
        result
    }).await.map_err(|e| {
        let error = format!("Meeting import task failed: {e}");
        let _ = completion_app.emit("meeting-processing-finished", serde_json::json!({"meeting_id": null, "error": error}));
        error
    })?
}

fn validate_speaker_hint(count: Option<usize>) -> Result<(), String> {
    if count.is_some_and(|n| !(1..=32).contains(&n)) { return Err("Choose about 1–32 people, or leave the hint blank".into()); }
    Ok(())
}

fn import_model_id(engine: ASREngine, whisper: &std::sync::Arc<std::sync::Mutex<crate::whisper::WhisperManager>>, gguf: Option<&std::sync::Arc<std::sync::Mutex<crate::gguf_asr::GgufAsrManager>>>) -> Option<String> {
    match engine {
        ASREngine::Whisper => whisper.lock().ok().and_then(|m| m.get_current_model().cloned()),
        _ => gguf.and_then(|g| g.lock().ok().and_then(|m| m.get_status().model_id)),
    }
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
pub async fn reseparate_meeting_speakers(app: AppHandle, state: State<'_, AudioState>, meeting_id: i64, expected_speakers: Option<usize>, model_id: Option<String>, use_gpu: Option<bool>, clean_audio: Option<bool>) -> Result<(), String> {
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
            let _ = app.emit("meeting-processing-progress", serde_json::json!({"stage":"Decoding the original recording"}));
            let (samples, rate, channels) = crate::audio_decode::decode_audio_interleaved_strict(Path::new(&audio))?;
            // Older playback copies are mono mixes and no longer contain a
            // distinct microphone/caller reference. Reprocess those as mixed
            // speech; never interpret the whole meeting as the user's mic.
            let room = platform == "In person" || platform == "Imported recording" || channels == 1;
            let (samples, rate, channels) = if clean_audio.unwrap_or(false) {
                let _ = app.emit("meeting-processing-progress", serde_json::json!({"stage": if room {"Cleaning noise and preparing 16 kHz speech"} else {"Reducing microphone echo using the caller channel"}}));
                clean_pipeline_audio(&samples, rate, channels, room)?
            } else { (samples, rate, channels) };
            if channels == 0 || channels > u16::MAX as u32 { return Err("Invalid meeting audio channel count".into()); }
            let spec = hound::WavSpec { channels: channels as u16, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
            let mut writer = hound::WavWriter::create(&wav, spec).map_err(|e| e.to_string())?;
            for sample in samples { writer.write_sample(sample).map_err(|e| e.to_string())?; }
            writer.finalize().map_err(|e| e.to_string())?;
            let mut transcribe = super::recording::meeting_turn_transcriber(engine, &whisper, gguf.as_ref()).ok_or("Speech engine unavailable")?;
            let stamp = chrono::Utc::now().timestamp_millis();
            let _ = app.emit("meeting-processing-progress", serde_json::json!({"stage":"Re-identifying speakers and transcribing speech"}));
            let turns = crate::diarization::diarize_meeting_recording_with_hint(&wav, "", &dir.join("snippets"), stamp, Some(transcribe.as_mut()), room, expected_speakers)?;
            if turns.is_empty() || turns.iter().all(|t| t.text.trim().is_empty()) { return Err("No speech was recovered; existing transcript is unchanged".into()); }
            let _ = app.emit("meeting-processing-progress", serde_json::json!({"stage":"Saving the rebuilt transcript"}));
            super::meetings::replace_separated_turns(&conn, meeting_id, &turns, &revision)?;
            super::meetings::save_transcription_model(&conn, meeting_id, import_model_id(engine, &whisper, gguf.as_ref()).as_deref())
        })();
        activity.store(chrono::Utc::now().timestamp() as u64, Ordering::Relaxed);
        let _ = app.emit("meeting-processing-finished", serde_json::json!({"meeting_id":meeting_id,"error":result.as_ref().err()}));
        result
    }).await.map_err(|e| e.to_string())?
}


#[tauri::command]
pub async fn reprocess_meeting(app: AppHandle, state: State<'_, AudioState>, meeting_id: i64, expected_speakers: Option<usize>, model_id: Option<String>, use_gpu: Option<bool>) -> Result<(), String> {
    reseparate_meeting_speakers(app,state,meeting_id,expected_speakers,model_id,use_gpu,Some(true)).await
}

/// Echo cancellation requires the app's known mic/caller channel layout. A
/// stereo import is a room mix, so it must never be treated as that reference.
fn clean_pipeline_audio(samples: &[f32], rate: u32, channels: u32, room: bool) -> Result<(Vec<f32>,u32,u32),String> {
    if channels==0 || samples.len()%channels as usize!=0 {return Err("Invalid audio channel alignment".into());}
    if !room && channels!=2 {return Err("Echo cleanup needs the meeting's microphone and caller channels".into());}
    if room {
        let mono:Vec<_>=samples.chunks_exact(channels as usize).map(|frame|frame.iter().sum::<f32>()/channels as f32).collect();
        let mut cleaned=crate::audio_preprocess::resample_mono_to_16k(&mono,rate)?;
        crate::audio_preprocess::preprocess_assembled_speech_16k(&mut cleaned);
        return Ok((cleaned,16000,1));
    }
    let mut mic=Vec::with_capacity(samples.len()/2);let mut callers=Vec::with_capacity(samples.len()/2);
    for frame in samples.chunks_exact(2){mic.push(frame[0]);callers.push(frame[1]);}
    let mic=crate::audio_preprocess::resample_mono_ratio(&mic,rate,48000)?;
    let callers=crate::audio_preprocess::resample_mono_ratio(&callers,rate,48000)?;
    if mic.len()!=callers.len(){return Err("Resampled meeting channels lost alignment".into());}
    let mut echo=crate::echo_cancel::EchoCanceller::new();
    let mut cleaned=Vec::with_capacity(mic.len());let mut reference=Vec::with_capacity(callers.len());
    for (m,r) in mic.chunks(48000).zip(callers.chunks(48000)) {let (m,r)=echo.push(m,r);cleaned.extend(m);reference.extend(r);}
    let (m,r)=echo.flush();cleaned.extend(m);reference.extend(r);
    let mut mic=crate::audio_preprocess::resample_mono_to_16k(&cleaned,48000)?;
    let mut callers=crate::audio_preprocess::resample_mono_to_16k(&reference,48000)?;
    crate::audio_preprocess::preprocess_assembled_speech_16k(&mut mic);
    crate::audio_preprocess::preprocess_assembled_speech_16k(&mut callers);
    if mic.len()!=callers.len(){return Err("Cleaned meeting channels lost alignment".into());}
    let interleaved=mic.into_iter().zip(callers).flat_map(|(m,r)|[m,r]).collect();
    Ok((interleaved,16000,2))
}


#[cfg(test)]
mod pipeline_audio_tests {
    use super::*;
    #[test]
    #[ignore = "Requires installed Granite and optional diarization models"]
    fn native_clean_pipeline_preserves_public_speech() {
        let fixture=std::env::var("TAURSCRIBE_CLEAN_PIPELINE_FIXTURE").map(std::path::PathBuf::from).unwrap_or_else(|_|std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../taurscribe-runtime/samples/jfk.wav"));
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let tmp=Fixture(std::env::temp_dir().join(format!("taurscribe-clean-quality-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())));
        std::fs::create_dir(&tmp.0).unwrap();let wav=tmp.0.join("cleaned.wav");
        let copy=tmp.0.join("source.wav");std::fs::copy(&fixture,&copy).unwrap();
        let playback=crate::meeting_audio::compress_for_playback(&copy).unwrap();
        let (samples,rate,channels)=crate::audio_decode::decode_audio_interleaved_strict(&playback).unwrap();
        let (cleaned,rate,channels)=clean_pipeline_audio(&samples,rate,channels,true).unwrap();
        assert_eq!(channels,1);
        let mut model=crate::gguf_asr::GgufAsrManager::granite();model.initialize(Some("granite-speech-5-nc"),false).unwrap();
        let direct=model.transcribe_chunk(&cleaned,rate,None).unwrap();
        assert!(direct.to_lowercase().contains("country"),"cleaned ASR: {direct}");

        let mut writer=hound::WavWriter::create(&wav,hound::WavSpec {channels:1,sample_rate:rate,bits_per_sample:32,sample_format:hound::SampleFormat::Float}).unwrap();
        for sample in cleaned {writer.write_sample(sample).unwrap();}writer.finalize().unwrap();
        let mut transcribe=|samples:&[f32],rate:u32|model.transcribe_chunk(samples,rate,None);
        let turns=crate::diarization::diarize_meeting_recording_with_hint(&wav,"",&tmp.0.join("snippets"),0,Some(&mut transcribe),true,Some(1)).unwrap();
        let text=turns.iter().map(|turn|turn.text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(text.to_lowercase().contains("country"),"diarized cleaned ASR: {text}");
        println!("NATIVE_CLEAN_PIPELINE text={text}");
    }

    #[test]
    fn room_import_is_mixed_without_treating_stereo_as_an_echo_reference(){
        let samples:Vec<f32>=(0..48000).flat_map(|i|[(i as f32*0.05).sin()*0.15,(i as f32*0.07).sin()*0.1]).collect();
        let (cleaned,rate,channels)=clean_pipeline_audio(&samples,48000,2,true).unwrap();
        assert_eq!(rate,16000);assert_eq!(channels,1);assert_eq!(cleaned.len(),16000);
        assert!(cleaned.iter().all(|v|v.is_finite()));assert!(cleaned.iter().any(|v|v.abs()>0.001));
        assert!(clean_pipeline_audio(&[0.1;9],48000,2,false).is_err());
        assert!(clean_pipeline_audio(&[0.1;10],48000,1,false).is_err());
    }
    #[test]
    fn call_pipeline_preserves_channel_alignment_and_duration(){
        let samples:Vec<f32>=(0..48000*3).flat_map(|i|[(i as f32*0.04).sin()*0.1,(i as f32*0.065).sin()*0.1]).collect();
        let (cleaned,rate,channels)=clean_pipeline_audio(&samples,48000,2,false).unwrap();
        assert_eq!(rate,16000);assert_eq!(channels,2);assert_eq!(cleaned.len(),16000*3*2);
        assert!(cleaned.iter().all(|v|v.is_finite()));
    }
}

#[cfg(test)]
mod import_batch_tests {
    #[test]
    fn inspects_every_path_above_the_old_hundred_file_limit() {
        // Invalid entries must be reported individually, not reject the whole batch.
        let paths: Vec<String> = (0..151).map(|n| format!("/missing/recording-{n}.wav")).collect();
        let result = super::inspect_recording_paths(&paths);
        assert!(result.recordings.is_empty());
        assert_eq!(result.errors.len(), paths.len());
    }
}
