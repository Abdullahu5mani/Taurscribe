//! Continuing a meeting: if the user stops recording a call and starts recording
//! the same call again within a few minutes, the second recording is appended to
//! the first meeting instead of saving a second one.
//!
//! Saved meetings keep only a mono playback copy, but speaker separation needs the
//! two channels (mic = you, other = the call). So after a meeting from a known call
//! is saved, its stereo WAV is kept in `meetings/continuation/` until the window
//! closes, then deleted. Only the most recent meeting is kept.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::meeting_detector::MeetingInfo;

/// Minutes after a recording ends in which recording the same call continues it
/// (0 = never). Settings → Meetings.
static WINDOW_MINUTES: AtomicU64 = AtomicU64::new(10);
static CLAIMED_MEETING: std::sync::Mutex<Option<i64>> = std::sync::Mutex::new(None);

/// Silence between the joined recordings (helps speaker separation see the seam).
const GAP_SECONDS: f32 = 1.0;

pub fn window_minutes() -> u64 {
    WINDOW_MINUTES.load(Ordering::Relaxed)
}

pub fn set_window_minutes(minutes: u64) -> u64 {
    let m = minutes.min(120);
    WINDOW_MINUTES.store(m, Ordering::Relaxed);
    if m == 0 {
        discard();
    }
    m
}

/// The recording that can still be continued.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pending {
    pub meeting_id: i64,
    pub call_key: String,
    pub wav: PathBuf,
    /// Unix seconds when the recording ended.
    pub ended_at: u64,
    /// Length of the kept recording, so names from it can be carried over.
    pub duration_ms: u64,
}

/// Identifies "the same call": the app plus its meeting link, or its window title
/// when there is no link. None for recordings without a detected call.
pub fn call_key(info: Option<&MeetingInfo>) -> Option<String> {
    let m = info?;
    let place = if m.url.trim().is_empty() { m.title.trim() } else { m.url.trim() };
    if place.is_empty() {
        return None;
    }
    Some(format!("{}|{}|{}", m.platform.to_lowercase(), m.app_name.to_lowercase(), place.to_lowercase()))
}

fn dir() -> Option<PathBuf> {
    let d = crate::commands::meetings::get_meetings_dir().ok()?.join("continuation");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn state_file() -> Option<PathBuf> {
    dir().map(|d| d.join("pending.json"))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn load() -> Option<Pending> {
    serde_json::from_str(&std::fs::read_to_string(state_file()?).ok()?).ok()
}

/// Deletes the kept recording.
pub fn discard() {
    if let Some(p) = load() {
        let _ = std::fs::remove_file(&p.wav);
    }
    if let Some(f) = state_file() {
        let _ = std::fs::remove_file(f);
    }
}

/// Forgets the kept recording if it belongs to `meeting_id` (the meeting was deleted).
pub fn forget_meeting(meeting_id: i64) {
    if load().is_some_and(|p| p.meeting_id == meeting_id) {
        discard();
    }
}

/// Deletes the kept recording once its window has closed (or it is unusable).
pub fn cleanup_expired() {
    if let Some(p) = load() {
        let claimed = CLAIMED_MEETING.lock().ok().is_some_and(|claim| *claim == Some(p.meeting_id));
        if should_discard(&p, claimed, now(), window_minutes()) {
            discard();
        }
    }
}

fn should_discard(p: &Pending, claimed: bool, current_time: u64, minutes: u64) -> bool {
    !claimed && (current_time.saturating_sub(p.ended_at) > minutes * 60 || !p.wav.exists())
}

/// Pin the previous WAV as soon as a matching recording starts. It must not be
/// expired while the new recording is still in progress.
pub fn claim_for_recording(call_key: &str) {
    if window_minutes() == 0 {
        return;
    }
    if let Some(p) = load() {
        if p.call_key == call_key
            && now().saturating_sub(p.ended_at) <= window_minutes() * 60
            && p.wav.exists()
        {
            if let Ok(mut claim) = CLAIMED_MEETING.lock() {
                *claim = Some(p.meeting_id);
            }
        }
    }
}

pub fn release_recording_claim() {
    if let Ok(mut claim) = CLAIMED_MEETING.lock() {
        *claim = None;
    }
    cleanup_expired();
}

/// The kept recording, if `call_key` names the same call and the recording that
/// is ending now started within the window after it ended.
pub fn take_match(call_key: &str, recording_ms: u64) -> Option<Pending> {
    let minutes = window_minutes();
    if minutes == 0 {
        return None;
    }
    let p = load()?;
    let started_at = now().saturating_sub(recording_ms / 1000);
    let gap = started_at.saturating_sub(p.ended_at);
    let claimed = CLAIMED_MEETING.lock().ok().is_some_and(|claim| *claim == Some(p.meeting_id));
    (p.call_key == call_key && (claimed || gap <= minutes * 60) && p.wav.exists()).then_some(p)
}

/// Keeps `wav` (the full stereo recording of meeting `meeting_id`) so a restart
/// of the same call can continue it. Replaces any older kept recording.
pub fn remember(meeting_id: i64, call_key: &str, wav: &Path, duration_ms: u64) {
    if window_minutes() == 0 {
        return;
    }
    let Some(d) = dir() else { return };
    let dest = d.join(format!("meeting_{meeting_id}.wav"));
    let old = load();
    // A hard link costs nothing; the caller then deletes its own name for the file.
    if dest != wav {
        let _ = std::fs::remove_file(&dest); // an older version of the same meeting
    }
    let kept = dest == wav || std::fs::hard_link(wav, &dest).is_ok() || std::fs::copy(wav, &dest).is_ok();
    if !kept {
        return;
    }
    if let Some(o) = old.filter(|o| o.wav != dest) {
        let _ = std::fs::remove_file(o.wav);
    }
    let p = Pending { meeting_id, call_key: call_key.to_string(), wav: dest, ended_at: now(), duration_ms };
    if let (Some(f), Ok(json)) = (state_file(), serde_json::to_string(&p)) {
        let _ = std::fs::write(f, json);
    }
    // Delete it when the window closes, even if nothing else happens.
    let window = Duration::from_secs(window_minutes() * 60 + 5);
    std::thread::spawn(move || {
        std::thread::sleep(window);
        cleanup_expired();
    });
}

/// Writes `first` + `GAP_SECONDS` of silence + `second` to `out`. Both must have
/// the same channels, rate and sample format (recordings from this app do).
pub fn join_wavs(first: &Path, second: &Path, out: &Path) -> Result<(), String> {
    let a = hound::WavReader::open(first).map_err(|e| format!("{}: {e}", first.display()))?;
    let b = hound::WavReader::open(second).map_err(|e| format!("{}: {e}", second.display()))?;
    let spec = a.spec();
    if b.spec() != spec {
        return Err(format!("recordings differ ({:?} vs {:?})", spec, b.spec()));
    }
    let mut w = hound::WavWriter::create(out, spec).map_err(|e| e.to_string())?;
    let gap = (spec.sample_rate as f32 * GAP_SECONDS) as usize * spec.channels as usize;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in a.into_samples::<f32>() {
                w.write_sample(s.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
            for _ in 0..gap {
                w.write_sample(0.0f32).map_err(|e| e.to_string())?;
            }
            for s in b.into_samples::<f32>() {
                w.write_sample(s.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
        }
        hound::SampleFormat::Int => {
            for s in a.into_samples::<i32>() {
                w.write_sample(s.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
            for _ in 0..gap {
                w.write_sample(0i32).map_err(|e| e.to_string())?;
            }
            for s in b.into_samples::<i32>() {
                w.write_sample(s.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
        }
    }
    w.finalize().map_err(|e| e.to_string())
}

/// Carries names from the first recording onto the re-separated whole: a caller
/// whose speech in the first part overlaps most with a speaker the user had named
/// (or one the vault recognised) takes that name and id, unless the new run
/// already recognised them from the vault. `first_part_ms` is where part 1 ends.
pub fn carry_names(
    turns: &mut [crate::diarization::DiarizedTurn],
    old: &[crate::commands::meetings::StoredTurn],
    first_part_ms: u64,
) {
    use std::collections::HashMap;
    let named = |name: &str| !(name.starts_with("Speaker ") || name == "Remote Participant" || name == "You");
    // Overlap (ms) between each new caller id and each old named caller id, in part 1.
    let mut overlap: HashMap<(String, String), u64> = HashMap::new();
    for t in turns.iter().filter(|t| t.channel == 1 && t.start_ms < first_part_ms) {
        for (oid, oname, os, oe, och, _, _) in old.iter() {
            if *och != 1 || !named(oname) {
                continue;
            }
            let (s, e) = (t.start_ms.max(*os as u64), t.end_ms.min(*oe as u64));
            if e > s {
                *overlap.entry((t.speaker_id.clone(), oid.clone())).or_default() += e - s;
            }
        }
    }
    // Best old speaker for each new one, most overlap first; each old speaker used once.
    let mut pairs: Vec<((String, String), u64)> = overlap.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1));
    let mut used_new = std::collections::HashSet::new();
    let mut used_old = std::collections::HashSet::new();
    let mut rename: HashMap<String, (String, String)> = HashMap::new();
    for ((new_id, old_id), _) in pairs {
        if used_new.contains(&new_id) || used_old.contains(&old_id) {
            continue;
        }
        // Already a vault person in this run: the voice match wins.
        if !new_id.starts_with("speaker_remote_") {
            continue;
        }
        let name = old.iter().find(|o| o.0 == old_id).map(|o| o.1.clone()).unwrap_or_default();
        used_new.insert(new_id.clone());
        used_old.insert(old_id.clone());
        // Vault people keep their id; a per-meeting label ("speaker_remote_N") is
        // not reused, since the new run may give it to someone else.
        let id = if old_id.starts_with("speaker_remote_") { new_id.clone() } else { old_id };
        rename.insert(new_id, (id, name));
    }
    for t in turns.iter_mut() {
        if let Some((id, name)) = rename.get(&t.speaker_id) {
            t.speaker_id = id.clone();
            t.speaker_name = name.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::DiarizedTurn;

    #[test]
    fn active_continuation_survives_original_window() {
        let p = Pending {
            meeting_id: 1,
            call_key: "meet|chrome|abc".into(),
            wav: PathBuf::from("/nonexistent/taurscribe-test.wav"),
            ended_at: 100,
            duration_ms: 1_000,
        };
        assert!(!should_discard(&p, true, 1_000, 10));
        assert!(should_discard(&p, false, 1_000, 10));
    }

    fn info(url: &str, title: &str) -> MeetingInfo {
        MeetingInfo {
            pid: 1,
            app_name: "Google Chrome".into(),
            title: title.into(),
            url: url.into(),
            platform: "meet".into(),
            confidence: 90,
            should_record: true,
            is_using_mic: true,
            is_playing_audio: true,
        }
    }

    #[test]
    fn same_call_by_link_then_title() {
        let a = call_key(Some(&info("https://meet.google.com/abc-defg-hij", "Standup"))).unwrap();
        let b = call_key(Some(&info("https://meet.google.com/abc-defg-hij", "Standup (2)"))).unwrap();
        assert_eq!(a, b, "the link identifies the call; the title can change");
        assert_ne!(a, call_key(Some(&info("https://meet.google.com/xyz", "Standup"))).unwrap());
        assert!(call_key(Some(&info("", ""))).is_none());
        assert!(call_key(None).is_none());
    }

    #[test]
    fn joins_with_a_gap() {
        let d = std::env::temp_dir().join(format!("ts_join_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let spec = hound::WavSpec { channels: 2, sample_rate: 1000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        for (name, v, n) in [("a.wav", 0.5f32, 300), ("b.wav", -0.5, 200)] {
            let mut w = hound::WavWriter::create(d.join(name), spec).unwrap();
            for _ in 0..n * 2 {
                w.write_sample(v).unwrap();
            }
            w.finalize().unwrap();
        }
        join_wavs(&d.join("a.wav"), &d.join("b.wav"), &d.join("ab.wav")).unwrap();
        let r = hound::WavReader::open(d.join("ab.wav")).unwrap();
        assert_eq!(r.duration(), 300 + 1000 + 200);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn names_carry_over_by_overlap() {
        let t = |id: &str, name: &str, s: u64, e: u64| DiarizedTurn {
            speaker_id: id.into(),
            speaker_name: name.into(),
            start_ms: s,
            end_ms: e,
            channel: 1,
            text: String::new(),
            snippet_path: None,
            candidate_snippets: vec![],
            current_snippet_idx: 0,
        };
        let old: Vec<crate::commands::meetings::StoredTurn> = vec![
            ("person_bob".into(), "Bob".into(), 0, 5000, 1, None, None),
            ("speaker_remote_2".into(), "Speaker 2".into(), 5000, 9000, 1, None, None),
        ];
        let mut turns = vec![t("speaker_remote_1", "Speaker 1", 0, 5000), t("speaker_remote_2", "Speaker 2", 5000, 9000), t("speaker_remote_1", "Speaker 1", 12000, 15000)];
        carry_names(&mut turns, &old, 10_000);
        assert_eq!(turns[0].speaker_name, "Bob");
        assert_eq!(turns[2].speaker_id, "person_bob", "Bob's later turns follow");
        assert_eq!(turns[1].speaker_name, "Speaker 2", "unnamed speakers are left alone");
    }
}
