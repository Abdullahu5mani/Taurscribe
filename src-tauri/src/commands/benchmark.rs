//! Settings → Models speed test: compare a genuinely unloaded model with a
//! resident model on the same speech clip, optionally including FlowScribe V3.

use crate::{gpu, llm::{FlowRequest, LLMEngine}, state::AudioState};
use crate::types::{ASREngine, CommandResult};
use serde::{Serialize, Deserialize};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Instant;
use tauri::{Emitter, State};

const ROUNDS: usize = 3;
type Shared<T> = Arc<Mutex<T>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedTrial {
    pub load_s: f32,
    pub transcribe_s: f32,
    pub flowscribe_load_s: f32,
    pub flowscribe_s: f32,
    pub total_s: f32,
    pub transcript: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedPair {
    pub unloaded: SpeedTrial,
    pub loaded: SpeedTrial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSpeed {
    pub device_id: String,
    pub name: String,
    pub backend: String,
    pub load_s: f32,
    pub transcribe_s: f32,
    pub unloaded_total_s: f32,
    pub loaded_total_s: f32,
    pub flowscribe_load_s: f32,
    pub flowscribe_s: f32,
    pub flowscribe_device_id: Option<String>,
    pub realtime: f32,
    pub trials: Vec<SpeedPair>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedReport {
    pub engine: ASREngine,
    pub model: String,
    pub clip_seconds: f32,
    pub rounds: usize,
    pub include_flowscribe: bool,
    pub results: Vec<DeviceSpeed>,
    pub restore_error: Option<String>,
}

/// A public-domain speech fixture, not a synthetic tone. Its actual transcript
/// also drives FlowScribe, so clean-up timing reflects the full speech pipeline.
fn test_clip() -> Result<Vec<f32>, String> {
    let wav = include_bytes!("../../../taurscribe-runtime/samples/jfk.wav");
    let mut reader = hound::WavReader::new(std::io::Cursor::new(wav.as_slice())).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 || spec.channels != 1 || spec.bits_per_sample != 16 {
        return Err("Speed test fixture must be 16 kHz mono PCM16".into());
    }
    let mut clip = reader.samples::<i16>().map(|s| s.map(|n| n as f32 / 32768.0)).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    // The importer uses this same speech assembly/preprocessing chain.
    clip = crate::vad::assemble_speech_audio(&clip, None)?;
    crate::audio_preprocess::preprocess_assembled_speech_16k(&mut clip);
    Ok(clip)
}

struct RestoreDevice(String);
impl Drop for RestoreDevice {
    fn drop(&mut self) { gpu::set_preferred_device(&self.0); }
}
struct ClearLoading(Arc<AtomicBool>);
impl Drop for ClearLoading {
    fn drop(&mut self) { self.0.store(false, Ordering::Release); }
}

fn median(values: impl Iterator<Item = f32>) -> f32 {
    let mut values: Vec<_> = values.collect();
    values.sort_by(f32::total_cmp);
    if values.is_empty() { return 0.0; }
    let i = values.len() / 2;
    if values.len() % 2 == 1 { values[i] } else { (values[i - 1] + values[i]) / 2.0 }
}

fn load_asr(engine: ASREngine, whisper: &Shared<crate::whisper::WhisperManager>, gguf: Option<&Shared<crate::gguf_asr::GgufAsrManager>>, model: &str, device: &str) -> Result<(), String> {
    gpu::set_preferred_device(device);
    match engine {
        ASREngine::Whisper => { whisper.lock().map_err(|e| e.to_string())?.initialize(Some(model), device == gpu::CPU_DEVICE)?; }
        _ => { gguf.ok_or("No such engine")?.lock().map_err(|e| e.to_string())?.initialize(Some(model), device == gpu::CPU_DEVICE)?; }
    }
    Ok(())
}

fn unload_asr(whisper: &Shared<crate::whisper::WhisperManager>, gguf: Option<&Shared<crate::gguf_asr::GgufAsrManager>>) -> Result<(), String> {
    whisper.lock().map_err(|e| e.to_string())?.unload();
    if let Some(gguf) = gguf { gguf.lock().map_err(|e| e.to_string())?.unload(); }
    crate::memory::trim_process_memory();
    Ok(())
}

fn transcribe(engine: ASREngine, whisper: &Shared<crate::whisper::WhisperManager>, gguf: Option<&Shared<crate::gguf_asr::GgufAsrManager>>, clip: &[f32]) -> Result<String, String> {
    let text = match engine {
        ASREngine::Whisper => {
            let mut w = whisper.lock().map_err(|e| e.to_string())?;
            w.clear_context();
            w.transcribe_chunk(clip, 16_000)?
        }
        _ => gguf.ok_or("No such engine")?.lock().map_err(|e| e.to_string())?.transcribe_audio_data(clip, None)?,
    };
    let text = crate::utils::clean_transcript(&text);
    if text.trim().is_empty() { return Err("The speech model returned no text for the test clip".into()); }
    Ok(text)
}

fn time_speech(engine: ASREngine, whisper: &Shared<crate::whisper::WhisperManager>, gguf: Option<&Shared<crate::gguf_asr::GgufAsrManager>>, clip: &[f32], flow: &mut Option<LLMEngine>, request: &FlowRequest) -> Result<SpeedTrial, String> {
    let total = Instant::now();
    let started = Instant::now();
    let text = transcribe(engine, whisper, gguf, clip)?;
    let transcribe_s = started.elapsed().as_secs_f32();
    let started = Instant::now();
    let output = if let Some(flow) = flow { flow.clean_transcript(&text, request).map_err(|e| e.to_string())? } else { text.clone() };
    let flowscribe_s = if flow.is_some() { started.elapsed().as_secs_f32() } else { 0.0 };
    Ok(SpeedTrial { load_s: 0.0, transcribe_s, flowscribe_load_s: 0.0, flowscribe_s, total_s: total.elapsed().as_secs_f32(), transcript: text, output })
}

/// Every cold trial drops both test models, then loads and transcribes. Its
/// partner immediately transcribes again with both models resident. OS disk
/// caches are left alone. Restore the user's model/device even on test errors.
#[tauri::command]
pub async fn benchmark_compute_devices(
    state: State<'_, AudioState>, app: tauri::AppHandle,
    include_flowscribe: Option<bool>, flowscribe_device_id: Option<String>,
    flowscribe_use_gpu: Option<bool>, style: Option<String>, device_ids: Option<Vec<String>>,
) -> Result<CommandResult<SpeedReport>, String> {
    let include_flow = include_flowscribe.unwrap_or(false);
    if include_flow && crate::llm::v3_model_path().is_none() {
        return Ok(CommandResult::err("model_missing", "Download FlowScribe V3 from Models before including it in the speed test"));
    }
    if state.recording_handle.lock().unwrap().is_some() {
        return Ok(CommandResult::err("already_recording", "Stop recording first"));
    }
    if super::file_job_active() {
        return Ok(CommandResult::err("busy", "Wait for the file transcription to finish"));
    }
    let available = gpu::compute_devices();
    if let Some(ids) = &device_ids {
        if ids.is_empty() || ids.iter().any(|id| !available.iter().any(|device| device.usable && &device.id == id)) {
            return Ok(CommandResult::err("invalid_device", "Choose at least one available compute device"));
        }
    }
    let devices: Vec<_> = available.into_iter().filter(|device| device.usable && device_ids.as_ref().is_none_or(|ids| ids.contains(&device.id))).collect();
    let operation = state.begin_exclusive_model_operation()?;
    if state.engine_loading.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        return Ok(CommandResult::err("engine_loading", "A model is already loading — please wait"));
    }
    let loading = ClearLoading(state.engine_loading.clone());
    let engine = *state.active_engine.lock().unwrap();
    let model = match engine {
        ASREngine::Whisper => state.whisper_snapshot().model,
        other => state.gguf_snapshot(other).filter(|s| s.loaded).and_then(|s| s.model_id),
    };
    let Some(model) = model else { return Ok(CommandResult::err("model_missing", "Load a speech model first")); };
    let state = (*state).clone();
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // Cancellation of the IPC caller must not release model exclusivity
        // while this native worker is still unloading/loading/inferencing.
        let _operation = operation;
        let _loading = loading;
        let saved = gpu::preferred_device();
        let _restore = RestoreDevice(saved.clone());
        let whisper = &state.whisper;
        let gguf = state.gguf_manager(engine);
        let loaded_on = match &gguf {
            None => whisper.lock().map_err(|e| e.to_string())?.device_key().to_string(),
            Some(g) => g.lock().map_err(|e| e.to_string())?.device_key().to_string(),
        };
        unload_asr(whisper, gguf.as_ref())?;
        let clip_seconds = 11.0; // Original WAV duration; VAD removes silence.
        let request = FlowRequest {
            engine: match engine { ASREngine::Whisper => "whisper", ASREngine::Granite => "granite", ASREngine::Qwen3 => "qwen3" }.into(),
            level: crate::llm::level_for_style(style.as_deref()).into(),
            app: "generic".into(), vocab: crate::context::load_custom_vocabulary_from_settings().0, prev: None,
        };
        let original_flow = state.llm.lock().map_err(|e| e.to_string())?.take();
        let flow_loaded_on = original_flow.as_ref().map(|f| f.device_key().to_string());
        drop(original_flow); // Do not keep an extra GPU copy during measurements.
        let mut results = Vec::new();
        for (i, d) in devices.iter().enumerate() {
            let job = BenchmarkJob { engine, model:model.clone(), id:d.id.clone(),name:d.name.clone(),backend:d.backend.clone(),
                include_flow,flowscribe_device_id:flowscribe_device_id.clone(),flowscribe_use_gpu,request:request.clone() };
            let measured=isolated_trial(&job, |round| {
                let _=worker_app.emit("compute-benchmark-progress",serde_json::json!({"index":i,"total":devices.len(),"device":d.name,"round":round,"rounds":ROUNDS}));
            });
            results.push(measured.unwrap_or_else(|error| failed_device(&job,error)));
        }
        let mut restore_errors = Vec::new();
        if let Err(e) = load_asr(engine, whisper, gguf.as_ref(), &model, &loaded_on) { restore_errors.push(format!("Speech model: {e}")); }
        if let Some(device) = flow_loaded_on {
            match LLMEngine::new_on(device != gpu::CPU_DEVICE, Some(&device)) {
                Ok(flow) => { *state.llm.lock().map_err(|e| e.to_string())? = Some(flow); }
                Err(e) => restore_errors.push(format!("FlowScribe: {e}")),
            }
        }
        crate::tray::reconcile_model_loaded_tray(&worker_app, &state);
        Ok::<_, String>(SpeedReport {
            engine, model, clip_seconds, rounds: ROUNDS, include_flowscribe: include_flow, results,
            restore_error: (!restore_errors.is_empty()).then(|| restore_errors.join("; ")),
        })
    }).await.map_err(|e| e.to_string())?;
    result.map(CommandResult::ok)
}


#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchmarkJob {
    engine: ASREngine, model: String, id: String, name: String, backend: String,
    include_flow: bool, flowscribe_device_id: Option<String>, flowscribe_use_gpu: Option<bool>, request: FlowRequest,
}

fn worker_trials(d: &BenchmarkJob, progress_path: &std::path::Path) -> DeviceSpeed {
    let engine = d.engine;
    let model = &d.model;
    let include_flow = d.include_flow;
    let flowscribe_device_id = &d.flowscribe_device_id;
    let flowscribe_use_gpu = d.flowscribe_use_gpu;
    let request = &d.request;
    let whisper = Arc::new(Mutex::new(crate::whisper::WhisperManager::new()));
    let gguf = match engine {
        ASREngine::Granite => Some(Arc::new(Mutex::new(crate::gguf_asr::GgufAsrManager::granite()))),
        ASREngine::Qwen3 => Some(Arc::new(Mutex::new(crate::gguf_asr::GgufAsrManager::qwen3()))),
        _ => None,
    };
    let whisper = &whisper;
    let clip = match test_clip() { Ok(clip) => clip, Err(error) => return failed_device(d, error) };
    let clip_seconds = 11.0;
            let mut pairs = Vec::new();
            let mut actual_flow_device = None;
            let mut error = None;
            for round in 0..ROUNDS {
                let _ = std::fs::write(progress_path, (round + 1).to_string());
                let pair = (|| -> Result<SpeedPair, String> {
                    unload_asr(whisper, gguf.as_ref())?;
                    let total = Instant::now();
                    let started = Instant::now();
                    load_asr(engine, whisper, gguf.as_ref(), &model, &d.id)?;
                    let load_s = started.elapsed().as_secs_f32();
                    let started = Instant::now();
                    let flow_device = flowscribe_device_id.as_deref().filter(|id| *id != "same").unwrap_or(&d.id);
                    let mut flow = if include_flow { Some(LLMEngine::new_on(flowscribe_use_gpu.unwrap_or(true) && flow_device != gpu::CPU_DEVICE, Some(flow_device)).map_err(|e| e.to_string())?) } else { None };
                    let flow_load_s = if include_flow { started.elapsed().as_secs_f32() } else { 0.0 };
                    actual_flow_device = flow.as_ref().map(|f| f.device_key().to_string());
                    let mut cold = time_speech(engine, whisper, gguf.as_ref(), &clip, &mut flow, &request)?;
                    cold.load_s = load_s;
                    cold.flowscribe_load_s = flow_load_s;
                    cold.total_s = total.elapsed().as_secs_f32();
                    let warm = time_speech(engine, whisper, gguf.as_ref(), &clip, &mut flow, &request)?;
                    // The local FlowScribe engine is dropped before the next round.
                    Ok(SpeedPair { unloaded: cold, loaded: warm })
                })();
                match pair { Ok(pair) => pairs.push(pair), Err(e) => { error = Some(e); break; } }
            }
            let loaded_total_s = median(pairs.iter().map(|p| p.loaded.total_s));
            DeviceSpeed {
                device_id: d.id.clone(), name: d.name.clone(), backend: d.backend.clone(),
                load_s: median(pairs.iter().map(|p| p.unloaded.load_s)),
                transcribe_s: median(pairs.iter().map(|p| p.loaded.transcribe_s)),
                unloaded_total_s: median(pairs.iter().map(|p| p.unloaded.total_s)), loaded_total_s,
                flowscribe_load_s: median(pairs.iter().map(|p| p.unloaded.flowscribe_load_s)),
                flowscribe_s: median(pairs.iter().map(|p| p.loaded.flowscribe_s)),
                flowscribe_device_id: actual_flow_device,
                realtime: if loaded_total_s > 0.0 { clip_seconds / loaded_total_s } else { 0.0 }, trials: pairs, error,
            }
}

fn failed_device(d: &BenchmarkJob, error: String) -> DeviceSpeed {
    DeviceSpeed { device_id:d.id.clone(), name:d.name.clone(), backend:d.backend.clone(), load_s:0.0,transcribe_s:0.0,
        unloaded_total_s:0.0,loaded_total_s:0.0,flowscribe_load_s:0.0,flowscribe_s:0.0,
        flowscribe_device_id:None,realtime:0.0,trials:vec![],error:Some(error) }
}

/// Private child entry point. No GUI, single-instance lock, or shared parent model state.
pub fn run_benchmark_worker(args: &[String]) -> Result<(), String> {
    if args.len()!=3 { return Err("Invalid benchmark worker arguments".into()); }
    let job: BenchmarkJob = serde_json::from_slice(&std::fs::read(&args[0]).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let result = worker_trials(&job, std::path::Path::new(&args[2]));
    std::fs::write(&args[1], serde_json::to_vec(&result).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}

fn isolated_trial(job: &BenchmarkJob, mut progress: impl FnMut(usize)) -> Result<DeviceSpeed, String> {
    isolated_trial_with(job, &std::env::current_exe().map_err(|e|e.to_string())?, &mut progress)
}

fn isolated_trial_with(job: &BenchmarkJob, executable: &std::path::Path, progress: &mut impl FnMut(usize)) -> Result<DeviceSpeed, String> {
    use std::process::{Command, Stdio};
    let root=dirs::data_local_dir().ok_or("No application data folder")?.join("Taurscribe").join("benchmark-runs");
    std::fs::create_dir_all(&root).map_err(|e|e.to_string())?;
    let key=format!("{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e|e.to_string())?.as_nanos());
    let dir=root.join(key);std::fs::create_dir(&dir).map_err(|e|e.to_string())?;
    let request=dir.join("request.json");let result=dir.join("result.json");let rounds=dir.join("progress.txt");let log=dir.join("native.log");
    std::fs::write(&request,serde_json::to_vec(job).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let mut command=Command::new(executable);
    command.arg("--_benchmark-worker").arg(&request).arg(&result).arg(&rounds)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(std::fs::File::create(&log).map_err(|e|e.to_string())?);
    #[cfg(windows)] { use std::os::windows::process::CommandExt;command.creation_flags(0x08000000); }
    let mut child=command.spawn().map_err(|e|format!("Could not start benchmark worker: {e}"))?;
    let started=Instant::now();let mut last_round=0;
    let status=loop {
        match child.try_wait() { Ok(Some(status))=>break status,Ok(None)=>{},Err(error)=>{let _=child.kill();let _=child.wait();return Err(error.to_string());} }
        if started.elapsed()>std::time::Duration::from_secs(20*60) {
            let _=child.kill();let _=child.wait();let _=std::fs::remove_file(&request);
            return Err(format!("Speed test timed out; the app is still running. Details: {}",log.display()));
        }
        if let Ok(round)=std::fs::read_to_string(&rounds).and_then(|text|text.parse::<usize>().map_err(std::io::Error::other)) {
            if round!=last_round { last_round=round;progress(round); }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let _=std::fs::remove_file(&request); // Do not retain custom vocabulary in crash diagnostics.
    if !status.success() {return Err(format!("The native speed-test worker stopped ({status}); the app is still running. Details: {}",log.display()));}
    let report=serde_json::from_slice(&std::fs::read(&result).map_err(|e|format!("No benchmark result: {e}"))?).map_err(|e|e.to_string())?;
    let _=std::fs::remove_dir_all(dir);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn child_native_failure_returns_error_without_terminating_parent() {
        let job = BenchmarkJob { engine: ASREngine::Granite, model: "granite-speech-5-nc".into(), id:"cpu".into(), name:"CPU".into(), backend:"CPU".into(), include_flow:true,
            flowscribe_device_id:None, flowscribe_use_gpu:Some(false), request:FlowRequest { engine:"granite".into(),level:"clean".into(),app:"general".into(),vocab:vec!["private-test-token".into()],prev:None } };
        let error=isolated_trial_with(&job,std::path::Path::new("/usr/bin/false"),&mut |_|{}).unwrap_err();
        assert!(error.contains("app is still running"));
        let log=std::path::PathBuf::from(error.split("Details: ").nth(1).unwrap());
        let dir=log.parent().unwrap();
        assert!(!dir.join("request.json").exists(),"failed workers must not retain vocabulary");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn benchmark_uses_real_nonempty_bounded_speech() {
        let clip = test_clip().unwrap();
        assert!(!clip.is_empty() && clip.len() <= 11 * 16_000);
        assert_eq!(clip, test_clip().unwrap());
        assert!(clip.iter().any(|s| s.abs() > 0.1));
    }
    #[test]
    fn typical_time_uses_all_runs_instead_of_the_fastest_outlier() {
        assert_eq!(median([0.5, 9.0, 0.7].into_iter()), 0.7);
        assert_eq!(median([0.5, 0.7].into_iter()), 0.6);
    }
    #[test]
    fn cancelled_worker_clears_loading_only_when_its_guard_is_dropped() {
        let flag = Arc::new(AtomicBool::new(true));
        let guard = ClearLoading(flag.clone());
        assert!(flag.load(Ordering::Acquire));
        drop(guard);
        assert!(!flag.load(Ordering::Acquire));
    }
}
