use crate::{
    recording_import::{self, RecordingSource},
    state::AudioState,
    types::ASREngine,
};
use std::path::Path;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

// Keep the in-flight draft available when the user visits Mic/Files and returns.
static IMPORT_STATUS: std::sync::Mutex<Option<serde_json::Value>> = std::sync::Mutex::new(None);
#[tauri::command]
pub fn get_meeting_import_status() -> Result<Option<serde_json::Value>, String> {
    IMPORT_STATUS.lock().map(|status| status.clone()).map_err(|e|e.to_string())
}
fn update_import_status(update: impl FnOnce(&mut serde_json::Value)) {
    if let Ok(mut status)=IMPORT_STATUS.lock() { if let Some(status)=status.as_mut(){update(status);} }
}

pub(crate) fn remember_meeting_processing(mut payload: serde_json::Value) {
    payload["running"]=true.into();payload["startedAt"]=chrono::Utc::now().timestamp_millis().into();payload["error"]=serde_json::Value::Null;
    if let Ok(mut status)=IMPORT_STATUS.lock(){*status=Some(payload);}
}
pub(crate) fn finish_meeting_processing(error: Option<&str>) {
    if let Ok(mut status)=IMPORT_STATUS.lock(){
        if let Some(error)=error {if let Some(status)=status.as_mut(){status["running"]=false.into();status["error"]=error.into();}}
        else {*status=None;}
    }
}

#[tauri::command]
pub fn cancel_meeting_processing(app: AppHandle, job_id: Option<String>) -> Result<bool,String> {
    let stopped=crate::meeting_job::cancel(job_id.as_deref())?;
    if stopped {update_import_status(|status|status["stage"]="Stopping meeting processing…".into());let _=app.emit("meeting-processing-progress",serde_json::json!({"stage":"Stopping meeting processing…"}));}
    Ok(stopped)
}

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
    job_id: Option<String>,
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
    let engine = model_id.as_deref().map(|id|if id.starts_with("granite-"){ASREngine::Granite}else if id.starts_with("qwen3-"){ASREngine::Qwen3}else{ASREngine::Whisper}).unwrap_or(*state.active_engine.lock().map_err(|e| e.to_string())?);
    let whisper = state.whisper.clone();
    let gguf = state.gguf_manager(engine);
    let activity = state.last_activity_timestamp.clone();
    let job=crate::meeting_job::MeetingJob::begin(job_id)?;
    let cancel=job.cancel.clone();
    let title = title.trim().to_string();
    if let Ok(mut status)=IMPORT_STATUS.lock() {
        *status=Some(serde_json::json!({"title":title,"platform":"Imported recording","job_id":job.id,"running":true,"startedAt":chrono::Utc::now().timestamp_millis(),"preview":null,"error":null}));
    }
    let _ = app.emit(
        "meeting-processing-started",
        serde_json::json!({"title": title, "platform": "Imported recording", "job_id":job.id}),
    );
    let completion_app = app.clone();
    // The worker owns both gates even if its caller disappears or switches views.
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        let _models = models;
        let _job = job;
        let progress = |stage: &str| { update_import_status(|status|status["stage"]=stage.into()); let _ = app.emit("meeting-processing-progress", serde_json::json!({"stage": stage})); };
        let result = (|| {
            crate::meeting_job::check(&cancel)?;
            progress("Loading the selected speech model");
            load_import_engine(engine, &whisper, gguf.as_ref(), model_id.as_deref(), !use_gpu.unwrap_or(true))?;
            let actual_model = import_model_id(engine, &whisper, gguf.as_ref());
            let dir = super::meetings::get_meetings_dir()?;
            let wav = recording_import::unique_audio_path(&dir.join("audio"));
            progress("Converting and joining recordings");
            let sources = recording_import::join_cancellable(&paths, &wav, ||crate::meeting_job::check(&cancel))?;
            let retained = |error: String| format!("{error}. Combined recording retained at {}. Your original files are unchanged.", wav.display());
            let stamp = chrono::Utc::now().timestamp_millis();
            let mut transcribe = super::recording::meeting_turn_transcriber(engine, &whisper, gguf.as_ref()).ok_or_else(|| retained("Speech engine is unavailable".into()))?;
            let (samples, rate) = crate::diarization::read_room_audio(&wav).map_err(retained)?;
            let pcm = crate::audio_preprocess::resample_mono_to_16k(&samples, rate).map_err(retained)?;
            let began = std::time::Instant::now();
            let (cached, neural) = std::thread::scope(|scope| -> Result<_, String> {
                let mut cached = crate::meeting_transcript::TimedTranscript::default();
                let mut paragraphs = Vec::new();
                let mut worker = None;
                let overlap = use_gpu.unwrap_or(true) && crate::neural_diarizer::model_path().is_some();
                for (index, (start, end)) in crate::meeting_transcript::chunk_ranges(&pcm).into_iter().enumerate() {
                    crate::meeting_job::check(&cancel)?;
                    progress(if worker.is_some() { "Transcribing audio while GPU speaker labeling runs" } else { "Transcribing audio — text appears as it is ready" });
                    let part = match engine {
                        ASREngine::Whisper => whisper.lock().map_err(|e| e.to_string())?.transcribe_audio_data_timed_cancellable(&pcm[start..end], None, Some(cancel.clone()))?,
                        _ => gguf.as_ref().ok_or("Speech engine unavailable")?.lock().map_err(|e| e.to_string())?.transcribe_timed_cancellable(&pcm[start..end], &cancel)?,
                    };
                    let start_ms = start as u64 / 16;
                    let end_ms = end as u64 / 16;
                    if !part.text.trim().is_empty() {
                        paragraphs.push(crate::meeting_transcript::TimedText { start_ms, end_ms, text: part.text.clone() });
                    }
                    cached.append(part, start_ms);
                    let preview = serde_json::json!({"title":title,"model_id":actual_model,"paragraphs":paragraphs,"completed_ms":end_ms,"total_ms":pcm.len() as u64 / 16});
                    update_import_status(|status|status["preview"]=preview.clone());
                    let _ = app.emit("meeting-processing-preview", preview);
                    if index == 0 {
                        eprintln!("[IMPORT] First transcript chunk: {:.2}s", began.elapsed().as_secs_f64());
                        if overlap && crate::meeting_transcript::can_overlap_diarization() {
                            let audio = &pcm;
                            worker = Some(scope.spawn(move || crate::neural_diarizer::diarize_16k(audio)));
                        }
                    }
                }
                progress("Finishing speaker labeling and aligning the transcript");
                let neural = worker.map(|worker| worker.join().unwrap_or_else(|_| Err("GPU diarization worker failed; retrying sequentially".into())));
                Ok((cached, neural))
            }).map_err(retained)?;
            let turns = crate::diarization::finish_room_import(&samples, rate, &dir.join("snippets"), stamp, transcribe.as_mut(), expected_speakers, &cached, neural).map_err(retained)?;
            eprintln!("[IMPORT] Transcription and labeling: {:.2}s", began.elapsed().as_secs_f64());
            crate::meeting_job::check(&cancel)?;
            let transcript = turns.iter().map(|t| t.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
            if transcript.is_empty() { return Err(retained("No speech was transcribed; the meeting was not saved".into())); }
            let duration = hound::WavReader::open(&wav).map_err(|e| retained(e.to_string()))?.duration() as i64 * 1000 / 48_000;
            progress("Saving the meeting and preparing playback");
            let conn = super::meetings::open_connection().map_err(retained)?;
            crate::meeting_job::finish(&cancel)?;
            let id = super::meetings::insert_meeting_with_sources(&conn, &title, "Imported recording", "Taurscribe", "", duration, Some(&wav.to_string_lossy()), &transcript, &turns, &sources).map_err(retained)?;
            super::meetings::save_transcription_model(&conn, id, actual_model.as_deref()).map_err(retained)?;
            if let Err(error) = super::meetings::convert_meeting_audio(&conn, id, &wav) { eprintln!("[IMPORT] Keeping playable WAV: {error}"); }
            Ok(id)
        })().map_err(|error|if cancel.load(Ordering::Relaxed){format!("{}; original recordings are unchanged",crate::meeting_job::STOPPED)}else{error});
        activity.store(chrono::Utc::now().timestamp() as u64, Ordering::Relaxed);
        let (id, error) = match &result { Ok(id) => (Some(*id), None), Err(error) => (None, Some(error.as_str())) };
        if let Ok(mut status)=IMPORT_STATUS.lock() {
            if error.is_none() { *status=None; } else if let Some(status)=status.as_mut(){status["running"]=false.into();status["error"]=serde_json::json!(error);}
        }
        let _ = app.emit("meeting-processing-finished", serde_json::json!({"meeting_id": id, "error": error,"cancelled":cancel.load(Ordering::Relaxed)}));
        if let Ok(id) = result.as_ref() { let _ = app.emit("meeting-processing-complete", serde_json::json!({"meeting_id": id, "imported": true})); }
        result
    }).await.map_err(|e| {
        let error = format!("Meeting import task failed: {e}");
        update_import_status(|status|{status["running"]=false.into();status["error"]=error.clone().into();});
        let _ = completion_app.emit("meeting-processing-finished", serde_json::json!({"meeting_id": null, "error": error}));
        error
    });
    after_import_model_use(&completion_app, &state);
    outcome?
}

/// An import or re-separation may have loaded a speech model. Keep the
/// "model loaded" flag (which the auto-unload watchdog checks) and the tray in
/// step, and honour "unload immediately", as file transcription does. Runs
/// after the job's exclusive model gate is released.
fn after_import_model_use(app: &AppHandle, state: &AudioState) {
    crate::tray::reconcile_model_loaded_tray(app, state);
    super::file_transcription::maybe_unload_after_file_batch(app, state);
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

fn import_model_needs_load(requested: Option<&str>, loaded: Option<&str>) -> bool {
    loaded.is_none() || requested.is_some_and(|id| Some(id) != loaded)
}

fn load_import_engine(engine: ASREngine, whisper: &std::sync::Arc<std::sync::Mutex<crate::whisper::WhisperManager>>, gguf: Option<&std::sync::Arc<std::sync::Mutex<crate::gguf_asr::GgufAsrManager>>>, model_id: Option<&str>, force_cpu: bool) -> Result<(), String> {
    let device_key=crate::gpu::Target::resolve(force_cpu).key();
    match engine {
        ASREngine::Whisper => {
            let mut model = whisper.lock().map_err(|e| e.to_string())?;
            if import_model_needs_load(model_id, model.get_current_model().map(String::as_str)) || model.device_key()!=device_key { model.initialize(model_id, force_cpu)?; }
        }
        _ => {
            let mut model = gguf.ok_or("Speech engine unavailable")?.lock().map_err(|e| e.to_string())?;
            if import_model_needs_load(model_id, model.get_status().model_id.as_deref()) || model.device_key()!=device_key { model.initialize(model_id, force_cpu)?; }
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
    let job=crate::meeting_job::MeetingJob::begin(None)?;
    let cancel=job.cancel.clone();
    let payload=serde_json::json!({"title":"Re-separating speakers", "platform":"Saved meeting","job_id":job.id});
    remember_meeting_processing(payload.clone());
    let _ = app.emit("meeting-processing-started",payload);
    let completion_app = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition; let _models = models;let _job=job;
        let result = (|| {
            let conn = super::meetings::open_connection()?;
            crate::meeting_job::check(&cancel)?;
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
            for (index, sample) in samples.into_iter().enumerate() { if index % 48000 == 0 { crate::meeting_job::check(&cancel)?; } writer.write_sample(sample).map_err(|e| e.to_string())?; }
            writer.finalize().map_err(|e| e.to_string())?;
            let mut transcribe = super::recording::meeting_turn_transcriber(engine, &whisper, gguf.as_ref()).ok_or("Speech engine unavailable")?;
            let stamp = chrono::Utc::now().timestamp_millis();
            let _ = app.emit("meeting-processing-progress", serde_json::json!({"stage":"Re-identifying speakers and transcribing speech"}));
            let turns = crate::diarization::diarize_meeting_recording_with_hint(&wav, "", &dir.join("snippets"), stamp, Some(transcribe.as_mut()), room, expected_speakers)?;
            if turns.is_empty() || turns.iter().all(|t| t.text.trim().is_empty()) { return Err("No speech was recovered; existing transcript is unchanged".into()); }
            let _ = app.emit("meeting-processing-progress", serde_json::json!({"stage":"Saving the rebuilt transcript"}));
            crate::meeting_job::finish(&cancel)?;
            super::meetings::replace_separated_turns(&conn, meeting_id, &turns, &revision)?;
            super::meetings::save_transcription_model(&conn, meeting_id, import_model_id(engine, &whisper, gguf.as_ref()).as_deref())
        })();
        activity.store(chrono::Utc::now().timestamp() as u64, Ordering::Relaxed);
        finish_meeting_processing(result.as_ref().err().map(String::as_str));
        let _ = app.emit("meeting-processing-finished", serde_json::json!({"meeting_id":meeting_id,"error":result.as_ref().err(),"cancelled":cancel.load(Ordering::Relaxed)}));
        result
    }).await.map_err(|e| e.to_string());
    after_import_model_use(&completion_app, &state);
    outcome?
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

#[cfg(test)]
mod explicit_model_selection_tests {
    use super::*;
    #[test]
    fn explicit_import_switches_models_but_default_reuses_the_loaded_one() {
        assert!(import_model_needs_load(Some("whisper-tiny"),Some("whisper-base")));
        assert!(import_model_needs_load(Some("whisper-tiny"),None));
        assert!(import_model_needs_load(None,None));
        assert!(!import_model_needs_load(None,Some("whisper-base")));
        assert!(!import_model_needs_load(Some("whisper-base"),Some("whisper-base")));
        // Keep the queue guard strict: its request is an identity promise.
        assert!(super::super::file_transcription::file_model_needs_load(Some("whisper-tiny"),Some("whisper-base")).is_err());
    }
}
