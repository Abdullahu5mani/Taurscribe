//! Timestamped import text. Never distribute words by transcript length.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimedText {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}
#[derive(Clone, Debug, Default)]
pub struct TimedTranscript {
    pub text: String,
    pub units: Vec<TimedText>,
}
impl TimedTranscript {
    pub fn append(&mut self, mut other: Self, offset_ms: u64) {
        for unit in &mut other.units {
            unit.start_ms += offset_ms;
            unit.end_ms += offset_ms;
        }
        if !other.text.trim().is_empty() {
            if !self.text.is_empty() {
                self.text.push(' ');
            }
            self.text.push_str(other.text.trim());
        }
        self.units.extend(other.units);
    }
    /// Require timestamped units to cover exactly the decoded words before reuse.
    /// Untimed decoders retain the original per-speaker transcription path.
    pub fn covers_text(&self) -> bool {
        let normalized = |text: &str| {
            text.split_whitespace()
                .map(|w| {
                    w.chars()
                        .filter(|c| c.is_alphanumeric())
                        .flat_map(char::to_lowercase)
                        .collect::<String>()
                })
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
        };
        !self.units.is_empty()
            && normalized(&self.text)
                == normalized(
                    &self
                        .units
                        .iter()
                        .map(|u| u.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            && self.units.iter().all(|u| u.end_ms > u.start_ms)
            && self.units.windows(2).all(|w| w[0].end_ms <= w[1].start_ms)
    }
    /// A unit crossing a different speaker's interval is ambiguous: re-decode
    /// that speaker's audio. Every reusable unit must belong to exactly one turn.
    pub fn for_turn(&self, index: usize, turns: &[(u64, u64)]) -> Option<String> {
        if !self.covers_text() {
            return None;
        }
        self.for_turn_validated(index, turns)
    }
    pub(crate) fn for_turn_validated(&self, index: usize, turns: &[(u64, u64)]) -> Option<String> {
        let (start, end) = *turns.get(index)?;
        let overlaps = |u: &TimedText, a: u64, b: u64| u.start_ms < b && u.end_ms > a;
        let selected: Vec<_> = self
            .units
            .iter()
            .filter(|u| overlaps(u, start, end))
            .collect();
        if selected.is_empty() {
            return None;
        }
        if selected.iter().any(|u| {
            u.start_ms < start
                || u.end_ms > end
                || turns
                    .iter()
                    .enumerate()
                    .any(|(other, (a, b))| other != index && overlaps(u, *a, *b))
        }) {
            return None;
        }
        // Speech just outside a diarization boundary can belong to this speaker.
        // If it is not assigned anywhere, do not silently drop it from the cache.
        if self.units.iter().any(|u| {
            u.start_ms <= end.saturating_add(250)
                && u.end_ms.saturating_add(250) >= start
                && !turns.iter().any(|(a, b)| overlaps(u, *a, *b))
        }) {
            return None;
        }
        Some(
            selected
                .iter()
                .map(|u| u.text.trim())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

/// Query fresh backend memory after ASR has loaded. Unknown memory means serial.
/// Require 2 GiB headroom for diarization weights and working buffers.
/// Check every GPU candidate because the diarizer chooses its device automatically.
pub fn can_overlap_diarization() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        false
    } // MLX and ggml share unified memory; enable after native profiling.
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let devices = transcribe_cpp::devices();
        let candidates: Vec<_> = devices
            .iter()
            .filter(|d| {
                matches!(
                    d.device_type,
                    transcribe_cpp::DeviceType::Gpu | transcribe_cpp::DeviceType::Igpu
                )
            })
            .collect();
        !candidates.is_empty()
            && candidates
                .iter()
                .all(|d| d.memory_free >= 2 * 1024 * 1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unit(start: u64, end: u64, text: &str) -> TimedText {
        TimedText {
            start_ms: start,
            end_ms: end,
            text: text.into(),
        }
    }
    #[test]
    fn words_are_reused_only_for_unambiguous_speaker_intervals() {
        let t = TimedTranscript {
            text: "Hello Bob!".into(),
            units: vec![unit(100, 300, "Hello"), unit(1100, 1300, "Bob!")],
        };
        assert_eq!(
            t.for_turn(0, &[(0, 900), (1000, 1800)]),
            Some("Hello".into())
        );
        assert_eq!(
            t.for_turn(1, &[(0, 900), (1000, 1800)]),
            Some("Bob!".into())
        );
        assert_eq!(t.for_turn(0, &[(0, 1200), (1000, 1800)]), None);
    }
    #[test]
    fn incomplete_invalid_or_boundary_crossing_alignment_is_not_reused() {
        for t in [
            TimedTranscript {
                text: "missing words".into(),
                units: vec![unit(0, 100, "missing")],
            },
            TimedTranscript {
                text: "Hello".into(),
                units: vec![unit(0, 0, "Hello")],
            },
            TimedTranscript {
                text: "Hello".into(),
                units: vec![unit(700, 1100, "Hello")],
            },
        ] {
            assert_eq!(t.for_turn(0, &[(0, 1000), (1000, 2000)]), None);
        }
    }
    #[test]
    fn bounded_chunks_cover_audio_without_gaps_or_duplication() {
        for length in [0, 10, 16_000 * 31, 16_000 * 95] {
            let pcm = vec![0.1; length];
            let ranges = chunk_ranges(&pcm);
            assert_eq!(ranges.iter().map(|(a, b)| b - a).sum::<usize>(), length);
            assert!(ranges.windows(2).all(|w| w[0].1 == w[1].0));
            assert!(ranges.iter().all(|(a, b)| b > a && b - a <= 32 * 16000));
        }
    }
    #[test]
    fn chunk_offsets_preserve_every_word() {
        let mut t = TimedTranscript::default();
        t.append(
            TimedTranscript {
                text: "Hello".into(),
                units: vec![unit(0, 100, "Hello")],
            },
            5000,
        );
        assert_eq!(t.units[0].start_ms, 5000);
        assert_eq!(t.units[0].end_ms, 5100);
        assert!(t.covers_text());
    }
}

/// Bound the first-text wait, choosing the quietest 100 ms near each boundary.
/// No samples are discarded or duplicated.
pub fn chunk_ranges(pcm: &[f32]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < pcm.len() {
        let target = start + 30 * 16000;
        let end = if target + 2 * 16000 >= pcm.len() {
            pcm.len()
        } else {
            ((target - 2 * 16000)..=(target + 2 * 16000))
                .step_by(1600)
                .min_by(|a, b| {
                    let energy =
                        |i: usize| pcm[i - 800..i + 800].iter().map(|s| s * s).sum::<f32>();
                    energy(*a).total_cmp(&energy(*b))
                })
                .unwrap_or(target)
        };
        ranges.push((start, end));
        start = end;
    }
    ranges
}
