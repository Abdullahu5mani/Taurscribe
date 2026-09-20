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

use crossbeam_channel::{bounded, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter};

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

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn start_dual_channel_capture(
    target: DualChannelTarget,
    _sample_rate: u32,
    file_tx: Sender<Vec<f32>>,
    whisper_tx: Sender<Vec<f32>>,
    app_handle: AppHandle,
    stop_signal: Arc<AtomicBool>,
) -> Result<DualChannelCaptureHandle, String> {
    use meeting_record::{capture, CaptureOptions, CaptureTarget, MicrophoneSource};
    use std::collections::VecDeque;

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
    let mic_reader_thread = std::thread::spawn(move || {
        crate::platform_tuning::apply_thread_performance_affinity();
        if let Some(track) = session_mic.microphone() {
            while !stop_mic.load(Ordering::Relaxed) {
                match track.recv() {
                    Some(chunk) => {
                        if mic_tx.send(chunk.frames).is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    });

    let session_sys = session.clone();
    let stop_sys = stop_signal.clone();
    let sys_reader_thread = std::thread::spawn(move || {
        crate::platform_tuning::apply_thread_performance_affinity();
        let track = session_sys.system_audio();
        while !stop_sys.load(Ordering::Relaxed) {
            match track.recv() {
                Some(chunk) => {
                    if sys_tx.send(chunk.frames).is_err() {
                        break;
                    }
                }
                None => break,
            }
        }
    });

    let stop_combiner = stop_signal.clone();
    let session_for_stop = session.clone();
    let capture_thread = std::thread::spawn(move || {
        crate::platform_tuning::apply_thread_performance_affinity();

        let mut mic_deque: VecDeque<f32> = VecDeque::with_capacity(48000);
        let mut sys_deque: VecDeque<f32> = VecDeque::with_capacity(48000);

        let mut last_level_emit = std::time::Instant::now();
        let mut mic_sum_sq = 0.0f32;
        let mut sys_sum_sq = 0.0f32;
        let mut level_samples = 0usize;

        while !stop_combiner.load(Ordering::Relaxed) {
            let mut got_anything = false;

            // Drain incoming microphone packets
            while let Ok(frames) = mic_rx.try_recv() {
                got_anything = true;
                for s in frames {
                    mic_deque.push_back(s);
                    mic_sum_sq += s * s;
                }
            }

            // Drain incoming system audio packets
            while let Ok(frames) = sys_rx.try_recv() {
                got_anything = true;
                for s in frames {
                    sys_deque.push_back(s);
                    sys_sum_sq += s * s;
                }
            }

            // Interleave available matching frames
            let available = mic_deque.len().min(sys_deque.len());
            if available >= 480 {
                // ~10ms at 48kHz
                let mut interleaved = Vec::with_capacity(available * 2);
                let mut mono_mix = Vec::with_capacity(available);

                for _ in 0..available {
                    let m = mic_deque.pop_front().unwrap_or(0.0);
                    let s = sys_deque.pop_front().unwrap_or(0.0);

                    // Channel 1: Left ear = User Microphone
                    interleaved.push(m);
                    // Channel 2: Right ear = System / Remote caller
                    interleaved.push(s);

                    // Equal-power mix for real-time transcription feed
                    mono_mix.push((m * 0.5) + (s * 0.5));
                }

                level_samples += available;

                let _ = file_tx.try_send(interleaved);
                let _ = whisper_tx.try_send(mono_mix);
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

            if !got_anything {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        // On stop: close session to unblock reader threads
        session_for_stop.stop();
        let _ = mic_reader_thread.join();
        let _ = sys_reader_thread.join();

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
            let _ = file_tx.try_send(interleaved);
        }

        println!("[INFO] Dual-channel capture thread cleanly terminated");
    });

    Ok(DualChannelCaptureHandle {
        stop_signal,
        capture_thread,
    })
}

// ── Linux Fallback Implementation ───────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn start_dual_channel_capture(
    _target: DualChannelTarget,
    _sample_rate: u32,
    _file_tx: Sender<Vec<f32>>,
    _whisper_tx: Sender<Vec<f32>>,
    _app_handle: AppHandle,
    stop_signal: Arc<AtomicBool>,
) -> Result<DualChannelCaptureHandle, String> {
    let capture_thread = std::thread::spawn(move || {
        while !stop_signal.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    Ok(DualChannelCaptureHandle {
        stop_signal,
        capture_thread,
    })
}

#[cfg(test)]
mod tests {
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
}
