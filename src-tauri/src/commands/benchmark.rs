//! Settings → Models "Measure speed": runs the loaded speech model on every
//! device it can use, so the user can see which one transcribes fastest.

use crate::gpu;
use crate::state::AudioState;
use crate::types::{ASREngine, CommandResult};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::{Emitter, State};

/// Seconds of audio each device transcribes.
const CLIP_SECONDS: usize = 30;

#[derive(Debug, Clone, Serialize)]
pub struct DeviceSpeed {
    pub device_id: String,
    pub name: String,
    pub backend: String,
    /// Seconds to load the model on this device.
    pub load_s: f32,
    /// Seconds to transcribe the clip.
    pub transcribe_s: f32,
    /// Seconds of audio per second of compute (higher is faster).
    pub realtime: f32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeedReport {
    pub engine: ASREngine,
    pub model: String,
    pub clip_seconds: f32,
    pub results: Vec<DeviceSpeed>,
}

/// The same audio on every device: voiced, syllable-paced sound (harmonics
/// under a ~4 Hz envelope, with pauses), so the encoder and decoder both work
/// as on speech. Deterministic, so runs are comparable.
fn test_clip() -> Vec<f32> {
    let rate = 16_000.0_f32;
    let mut seed = 0x2545_f491_u32;
    (0..CLIP_SECONDS * 16_000)
        .map(|i| {
            let t = i as f32 / rate;
            let pitch = 120.0 + 30.0 * (t * 0.7).sin();
            let voice: f32 = (1..=8).map(|h| (std::f32::consts::TAU * pitch * h as f32 * t).sin() / h as f32).sum();
            let syllable = (std::f32::consts::TAU * 4.0 * t).sin().max(0.0);
            let phrase = if (t % 3.0) < 2.4 { 1.0 } else { 0.0 };
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.02;
            0.15 * voice * syllable * phrase + noise
        })
        .collect()
}

/// Restores the saved device choice however the benchmark ends.
struct RestoreDevice(String);
impl Drop for RestoreDevice {
    fn drop(&mut self) {
        gpu::set_preferred_device(&self.0);
    }
}

/// The faster of two runs, in seconds. The first run on a GPU can include
/// one-time work (Vulkan compiles shaders for each new input size).
fn fastest_of_two(mut run: impl FnMut() -> Result<(), String>) -> Result<f32, String> {
    let mut best = f32::MAX;
    for _ in 0..2 {
        let started = Instant::now();
        run()?;
        best = best.min(started.elapsed().as_secs_f32());
    }
    Ok(best)
}

type Shared<T> = std::sync::Arc<std::sync::Mutex<T>>;

/// Loads `model` on `device`, then times `clip` (when given).
fn run_on(engine: ASREngine, whisper: &Shared<crate::whisper::WhisperManager>, gguf: Option<&Shared<crate::gguf_asr::GgufAsrManager>>, model: &str, device: &str, clip: Option<&[f32]>) -> Result<(f32, f32), String> {
    gpu::set_preferred_device(device);
    let force_cpu = device == gpu::CPU_DEVICE;
    let started = Instant::now();
    match engine {
        ASREngine::Whisper => {
            let mut w = whisper.lock().map_err(|e| e.to_string())?;
            w.initialize(Some(model), force_cpu)?;
            let load_s = started.elapsed().as_secs_f32();
            let Some(clip) = clip else { return Ok((load_s, 0.0)) };
            let best = fastest_of_two(|| {
                w.clear_context();
                w.transcribe_chunk(clip, 16_000).map(|_| ())
            })?;
            Ok((load_s, best))
        }
        _ => {
            let mut g = gguf.ok_or("No such engine")?.lock().map_err(|e| e.to_string())?;
            g.initialize(Some(model), force_cpu)?;
            let load_s = started.elapsed().as_secs_f32();
            let Some(clip) = clip else { return Ok((load_s, 0.0)) };
            let best = fastest_of_two(|| g.transcribe_audio_data(clip, None).map(|_| ()))?;
            Ok((load_s, best))
        }
    }
}

/// Loads the current model on each device, times a 30 s clip, then reloads
/// it on the saved device. Emits `compute-benchmark-progress` per device.
#[tauri::command]
pub async fn benchmark_compute_devices(state: State<'_, AudioState>, app: tauri::AppHandle) -> Result<CommandResult<SpeedReport>, String> {
    if state.recording_handle.lock().unwrap().is_some() {
        return Ok(CommandResult::err("already_recording", "Stop recording first"));
    }
    if super::file_job_active() {
        return Ok(CommandResult::err("busy", "Wait for the file transcription to finish"));
    }
    let _model_operation = state.begin_exclusive_model_operation()?;
    if state.engine_loading.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        return Ok(CommandResult::err("engine_loading", "A model is already loading — please wait"));
    }

    let engine = *state.active_engine.lock().unwrap();
    let whisper = state.whisper.clone();
    let gguf = state.gguf_manager(engine);
    let model = match engine {
        ASREngine::Whisper => state.whisper_snapshot().model,
        other => state.gguf_snapshot(other).filter(|s| s.loaded).and_then(|s| s.model_id),
    };
    let Some(model) = model else {
        state.engine_loading.store(false, Ordering::Relaxed);
        return Ok(CommandResult::err("model_missing", "Load a speech model first"));
    };

    let result = tauri::async_runtime::spawn_blocking(move || {
        let saved = gpu::preferred_device();
        let _restore = RestoreDevice(saved.clone());
        // Reload afterwards exactly as it was loaded (CPU if it was on the CPU).
        let loaded_on = match &gguf {
            None => whisper.lock().map(|w| w.device_key().to_string()).unwrap_or_default(),
            Some(g) => g.lock().map(|g| g.device_key().to_string()).unwrap_or_default(),
        };
        let devices: Vec<_> = gpu::compute_devices().into_iter().filter(|d| d.usable).collect();
        let clip = test_clip();
        let mut results = Vec::new();
        for (i, d) in devices.iter().enumerate() {
            let _ = app.emit("compute-benchmark-progress", serde_json::json!({ "index": i, "total": devices.len(), "device": d.name }));
            println!("[BENCH] {} on {} ({})", model, d.name, d.backend);
            let (load_s, transcribe_s, error) = match run_on(engine, &whisper, gguf.as_ref(), &model, &d.id, Some(&clip)) {
                Ok((l, t)) => (l, t, None),
                Err(e) => (0.0, 0.0, Some(e)),
            };
            if let Some(e) = &error {
                println!("[BENCH] {} failed: {e}", d.name);
            } else {
                println!("[BENCH] {}: load {load_s:.2}s, {CLIP_SECONDS}s clip in {transcribe_s:.2}s", d.name);
            }
            results.push(DeviceSpeed {
                device_id: d.id.clone(),
                name: d.name.clone(),
                backend: d.backend.clone(),
                load_s,
                transcribe_s,
                realtime: if transcribe_s > 0.0 { CLIP_SECONDS as f32 / transcribe_s } else { 0.0 },
                error,
            });
        }
        // Back on the device it was on before.
        let device = if loaded_on == gpu::CPU_DEVICE { gpu::CPU_DEVICE } else { saved.as_str() };
        let reload = run_on(engine, &whisper, gguf.as_ref(), &model, device, None);
        if let Err(e) = reload {
            println!("[BENCH] Reloading on the saved device failed: {e}");
        }
        SpeedReport { engine, model, clip_seconds: CLIP_SECONDS as f32, results }
    })
    .await
    .map_err(|e| e.to_string());
    state.engine_loading.store(false, Ordering::Relaxed);
    result.map(CommandResult::ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clip_is_deterministic_and_audible() {
        let a = test_clip();
        assert_eq!(a.len(), CLIP_SECONDS * 16_000);
        assert_eq!(a, test_clip());
        let peak = a.iter().fold(0.0_f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.1 && peak <= 1.0, "peak {peak}");
    }
}
