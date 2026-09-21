//! Dual-Channel Audio Capture Module for Taurscribe
//!
//! Captures user microphone on Channel 1 (Left) and system / meeting audio
//! on Channel 2 (Right), generating an interleaved stereo audio stream.
//!
//! Provides:
//! - 100% physical speaker isolation between user and remote participants
//! - Standard stereo WAV output
//! - Real-time mixed mono audio stream for live transcription models
//! - Real-time dual level telemetry for frontend visualization

#[cfg(any(target_os = "macos", target_os = "windows"))]
use crossbeam_channel::bounded;
use crossbeam_channel::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::AppHandle;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::Emitter;

pub struct DualChannelCaptureHandle {
    pub stop_signal: Arc<AtomicBool>,
    pub capture_thread: std::thread::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DualChannelTarget {
    System,
    Process(u32),
}

// ── macOS & Windows Implementation ──────────────────────────────────────────

/// How often the watchdog re-checks which of the meeting app's processes do audio.
const AUDIO_PROCESS_CHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// The outermost `.app` bundle in an executable path, which an app and all of
/// its helpers share: `/Applications/Google Chrome.app/Contents/Frameworks/…/
/// Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper` →
/// `/Applications/Google Chrome.app`. Non-bundled executables use their own path.
pub(crate) fn app_bundle_path(exe: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for comp in exe.components() {
        out.push(comp);
        if comp.as_os_str().to_string_lossy().ends_with(".app") {
            return out;
        }
    }
    exe.to_path_buf()
}

/// Audio processes of the meeting app that are not yet part of the tap. The crate
/// fixes the tapped processes when capture starts; if the process that ends up
/// carrying the call was idle then (or the browser moves its audio to another
/// process later), the callers track would stay empty. A non-empty result means:
/// restart the tap so it picks the new process up.
pub(crate) fn untapped_meeting_audio_pids(
    tapped: &std::collections::HashSet<u32>,
    app: &std::path::Path,
    current: &[(u32, std::path::PathBuf)],
) -> Vec<u32> {
    current
        .iter()
        .filter(|(pid, path)| !tapped.contains(pid) && path.as_path() == app)
        .map(|(pid, _)| *pid)
        .collect()
}

/// What the current dual-channel capture is doing (exposed on /api/status).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CaptureDiagnostics {
    pub active: bool,
    pub target: String,
    pub watchdog_app: Option<String>,
    pub tapped_pids: Vec<u32>,
    pub tap_restarts: u32,
    pub fell_back_to_system: bool,
    pub dropped_file_seconds: f64,
    pub dropped_transcriber_seconds: f64,
}

static DIAGNOSTICS: std::sync::Mutex<Option<CaptureDiagnostics>> = std::sync::Mutex::new(None);

/// Snapshot of the current (or last) dual-channel capture's diagnostics.
pub fn capture_diagnostics() -> Option<CaptureDiagnostics> {
    DIAGNOSTICS.lock().ok().and_then(|d| d.clone())
}

fn update_diagnostics(f: impl FnOnce(&mut CaptureDiagnostics)) {
    if let Ok(mut guard) = DIAGNOSTICS.lock() {
        f(guard.get_or_insert_with(CaptureDiagnostics::default));
    }
}

/// One capture session plus the reader threads feeding its two tracks.
#[cfg(any(target_os = "macos", target_os = "windows"))]
struct CaptureRig {
    session: Arc<meeting_record::CaptureSession>,
    mic_rx: crossbeam_channel::Receiver<Vec<f32>>,
    sys_rx: crossbeam_channel::Receiver<Vec<f32>>,
    mic_thread: std::thread::JoinHandle<()>,
    sys_thread: std::thread::JoinHandle<()>,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl CaptureRig {
    /// Stops the session (which unblocks the readers) and joins them.
    fn shutdown(self) -> (crossbeam_channel::Receiver<Vec<f32>>, crossbeam_channel::Receiver<Vec<f32>>) {
        self.session.stop();
        let _ = self.mic_thread.join();
        let _ = self.sys_thread.join();
        (self.mic_rx, self.sys_rx)
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn start_rig(target: DualChannelTarget, stop_signal: &Arc<AtomicBool>) -> Result<CaptureRig, String> {
    use meeting_record::{capture, CaptureOptions, CaptureTarget, MicrophoneSource};

    let capture_target = match target {
        DualChannelTarget::System => CaptureTarget::System,
        DualChannelTarget::Process(pid) => CaptureTarget::Process { pid },
    };

    let session = Arc::new(
        capture::start(
            capture_target,
            CaptureOptions {
                mono: true,
                microphone: Some(MicrophoneSource::Default),
            },
        )
        .map_err(|e| format!("Failed to initialize dual-channel audio capture: {}", e))?,
    );

    println!(
        "[INFO] Dual-channel capture session initialized (target: {:?})",
        capture_target
    );

    let (mic_tx, mic_rx) = bounded::<Vec<f32>>(256);
    let (sys_tx, sys_rx) = bounded::<Vec<f32>>(256);

    let session_mic = session.clone();
    let stop_mic = stop_signal.clone();
    let mic_thread = std::thread::spawn(move || {
        crate::platform_tuning::apply_thread_performance_affinity();
        let mut mic_frames = 0usize;
        let mic_started = std::time::Instant::now();
        if let Some(track) = session_mic.microphone() {
            println!(
                "[INFO] Dual-channel mic track started: rate={:.1}Hz, channels={}",
                track.sample_rate(),
                track.channels()
            );
            while !stop_mic.load(Ordering::Relaxed) {
                match track.recv() {
                    Some(chunk) => {
                        let mono_48k = process_chunk_to_mono_48k(
                            chunk.frames,
                            chunk.channels,
                            chunk.sample_rate,
                        );
                        mic_frames += mono_48k.len();
                        if mic_tx.send(mono_48k).is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
        println!("[CAPTURE] mic delivered {:.2}s of 48k samples over {:.2}s wall", mic_frames as f64 / 48_000.0, mic_started.elapsed().as_secs_f64());
    });

    let session_sys = session.clone();
    let stop_sys = stop_signal.clone();
    let sys_thread = std::thread::spawn(move || {
        crate::platform_tuning::apply_thread_performance_affinity();
        let mut sys_frames = 0usize;
        let sys_started = std::time::Instant::now();
        let track = session_sys.system_audio();
        println!(
            "[INFO] Dual-channel sys track started: rate={:.1}Hz, channels={}",
            track.sample_rate(),
            track.channels()
        );
        while !stop_sys.load(Ordering::Relaxed) {
            match track.recv() {
                Some(chunk) => {
                    let mono_48k = process_chunk_to_mono_48k(
                        chunk.frames,
                        chunk.channels,
                        chunk.sample_rate,
                    );
                    sys_frames += mono_48k.len();
                    if sys_tx.send(mono_48k).is_err() {
                        break;
                    }
                }
                None => break,
            }
        }
        println!("[CAPTURE] callers delivered {:.2}s of 48k samples over {:.2}s wall", sys_frames as f64 / 48_000.0, sys_started.elapsed().as_secs_f64());
    });

    Ok(CaptureRig { session, mic_rx, sys_rx, mic_thread, sys_thread })
}

/// (pid, owning app bundle path) of every process currently doing audio IO.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn current_audio_processes(sys: &mut sysinfo::System) -> Vec<(u32, std::path::PathBuf)> {
    let pids: Vec<u32> = meeting_record::meetings::audio_processes().into_iter().map(|p| p.pid).collect();
    pids.into_iter()
        .filter_map(|pid| app_path_of(sys, pid).map(|path| (pid, path)))
        .collect()
}

/// App bundle path of any process, whether or not it is doing audio.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn app_path_of(sys: &mut sysinfo::System, pid: u32) -> Option<std::path::PathBuf> {
    let spid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[spid]), true);
    sys.process(spid).and_then(|p| p.exe()).map(app_bundle_path)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn start_dual_channel_capture(
    target: DualChannelTarget,
    _sample_rate: u32,
    file_tx: Sender<Vec<f32>>,
    whisper_tx: Sender<Vec<f32>>,
    app_handle: AppHandle,
    stop_signal: Arc<AtomicBool>,
) -> Result<DualChannelCaptureHandle, String> {
    use std::collections::VecDeque;

    let rig = start_rig(target, &stop_signal)?;

    let stop_combiner = stop_signal.clone();
    let stop_for_rig = stop_signal.clone();
    let capture_thread = std::thread::spawn(move || {
        crate::platform_tuning::apply_thread_performance_affinity();

        let mut rig = rig;
        let mut mic_deque: VecDeque<f32> = VecDeque::with_capacity(48000);
        let mut sys_deque: VecDeque<f32> = VecDeque::with_capacity(48000);

        // try_send never blocks this capture thread, so a full queue drops audio.
        // Count it (per queue) so a lagging transcriber or writer is visible.
        let mut dropped_transcriber_samples: u64 = 0;
        let mut dropped_file_samples: u64 = 0;
        let mut warned_drop = false;

        // Callers-track watchdog (meeting-process captures only): the tap covers
        // the meeting app's processes that did audio at start; restart it when
        // another process of that app starts doing audio.
        let mut sys = sysinfo::System::new();
        let meeting_app: Option<std::path::PathBuf> = match target {
            DualChannelTarget::Process(pid) => app_path_of(&mut sys, pid),
            DualChannelTarget::System => None,
        };
        let snapshot_tapped = |sys: &mut sysinfo::System, app: &std::path::Path| -> std::collections::HashSet<u32> {
            current_audio_processes(sys)
                .into_iter()
                .filter(|(_, path)| path.as_path() == app)
                .map(|(pid, _)| pid)
                .collect()
        };
        let mut tapped: std::collections::HashSet<u32> = meeting_app
            .as_deref()
            .map(|app| snapshot_tapped(&mut sys, app))
            .unwrap_or_default();
        let mut last_process_check = std::time::Instant::now();
        match (&meeting_app, target) {
            (Some(app), _) => println!("[INFO] Callers track watchdog on {} (tapped pids {:?})", app.display(), tapped),
            (None, DualChannelTarget::Process(pid)) => {
                println!("[INFO] Callers track watchdog off: cannot resolve the app of pid {}", pid)
            }
            _ => {}
        }
        *DIAGNOSTICS.lock().unwrap() = Some(CaptureDiagnostics {
            active: true,
            target: format!("{:?}", target),
            watchdog_app: meeting_app.as_ref().map(|a| a.display().to_string()),
            tapped_pids: tapped.iter().copied().collect(),
            ..Default::default()
        });

        let mut last_level_emit = std::time::Instant::now();
        let mut mic_sum_sq = 0.0f32;
        let mut sys_sum_sq = 0.0f32;
        let mut level_samples = 0usize;

        const CHUNK_SIZE: usize = 960; // 20ms @ 48kHz

        while !stop_combiner.load(Ordering::Relaxed) {
            let mut got_anything = false;

            // Drain incoming microphone packets
            while let Ok(frames) = rig.mic_rx.try_recv() {
                got_anything = true;
                for s in frames {
                    mic_deque.push_back(s);
                    mic_sum_sq += s * s;
                }
            }

            // Drain incoming system audio packets
            while let Ok(frames) = rig.sys_rx.try_recv() {
                got_anything = true;
                for s in frames {
                    sys_deque.push_back(s);
                    sys_sum_sq += s * s;
                }
            }

            if let (Some(app), DualChannelTarget::Process(pid)) = (&meeting_app, target) {
                if last_process_check.elapsed() >= AUDIO_PROCESS_CHECK {
                    last_process_check = std::time::Instant::now();
                    let new_pids = untapped_meeting_audio_pids(&tapped, app, &current_audio_processes(&mut sys));
                    if !new_pids.is_empty() {
                        eprintln!(
                            "[WARN] Meeting app started audio in process(es) {:?} outside the callers tap; restarting the tap",
                            new_pids
                        );
                        let (old_mic_rx, old_sys_rx) = restart_rig(&mut rig, DualChannelTarget::Process(pid), &stop_for_rig);
                        // Keep whatever the old session had already delivered.
                        while let Ok(frames) = old_mic_rx.try_recv() {
                            mic_deque.extend(frames);
                        }
                        while let Ok(frames) = old_sys_rx.try_recv() {
                            sys_deque.extend(frames);
                        }
                        tapped = snapshot_tapped(&mut sys, app);
                        tapped.extend(new_pids);
                        let pids: Vec<u32> = tapped.iter().copied().collect();
                        update_diagnostics(|d| {
                            d.tap_restarts += 1;
                            d.tapped_pids = pids;
                        });
                    }
                }
            }

            // Arrival jitter is not missing audio. Padding the shorter queue on
            // every poll accumulated silence repeatedly and made a 38-second
            // meeting play back as ~57 seconds. Pair only frames that both tracks
            // actually delivered; pad a final unmatched tail once on stop below.

            // Interleave available matching frames in steady 20ms chunks
            let available = mic_deque.len().min(sys_deque.len());
            if available >= CHUNK_SIZE {
                let chunks_to_process = available / CHUNK_SIZE;
                for _ in 0..chunks_to_process {
                    let mut interleaved = Vec::with_capacity(CHUNK_SIZE * 2);
                    let mut mono_mix = Vec::with_capacity(CHUNK_SIZE);

                    for _ in 0..CHUNK_SIZE {
                        let m = mic_deque.pop_front().unwrap_or(0.0);
                        let s = sys_deque.pop_front().unwrap_or(0.0);

                        // Channel 1: Left ear = User Microphone
                        interleaved.push(m);
                        // Channel 2: Right ear = System / Remote caller
                        interleaved.push(s);

                        // Equal-power mix for real-time transcription feed
                        mono_mix.push((m * 0.5) + (s * 0.5));
                    }

                    level_samples += CHUNK_SIZE;

                    if file_tx.try_send(interleaved).is_err() {
                        dropped_file_samples += CHUNK_SIZE as u64;
                    }
                    if whisper_tx.try_send(mono_mix).is_err() {
                        dropped_transcriber_samples += CHUNK_SIZE as u64;
                    }
                    if !warned_drop && (dropped_file_samples > 0 || dropped_transcriber_samples > 0) {
                        warned_drop = true;
                        eprintln!(
                            "[WARN] Dual-channel capture is dropping audio: {} (queue full)",
                            if dropped_file_samples > 0 { "recording file" } else { "live transcriber" }
                        );
                    }
                }
            }

            // Emit dual-channel level indicators to UI (~20 updates/sec)
            if last_level_emit.elapsed() >= std::time::Duration::from_millis(50) {
                if level_samples > 0 {
                    let mic_rms = (mic_sum_sq / level_samples as f32).sqrt().min(1.0);
                    let sys_rms = (sys_sum_sq / level_samples as f32).sqrt().min(1.0);

                    let _ = app_handle.emit(
                        "dual-audio-levels",
                        serde_json::json!({
                            "mic": mic_rms,
                            "system": sys_rms
                        }),
                    );
                }
                mic_sum_sq = 0.0;
                sys_sum_sq = 0.0;
                level_samples = 0;
                last_level_emit = std::time::Instant::now();
            }

            if !got_anything || mic_deque.len() < CHUNK_SIZE || sys_deque.len() < CHUNK_SIZE {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        // On stop: close session to unblock reader threads
        let (mic_rx, sys_rx) = rig.shutdown();

        // Drain any remaining buffered frames
        while let Ok(frames) = mic_rx.try_recv() {
            mic_deque.extend(frames);
        }
        while let Ok(frames) = sys_rx.try_recv() {
            sys_deque.extend(frames);
        }

        let remaining = mic_deque.len().max(sys_deque.len());
        if remaining > 0 {
            let mut interleaved = Vec::with_capacity(remaining * 2);
            for _ in 0..remaining {
                let m = mic_deque.pop_front().unwrap_or(0.0);
                let s = sys_deque.pop_front().unwrap_or(0.0);
                interleaved.push(m);
                interleaved.push(s);
            }
            if file_tx.try_send(interleaved).is_err() {
                dropped_file_samples += remaining as u64;
            }
        }

        let secs = |n: u64| n as f64 / 48_000.0;
        update_diagnostics(|d| {
            d.active = false;
            d.dropped_file_seconds = secs(dropped_file_samples);
            d.dropped_transcriber_seconds = secs(dropped_transcriber_samples);
        });
        if dropped_file_samples > 0 || dropped_transcriber_samples > 0 {
            eprintln!(
                "[WARN] Dual-channel audio dropped: {:.1}s from the recording file, {:.1}s from the live transcriber",
                secs(dropped_file_samples),
                secs(dropped_transcriber_samples)
            );
        } else {
            println!("[INFO] Dual-channel capture: no audio dropped");
        }
        println!("[INFO] Dual-channel capture thread cleanly terminated");
    });

    Ok(DualChannelCaptureHandle {
        stop_signal,
        capture_thread,
    })
}

/// Replaces `rig` with a fresh session on `target` and returns the old session's
/// receivers (already shut down) so their buffered audio can be kept. The crate
/// resolves a process target's audio processes when a session starts, so a restart
/// is how newly active processes get included. If the new session cannot start,
/// falls back to system audio, and failing that leaves the callers track silent.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn restart_rig(
    rig: &mut CaptureRig,
    target: DualChannelTarget,
    stop_signal: &Arc<AtomicBool>,
) -> (crossbeam_channel::Receiver<Vec<f32>>, crossbeam_channel::Receiver<Vec<f32>>) {
    // The crate allows one capture session at a time: stop before starting anew.
    rig.session.stop();
    let fresh = start_rig(target, stop_signal).or_else(|e| {
        eprintln!("[WARN] Restarting the meeting tap failed ({}); falling back to system audio", e);
        update_diagnostics(|d| d.fell_back_to_system = true);
        start_rig(DualChannelTarget::System, stop_signal)
    });
    match fresh {
        Ok(new_rig) => std::mem::replace(rig, new_rig).shutdown(),
        Err(e) => {
            eprintln!("[ERROR] Could not restart dual-channel capture ({}); callers track stays silent", e);
            let (_, empty_mic) = bounded::<Vec<f32>>(1);
            let (_, empty_sys) = bounded::<Vec<f32>>(1);
            (empty_mic, empty_sys)
        }
    }
}

/// Downmix to mono and resample to 48 kHz if needed to ensure both channels
/// are perfectly rate-matched before interleaving.
fn process_chunk_to_mono_48k(frames: Vec<f32>, channels: u32, sample_rate: f64) -> Vec<f32> {
    if frames.is_empty() {
        return Vec::new();
    }

    // 1. Downmix to mono if multi-channel
    let mono = if channels > 1 {
        let ch = channels as usize;
        frames
            .chunks(ch)
            .map(|chunk| chunk.iter().sum::<f32>() / ch as f32)
            .collect()
    } else {
        frames
    };

    // 2. Resample to 48,000 Hz if needed
    let target_rate = 48000.0;
    if (sample_rate - target_rate).abs() > 1.0 && sample_rate > 1000.0 {
        resample_linear(&mono, sample_rate, target_rate)
    } else {
        mono
    }
}

/// Fast, low-latency linear interpolation resampler for streaming audio buffers.
fn resample_linear(input: &[f32], from_rate: f64, to_rate: f64) -> Vec<f32> {
    if input.is_empty() || (from_rate - to_rate).abs() < 1.0 {
        return input.to_vec();
    }
    let ratio = from_rate / to_rate;
    let out_len = ((input.len() as f64) / ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_idx = (i as f64) * ratio;
        let idx0 = src_idx.floor() as usize;
        let frac = (src_idx - idx0 as f64) as f32;
        let idx1 = (idx0 + 1).min(input.len().saturating_sub(1));
        let s0 = input.get(idx0).copied().unwrap_or(0.0);
        let s1 = input.get(idx1).copied().unwrap_or(s0);
        out.push(s0 + frac * (s1 - s0));
    }
    out
}

// ── Linux Fallback Implementation ───────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn start_dual_channel_capture(
    _target: DualChannelTarget,
    _sample_rate: u32,
    _file_tx: Sender<Vec<f32>>,
    _whisper_tx: Sender<Vec<f32>>,
    _app_handle: AppHandle,
    _stop_signal: Arc<AtomicBool>,
) -> Result<DualChannelCaptureHandle, String> {
    Err("Dual-channel meeting audio capture is not supported on Linux".into())
}

#[cfg(test)]
mod tests {
    use super::{app_bundle_path, untapped_meeting_audio_pids};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    #[test]
    fn app_bundle_path_groups_an_app_with_its_helpers() {
        let chrome = PathBuf::from("/Applications/Google Chrome.app");
        assert_eq!(app_bundle_path(Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")), chrome);
        assert_eq!(
            app_bundle_path(Path::new(
                "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/1/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper"
            )),
            chrome
        );
        assert_eq!(app_bundle_path(Path::new("/usr/bin/afplay")), PathBuf::from("/usr/bin/afplay"));
    }

    #[test]
    fn watchdog_flags_only_new_processes_of_the_meeting_app() {
        let chrome = PathBuf::from("/Applications/Google Chrome.app");
        let tapped: HashSet<u32> = [100].into_iter().collect();
        let current = vec![
            (100, chrome.clone()),                                   // already tapped
            (200, chrome.clone()),                                   // new Chrome audio -> restart
            (300, PathBuf::from("/Applications/Spotify.app")),       // other app -> ignored
        ];
        assert_eq!(untapped_meeting_audio_pids(&tapped, &chrome, &current), vec![200]);
    }

    #[test]
    fn watchdog_is_quiet_when_nothing_new_is_playing() {
        let chrome = PathBuf::from("/Applications/Google Chrome.app");
        let tapped: HashSet<u32> = [100, 200].into_iter().collect();
        let current = vec![(200, chrome.clone())];
        assert!(untapped_meeting_audio_pids(&tapped, &chrome, &current).is_empty());
    }

    #[test]
    fn test_stereo_interleaving() {
        let mic = vec![0.1f32, 0.2, 0.3];
        let sys = vec![0.7f32, 0.8, 0.9];
        let mut interleaved = Vec::with_capacity(mic.len() * 2);
        let mut mono_mix = Vec::with_capacity(mic.len());
        for i in 0..mic.len() {
            interleaved.push(mic[i]);
            interleaved.push(sys[i]);
            mono_mix.push((mic[i] * 0.5) + (sys[i] * 0.5));
        }
        assert_eq!(interleaved, vec![0.1, 0.7, 0.2, 0.8, 0.3, 0.9]);
        assert_eq!(mono_mix, vec![0.4, 0.5, 0.6]);
    }

    #[test]
    fn test_resample_linear_and_downmix() {
        let stereo = vec![0.2, 0.4, 0.6, 0.8]; // 2 frames of stereo
        let mono_48k = super::process_chunk_to_mono_48k(stereo, 2, 48000.0);
        assert_eq!(mono_48k.len(), 2);
        assert!((mono_48k[0] - 0.3).abs() < 1e-5);
        assert!((mono_48k[1] - 0.7).abs() < 1e-5);

        // 44.1k to 48k resampling length test
        let input_44k = vec![0.5f32; 4410]; // 100ms at 44.1kHz
        let resampled_48k = super::resample_linear(&input_44k, 44100.0, 48000.0);
        assert_eq!(resampled_48k.len(), 4800); // 100ms at 48kHz
        for &s in &resampled_48k {
            assert!((s - 0.5).abs() < 1e-4);
        }
    }
}
