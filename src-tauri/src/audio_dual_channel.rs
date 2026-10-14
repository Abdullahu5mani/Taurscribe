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
use std::collections::VecDeque;
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
    /// Silence put in for a track that stopped delivering (see fill_stalled_track).
    pub gap_filled_seconds: f64,
    pub call_audio_seen: bool,
    pub call_audio_warning: bool,
}

/// Only a never-heard track is suspect. Quiet participants after speech are
/// normal; zero-valued packets are distinct from a stopped native device.
#[derive(Default)]
struct CallAudioHealth {
    seen: bool,
    warned: bool,
}

impl CallAudioHealth {
    fn observe(&mut self, frames: &[f32]) -> bool {
        if frames.iter().any(|s| s.is_finite() && s.abs() > 0.000001) {
            self.seen = true;
            return std::mem::take(&mut self.warned);
        }
        false
    }
    fn needs_warning(&mut self, elapsed: std::time::Duration) -> bool {
        if !self.seen && !self.warned && elapsed >= std::time::Duration::from_secs(15) {
            self.warned = true;
            true
        } else { false }
    }
}

/// A track this far behind the other has stopped delivering (a stalled or
/// restarted tap), not jittered: arrival jitter is tens of milliseconds.
const MAX_TRACK_LAG: usize = 48_000; // 1 s @ 48 kHz

/// When one track is more than MAX_TRACK_LAG behind the other, fills the gap
/// with silence so the other track is still recorded in step, and the backlog
/// can't grow without bound. Returns the samples filled.
fn fill_stalled_track(mic: &mut VecDeque<f32>, sys: &mut VecDeque<f32>) -> usize {
    let lag = mic.len().abs_diff(sys.len());
    if lag <= MAX_TRACK_LAG {
        return 0;
    }
    let behind = if mic.len() < sys.len() { mic } else { sys };
    behind.extend(std::iter::repeat(0.0).take(lag));
    lag
}

/// Pairs the tracks into the stereo file frames (mic left, call right) and the
/// mono mix the live transcriber gets.
fn interleave_and_mix(mic: &mut VecDeque<f32>, sys: &mut VecDeque<f32>, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut interleaved = Vec::with_capacity(frames * 2);
    let mut mono_mix = Vec::with_capacity(frames);
    for _ in 0..frames {
        let m = mic.pop_front().unwrap_or(0.0);
        let s = sys.pop_front().unwrap_or(0.0);
        interleaved.push(m);
        interleaved.push(s);
        mono_mix.push((m * 0.5) + (s * 0.5));
    }
    (interleaved, mono_mix)
}

/// Takes `frames` of each track, removes the callers' echo from the mic, and
/// pairs what comes out (see `interleave_and_mix`). The canceller holds back a
/// short lead, so early calls can return nothing; `flush` empties it on stop.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn cancel_echo_and_mix(
    aec: &mut crate::echo_cancel::EchoCanceller,
    mic: &mut VecDeque<f32>,
    sys: &mut VecDeque<f32>,
    frames: usize,
    flush: bool,
) -> (Vec<f32>, Vec<f32>) {
    let take = |q: &mut VecDeque<f32>| (0..frames).map(|_| q.pop_front().unwrap_or(0.0)).collect::<Vec<f32>>();
    let (mic_in, sys_in) = (take(mic), take(sys));
    let (mut mic_out, mut sys_out) = aec.push(&mic_in, &sys_in);
    if flush {
        let (m, s) = aec.flush();
        mic_out.extend(m);
        sys_out.extend(s);
    }
    let n = mic_out.len();
    interleave_and_mix(&mut mic_out.into(), &mut sys_out.into(), n)
}

static DIAGNOSTICS: std::sync::Mutex<Option<CaptureDiagnostics>> = std::sync::Mutex::new(None);
static SYSTEM_AUDIO_RETRY: AtomicBool = AtomicBool::new(false);

pub fn request_system_audio_retry() -> Result<(), String> {
    if !capture_diagnostics().is_some_and(|d| d.active) {
        return Err("No meeting recording is active".into());
    }
    SYSTEM_AUDIO_RETRY.store(true, Ordering::Release);
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn emit_call_audio_status(app: &AppHandle, status: &str, message: &str) {
    let _ = app.emit("meeting-audio-status", serde_json::json!({ "status": status, "message": message }));
}

/// Snapshot of the current (or last) dual-channel capture's diagnostics.
pub fn capture_diagnostics() -> Option<CaptureDiagnostics> {
    DIAGNOSTICS.lock().ok().and_then(|d| d.clone())
}

pub fn mark_system_fallback() {
    update_diagnostics(|d| d.fell_back_to_system = true);
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
    mic_thread: Option<std::thread::JoinHandle<()>>,
    sys_thread: Option<std::thread::JoinHandle<()>>,
    /// Stops a microphone opened through cpal (a mic other than the default).
    mic_stop: Arc<AtomicBool>,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl CaptureRig {
    /// Drain while joining: stopping the native tracks does not unblock a
    /// reader already sending to one of our full bounded queues.
    fn shutdown(&mut self) -> (crossbeam_channel::Receiver<Vec<f32>>, crossbeam_channel::Receiver<Vec<f32>>) {
        self.session.stop();
        self.mic_stop.store(true, Ordering::Relaxed);
        drain_capture_readers(&self.mic_rx, &self.sys_rx, self.mic_thread.take(), self.sys_thread.take())
    }
}

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
fn drain_capture_readers(
    mic: &crossbeam_channel::Receiver<Vec<f32>>,
    sys: &crossbeam_channel::Receiver<Vec<f32>>,
    mic_thread: Option<std::thread::JoinHandle<()>>,
    sys_thread: Option<std::thread::JoinHandle<()>>,
) -> (crossbeam_channel::Receiver<Vec<f32>>, crossbeam_channel::Receiver<Vec<f32>>) {
    let (mic_tx, mic_tail) = crossbeam_channel::unbounded();
    let (sys_tx, sys_tail) = crossbeam_channel::unbounded();
    let drain = || {
        for frames in mic.try_iter() { let _ = mic_tx.send(frames); }
        for frames in sys.try_iter() { let _ = sys_tx.send(frames); }
    };
    while mic_thread.as_ref().is_some_and(|t| !t.is_finished())
        || sys_thread.as_ref().is_some_and(|t| !t.is_finished())
    {
        drain();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if let Some(thread) = mic_thread { let _ = thread.join(); }
    if let Some(thread) = sys_thread { let _ = thread.join(); }
    drain();
    (mic_tail, sys_tail)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn start_rig(target: DualChannelTarget, stop_signal: &Arc<AtomicBool>, mic_device: Option<&str>, loss: Arc<crate::audio::CaptureLoss>) -> Result<CaptureRig, String> {
    use meeting_record::{capture, CaptureOptions, CaptureTarget, MicrophoneSource};

    let capture_target = match target {
        DualChannelTarget::System => CaptureTarget::System,
        DualChannelTarget::Process(pid) => CaptureTarget::Process { pid },
    };

    let (mic_tx, mic_rx) = bounded::<Vec<f32>>(256);
    let (sys_tx, sys_rx) = bounded::<Vec<f32>>(256);

    // meeting-record can only open the system default microphone, so a mic
    // chosen in Settings is captured through cpal instead.
    let mic_stop = Arc::new(AtomicBool::new(false));
    let custom_mic = mic_device.and_then(|name| {
        spawn_named_mic_capture(name, mic_tx.clone(), [stop_signal.clone(), mic_stop.clone()], loss.clone())
            .unwrap_or_else(|e| {
                eprintln!("[WARN] Selected microphone '{}' unavailable for the meeting ({}); using the default", name, e);
                None
            })
    });

    let session = capture::start(
        capture_target,
        CaptureOptions {
            mono: true,
            microphone: if custom_mic.is_some() { None } else { Some(MicrophoneSource::Default) },
        },
    )
    .map_err(|e| format!("Failed to initialize dual-channel audio capture: {}", e));
    let session = match session {
        Ok(s) => Arc::new(s),
        Err(e) => {
            mic_stop.store(true, Ordering::Relaxed);
            if let Some(h) = custom_mic {
                let _ = h.join();
            }
            return Err(e);
        }
    };

    println!(
        "[INFO] Dual-channel capture session initialized (target: {:?})",
        capture_target
    );

    let session_mic = session.clone();
    let stop_mic = stop_signal.clone();
    let mic_thread = if let Some(handle) = custom_mic { handle } else { std::thread::spawn(move || {
        crate::platform_tuning::apply_thread_performance_affinity();
        let mut mic_frames = 0usize;
        let mic_started = std::time::Instant::now();
        let mut to_48k = MonoTo48k::new();
        if let Some(track) = session_mic.microphone() {
            println!(
                "[INFO] Dual-channel mic track started: rate={:.1}Hz, channels={}",
                track.sample_rate(),
                track.channels()
            );
            while !stop_mic.load(Ordering::Relaxed) {
                match track.recv() {
                    Some(chunk) => {
                        let mono_48k = to_48k.push(&chunk.frames, chunk.channels, chunk.sample_rate);
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
    }) };

    let session_sys = session.clone();
    let stop_sys = stop_signal.clone();
    let sys_thread = std::thread::spawn(move || {
        crate::platform_tuning::apply_thread_performance_affinity();
        let mut sys_frames = 0usize;
        let sys_started = std::time::Instant::now();
        let mut to_48k = MonoTo48k::new();
        let track = session_sys.system_audio();
        println!(
            "[INFO] Dual-channel sys track started: rate={:.1}Hz, channels={}",
            track.sample_rate(),
            track.channels()
        );
        while !stop_sys.load(Ordering::Relaxed) {
            match track.recv() {
                Some(chunk) => {
                    let mono_48k = to_48k.push(&chunk.frames, chunk.channels, chunk.sample_rate);
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

    Ok(CaptureRig { session, mic_rx, sys_rx, mic_thread: Some(mic_thread), sys_thread: Some(sys_thread), mic_stop })
}

/// (pid, owning app bundle path) of every process currently producing output.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn current_audio_processes(sys: &mut sysinfo::System) -> Vec<(u32, std::path::PathBuf)> {
    let pids: Vec<u32> = meeting_record::meetings::audio_processes().into_iter().filter(|p| p.is_playing_audio).map(|p| p.pid).collect();
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
    mic_device: Option<String>,
    loss: Arc<crate::audio::CaptureLoss>,
) -> Result<DualChannelCaptureHandle, String> {
    // Snapshot BEFORE opening the tap. Taking this afterwards could classify a
    // helper that started in between as already tapped and never recover it.
    let mut sys = sysinfo::System::new();
    let meeting_app = match target {
        DualChannelTarget::Process(pid) => app_path_of(&mut sys, pid),
        DualChannelTarget::System => None,
    };
    let initial_tapped: std::collections::HashSet<u32> = meeting_app.as_deref().map(|app| {
        current_audio_processes(&mut sys).into_iter().filter(|(_, path)| path.as_path() == app).map(|(pid, _)| pid).collect()
    }).unwrap_or_default();
    SYSTEM_AUDIO_RETRY.store(false, Ordering::Release);
    let rig = start_rig(target, &stop_signal, mic_device.as_deref(), loss.clone())?;
    *DIAGNOSTICS.lock().unwrap() = Some(CaptureDiagnostics {
        active: true,
        target: format!("{:?}", target),
        watchdog_app: meeting_app.as_ref().map(|a| a.display().to_string()),
        tapped_pids: initial_tapped.iter().copied().collect(),
        ..Default::default()
    });

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
        let mut gap_filled_samples: u64 = 0;
        let started = std::time::Instant::now();
        let mut call_health = CallAudioHealth::default();
        let mut current_target = target;
        let mut health_retries = 0;
        let mut recovery_waiting = false;
        let mut last_call_packet = std::time::Instant::now();
        let mut last_health_check = std::time::Instant::now();

        // Callers-track watchdog (meeting-process captures only): the tap covers
        // the meeting app's processes that did audio at start; restart it when
        // another process of that app starts doing audio.
        let mut tapped = initial_tapped;
        let mut last_process_check = std::time::Instant::now();
        match (&meeting_app, target) {
            (Some(app), _) => println!("[INFO] Callers track watchdog on {} (tapped pids {:?})", app.display(), tapped),
            (None, DualChannelTarget::Process(pid)) => {
                println!("[INFO] Callers track watchdog off: cannot resolve the app of pid {}", pid)
            }
            _ => {}
        }

        let mut last_level_emit = std::time::Instant::now();
        let mut mic_sum_sq = 0.0f32;
        let mut sys_sum_sq = 0.0f32;
        let mut level_samples = 0usize;

        const CHUNK_SIZE: usize = 960; // 20ms @ 48kHz
        // Without headphones the mic hears the callers too; take them back out.
        let mut aec = crate::echo_cancel::EchoCanceller::new();

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
                if !frames.is_empty() {
                    last_call_packet = std::time::Instant::now();
                    if recovery_waiting {
                        recovery_waiting = false;
                        health_retries = 0;
                        emit_call_audio_status(&app_handle, "restored", "Meeting audio capture reconnected.");
                    }
                }
                if call_health.observe(&frames) {
                    emit_call_audio_status(&app_handle, "restored", "Call audio is coming through.");
                }
                for s in frames {
                    sys_deque.push_back(s);
                    sys_sum_sq += s * s;
                }
            }

            if SYSTEM_AUDIO_RETRY.swap(false, Ordering::AcqRel) {
                let (mic_tail, sys_tail) = restart_rig(&mut rig, DualChannelTarget::System, &stop_for_rig, mic_device.as_deref(), &app_handle, loss.clone());
                mic_deque.extend(mic_tail.try_iter().flatten());
                sys_deque.extend(sys_tail.try_iter().flatten());
                current_target = DualChannelTarget::System;
                update_diagnostics(|d| { d.fell_back_to_system = true; d.target = "System".into(); d.tap_restarts += 1; });
                emit_call_audio_status(&app_handle, "fallback", "Retrying with all system audio. Other apps' sounds may also be recorded.");
                last_call_packet = std::time::Instant::now();
            }

            if stop_combiner.load(Ordering::Relaxed) { break; }
            // Absence of packets while the call app has active output is a
            // stalled tap. Delivered silent packets are ordinary quiet time.
            let check_health = last_health_check.elapsed() >= AUDIO_PROCESS_CHECK;
            let stalled = if check_health {
                last_health_check = std::time::Instant::now();
                last_call_packet.elapsed() >= std::time::Duration::from_secs(5)
                    && meeting_app.as_deref().is_some_and(|app| current_audio_processes(&mut sys).iter().any(|(_, path)| path.as_path() == app))
            } else { false };

            if stalled || (check_health && rig.session.failure().is_some())
                || rig.mic_thread.as_ref().is_some_and(|t| t.is_finished())
                || rig.sys_thread.as_ref().is_some_and(|t| t.is_finished())
            {
                loss.warn_meeting("Meeting audio capture was interrupted; some audio may be missing.".into());
                if health_retries >= 2 {
                    emit_call_audio_status(&app_handle, "failed", "Meeting audio capture could not recover. The captured recording will be saved.");
                    let _ = app_handle.emit("audio-disconnected", serde_json::json!({ "code": "meeting_capture_lost", "message": "Meeting audio capture could not recover" }));
                    stop_combiner.store(true, Ordering::Relaxed);
                    break;
                }
                health_retries += 1;
                recovery_waiting = true;
                emit_call_audio_status(&app_handle, "recovering", "Meeting audio capture was interrupted. Reconnecting…");
                let retry_target = if health_retries == 1 { current_target } else { DualChannelTarget::System };
                let (mic_tail, sys_tail) = restart_rig(&mut rig, retry_target, &stop_for_rig, mic_device.as_deref(), &app_handle, loss.clone());
                mic_deque.extend(mic_tail.try_iter().flatten());
                sys_deque.extend(sys_tail.try_iter().flatten());
                current_target = if capture_diagnostics().is_some_and(|d| d.fell_back_to_system) { DualChannelTarget::System } else { retry_target };
                update_diagnostics(|d| { d.tap_restarts += 1; d.target = format!("{:?}", current_target); d.fell_back_to_system |= current_target == DualChannelTarget::System; });
                last_call_packet = std::time::Instant::now();
                if stop_combiner.load(Ordering::Relaxed) { break; }
            }

            if call_health.needs_warning(started.elapsed()) {
                emit_call_audio_status(&app_handle, "silent", "No sound from other participants has been captured yet. If someone is speaking, retry call audio or check System Audio Recording permission.");
            }

            if let (Some(app), DualChannelTarget::Process(pid)) = (&meeting_app, current_target) {
                if last_process_check.elapsed() >= AUDIO_PROCESS_CHECK {
                    last_process_check = std::time::Instant::now();
                    let new_pids = untapped_meeting_audio_pids(&tapped, app, &current_audio_processes(&mut sys));
                    if !new_pids.is_empty() {
                        eprintln!(
                            "[WARN] Meeting app started audio in process(es) {:?} outside the callers tap; restarting the tap",
                            new_pids
                        );
                        let (old_mic_rx, old_sys_rx) =
                            restart_rig(&mut rig, DualChannelTarget::Process(pid), &stop_for_rig, mic_device.as_deref(), &app_handle, loss.clone());
                        // Keep whatever the old session had already delivered.
                        while let Ok(frames) = old_mic_rx.try_recv() {
                            mic_deque.extend(frames);
                        }
                        while let Ok(frames) = old_sys_rx.try_recv() {
                            call_health.observe(&frames);
                            sys_deque.extend(frames);
                        }
                        last_call_packet = std::time::Instant::now();
                        // Keep the pre-start snapshot; a helper starting during
                        // restart must remain eligible for the next check.
                        tapped.extend(new_pids);
                        let pids: Vec<u32> = tapped.iter().copied().collect();
                        update_diagnostics(|d| {
                            d.tap_restarts += 1;
                            d.tapped_pids = pids;
                        });
                        if capture_diagnostics().is_some_and(|d| d.fell_back_to_system) {
                            current_target = DualChannelTarget::System;
                        }
                    }
                }
            }

            // Arrival jitter is not missing audio. Padding the shorter queue on
            // every poll accumulated silence repeatedly and made a 38-second
            // meeting play back as ~57 seconds. Pair only frames that both tracks
            // actually delivered; pad a final unmatched tail once on stop below.
            // Only a track that has stopped delivering for a second is filled.
            let filled = fill_stalled_track(&mut mic_deque, &mut sys_deque);
            if filled > 0 {
                if gap_filled_samples == 0 {
                    eprintln!("[WARN] A dual-channel track stopped delivering audio; recording silence for it meanwhile");
                }
                loss.gap_frames.fetch_add(filled as u64, Ordering::Relaxed);
                gap_filled_samples += filled as u64;
            }

            // Interleave available matching frames in steady 20ms chunks
            let available = mic_deque.len().min(sys_deque.len());
            if available >= CHUNK_SIZE {
                let chunks_to_process = available / CHUNK_SIZE;
                for _ in 0..chunks_to_process {
                    let (interleaved, mono_mix) = cancel_echo_and_mix(&mut aec, &mut mic_deque, &mut sys_deque, CHUNK_SIZE, false);
                    let frames = mono_mix.len();
                    if frames == 0 {
                        continue;
                    }
                    level_samples += frames;

                    let before = loss.file_frames.load(Ordering::Relaxed);
                    if loss.send_file(&file_tx, interleaved, 2) {
                        let _ = app_handle.emit("recording-audio-dropped", "The disk fell behind and a moment of meeting audio was lost. Recording continues; the recording will be kept.");
                    }
                    dropped_file_samples += loss.file_frames.load(Ordering::Relaxed) - before;
                    if whisper_tx.try_send(mono_mix).is_err() {
                        loss.preview_frames.fetch_add(frames as u64, Ordering::Relaxed);
                        dropped_transcriber_samples += frames as u64;
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
                update_diagnostics(|d| { d.call_audio_seen = call_health.seen; d.call_audio_warning = call_health.warned; });
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
            call_health.observe(&frames);
            sys_deque.extend(frames);
        }
        if !call_health.seen && started.elapsed() >= std::time::Duration::from_secs(15) {
            loss.warn_meeting("No sound was captured from other participants. They may have been silent; verify call audio before relying on this recording.".into());
        }

        // The tail goes to the live transcriber as well as the file.
        let remaining = mic_deque.len().max(sys_deque.len());
        let (interleaved, mono_mix) = cancel_echo_and_mix(&mut aec, &mut mic_deque, &mut sys_deque, remaining, true);
        let remaining = mono_mix.len();
        if remaining > 0 {
            let before = loss.file_frames.load(Ordering::Relaxed);
            if loss.send_file(&file_tx, interleaved, 2) {
                let _ = app_handle.emit("recording-audio-dropped", "The last moment of the meeting audio could not be saved; the recording will be kept.");
            }
            dropped_file_samples += loss.file_frames.load(Ordering::Relaxed) - before;
            if whisper_tx.try_send(mono_mix).is_err() {
                loss.preview_frames.fetch_add(remaining as u64, Ordering::Relaxed);
                dropped_transcriber_samples += remaining as u64;
            }
        }

        let secs = |n: u64| n as f64 / 48_000.0;
        update_diagnostics(|d| {
            d.active = false;
            d.call_audio_seen = call_health.seen;
            d.call_audio_warning = !call_health.seen;
            d.dropped_file_seconds = secs(dropped_file_samples);
            d.dropped_transcriber_seconds = secs(dropped_transcriber_samples);
            d.gap_filled_seconds = secs(gap_filled_samples);
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
    mic_device: Option<&str>,
    app: &AppHandle,
    loss: Arc<crate::audio::CaptureLoss>,
) -> (crossbeam_channel::Receiver<Vec<f32>>, crossbeam_channel::Receiver<Vec<f32>>) {
    // The crate allows one capture session at a time: stop before starting anew.
    // A cpal microphone has to be released too before the new rig reopens it.
    let tail = rig.shutdown();
    let fresh = start_rig(target, stop_signal, mic_device, loss.clone()).or_else(|e| {
        if target == DualChannelTarget::System { return Err(e); }
        eprintln!("[WARN] Restarting the meeting tap failed ({}); falling back to system audio", e);
        update_diagnostics(|d| d.fell_back_to_system = true);
        emit_call_audio_status(app, "fallback", "Using all system audio because meeting-only capture failed. Other apps' sounds may also be recorded.");
        start_rig(DualChannelTarget::System, stop_signal, mic_device, loss.clone())
    });
    match fresh {
        Ok(new_rig) => { *rig = new_rig; tail },
        Err(e) => {
            loss.warn_meeting(format!("Meeting audio stopped and could not restart: {e}"));
            stop_signal.store(true, Ordering::Relaxed);
            // Both tracks are stopped now, and the recording would silently write
            // nothing for the rest of the call. Tell the UI, which stops the
            // recording and saves the meeting up to here.
            eprintln!("[ERROR] Could not restart dual-channel capture ({}); meeting audio stopped", e);
            let _ = app.emit(
                "audio-disconnected",
                serde_json::json!({
                    "code": "meeting_capture_lost",
                    "message": format!("meeting audio capture stopped and could not restart ({e})"),
                }),
            );
            tail
        }
    }
}

/// Captures the input device named `name` through cpal on its own thread,
/// sending mono 48 kHz chunks to `tx` until either `stops` flag is set.
/// Returns Ok(None) when `name` is the system default microphone (the capture
/// session's own microphone track is used then), and an error when the device
/// is missing or cannot be opened.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn spawn_named_mic_capture(
    name: &str,
    tx: Sender<Vec<f32>>,
    stops: [Arc<AtomicBool>; 2],
    loss: Arc<crate::audio::CaptureLoss>,
) -> Result<Option<std::thread::JoinHandle<()>>, String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());
    if default_name.as_deref() == Some(name) {
        return Ok(None);
    }
    if !host.input_devices().map_err(|e| e.to_string())?.any(|d| d.name().ok().as_deref() == Some(name)) {
        return Err("device not found".into());
    }

    // cpal streams are not Send on macOS: open and own it on the capture thread.
    let name = name.to_string();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let handle = std::thread::spawn(move || {
        use cpal::traits::StreamTrait;
        let opened = (|| -> Result<cpal::Stream, String> {
            let device = cpal::default_host()
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().ok().as_deref() == Some(name.as_str()))
                .ok_or("device not found")?;
            let supported = device.default_input_config().map_err(|e| e.to_string())?;
            let format = supported.sample_format();
            let config: cpal::StreamConfig = supported.into();
            let (channels, rate) = (config.channels as u32, config.sample_rate.0);
            let mut to_48k = MonoTo48k::new();
            let callback_stops = stops.clone();
            let stream = crate::audio::build_input_stream_f32(
                &device,
                &config,
                format,
                move |data: &[f32]| {
                    let out = to_48k.push(data, channels, rate as f64);
                    if !out.is_empty() {
                        send_named_mic_frames(&tx, out, &callback_stops, &loss);
                    }
                },
                |e| eprintln!("[ERROR] Meeting microphone stream error: {}", e),
            )
            .map_err(|e| e.to_string())?;
            stream.play().map_err(|e| e.to_string())?;
            println!("[INFO] Dual-channel mic track: '{}' via cpal ({} ch, {} Hz, {:?})", name, channels, rate, format);
            Ok(stream)
        })();
        match opened {
            Ok(stream) => {
                let _ = ready_tx.send(Ok(()));
                while !stops.iter().any(|s| s.load(Ordering::Relaxed)) {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        }
    });
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Some(handle)),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => Err("microphone thread exited".into()),
    }
}

fn send_named_mic_frames(
    tx: &Sender<Vec<f32>>, frames: Vec<f32>, stops: &[Arc<AtomicBool>; 2], loss: &crate::audio::CaptureLoss,
) {
    // Full queues always lose requested audio. A callback after intentional
    // Stop can meet a closed receiver; that is outside the capture window.
    let lost = match tx.try_send(frames) {
        Err(crossbeam_channel::TrySendError::Full(frames)) => frames.len(),
        Err(crossbeam_channel::TrySendError::Disconnected(frames))
            if !stops.iter().any(|s| s.load(Ordering::Relaxed)) => frames.len(),
        _ => 0,
    };
    loss.input_frames.fetch_add(lost as u64, Ordering::Relaxed);
}

/// One track's audio as mono 48 kHz, through a single resampler for the whole
/// stream. Resampling each chunk on its own rounds every chunk's length, and
/// the mic and callers tracks drift apart over a long meeting.
struct MonoTo48k {
    rate: u32,
    resampler: Option<crate::audio_preprocess::StreamingResampler>,
}

impl MonoTo48k {
    fn new() -> Self {
        Self { rate: 0, resampler: None }
    }

    fn push(&mut self, frames: &[f32], channels: u32, sample_rate: f64) -> Vec<f32> {
        let mono = crate::audio_preprocess::downmix_interleaved_to_mono(frames, channels as usize);
        let rate = sample_rate.round() as u32;
        if rate < 1000 {
            return mono;
        }
        let mut out = Vec::new();
        // Created from the first chunk's rate, and again if the device changes it.
        if self.resampler.is_none() || self.rate != rate {
            // Flush what the old resampler still holds: the tracks are paired by
            // sample count, so dropping it would shift this track for good.
            if let Some(mut old) = self.resampler.take() {
                out = old.finish().unwrap_or_default();
            }
            // 1024-frame blocks keep the added latency around 20 ms.
            match crate::audio_preprocess::StreamingResampler::with_target(rate, 48_000, 1024) {
                Ok(r) => {
                    self.resampler = Some(r);
                    self.rate = rate;
                }
                Err(e) => {
                    eprintln!("[ERROR] Meeting audio resampler: {}", e);
                    out.extend(mono);
                    return out;
                }
            }
        }
        match self.resampler.as_mut().unwrap().push(&mono) {
            Ok(resampled) => out.extend(resampled),
            Err(e) => eprintln!("[ERROR] Meeting audio resample failed: {}", e),
        }
        out
    }
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
    _mic_device: Option<String>,
    _loss: Arc<crate::audio::CaptureLoss>,
) -> Result<DualChannelCaptureHandle, String> {
    Err("Dual-channel meeting audio capture is not supported on Linux".into())
}

#[cfg(test)]
mod track_tests {
    use super::*;

    fn deque(n: usize, v: f32) -> VecDeque<f32> {
        std::iter::repeat(v).take(n).collect()
    }

    #[test]
    fn jitter_is_left_alone() {
        let (mut mic, mut sys) = (deque(48_000, 0.1), deque(1_000, 0.2));
        assert_eq!(fill_stalled_track(&mut mic, &mut sys), 0);
        assert_eq!((mic.len(), sys.len()), (48_000, 1_000));
    }

    #[test]
    fn a_stalled_track_is_filled_so_the_other_keeps_recording() {
        let (mut mic, mut sys) = (deque(60_000, 0.1), deque(1_000, 0.2));
        assert_eq!(fill_stalled_track(&mut mic, &mut sys), 59_000);
        assert_eq!(sys.len(), 60_000);
        // The call audio that did arrive stays first; silence follows it.
        assert_eq!((sys[999], sys[1_000]), (0.2, 0.0));
        let (file, mono) = interleave_and_mix(&mut mic, &mut sys, 60_000);
        assert_eq!((file.len(), mono.len()), (120_000, 60_000));
        assert_eq!((file[2 * 1_000], file[2 * 1_000 + 1]), (0.1, 0.0));
    }

    #[test]
    fn the_tail_is_mixed_for_the_live_transcriber_too() {
        let (mut mic, mut sys) = (deque(959, 0.4), deque(900, 0.2));
        let (file, mono) = interleave_and_mix(&mut mic, &mut sys, 959);
        assert_eq!((file.len(), mono.len()), (1_918, 959));
        assert!((mono[0] - 0.3).abs() < 1e-6);
        assert!((mono[958] - 0.2).abs() < 1e-6);
    }
}

#[cfg(test)]
mod tests {
    use super::{app_bundle_path, untapped_meeting_audio_pids};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    #[test]
    fn late_mic_callback_after_stop_is_not_loss_but_full_queues_still_are() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let stops = [std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))];
        let loss = crate::audio::CaptureLoss::default();
        super::send_named_mic_frames(&tx, vec![0.1; 1024], &stops, &loss);
        super::send_named_mic_frames(&tx, vec![0.2; 1024], &stops, &loss);
        assert_eq!(rx.recv().unwrap(), vec![0.1; 1024]);
        assert_eq!(loss.input_frames.load(std::sync::atomic::Ordering::Relaxed), 1024);
        drop(rx);
        super::send_named_mic_frames(&tx, vec![0.3; 1024], &stops, &loss);
        assert_eq!(loss.input_frames.load(std::sync::atomic::Ordering::Relaxed), 2048);
        stops[0].store(true, std::sync::atomic::Ordering::Relaxed);
        super::send_named_mic_frames(&tx, vec![0.4; 1024], &stops, &loss);
        assert_eq!(loss.input_frames.load(std::sync::atomic::Ordering::Relaxed), 2048);
    }

    #[test]
    fn zero_packets_warn_once_and_quiet_speech_clears_the_warning() {
        let mut health = super::CallAudioHealth::default();
        assert!(!health.observe(&[0.0, f32::NAN]));
        assert!(!health.needs_warning(std::time::Duration::from_secs(14)));
        assert!(health.needs_warning(std::time::Duration::from_secs(15)));
        assert!(!health.needs_warning(std::time::Duration::from_secs(30)));
        assert!(health.observe(&[0.00001]));
        assert!(health.seen);
        assert!(!health.needs_warning(std::time::Duration::from_secs(600)));
        assert!(!health.observe(&[0.0]));
    }

    #[test]
    fn caller_warning_does_not_change_the_recorded_samples() {
        let mut mic = vec![0.1, 0.2, 0.3].into();
        let mut sys = vec![0.0, 0.0, 0.0].into();
        let (stereo, _) = super::interleave_and_mix(&mut mic, &mut sys, 3);
        assert_eq!(stereo, vec![0.1, 0.0, 0.2, 0.0, 0.3, 0.0]);
    }

    #[test]
    fn shutdown_drains_full_queues_before_join_and_preserves_the_tail() {
        let (mic_tx, mic_rx) = crossbeam_channel::bounded(2);
        let (sys_tx, sys_rx) = crossbeam_channel::bounded(2);
        for n in 0..2 {
            mic_tx.send(vec![n as f32]).unwrap();
            sys_tx.send(vec![10.0 + n as f32]).unwrap();
        }
        let mic = std::thread::spawn(move || mic_tx.send(vec![2.0]).unwrap());
        let sys = std::thread::spawn(move || sys_tx.send(vec![12.0]).unwrap());
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mic, sys) = super::drain_capture_readers(&mic_rx, &sys_rx, Some(mic), Some(sys));
            done.send((mic.iter().flatten().collect::<Vec<_>>(), sys.iter().flatten().collect::<Vec<_>>())).unwrap();
        });
        let (mic, sys) = result.recv_timeout(std::time::Duration::from_secs(2)).expect("full queues must not deadlock shutdown");
        assert_eq!(mic, vec![0.0, 1.0, 2.0]);
        assert_eq!(sys, vec![10.0, 11.0, 12.0]);
    }

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
    fn missing_selected_microphone_is_an_error_not_a_silent_track() {
        use std::sync::{atomic::AtomicBool, Arc};
        let (tx, _rx) = crossbeam_channel::bounded(4);
        let stops = [Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false))];
        // start_rig falls back to the default microphone on this error.
        assert!(super::spawn_named_mic_capture("Taurscribe test mic that does not exist", tx, stops, Arc::new(crate::audio::CaptureLoss::default())).is_err());
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
    fn mono_48k_downmixes_and_keeps_the_stream_length() {
        let mut t = super::MonoTo48k::new();
        let mono_48k = t.push(&[0.2, 0.4, 0.6, 0.8], 2, 48000.0); // 2 stereo frames, no resampling
        assert_eq!(mono_48k.len(), 2);
        assert!((mono_48k[0] - 0.3).abs() < 1e-5);
        assert!((mono_48k[1] - 0.7).abs() < 1e-5);

        // 44.1 kHz in 512-frame chunks: resampled per chunk this lost 0.28 samples
        // every time (about 2 230 over these 8 000 chunks). One resampler only lags
        // by what is still buffered: less than one 1024-frame block (1 115 samples).
        let mut t = super::MonoTo48k::new();
        let chunk = vec![0.5f32; 512];
        let out: usize = (0..8000).map(|_| t.push(&chunk, 1, 44100.0).len()).sum();
        let exact = 8000.0 * 512.0 * 48000.0 / 44100.0;
        assert!(out as f64 <= exact + 1.0 && out as f64 >= exact - 1115.0, "{out} vs {exact}");

        // A rate change flushes what the old resampler held instead of dropping it.
        let mut t = super::MonoTo48k::new();
        assert!(t.push(&vec![0.5f32; 700], 1, 44100.0).is_empty(), "less than one block is buffered");
        let after_switch = t.push(&vec![0.5f32; 480], 1, 48000.0);
        assert_eq!(after_switch.len(), 762 + 480, "700 frames at 44.1 kHz = 762 at 48 kHz, then 480 as-is");
    }
}
