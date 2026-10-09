//! Settings → Models speed test: compare a genuinely unloaded model with a
//! resident model on the same speech clip, optionally including FlowScribe V3.

use crate::{gpu, llm::{FlowRequest, LLMEngine}, state::AudioState};
use crate::types::{ASREngine, CommandResult};
use serde::Serialize;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Instant;
use tauri::{Emitter, State};

const ROUNDS: usize = 3;
type Shared<T> = Arc<Mutex<T>>;

#[derive(Debug, Clone, Serialize)]
pub struct SpeedTrial {
    pub load_s: f32,
    pub transcribe_s: f32,
    pub flowscribe_load_s: f32,
    pub flowscribe_s: f32,
    pub total_s: f32,
    pub transcript: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeedPair {
    pub unloaded: SpeedTrial,
    pub loaded: SpeedTrial,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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
    flowscribe_use_gpu: Option<bool>, style: Option<String>,
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
        let clip = test_clip()?;
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
        let devices: Vec<_> = gpu::compute_devices().into_iter().filter(|d| d.usable).collect();
        for (i, d) in devices.iter().enumerate() {
            let mut pairs = Vec::new();
            let mut actual_flow_device = None;
            let mut error = None;
            for round in 0..ROUNDS {
                let _ = worker_app.emit("compute-benchmark-progress", serde_json::json!({ "index": i, "total": devices.len(), "device": d.name, "round": round + 1, "rounds": ROUNDS }));
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
            results.push(DeviceSpeed {
                device_id: d.id.clone(), name: d.name.clone(), backend: d.backend.clone(),
                load_s: median(pairs.iter().map(|p| p.unloaded.load_s)),
                transcribe_s: median(pairs.iter().map(|p| p.loaded.transcribe_s)),
                unloaded_total_s: median(pairs.iter().map(|p| p.unloaded.total_s)), loaded_total_s,
                flowscribe_load_s: median(pairs.iter().map(|p| p.unloaded.flowscribe_load_s)),
                flowscribe_s: median(pairs.iter().map(|p| p.loaded.flowscribe_s)),
                flowscribe_device_id: actual_flow_device,
                realtime: if loaded_total_s > 0.0 { clip_seconds / loaded_total_s } else { 0.0 }, trials: pairs, error,
            });
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

#[cfg(test)]
mod tests {
    use super::*;
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
