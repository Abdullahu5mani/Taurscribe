//! Removes the call audio the microphone picks up from the speakers, so the
//! "You" channel of a meeting holds only the local voice. Without headphones the
//! callers otherwise come back on the mic track and get transcribed as "You".
//!
//! The callers track is the echo reference for WebRTC's AEC3 (the pure-Rust
//! `sonora` port). With headphones there is no echo and AEC3's transparent mode
//! leaves the mic alone.

use sonora::config::EchoCanceller as AecConfig;
use sonora::{AudioProcessing, Config, StreamConfig};
use std::collections::VecDeque;

const RATE: u32 = 48_000;
/// AEC3 processes 10 ms frames.
const FRAME: usize = RATE as usize / 100;
/// The tracks are paired by arrival, not by capture time, so the callers audio
/// can reach us slightly after its echo reaches the mic. AEC3 only cancels echo
/// whose reference it has already seen: the reference is fed this far ahead of
/// the mic. Both tracks come out delayed by it, so they stay aligned.
const LEAD_FRAMES: usize = 10; // 100 ms

pub struct EchoCanceller {
    apm: AudioProcessing,
    mic: VecDeque<f32>,
    sys: VecDeque<f32>,
    /// Samples at the front of `sys` already fed to AEC3 as reference.
    rendered: usize,
}

impl Default for EchoCanceller {
    fn default() -> Self {
        Self::new()
    }
}

impl EchoCanceller {
    pub fn new() -> Self {
        let mut apm = AudioProcessing::builder()
            .config(Config { echo_canceller: Some(AecConfig::default()), ..Default::default() })
            .capture_config(StreamConfig::new(RATE, 1))
            .render_config(StreamConfig::new(RATE, 1))
            .build();
        // AEC3 estimates the actual delay itself; this is only its starting point.
        let _ = apm.set_stream_delay_ms(0);
        Self { apm, mic: VecDeque::new(), sys: VecDeque::new(), rendered: 0 }
    }

    /// Takes equal-length spans of mic and callers audio, and returns the
    /// echo-cancelled mic with the matching callers audio. The output trails the
    /// input by the reference lead, in whole 10 ms frames.
    pub fn push(&mut self, mic: &[f32], sys: &[f32]) -> (Vec<f32>, Vec<f32>) {
        debug_assert_eq!(mic.len(), sys.len());
        self.mic.extend(mic);
        self.sys.extend(sys);
        self.feed_reference();
        self.drain(|this| this.rendered >= FRAME * (LEAD_FRAMES + 1))
    }

    /// Returns everything still buffered. A final partial frame passes through
    /// unprocessed.
    pub fn flush(&mut self) -> (Vec<f32>, Vec<f32>) {
        self.feed_reference();
        let (mut mic, mut sys) = self.drain(|this| this.rendered >= FRAME);
        mic.extend(self.mic.drain(..));
        sys.extend(self.sys.drain(..));
        self.rendered = 0;
        (mic, sys)
    }

    fn feed_reference(&mut self) {
        let mut out = [0.0f32; FRAME];
        while self.sys.len() - self.rendered >= FRAME {
            let frame: Vec<f32> = self.sys.range(self.rendered..self.rendered + FRAME).copied().collect();
            let _ = self.apm.process_render_f32(&[&frame], &mut [&mut out]);
            self.rendered += FRAME;
        }
    }

    fn drain(&mut self, ready: impl Fn(&Self) -> bool) -> (Vec<f32>, Vec<f32>) {
        let mut mic_out = Vec::new();
        let mut sys_out = Vec::new();
        let mut cleaned = [0.0f32; FRAME];
        while self.mic.len() >= FRAME && ready(self) {
            let frame: Vec<f32> = self.mic.drain(..FRAME).collect();
            match self.apm.process_capture_f32(&[&frame], &mut [&mut cleaned]) {
                Ok(()) => mic_out.extend_from_slice(&cleaned),
                Err(_) => mic_out.extend_from_slice(&frame),
            }
            sys_out.extend(self.sys.drain(..FRAME));
            self.rendered -= FRAME;
        }
        (mic_out, sys_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise in [-1, 1].
    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt()
    }

    fn run(aec: &mut EchoCanceller, mic: &[f32], sys: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut m, mut s) = (Vec::new(), Vec::new());
        for (mc, sc) in mic.chunks(960).zip(sys.chunks(960)) {
            let (a, b) = aec.push(mc, sc);
            m.extend(a);
            s.extend(b);
        }
        let (a, b) = aec.flush();
        m.extend(a);
        s.extend(b);
        (m, s)
    }

    #[test]
    fn output_keeps_every_sample_and_the_tracks_aligned() {
        let mut aec = EchoCanceller::new();
        let sys: Vec<f32> = (0..48_123).map(|i| i as f32 / 100_000.0).collect();
        let mic = vec![0.0; sys.len()];
        let (m, s) = run(&mut aec, &mic, &sys);
        assert_eq!(m.len(), mic.len());
        assert_eq!(s, sys, "callers track must come out unchanged and in order");
    }

    #[test]
    fn removes_a_delayed_copy_of_the_callers_audio() {
        let mut aec = EchoCanceller::new();
        let secs = 8;
        let far = noise(48_000 * secs, 7).iter().map(|v| v * 0.3).collect::<Vec<_>>();
        // The mic hears the speakers 30 ms later, quieter.
        let delay = 1_440;
        let mut mic = vec![0.0f32; far.len()];
        for i in delay..far.len() {
            mic[i] = far[i - delay] * 0.5;
        }
        let (m, _) = run(&mut aec, &mic, &far);
        // After convergence, the last 2 s should be far quieter than the echo.
        let tail = m.len() - 96_000;
        let before = rms(&mic[tail..]);
        let after = rms(&m[tail..]);
        assert!(after < before * 0.25, "echo not removed: {before} -> {after}");
    }

    #[test]
    fn keeps_local_speech_when_the_callers_are_silent() {
        let mut aec = EchoCanceller::new();
        let near = noise(48_000 * 3, 11).iter().map(|v| v * 0.2).collect::<Vec<_>>();
        let silent = vec![0.0f32; near.len()];
        let (m, _) = run(&mut aec, &near, &silent);
        let tail = m.len() - 48_000;
        assert!(rms(&m[tail..]) > rms(&near[tail..]) * 0.5, "local speech was suppressed");
    }
}
