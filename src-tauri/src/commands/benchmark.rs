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
