//! Keeps a meeting recording readable if the app quits while it is still capturing.
//!
//! The WAV header stores the audio length, and that length is only filled in when
//! the writer finishes or checkpoints. A force-quit used to leave hours of audio
//! on disk with a header that said the file was empty. Capture now checkpoints
//! while recording, and startup imports any meeting file that was still open.

use crate::diarization::DiarizedTurn;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MIN_RECOVERED_MS: u64 = 1_000;
const RECOVERED_NOTE: &str = "Taurscribe quit while this meeting was still recording. The audio was saved. Use Clean audio and rerun pipeline to transcribe it.";

/// Sibling of `meeting_….wav` that exists only while that recording is open.
pub fn open_marker(wav: &Path) -> PathBuf {
    let mut name = wav.file_name().unwrap_or_default().to_os_string();
    name.push(".open");
    wav.with_file_name(name)
}

pub fn mark_recording_open(wav: &Path) -> Result<(), String> {
    mark_recording_kind(wav, "open")
}

/// Recordings this process is still capturing. Startup recovery runs on a
/// background thread, and a meeting that starts meanwhile looks exactly like
/// one interrupted by a quit (an open marker, a header behind its audio, or
/// nothing on disk yet behind the write buffer): recovery must leave it alone.
static LIVE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
    std::sync::LazyLock::new(Default::default);

fn live() -> std::sync::MutexGuard<'static, std::collections::HashSet<PathBuf>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

fn is_live(wav: &Path) -> bool {
    live().contains(wav)
}

/// `kind` is `call`, `in_person`, or the older `open` marker that does not say which.
pub fn mark_recording_kind(wav: &Path, kind: &str) -> Result<(), String> {
    let body = match kind {
        "open" | "call" | "in_person" => kind,
        other => return Err(format!("Unknown recording kind {other}")),
    };
    // Registered before the marker exists, so a marker recovery can see always
    // belongs to a registered recording while this process captures it.
    live().insert(wav.to_path_buf());
    std::fs::write(open_marker(wav), body).map_err(|e| {
        live().remove(wav);
        format!("Could not mark {} for recovery: {e}", wav.display())
    })
}

pub fn clear_recording_open(wav: &Path) {
    let _ = std::fs::remove_file(open_marker(wav));
    live().remove(wav);
}

pub fn discard_recording_files(wav: &Path) {
    let _ = std::fs::remove_file(wav);
    clear_recording_open(wav);
}

/// Two seconds of audio, in samples (every channel counts).
pub fn should_checkpoint(channels: u16, sample_rate: u32, samples_since: u64) -> bool {
    let per_two_seconds = u64::from(sample_rate.max(1)) * u64::from(channels.max(1)) * 2;
    samples_since >= per_two_seconds
}

/// Makes `path` readable when its header length does not match the bytes on disk.
/// Returns whether the header was rewritten.
pub fn repair_wav_header(path: &Path) -> Result<bool, String> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len < 12 {
        return Err(format!("{} is too small to be a recording", path.display()));
    }
    let mut riff = [0u8; 12];
    file.read_exact(&mut riff).map_err(|e| e.to_string())?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(format!("{} is not a WAV recording", path.display()));
    }

    let mut pos = 12u64;
    let mut block_align = 1u64;
    loop {
        if pos + 8 > len {
            return Err(format!("{} has no audio data chunk", path.display()));
        }
        file.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let mut id = [0u8; 4];
        let mut size_buf = [0u8; 4];
        file.read_exact(&mut id).map_err(|e| e.to_string())?;
        file.read_exact(&mut size_buf).map_err(|e| e.to_string())?;
        let size = u32::from_le_bytes(size_buf) as u64;
        if &id == b"fmt " {
            let mut fmt = vec![0u8; size.min(32) as usize];
            file.read_exact(&mut fmt).map_err(|e| e.to_string())?;
            if fmt.len() >= 14 {
                block_align = u16::from_le_bytes([fmt[12], fmt[13]]) as u64;
            }
        }
        if &id == b"data" {
            let align = block_align.max(1);
            let available = len.saturating_sub(pos + 8);
            let mut aligned = available - (available % align);
            aligned = aligned.min(u32::MAX as u64);
            aligned -= aligned % align;
            // A finished file's declared length matches the bytes on disk
            // (apart from a single padding byte). A quit leaves 0, or the
            // length from the last checkpoint, while later audio is already there.
            let consistent = size <= available && available - size < align;
            if consistent {
                return Ok(false);
            }
            let riff_size = pos + 8 + aligned - 8;
            if riff_size > u32::MAX as u64 {
                return Err(format!("{} is larger than a WAV header can describe", path.display()));
            }
            file.seek(SeekFrom::Start(4)).map_err(|e| e.to_string())?;
            file.write_all(&(riff_size as u32).to_le_bytes()).map_err(|e| e.to_string())?;
            file.seek(SeekFrom::Start(pos + 4)).map_err(|e| e.to_string())?;
            file.write_all(&(aligned as u32).to_le_bytes()).map_err(|e| e.to_string())?;
            file.set_len(pos + 8 + aligned).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            return Ok(true);
        }
        if size == 0 {
            return Err(format!("{} has a truncated WAV header", path.display()));
        }
        pos += 8 + size + (size % 2);
    }
}

/// Imports meeting captures left open by a quit. A recording that was saved or
/// discarded has no open marker and a header that matches its audio, so it is
/// left alone. Returns how many meetings were created.
pub fn recover_interrupted_meetings(
    temp_dir: &Path,
    meetings_dir: &Path,
    conn: &rusqlite::Connection,
) -> Result<usize, String> {
    let entries = std::fs::read_dir(temp_dir)
        .map_err(|e| format!("Could not read recordings in {}: {e}", temp_dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| e.to_string())?.path());
    }
    let mut recovered = 0;
    for path in paths {
        match recover_one(&path, meetings_dir, conn) {
            Ok(true) => recovered += 1,
            Ok(false) => {}
            Err(e) => eprintln!("[WARN] Left interrupted recording {} in place: {e}", path.display()),
        }
    }
    Ok(recovered)
}

pub fn recover_from_app_dirs() -> Result<usize, String> {
    let temp = crate::utils::get_recordings_dir()?;
    let meetings = crate::commands::meetings::get_meetings_dir()?;
    let conn = crate::commands::meetings::open_connection()?;
    recover_interrupted_meetings(&temp, &meetings, &conn)
}

fn recover_one(path: &Path, meetings_dir: &Path, conn: &rusqlite::Connection) -> Result<bool, String> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
        return Ok(false);
    };
    if let Some(wav_name) = name.strip_suffix(".open") {
        if name.ends_with(".wav.open") && !path.with_file_name(wav_name).exists() && !is_live(&path.with_file_name(wav_name)) {
            let _ = std::fs::remove_file(path);
        }
        return Ok(false);
    }
    if !name.starts_with("meeting_") || !name.ends_with(".wav") {
        return Ok(false);
    }

    let marked = open_marker(path).exists();
    // Checked after reading the marker: a marker written by this process is
    // registered first, so a live recording is always caught here.
    if is_live(path) {
        return Ok(false);
    }
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let repaired = match repair_wav_header(path) {
        Ok(repaired) => repaired,
        Err(_) if marked && len < 44 => {
            discard_recording_files(path);
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    if !marked && !repaired {
        return Ok(false);
    }

    let duration_ms = match wav_duration_ms(path) {
        Ok(ms) => ms,
        Err(error) => return Err(format!("Could not time {}: {error}", path.display())),
    };
    if duration_ms < MIN_RECOVERED_MS {
        discard_recording_files(path);
        eprintln!("[INFO] Discarded a {duration_ms} ms interrupted meeting recording {}", path.display());
        return Ok(false);
    }

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name.as_str());
    let session_id = format!("recovered_{stem}");
    let audio_dir = meetings_dir.join("audio");
    let dest = audio_dir.join(format!("recovered_{name}"));
    let already_saved = conn
        .query_row::<i64, _, _>("SELECT id FROM meetings WHERE session_id = ?1", [&session_id], |row| row.get(0))
        .is_ok()
        || conn
            .query_row::<i64, _, _>(
                "SELECT id FROM meetings WHERE audio_path = ?1",
                [dest.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .is_ok();
    if already_saved {
        discard_recording_files(path);
        return Ok(false);
    }

    let channels = hound::WavReader::open(path).map(|r| r.spec().channels).unwrap_or(1);
    let kind = std::fs::read_to_string(open_marker(path)).unwrap_or_default();
    let platform = recovered_platform(kind.trim(), channels);
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or_else(|_| std::time::SystemTime::now());
    let local: chrono::DateTime<chrono::Local> = modified.into();
    let title = format!("Recovered meeting on {}", local.format("%b %-d, %Y"));
    std::fs::create_dir_all(&audio_dir).map_err(|e| e.to_string())?;
    std::fs::copy(path, &dest).map_err(|e| format!("Could not keep recovered audio: {e}"))?;
    let turns = vec![DiarizedTurn {
        speaker_id: "recovered".into(),
        speaker_name: "Recording".into(),
        start_ms: 0,
        end_ms: duration_ms,
        channel: 0,
        text: RECOVERED_NOTE.into(),
        snippet_path: None,
        candidate_snippets: Vec::new(),
        current_snippet_idx: 0,
    }];
    let saved = crate::commands::meetings::insert_completed_meeting(
        conn,
        &title,
        platform,
        "Taurscribe",
        "",
        duration_ms as i64,
        Some(&dest.to_string_lossy()),
        RECOVERED_NOTE,
        &turns,
    );
    let id = match saved {
        Ok(id) => id,
        Err(e) => {
            let _ = std::fs::remove_file(&dest);
            return Err(e);
        }
    };
    if let Err(e) = conn.execute(
        "UPDATE meetings SET session_id = ?1 WHERE id = ?2",
        rusqlite::params![session_id, id],
    ) {
        eprintln!("[WARN] Recovered meeting #{id} could not be marked so a later launch skips it: {e}");
    }
    discard_recording_files(path);
    println!("[INFO] Recovered interrupted meeting recording {name} as meeting #{id}");
    Ok(true)
}

fn wav_duration_ms(path: &Path) -> Result<u64, String> {
    let reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let rate = reader.spec().sample_rate.max(1) as u64;
    Ok(reader.duration() as u64 * 1000 / rate)
}

/// A stereo room recording must not be treated as a call: reprocessing a call
/// splits the channels into "you" and "the other people". An older marker that
/// only says the file was open still uses the channel count.
fn recovered_platform(kind: &str, channels: u16) -> &'static str {
    if kind == "in_person" || (kind != "call" && channels <= 1) {
        "In person"
    } else {
        "Direct Audio"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> hound::WavSpec {
        hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        }
    }

    fn write_meeting(path: &Path, frames: usize) {
        let mut writer = hound::WavWriter::create(path, spec()).unwrap();
        for _ in 0..frames * 2 {
            writer.write_sample(0.05).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn zero_data_length(path: &Path) {
        let bytes = std::fs::read(path).unwrap();
        let mut pos = 12usize;
        let mut patched = bytes.clone();
        while pos + 8 <= patched.len() {
            let size = u32::from_le_bytes(patched[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if &patched[pos..pos + 4] == b"data" {
                patched[4..8].copy_from_slice(&0u32.to_le_bytes());
                patched[pos + 4..pos + 8].copy_from_slice(&0u32.to_le_bytes());
                std::fs::write(path, patched).unwrap();
                return;
            }
            if size == 0 {
                break;
            }
            pos += 8 + size + (size % 2);
        }
        panic!("recording has no data chunk");
    }

    #[test]
    fn checkpoint_threshold_is_two_seconds_and_a_flush_is_readable_before_finish() {
        assert!(!should_checkpoint(2, 48_000, 192_000 - 1));
        assert!(should_checkpoint(2, 48_000, 192_000));
        let path = std::env::temp_dir().join(format!("taurscribe-checkpoint-{}.wav", std::process::id()));
        let mut writer = hound::WavWriter::create(&path, spec()).unwrap();
        for _ in 0..960 {
            writer.write_sample(0.2).unwrap();
        }
        writer.flush().unwrap();
        assert_eq!(hound::WavReader::open(&path).unwrap().duration(), 480);
        for _ in 0..960 {
            writer.write_sample(0.3).unwrap();
        }
        assert_eq!(hound::WavReader::open(&path).unwrap().duration(), 480, "audio past the last checkpoint stays invisible");
        writer.flush().unwrap();
        assert_eq!(hound::WavReader::open(&path).unwrap().duration(), 960);
        writer.finalize().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_quit_leaves_audio_bytes_with_a_zero_length_header_and_repair_restores_them() {
        let path = std::env::temp_dir().join(format!("taurscribe-repair-{}.wav", std::process::id()));
        write_meeting(&path, 2_400);
        let duration = hound::WavReader::open(&path).unwrap().duration();
        zero_data_length(&path);
        assert_eq!(hound::WavReader::open(&path).unwrap().duration(), 0);
        assert!(repair_wav_header(&path).unwrap());
        assert_eq!(hound::WavReader::open(&path).unwrap().duration(), duration);
        assert!(!repair_wav_header(&path).unwrap());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_open_meeting_is_imported_once_and_a_finished_file_is_not() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("taurscribe-recover-{nonce}"));
        let temp = root.join("temp");
        let meetings = root.join("meetings");
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::create_dir_all(meetings.join("audio")).unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&conn).unwrap();

        let finished = temp.join("meeting_finished.wav");
        write_meeting(&finished, 48_000);
        let open = temp.join("meeting_open.wav");
        write_meeting(&open, 48_000);
        std::fs::write(open_marker(&open), b"open").unwrap(); // left by an earlier run
        let short = temp.join("meeting_short.wav");
        write_meeting(&short, 480);
        std::fs::write(open_marker(&short), b"open").unwrap();

        assert_eq!(recover_interrupted_meetings(&temp, &meetings, &conn).unwrap(), 1);
        assert!(finished.exists(), "a finished recording without an open marker stays where it is");
        assert!(!open.exists());
        assert!(!open_marker(&open).exists());
        assert!(!short.exists());
        let (count, text, audio): (i64, String, String) = conn
            .query_row("SELECT COUNT(*), transcript_raw, audio_path FROM meetings", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        assert_eq!(count, 1);
        assert!(text.contains("quit while this meeting"));
        assert!(Path::new(&audio).exists());

        std::fs::copy(&audio, &open).unwrap();
        std::fs::write(open_marker(&open), b"open").unwrap(); // left by an earlier run
        assert_eq!(recover_interrupted_meetings(&temp, &meetings, &conn).unwrap(), 0);
        assert_eq!(conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM meetings", [], |row| row.get(0)).unwrap(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn audio_past_a_zero_header_is_recovered_even_without_the_open_marker() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("taurscribe-recover-header-{nonce}"));
        let temp = root.join("temp");
        let meetings = root.join("meetings");
        std::fs::create_dir_all(&temp).unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&conn).unwrap();
        let path = temp.join("meeting_crashed.wav");
        write_meeting(&path, 48_000);
        zero_data_length(&path);
        assert_eq!(recover_interrupted_meetings(&temp, &meetings, &conn).unwrap(), 1);
        assert!(!path.exists());
        let duration: i64 = conn.query_row("SELECT duration_ms FROM meetings", [], |row| row.get(0)).unwrap();
        assert_eq!(duration, 1_000);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_stereo_room_recording_is_not_recovered_as_a_call() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("taurscribe-recover-room-{nonce}"));
        let temp = root.join("temp");
        let meetings = root.join("meetings");
        std::fs::create_dir_all(&temp).unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&conn).unwrap();
        let path = temp.join("meeting_room.wav");
        write_meeting(&path, 48_000);
        std::fs::write(open_marker(&path), b"in_person").unwrap();
        assert_eq!(recover_interrupted_meetings(&temp, &meetings, &conn).unwrap(), 1);
        let platform: String = conn.query_row("SELECT platform FROM meetings", [], |row| row.get(0)).unwrap();
        assert_eq!(platform, "In person");
        let call = temp.join("meeting_call.wav");
        write_meeting(&call, 48_000);
        std::fs::write(open_marker(&call), b"call").unwrap();
        let room_mono = temp.join("meeting_room_mono.wav");
        let mut mono = hound::WavWriter::create(&room_mono, hound::WavSpec { channels: 1, ..spec() }).unwrap();
        for _ in 0..48_000 { mono.write_sample(0.05).unwrap(); }
        mono.finalize().unwrap();
        std::fs::write(open_marker(&room_mono), b"open").unwrap();
        assert_eq!(recover_interrupted_meetings(&temp, &meetings, &conn).unwrap(), 2);
        let mut platforms: Vec<String> = conn
            .prepare("SELECT platform FROM meetings ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        platforms.sort();
        assert_eq!(platforms, vec!["Direct Audio".to_string(), "In person".to_string(), "In person".to_string()]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_open_recording_that_cannot_be_read_is_kept() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("taurscribe-recover-unreadable-{nonce}"));
        let temp = root.join("temp");
        let meetings = root.join("meetings");
        std::fs::create_dir_all(&temp).unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&conn).unwrap();
        let path = temp.join("meeting_torn.wav");
        // A long WAV whose channel count is zero. The header length matches the
        // bytes, so repair leaves it alone, and a reader cannot time it.
        let payload = vec![0u8; 48_000 * 4];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36u32 + payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        std::fs::write(&path, bytes).unwrap();
        std::fs::write(open_marker(&path), b"open").unwrap();
        let _ = recover_interrupted_meetings(&temp, &meetings, &conn);
        assert!(path.exists(), "an unreadable interrupted recording must stay on disk");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_recording_that_is_still_capturing_is_never_recovered() {
        // Startup recovery runs on a background thread. A meeting recording that
        // starts meanwhile (auto-record, the CLI) must not be imported or deleted.
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("taurscribe-recover-live-{nonce}"));
        let temp = root.join("temp");
        let meetings = root.join("meetings");
        std::fs::create_dir_all(&temp).unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&conn).unwrap();
        // Created like recording.rs does: buffered, so nothing is on disk yet.
        let just_started = temp.join("meeting_just_started.wav");
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&just_started).unwrap();
        let mut fresh = hound::WavWriter::new(std::io::BufWriter::new(file), spec()).unwrap();
        mark_recording_kind(&just_started, "call").unwrap();
        // One that has been capturing for a while: its header lags the audio.
        let capturing = temp.join("meeting_capturing.wav");
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&capturing).unwrap();
        let mut writer = hound::WavWriter::new(std::io::BufWriter::new(file), spec()).unwrap();
        mark_recording_kind(&capturing, "call").unwrap();
        for _ in 0..96_000 { writer.write_sample(0.05).unwrap(); }
        writer.flush().unwrap();
        for _ in 0..96_000 { writer.write_sample(0.05).unwrap(); }

        assert_eq!(recover_interrupted_meetings(&temp, &meetings, &conn).unwrap(), 0);
        assert!(just_started.exists() && open_marker(&just_started).exists(), "a new recording was deleted");
        assert!(capturing.exists() && open_marker(&capturing).exists(), "a live recording was imported");
        assert_eq!(conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM meetings", [], |row| row.get(0)).unwrap(), 0);

        for _ in 0..2 { fresh.write_sample(0.0).unwrap(); }
        fresh.finalize().unwrap();
        writer.finalize().unwrap();
        clear_recording_open(&just_started);
        clear_recording_open(&capturing);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_meeting_saved_before_a_quit_is_not_imported_again() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("taurscribe-recover-twice-{nonce}"));
        let temp = root.join("temp");
        let meetings = root.join("meetings");
        std::fs::create_dir_all(meetings.join("audio")).unwrap();
        std::fs::create_dir_all(&temp).unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&conn).unwrap();
        let path = temp.join("meeting_saved.wav");
        write_meeting(&path, 48_000);
        std::fs::write(open_marker(&path), b"call").unwrap();
        // The insert committed and the copy was kept, then the process died
        // before the recovery id was stored and before the temp file was removed.
        let dest = meetings.join("audio").join("recovered_meeting_saved.wav");
        std::fs::copy(&path, &dest).unwrap();
        crate::commands::meetings::insert_completed_meeting(
            &conn, "Recovered meeting", "Direct Audio", "Taurscribe", "", 1_000,
            Some(&dest.to_string_lossy()), "already saved", &[],
        ).unwrap();
        assert_eq!(recover_interrupted_meetings(&temp, &meetings, &conn).unwrap(), 0);
        assert_eq!(conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM meetings", [], |row| row.get(0)).unwrap(), 1);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
