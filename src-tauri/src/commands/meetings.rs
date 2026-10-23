use crate::diarization::DiarizedTurn;
use crate::meeting_detector::{MeetingDetectionStatus, MeetingInfo};
use crate::meeting_summary::{extract_summary_heuristics, ActionItem};
use crate::state::AudioState;
use chrono::Utc;
use dirs::data_local_dir;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

// ── Database Access & Tables ────────────────────────────────────────────────

fn get_db_path() -> Result<PathBuf, String> {
    let app_data = data_local_dir().ok_or("Could not find AppData directory")?;
    let base = app_data.join("Taurscribe");
    if let Err(e) = std::fs::create_dir_all(&base) {
        return Err(format!("Failed to create Taurscribe data directory: {}", e));
    }
    Ok(base.join("transcript_history.db"))
}

pub fn get_meetings_dir() -> Result<PathBuf, String> {
    let dir = crate::storage::resolve_dir(crate::storage::Area::Recordings)?;
    let _ = std::fs::create_dir_all(dir.join("audio"));
    let _ = std::fs::create_dir_all(dir.join("snippets"));
    Ok(dir)
}

pub fn ensure_meetings_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meetings (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id          TEXT UNIQUE NOT NULL,
            title               TEXT NOT NULL,
            platform            TEXT NOT NULL,
            app_name            TEXT NOT NULL,
            url                 TEXT NOT NULL,
            created_at          TEXT NOT NULL,
            duration_ms         INTEGER NOT NULL,
            audio_path          TEXT,
            transcript_raw      TEXT NOT NULL,
            summary             TEXT,
            action_items        TEXT,
            category            TEXT NOT NULL,
            speaker_count       INTEGER NOT NULL DEFAULT 1,
            reviewed            INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS meeting_tags (
            meeting_id INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
            tag TEXT NOT NULL,
            PRIMARY KEY(meeting_id, tag)
        );

        CREATE INDEX IF NOT EXISTS idx_meetings_created_at
            ON meetings(created_at DESC);

        CREATE TABLE IF NOT EXISTS meeting_turns (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            meeting_id          INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
            speaker_id          TEXT NOT NULL,
            speaker_name        TEXT NOT NULL,
            start_ms            INTEGER NOT NULL,
            end_ms              INTEGER NOT NULL,
            channel             INTEGER NOT NULL,
            text                TEXT NOT NULL,
            snippet_path        TEXT,
            candidate_snippets  TEXT,
            current_snippet_idx INTEGER DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_meeting_turns_meeting_id
            ON meeting_turns(meeting_id, start_ms ASC);

        CREATE TABLE IF NOT EXISTS speaker_vault (
            id                  TEXT PRIMARY KEY,
            name                TEXT NOT NULL,
            created_at          TEXT NOT NULL,
            sample_count        INTEGER NOT NULL DEFAULT 1,
            last_seen           TEXT NOT NULL,
            snippet_path        TEXT,
            features_json       TEXT,
            embedding_json      TEXT,
            candidate_snippets  TEXT
        );

        -- Speakers the user merged in a meeting ("these two are one person"),
        -- newest last. Each row keeps what the merged turns were before, so
        -- the merge can be undone exactly.
        CREATE TABLE IF NOT EXISTS meeting_speaker_merges (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            meeting_id          INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
            from_speaker_id     TEXT NOT NULL,
            from_speaker_name   TEXT NOT NULL,
            into_speaker_id     TEXT NOT NULL,
            into_speaker_name   TEXT NOT NULL,
            turns_json          TEXT NOT NULL,
            created_at          TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_meeting_speaker_merges_meeting_id
            ON meeting_speaker_merges(meeting_id, id);

        -- The user's own voiceprint, for telling "You" apart in in-person
        -- meetings (one microphone). Never part of the speaker vault.
        CREATE TABLE IF NOT EXISTS my_voice (
            id                  INTEGER PRIMARY KEY CHECK (id = 1),
            embedding_json      TEXT NOT NULL,
            speech_seconds      REAL NOT NULL,
            created_at          TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("Failed to initialize meetings schema: {}", e))?;

    let _ = conn.execute("ALTER TABLE meetings ADD COLUMN transcription_model TEXT", []);
    // Incremental column migrations for existing databases
    let _ = conn.execute("ALTER TABLE meeting_turns ADD COLUMN candidate_snippets TEXT", []);
    let _ = conn.execute("ALTER TABLE meeting_turns ADD COLUMN current_snippet_idx INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE speaker_vault ADD COLUMN embedding_json TEXT", []);
    let _ = conn.execute("ALTER TABLE speaker_vault ADD COLUMN candidate_snippets TEXT", []);
    // Number of meetings a vault person appeared in (sample_count = voice clips averaged).
    let _ = conn.execute("ALTER TABLE speaker_vault ADD COLUMN meeting_count INTEGER NOT NULL DEFAULT 0", []);
    // Recordings joined into this meeting (restarting the same call continues it).
    let _ = conn.execute("ALTER TABLE meetings ADD COLUMN parts INTEGER NOT NULL DEFAULT 1", []);
    let _ = conn.execute("ALTER TABLE meetings ADD COLUMN recording_sources TEXT", []);
    // Older versions did not track review saves. Mark their existing meetings
    // reviewed, preserving metadata rather than risking loss of edits that cannot
    // be distinguished from generated notes. The column's default stays 0 so
    // meetings saved from now on start unreviewed; only the rows present when
    // the column is first added are marked.
    if conn.execute("ALTER TABLE meetings ADD COLUMN reviewed INTEGER NOT NULL DEFAULT 0", []).is_ok() {
        conn.execute("UPDATE meetings SET reviewed = 1", [])
            .map_err(|e| format!("Failed to migrate meeting reviews: {}", e))?;
    }

    super::voice_bank::ensure_voice_bank_schema(conn)

}

// ── Speaker vault helpers ───────────────────────────────────────────────────
//
// A vault entry is a real person with a permanent id. People enter the vault when
// the user names a caller; after that, the diarizer links callers whose voice
// matches (see diarization.rs) and each meeting refines the stored voiceprint.
// Per-meeting labels (speaker_remote_1, …) are never used as vault ids: they
// restart every meeting and would merge different people into one entry.

/// Voiceprint for a caller from their combined meeting speech (written by the
/// diarizer next to their playback clip), or None when there was too little.
fn embedding_from_wav(snippet_path: &str) -> Option<Vec<f32>> {
    let voice = crate::diarization::voice_sample_path(snippet_path);
    crate::speaker_embedding::embedding_from_wav(&voice.to_string_lossy())
}

/// Running mean of L2-normalised embeddings: `n` samples already averaged.
pub(crate) fn average_embedding(current: &[f32], n: i64, new: &[f32]) -> Vec<f32> {
    if current.len() != new.len() || current.is_empty() || n <= 0 {
        return new.to_vec();
    }
    let n = n as f32;
    let mut mean: Vec<f32> = current.iter().zip(new).map(|(c, x)| (c * n + x) / (n + 1.0)).collect();
    let norm = mean.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        mean.iter_mut().for_each(|v| *v /= norm);
    }
    mean
}

/// Whether the speaker recognition model is installed (the vault needs it).
pub(crate) fn speaker_model_installed() -> bool {
    if cfg!(test) {
        return std::env::var("TAURSCRIBE_TEST_NO_SPEAKER_MODEL").is_err();
    }
    crate::speaker_embedding::get_speaker_engine()
        .lock()
        .map(|e| e.uses_neural_model())
        .unwrap_or(false)
}

fn new_vault_id() -> String {
    format!("person_{}_{:04x}", Utc::now().timestamp_millis(), rand::random::<u16>())
}

/// Adds one meeting's voice sample to an existing vault person.
fn record_vault_appearance(conn: &Connection, vault_id: &str, snippet: Option<&str>, candidates_json: Option<&str>, meeting_id: Option<i64>, new_meeting: bool) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    let (emb_json, samples): (Option<String>, i64) = conn
        .query_row(
            "SELECT embedding_json, sample_count FROM speaker_vault WHERE id = ?1",
            params![vault_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let new_emb = snippet.and_then(embedding_from_wav);
    if let Some(new) = &new_emb {
        super::voice_bank::add_voiceprint(conn, vault_id, new, snippet, meeting_id)?;
    }
    let (emb_json, samples) = match (new_emb, emb_json.and_then(|j| serde_json::from_str::<Vec<f32>>(&j).ok())) {
        (Some(new), Some(cur)) => (Some(average_embedding(&cur, samples, &new)), samples + 1),
        (Some(new), None) => (Some(new), 1),
        (None, cur) => (cur, samples),
    };
    conn.execute(
        r#"
        UPDATE speaker_vault SET
            meeting_count = meeting_count + ?7,
            sample_count = ?2,
            last_seen = ?3,
            embedding_json = COALESCE(?4, embedding_json),
            snippet_path = COALESCE(snippet_path, ?5),
            candidate_snippets = COALESCE(candidate_snippets, ?6)
        WHERE id = ?1
        "#,
        params![
            vault_id,
            samples,
            now,
            emb_json.map(|e| serde_json::to_string(&e).unwrap_or_default()),
            snippet,
            candidates_json,
            i64::from(new_meeting)
        ],
    ).map_err(|e|e.to_string())?;
    Ok(())
}

pub fn load_vault_embeddings_internal(
    conn: &Connection,
) -> Result<Vec<crate::speaker_embedding::VaultSpeakerEmbedding>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, name, embedding_json, snippet_path
            FROM speaker_vault
            WHERE embedding_json IS NOT NULL AND embedding_json != ''
            "#,
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let emb_str: String = row.get(2)?;
            let snippet_path: Option<String> = row.get(3)?;
            let embedding: Vec<f32> = serde_json::from_str(&emb_str).unwrap_or_default();
            Ok(crate::speaker_embedding::VaultSpeakerEmbedding {
                speaker_id: id,
                display_name: name,
                embedding,
                snippet_path,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut list = Vec::new();
    for r in rows {
        if let Ok(entry) = r {
            if !entry.embedding.is_empty() {
                list.push(entry);
            }
        }
    }
    Ok(list)
}

pub fn open_connection() -> Result<Connection, String> {
    let path = get_db_path()?;
    let conn = Connection::open(&path)
        .map_err(|e| format!("Failed to open DB at {}: {}", path.display(), e))?;
    ensure_meetings_schema(&conn)?;
    Ok(conn)
}

/// Meetings saved before playback copies existed still point at their raw WAV
/// (~1.4 GB/hour). Converts each to the small Opus copy and repoints the row.
/// Returns how many were converted.
pub fn compress_legacy_meeting_audio(conn: &Connection) -> usize {
    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT id, audio_path FROM meetings WHERE lower(audio_path) LIKE '%.wav'")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map(|it| it.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();
    let mut converted = 0;
    for (id, wav) in rows {
        let wav_path = std::path::Path::new(&wav);
        if !wav_path.exists() {
            continue;
        }
        if convert_meeting_audio(conn, id, wav_path).is_ok() {
            converted += 1;
        }
    }
    converted
}

/// Keep the WAV until the new path has been committed to SQLite. A failed
/// conversion or DB update leaves the original meeting playable and retryable.
pub fn convert_meeting_audio(conn: &Connection, id: i64, wav: &std::path::Path) -> Result<(), String> {
    // Startup migration can overlap a recording's completion. Each attempt
    // owns its output so losing the DB race can only delete its own file.
    let destination = wav.with_extension(format!("{:032x}.webm", rand::random::<u128>()));
    let webm = crate::meeting_audio::compress_for_playback_to(wav, &destination)?;
    let updated = conn.execute(
        "UPDATE meetings SET audio_path = ?1 WHERE id = ?2 AND audio_path = ?3",
        params![webm.to_string_lossy(), id, wav.to_string_lossy()],
    )
    .map_err(|e| format!("Failed to update meeting audio path: {}", e))
    .and_then(|changed| if changed == 1 { Ok(()) } else { Err("Meeting audio path changed before conversion".into()) });
    if let Err(error) = updated {
        let _ = std::fs::remove_file(&webm);
        return Err(error);
    }
    if let Err(e) = std::fs::remove_file(wav) {
        eprintln!("[WARN] Could not remove old meeting WAV {}: {}", wav.display(), e);
    }
    Ok(())
}

// ── Data Transfer Objects ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSummaryRecord {
    pub id: i64,
    pub session_id: String,
    pub title: String,
    pub platform: String,
    pub app_name: String,
    pub url: String,
    pub created_at: String,
    pub duration_ms: i64,
    pub category: String,
    pub speaker_count: i64,
    pub snippet_preview: String,
    pub action_item_count: usize,
    pub has_audio: bool,
    /// Had a recording whose file is now gone.
    #[serde(default)]
    pub audio_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingDetailRecord {
    #[serde(default)]
    pub transcription_model: Option<String>,
    #[serde(default)]
    pub turn_ids: Vec<i64>,
    pub id: i64,
    pub session_id: String,
    pub title: String,
    pub platform: String,
    pub app_name: String,
    pub url: String,
    pub created_at: String,
    pub duration_ms: i64,
    pub audio_path: Option<String>,
    pub transcript_raw: String,
    pub summary: Vec<String>,
    pub action_items: Vec<ActionItem>,
    pub category: String,
    pub speaker_count: i64,
    pub turns: Vec<DiarizedTurn>,
    #[serde(default)]
    pub recording_sources: Vec<crate::recording_import::RecordingSource>,
    /// The meeting has an audio path but the file is gone (deleted or moved).
    /// The transcript is still complete.
    #[serde(default)]
    pub audio_missing: bool,
    /// Speaker clips that are gone; they are left out of `turns`.
    #[serde(default)]
    pub clips_missing: usize,
    /// Speaker merges the user can undo, newest first.
    #[serde(default)]
    pub merges: Vec<SpeakerMergeRecord>,
}

/// One "these two speakers are the same person" merge in a meeting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeakerMergeRecord {
    pub id: i64,
    pub from_speaker_id: String,
    pub from_speaker_name: String,
    pub into_speaker_id: String,
    pub into_speaker_name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerVaultRecord {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub sample_count: i64,
    /// Meetings this person appeared in (shown as "calls").
    #[serde(default)]
    pub meeting_count: i64,
    pub last_seen: String,
    pub snippet_path: Option<String>,
    #[serde(default)]
    pub candidate_snippets: Vec<String>,
    /// The person's voice clip is gone (the voiceprint still works).
    #[serde(default)]
    pub clip_missing: bool,
}

/// Whether a stored file path still points at a file.
fn file_exists(path: &Option<String>) -> bool {
    path.as_deref().is_some_and(|p| std::path::Path::new(p).exists())
}

/// Drops clips whose files are gone from a turn (or vault person); returns how
/// many were dropped. The selected clip falls back to the first one left.
fn drop_missing_clips(snippet: &mut Option<String>, candidates: &mut Vec<String>, idx: Option<&mut usize>) -> usize {
    let before = candidates.len() + snippet.iter().filter(|p| !candidates.contains(p)).count();
    candidates.retain(|c| std::path::Path::new(c).exists());
    if !file_exists(snippet) {
        *snippet = candidates.first().cloned();
    }
    if let Some(i) = idx {
        *i = snippet.as_ref().and_then(|s| candidates.iter().position(|c| c == s)).unwrap_or(0);
    }
    let after = candidates.len() + snippet.iter().filter(|p| !candidates.contains(p)).count();
    before.saturating_sub(after)
}

// ── Meeting Insertion & Turn Processing ────────────────────────────────────

fn insert_turns(tx: &Connection, meeting_id: i64, turns: &[DiarizedTurn]) -> Result<(), String> {
    for t in turns {
        let cand_json = serde_json::to_string(&t.candidate_snippets).unwrap_or_default();
        tx.execute(
            r#"
            INSERT INTO meeting_turns (
                meeting_id, speaker_id, speaker_name, start_ms, end_ms, channel, text, snippet_path,
                candidate_snippets, current_snippet_idx
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                meeting_id,
                t.speaker_id,
                t.speaker_name,
                t.start_ms as i64,
                t.end_ms as i64,
                t.channel as i64,
                t.text,
                t.snippet_path,
                cand_json,
                t.current_snippet_idx as i64,
            ],
        )
        .map_err(|e| format!("Failed to insert turn: {}", e))?;
    }
    Ok(())
}

/// Vault: only callers the diarizer linked to an enrolled person (their turns
/// carry that person's vault id) update the vault, once per meeting. People in
/// `already_counted` were counted for this meeting before (a continued call).
fn record_vault_appearances(tx: &Connection, meeting_id: i64, turns: &[DiarizedTurn], already_counted: &std::collections::HashSet<String>) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for t in turns.iter().filter(|t| t.channel == 1 && speaker_model_installed()) {
        if already_counted.contains(&t.speaker_id) || !seen.insert(t.speaker_id.clone()) {
            continue;
        }
        let enrolled: bool = tx
            .query_row("SELECT 1 FROM speaker_vault WHERE id = ?1", params![t.speaker_id], |_| Ok(true))
            .unwrap_or(false);
        if enrolled {
            let cand_json = serde_json::to_string(&t.candidate_snippets).ok();
            record_vault_appearance(tx, &t.speaker_id, t.snippet_path.as_deref(), cand_json.as_deref(), Some(meeting_id), true)?;
        }
    }
    Ok(())
}

/// A saved meeting's turns as (speaker_id, speaker_name, start_ms, end_ms, channel,
/// snippet_path, candidate_snippets JSON).
pub type StoredTurn = (String, String, i64, i64, i64, Option<String>, Option<String>);

pub fn stored_turns(conn: &Connection, meeting_id: i64) -> Vec<StoredTurn> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT speaker_id, speaker_name, start_ms, end_ms, channel, snippet_path, candidate_snippets \
         FROM meeting_turns WHERE meeting_id = ?1 ORDER BY start_ms",
    ) else {
        return Vec::new();
    };
    stmt.query_map(params![meeting_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Replaces a meeting's recording with a longer one that continues it (the user
/// stopped and restarted recording the same call): new audio, duration,
/// transcript and turns; one more part. The old turns' clips and the old audio
/// file are deleted. Vault people already counted for this meeting are not
/// counted again.
pub fn replace_meeting_content(
    conn: &Connection,
    meeting_id: i64,
    duration_ms: i64,
    audio_path: Option<&str>,
    transcript_raw: &str,
    turns: &[DiarizedTurn],
) -> Result<(), String> {
    let vault_audio = retained_vault_audio(conn)?;
    let old_turns = stored_turns(conn, meeting_id);
    let old_audio: Option<String> = conn
        .query_row("SELECT audio_path FROM meetings WHERE id = ?1", params![meeting_id], |r| r.get(0))
        .map_err(|e| format!("Meeting {meeting_id} not found: {e}"))?;
    let title: String = conn
        .query_row("SELECT title FROM meetings WHERE id = ?1", params![meeting_id], |r| r.get(0))
        .unwrap_or_default();

    let tx = conn.unchecked_transaction().map_err(|e| format!("Failed to begin meeting transaction: {}", e))?;
    let mut summary_output = extract_summary_heuristics(turns, &title);
    let (reviewed, previous_duration, category, summary, actions): (bool, i64, String, Option<String>, Option<String>) = tx
        .query_row("SELECT reviewed, duration_ms, category, summary, action_items FROM meetings WHERE id = ?1", [meeting_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .map_err(|e| format!("Failed to read meeting review: {e}"))?;
    if reviewed {
        summary_output.category = category;
        summary_output.summary = serde_json::from_str(summary.as_deref().unwrap_or("[]"))
            .map_err(|e| format!("Invalid saved meeting summary: {e}"))?;
        summary_output.action_items = serde_json::from_str(actions.as_deref().unwrap_or("[]"))
            .map_err(|e| format!("Invalid saved meeting action items: {e}"))?;
        // Only suggest notes from the appended recording. Re-extracting the old
        // speech would bring back deleted tasks or replace reviewed assignments.
        let appended: Vec<_> = turns.iter().filter(|t| t.start_ms >= previous_duration.max(0) as u64).cloned().collect();
        if !appended.is_empty() {
            let additions = extract_summary_heuristics(&appended, &title);
            for point in additions.summary {
                if !summary_output.summary.contains(&point) {
                    summary_output.summary.push(point);
                }
            }
            for mut action in additions.action_items {
                if summary_output.action_items.iter().any(|old| old.task.trim().eq_ignore_ascii_case(action.task.trim()) && old.assignee == action.assignee) {
                    continue;
                }
                loop {
                    action.id = format!("continued_action_{:016x}", rand::random::<u64>());
                    if !summary_output.action_items.iter().any(|old| old.id == action.id) { break; }
                }
                summary_output.action_items.push(action);
            }
        }
    }
    let mut speaker_ids: Vec<&str> = turns.iter().map(|t| t.speaker_id.as_str()).collect();
    speaker_ids.sort();
    speaker_ids.dedup();
    tx.execute(
        r#"
        UPDATE meetings SET duration_ms = ?2, audio_path = ?3, transcript_raw = ?4, summary = ?5,
            action_items = ?6, category = ?7, speaker_count = ?8, parts = parts + 1
        WHERE id = ?1
        "#,
        params![
            meeting_id,
            duration_ms,
            audio_path,
            transcript_raw,
            serde_json::to_string(&summary_output.summary).unwrap_or_default(),
            serde_json::to_string(&summary_output.action_items).unwrap_or_default(),
            summary_output.category,
            speaker_ids.len().max(1) as i64,
        ],
    )
    .map_err(|e| format!("Failed to update meeting: {}", e))?;
    tx.execute("DELETE FROM meeting_turns WHERE meeting_id = ?1", params![meeting_id])
        .map_err(|e| format!("Failed to clear meeting turns: {}", e))?;
    // The old turns are gone, so their merges can no longer be undone.
    tx.execute("DELETE FROM meeting_speaker_merges WHERE meeting_id = ?1", params![meeting_id])
        .map_err(|e| format!("Failed to clear speaker merges: {}", e))?;
    insert_turns(&tx, meeting_id, turns)?;
    let counted: std::collections::HashSet<String> = old_turns.iter().map(|t| t.0.clone()).collect();
    record_vault_appearances(&tx, meeting_id, turns, &counted)?;
    tx.commit().map_err(|e| format!("Failed to commit meeting: {}", e))?;

    // Files the old version used and neither the new one nor the vault uses.
    let mut keep: std::collections::HashSet<String> = turns
        .iter()
        .flat_map(|t| t.snippet_path.iter().chain(t.candidate_snippets.iter()).cloned())
        .collect();
    keep.extend(vault_audio);
    protect_voice_companions(&mut keep);
    for (_, _, _, _, _, snip, cands) in &old_turns {
        let mut paths: Vec<String> = cands.as_deref().and_then(|c| serde_json::from_str(c).ok()).unwrap_or_default();
        paths.extend(snip.clone());
        for p in paths.into_iter().filter(|p| !keep.contains(p)) {
            remove_unused_speaker_clip(&p, &keep);
        }
    }
    if let Some(old) = old_audio.filter(|a| Some(a.as_str()) != audio_path) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

pub fn insert_completed_meeting(
    conn: &Connection,
    title: &str,
    platform: &str,
    app_name: &str,
    url: &str,
    duration_ms: i64,
    audio_path: Option<&str>,
    transcript_raw: &str,
    turns: &[DiarizedTurn],
) -> Result<i64, String> {
    insert_meeting_with_sources(conn, title, platform, app_name, url, duration_ms, audio_path, transcript_raw, turns, &[])
}

pub(super) fn insert_meeting_with_sources(
    conn: &Connection,
    title: &str,
    platform: &str,
    app_name: &str,
    url: &str,
    duration_ms: i64,
    audio_path: Option<&str>,
    transcript_raw: &str,
    turns: &[DiarizedTurn],
    sources: &[crate::recording_import::RecordingSource],
) -> Result<i64, String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("Failed to begin meeting transaction: {}", e))?;
    // Random suffix: two meetings saved within the same millisecond must not collide.
    let session_id = format!(
        "call_{}_{}_{:06x}",
        Utc::now().timestamp_millis(),
        std::process::id(),
        rand::random::<u32>() & 0xff_ffff
    );
    let created_at = sources.iter().filter_map(|s| s.recorded_start.as_deref())
        .filter_map(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).min()
        .map(|d| d.with_timezone(&Utc).to_rfc3339()).unwrap_or_else(|| Utc::now().to_rfc3339());

    // Generate heuristics summary & action items
    let summary_output = extract_summary_heuristics(turns, title);
    let summary_json = serde_json::to_string(&summary_output.summary).unwrap_or_default();
    let actions_json = serde_json::to_string(&summary_output.action_items).unwrap_or_default();

    let mut speaker_ids: Vec<String> = turns.iter().map(|t| t.speaker_id.clone()).collect();
    speaker_ids.sort();
    speaker_ids.dedup();
    let speaker_count = speaker_ids.len().max(1) as i64;

    tx.execute(
        r#"
        INSERT INTO meetings (
            session_id, title, platform, app_name, url, created_at, duration_ms,
            audio_path, transcript_raw, summary, action_items, category, speaker_count,
            reviewed
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0)
        "#,
        params![
            session_id,
            title,
            platform,
            app_name,
            url,
            created_at,
            duration_ms,
            audio_path,
            transcript_raw,
            summary_json,
            actions_json,
            summary_output.category,
            speaker_count,
        ],
    )
    .map_err(|e| format!("Failed to insert meeting: {}", e))?;

    let meeting_id = tx.last_insert_rowid();
    if !sources.is_empty() {
        let json = serde_json::to_string(sources).map_err(|e| e.to_string())?;
        tx.execute("UPDATE meetings SET recording_sources = ?1, parts = ?2 WHERE id = ?3", params![json, sources.len() as i64, meeting_id]).map_err(|e| e.to_string())?;
    }

    insert_turns(&tx, meeting_id, turns)?;
    record_vault_appearances(&tx, meeting_id, turns, &std::collections::HashSet::new())?;

    tx.commit().map_err(|e| format!("Failed to commit meeting: {}", e))?;
    Ok(meeting_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCount {
    pub platform: String,
    pub count: i64,
}

// ── Tauri Commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_meeting_platform_counts() -> Result<Vec<PlatformCount>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        let mut stmt = conn
            .prepare("SELECT platform, COUNT(*) FROM meetings GROUP BY platform ORDER BY COUNT(*) DESC")
            .map_err(|e| format!("Failed to prepare platform counts query: {}", e))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(PlatformCount {
                    platform: row.get(0)?,
                    count: row.get(1)?,
                })
            })
            .map_err(|e| format!("Failed to execute query: {}", e))?;

        let mut counts = Vec::new();
        for r in rows {
            if let Ok(item) = r {
                counts.push(item);
            }
        }
        Ok(counts)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
pub async fn list_meetings(
    search: Option<String>,
    category: Option<String>,
    platform: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<MeetingSummaryRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || list_meetings_in(&open_connection()?, search, category, platform, limit, offset))
        .await
        .map_err(|e| format!("Task failed: {}", e))?
}

fn list_meetings_in(
    conn: &Connection,
    search: Option<String>,
    category: Option<String>,
    platform: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<MeetingSummaryRecord>, String> {
    let limit_val = limit.unwrap_or(50) as i64;
    let offset_val = offset.unwrap_or(0) as i64;

    let mut query = "SELECT id, session_id, title, platform, app_name, url, created_at, duration_ms, category, speaker_count, transcript_raw, action_items, audio_path FROM meetings WHERE 1=1".to_string();
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(ref cat) = category {
        if !cat.trim().is_empty() && cat.to_lowercase() != "all" {
            query.push_str(" AND category = ?");
            param_values.push(Box::new(cat.clone()));
        }
    }

    if let Some(ref plat) = platform {
        if !plat.trim().is_empty() && plat.to_lowercase() != "all" {
            query.push_str(" AND LOWER(platform) = LOWER(?)");
            param_values.push(Box::new(plat.trim().to_string()));
        }
    }

    if let Some(ref s) = search {
        if !s.trim().is_empty() {
            query.push_str(" AND (title LIKE ? ESCAPE '\\' OR transcript_raw LIKE ? ESCAPE '\\' OR EXISTS (SELECT 1 FROM meeting_tags WHERE meeting_tags.meeting_id = meetings.id AND tag LIKE ? ESCAPE '\\'))");
            // "100%" or "file_name" are text to find, not LIKE wildcards.
            let pattern = format!("%{}%", s.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
            param_values.push(Box::new(pattern.clone()));
            param_values.push(Box::new(pattern.clone()));
            param_values.push(Box::new(pattern));
        }
    }

    query.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
    param_values.push(Box::new(limit_val));
    param_values.push(Box::new(offset_val));

    let mut stmt = conn
        .prepare(&query)
        .map_err(|e| format!("Failed to prepare query: {}", e))?;

    let params_refs: Vec<&dyn rusqlite::ToSql> =
        param_values.iter().map(|b| b.as_ref()).collect();

    let rows = stmt
        .query_map(params_refs.as_slice(), |row| {
            let id: i64 = row.get(0)?;
            let session_id: String = row.get(1)?;
            let title: String = row.get(2)?;
            let platform: String = row.get(3)?;
            let app_name: String = row.get(4)?;
            let url: String = row.get(5)?;
            let created_at: String = row.get(6)?;
            let duration_ms: i64 = row.get(7)?;
            let cat: String = row.get(8)?;
            let speaker_count: i64 = row.get(9)?;
            let raw_text: String = row.get(10)?;
            let actions_raw: Option<String> = row.get(11)?;
            let audio_path: Option<String> = row.get(12)?;

            let action_count = actions_raw
                .and_then(|a| serde_json::from_str::<Vec<ActionItem>>(&a).ok())
                .map(|v| v.len())
                .unwrap_or(0);

            // Cut on a character boundary (byte slicing panicked on accents/emoji).
            let preview = if raw_text.chars().count() > 140 {
                format!("{}...", raw_text.chars().take(137).collect::<String>())
            } else {
                raw_text
            };

            let has_audio = file_exists(&audio_path);
            let audio_missing = audio_path.is_some() && !has_audio;

            Ok(MeetingSummaryRecord {
                id,
                session_id,
                title,
                platform,
                app_name,
                url,
                created_at,
                duration_ms,
                category: cat,
                speaker_count,
                snippet_preview: preview,
                action_item_count: action_count,
                has_audio,
                audio_missing,
            })
        })
        .map_err(|e| format!("Query failed: {}", e))?;

    let mut records = Vec::new();
    for r in rows {
        if let Ok(rec) = r {
            records.push(rec);
        }
    }

    Ok(records)
}

#[tauri::command]
pub async fn get_meeting_detail(meeting_id: i64) -> Result<MeetingDetailRecord, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;

        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, title, platform, app_name, url, created_at,
                       duration_ms, audio_path, transcript_raw, summary, action_items,
                       category, speaker_count
                FROM meetings WHERE id = ?1
                "#,
            )
            .map_err(|e| format!("Query error: {}", e))?;

        let meeting = stmt
            .query_row(params![meeting_id], |row| {
                let id: i64 = row.get(0)?;
                let session_id: String = row.get(1)?;
                let title: String = row.get(2)?;
                let platform: String = row.get(3)?;
                let app_name: String = row.get(4)?;
                let url: String = row.get(5)?;
                let created_at: String = row.get(6)?;
                let duration_ms: i64 = row.get(7)?;
                let audio_path: Option<String> = row.get(8)?;
                let transcript_raw: String = row.get(9)?;
                let summary_raw: Option<String> = row.get(10)?;
                let actions_raw: Option<String> = row.get(11)?;
                let category: String = row.get(12)?;
                let speaker_count: i64 = row.get(13)?;

                let summary = summary_raw
                    .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                    .unwrap_or_default();
                let action_items = actions_raw
                    .and_then(|a| serde_json::from_str::<Vec<ActionItem>>(&a).ok())
                    .unwrap_or_default();

                Ok((
                    id,
                    session_id,
                    title,
                    platform,
                    app_name,
                    url,
                    created_at,
                    duration_ms,
                    audio_path,
                    transcript_raw,
                    summary,
                    action_items,
                    category,
                    speaker_count,
                ))
            })
            .map_err(|e| format!("Meeting not found: {}", e))?;

        // Query conversational turns
        let mut turn_stmt = conn
            .prepare(
                r#"
                SELECT speaker_id, speaker_name, start_ms, end_ms, channel, text, snippet_path,
                       candidate_snippets, current_snippet_idx, id
                FROM meeting_turns WHERE meeting_id = ?1 ORDER BY start_ms ASC, id ASC
                "#,
            )
            .map_err(|e| format!("Turn query error: {}", e))?;

        let turn_rows = turn_stmt
            .query_map(params![meeting_id], |row| {
                let cand_str: Option<String> = row.get(7)?;
                let cand_vec: Vec<String> = cand_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                let cur_idx: i64 = row.get(8).unwrap_or(0);

                Ok((row.get::<_, i64>(9)?, DiarizedTurn {
                    speaker_id: row.get(0)?,
                    speaker_name: row.get(1)?,
                    start_ms: row.get::<_, i64>(2)? as u64,
                    end_ms: row.get::<_, i64>(3)? as u64,
                    channel: row.get::<_, i64>(4)? as u8,
                    text: row.get(5)?,
                    snippet_path: row.get(6)?,
                    candidate_snippets: cand_vec,
                    current_snippet_idx: cur_idx.max(0) as usize,
                }))
            })
            .map_err(|e| format!("Failed to read turns: {}", e))?;

        let mut turns = Vec::new();
        let mut turn_ids = Vec::new();
        let mut clips_missing = 0;
        for t in turn_rows {
            if let Ok((id, mut turn)) = t {
                turn_ids.push(id);
                clips_missing += drop_missing_clips(&mut turn.snippet_path, &mut turn.candidate_snippets, Some(&mut turn.current_snippet_idx));
                turns.push(turn);
            }
        }
        let audio_missing = meeting.8.is_some() && !file_exists(&meeting.8);
        if audio_missing || clips_missing > 0 {
            println!("[MEETINGS] Meeting #{}: audio missing={audio_missing}, {clips_missing} speaker clip(s) missing", meeting.0);
        }

        Ok(MeetingDetailRecord {
            turn_ids,
            transcription_model: conn.query_row("SELECT transcription_model FROM meetings WHERE id=?1", [meeting_id], |r| r.get(0)).map_err(|e| e.to_string())?,
            id: meeting.0,
            session_id: meeting.1,
            title: meeting.2,
            platform: meeting.3,
            app_name: meeting.4,
            url: meeting.5,
            created_at: meeting.6,
            duration_ms: meeting.7,
            audio_path: meeting.8,
            transcript_raw: meeting.9,
            summary: meeting.10,
            action_items: meeting.11,
            category: meeting.12,
            speaker_count: meeting.13,
            turns,
            recording_sources: conn.query_row("SELECT recording_sources FROM meetings WHERE id = ?1", [meeting_id], |r| r.get::<_, Option<String>>(0))
                .map_err(|e| e.to_string())?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
            audio_missing,
            clips_missing,
            merges: speaker_merges(&conn, meeting_id)?,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
pub async fn update_meeting(
    meeting_id: i64,
    title: String,
    category: String,
    summary: Vec<String>,
    action_items: Vec<ActionItem>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        let sum_json = serde_json::to_string(&summary).unwrap_or_default();
        let act_json = serde_json::to_string(&action_items).unwrap_or_default();

        conn.execute(
            r#"
            UPDATE meetings
            SET title = ?1, category = ?2, summary = ?3, action_items = ?4, reviewed = 1
            WHERE id = ?5
            "#,
            params![title, category, sum_json, act_json, meeting_id],
        )
        .map_err(|e| format!("Failed to update meeting: {}", e))?;

        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerRename {
    speaker_id: String,
    new_name: String,
}

#[tauri::command]
pub async fn update_meeting_title(meeting_id: i64, title: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || update_meeting_title_in(&open_connection()?, meeting_id, &title))
        .await.map_err(|e| e.to_string())?
}

fn update_meeting_title_in(conn: &Connection, meeting_id: i64, title: &str) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() { return Err("Meeting title cannot be empty".into()); }
    let changed = conn.execute("UPDATE meetings SET title=?1 WHERE id=?2", params![title, meeting_id])
        .map_err(|e| e.to_string())?;
    if changed != 1 { return Err("Meeting no longer exists".into()); }
    Ok(())
}

/// Save a completed meeting review as one database change. A failed speaker
/// rename or meeting update rolls back every edit in this review.
#[tauri::command]
pub async fn save_meeting_review(
    meeting_id: i64,
    title: String,
    category: String,
    summary: Vec<String>,
    action_items: Vec<ActionItem>,
    speaker_renames: Vec<SpeakerRename>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        save_meeting_review_internal(
            &conn, meeting_id, &title, &category, &summary, &action_items, &speaker_renames,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

fn save_meeting_review_internal(
    conn: &Connection,
    meeting_id: i64,
    title: &str,
    category: &str,
    summary: &[String],
    action_items: &[ActionItem],
    speaker_renames: &[SpeakerRename],
) -> Result<(), String> {
    let sum_json = serde_json::to_string(summary).map_err(|e| e.to_string())?;
    let act_json = serde_json::to_string(action_items).map_err(|e| e.to_string())?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for rename in speaker_renames {
        let name = rename.new_name.trim();
        if name.is_empty() {
            return Err("Speaker name cannot be empty".into());
        }
        rename_speaker_internal(&tx, meeting_id, &rename.speaker_id, name, true)?;
    }
    let changed = tx.execute(
        "UPDATE meetings SET title = ?1, category = ?2, summary = ?3, action_items = ?4, reviewed = 1 WHERE id = ?5",
        params![title, category, sum_json, act_json, meeting_id],
    ).map_err(|e| format!("Failed to update meeting: {}", e))?;
    if changed == 0 {
        return Err("Meeting no longer exists".into());
    }
    tx.commit().map_err(|e| format!("Failed to save meeting review: {}", e))
}

#[tauri::command]
pub async fn delete_meeting(meeting_id: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        delete_meeting_internal(&conn, meeting_id)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

/// Both primary previews and independently enrolled samples own their audio.
/// Candidates can share a voice companion, so protect companions separately.
fn retained_vault_audio(conn: &Connection) -> Result<std::collections::HashSet<String>, String> {
    let mut keep = std::collections::HashSet::new();
    let mut stmt = conn.prepare("SELECT snippet_path, candidate_snippets FROM speaker_vault").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))).map_err(|e| e.to_string())?;
    for row in rows {
        let (selected, candidates) = row.map_err(|e| e.to_string())?;
        keep.extend(selected);
        if let Some(json) = candidates {
            keep.extend(serde_json::from_str::<Vec<String>>(&json).map_err(|e| format!("Invalid vault sample paths: {e}"))?);
        }
    }
    let mut stmt = conn.prepare("SELECT clip_path FROM speaker_voiceprints WHERE clip_path IS NOT NULL").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
    for row in rows { keep.insert(row.map_err(|e| e.to_string())?); }
    protect_voice_companions(&mut keep);
    Ok(keep)
}

fn protect_voice_companions(paths: &mut std::collections::HashSet<String>) {
    let companions: Vec<_> = paths.iter().map(|p| crate::diarization::voice_sample_path(p).to_string_lossy().into_owned()).collect();
    paths.extend(companions);
}

fn remove_unused_speaker_clip(path: &str, keep: &std::collections::HashSet<String>) {
    let companion = crate::diarization::voice_sample_path(path);
    if !keep.contains(companion.to_string_lossy().as_ref()) { let _ = std::fs::remove_file(companion); }
    if !keep.contains(path) { let _ = std::fs::remove_file(path); }
}

fn delete_meeting_internal(conn: &Connection, meeting_id: i64) -> Result<(), String> {
        // Collect paths first, but only remove files after the database rows
        // are gone. Vault profiles may still use clips from this meeting.
        let audio_path: Option<String> = conn
            .query_row(
                "SELECT audio_path FROM meetings WHERE id = ?1",
                params![meeting_id],
                |row| row.get(0),
            )
            .ok();

        let mut stmt = conn
            .prepare("SELECT snippet_path, candidate_snippets FROM meeting_turns WHERE meeting_id = ?1")
            .map_err(|e| e.to_string())?;
        let snippets = stmt.query_map(params![meeting_id], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?))
        }).map_err(|e| e.to_string())?;
        let mut meeting_clips = std::collections::HashSet::new();
        for row in snippets {
            let (selected, candidates) = row.map_err(|e| e.to_string())?;
            if let Some(path) = selected { meeting_clips.insert(path); }
            if let Some(json) = candidates {
                meeting_clips.extend(serde_json::from_str::<Vec<String>>(&json).unwrap_or_default());
            }
        }
        drop(stmt);

        let vault_clips = retained_vault_audio(conn)?;

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM meeting_turns WHERE meeting_id = ?1", params![meeting_id])
            .map_err(|e| format!("Failed to delete turns: {}", e))?;
        tx.execute("DELETE FROM meeting_speaker_merges WHERE meeting_id = ?1", params![meeting_id])
            .map_err(|e| format!("Failed to delete speaker merges: {}", e))?;
        tx.execute("DELETE FROM meetings WHERE id = ?1", params![meeting_id])
            .map_err(|e| format!("Failed to delete meeting: {}", e))?;
        tx.commit().map_err(|e| format!("Failed to commit meeting deletion: {}", e))?;

        if let Some(path) = audio_path { let _ = std::fs::remove_file(path); }
        crate::meeting_continuation::forget_meeting(meeting_id);
        for path in meeting_clips {
            remove_unused_speaker_clip(&path, &vault_clips);
        }

    Ok(())
}

#[tauri::command]
pub async fn rename_vault_speaker(speaker_id: String, new_name: String) -> Result<(), String> {
    let name = new_name.trim().to_string();
    if name.is_empty() { return Err("Speaker name cannot be empty".into()); }
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        rename_vault_speaker_internal(&conn, &speaker_id, &name)
    }).await.map_err(|e| format!("Task failed: {}", e))?
}

fn rename_vault_speaker_internal(conn: &Connection, speaker_id: &str, name: &str) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    rename_vault_speaker_in(&tx, speaker_id, name)?;
    tx.commit().map_err(|e| e.to_string())
}

fn rename_vault_speaker_in(conn: &Connection, speaker_id: &str, name: &str) -> Result<(), String> {
    // Two people with one name can't be told apart, and naming a caller links
    // them to the person with that name: refuse instead of making a twin.
    let taken: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM speaker_vault WHERE lower(name) = lower(?1) AND id != ?2)",
            params![name, speaker_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if taken {
        return Err(format!("Someone else in the Speaker Vault is already called \"{name}\". Choose a different name."));
    }
    // A rename is not an appearance: last_seen stays when they were last heard.
    let changed = conn.execute("UPDATE speaker_vault SET name = ?1 WHERE id = ?2", params![name, speaker_id])
        .map_err(|e| e.to_string())?;
    if changed == 0 { return Err("Speaker not found in vault".into()); }
    conn.execute("UPDATE meeting_turns SET speaker_name = ?1 WHERE speaker_id = ?2", params![name, speaker_id])
        .map_err(|e| e.to_string())?;
    // Undo snapshots also own display names. Update them in this transaction
    // so undo cannot resurrect a globally renamed person's old name. Preserve
    // meeting-only aliases unless the user explicitly renames the vault person.
    let snapshots = {
        let mut stmt = conn.prepare("SELECT id, turns_json FROM meeting_speaker_merges WHERE from_speaker_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([speaker_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
    };
    for (id, json) in snapshots {
        let mut turns: Vec<MergedTurn> = serde_json::from_str(&json).map_err(|e| format!("Merge record is damaged: {e}"))?;
        for turn in &mut turns {
            if turn.speaker_id == speaker_id { turn.speaker_name = name.to_string(); }
        }
        let json = serde_json::to_string(&turns).map_err(|e| e.to_string())?;
        conn.execute("UPDATE meeting_speaker_merges SET turns_json = ?1 WHERE id = ?2", params![json, id])
            .map_err(|e| e.to_string())?;
    }
    conn.execute("UPDATE meeting_speaker_merges SET from_speaker_name = ?1 WHERE from_speaker_id = ?2", params![name, speaker_id])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE meeting_speaker_merges SET into_speaker_name = ?1 WHERE into_speaker_id = ?2", params![name, speaker_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Manual labels affect only the explicitly selected stored turns. They do not
/// enroll voiceprints or rewrite any other meeting.
#[tauri::command]
pub async fn assign_meeting_turns(meeting_id: i64, turn_ids: Vec<i64>, speaker_id: Option<String>, new_name: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        assign_meeting_turns_internal(&conn, meeting_id, &turn_ids, speaker_id.as_deref(), new_name.as_deref())
    }).await.map_err(|e| e.to_string())?
}

fn assign_meeting_turns_internal(conn: &Connection, meeting_id: i64, turn_ids: &[i64], speaker_id: Option<&str>, new_name: Option<&str>) -> Result<(), String> {
    if turn_ids.is_empty() { return Err("Choose a transcript passage".into()); }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let (id, name): (String, String) = if let Some(id) = speaker_id {
        tx.query_row("SELECT speaker_id, speaker_name FROM meeting_turns WHERE meeting_id=?1 AND speaker_id=?2 LIMIT 1", params![meeting_id, id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|_| "That person is no longer in this meeting".to_string())?
    } else {
        let name = new_name.unwrap_or("").trim();
        if name.is_empty() || name.len() > 200 { return Err("Enter a person’s name (up to 200 bytes)".into()); }
        (format!("manual_{}_{:016x}", Utc::now().timestamp_millis(), rand::random::<u64>()), name.into())
    };
    let mut affected = vec![id.clone()];
    for turn in turn_ids {
        let previous: String = tx.query_row("SELECT speaker_id FROM meeting_turns WHERE meeting_id=?1 AND id=?2", params![meeting_id, turn], |r| r.get(0)).map_err(|_| "This transcript changed. Reopen it and select the passage again".to_string())?;
        affected.push(previous);
        let changed = tx.execute("UPDATE meeting_turns SET speaker_id=?1, speaker_name=?2 WHERE meeting_id=?3 AND id=?4", params![id, name, meeting_id, turn]).map_err(|e| e.to_string())?;
        if changed != 1 { return Err("This transcript changed. Reopen it and select the passage again".into()); }
    }
    // Old group previews may contain either person. Regenerate from the corrected
    // turn ranges rather than offering mislabeled samples. Vault samples are kept.
    for speaker in affected {
        tx.execute("UPDATE meeting_turns SET snippet_path=NULL, candidate_snippets=NULL, current_snippet_idx=0 WHERE meeting_id=?1 AND speaker_id=?2", params![meeting_id, speaker]).map_err(|e| e.to_string())?;
    }
    // Merge undo snapshots predate these corrections and must not overwrite them.
    tx.execute("DELETE FROM meeting_speaker_merges WHERE meeting_id=?1", [meeting_id]).map_err(|e| e.to_string())?;
    recount_speakers(&tx, meeting_id)?;
    tx.commit().map_err(|e| e.to_string())
}

pub(crate) fn save_transcription_model(conn: &Connection, meeting_id: i64, model: Option<&str>) -> Result<(), String> {
    conn.execute("UPDATE meetings SET transcription_model=?1 WHERE id=?2", params![model, meeting_id]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn rename_speaker(
    meeting_id: i64,
    speaker_id: String,
    new_name: String,
    update_vault: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        rename_speaker_internal(&conn, meeting_id, &speaker_id, &new_name, update_vault)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

/// Renames a speaker in one meeting and, with `update_vault`, links them to a
/// vault person:
///   * already a vault person (voice matched)  -> renames that person
///   * a per-meeting caller, name already in vault -> links to that person
///   * a per-meeting caller, new name              -> enrolls a new person
/// The local user ("You", mic channel) is never enrolled as a voiceprint.
pub(crate) fn rename_speaker_internal(
    conn: &Connection,
    meeting_id: i64,
    speaker_id: &str,
    new_name: &str,
    update_vault: bool,
) -> Result<(), String> {
    // Review saves already own a transaction; standalone renames need the same
    // all-or-nothing behavior for local turns, vault identity and linked turns.
    if conn.is_autocommit() {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        rename_speaker_internal(&tx, meeting_id, speaker_id, new_name, update_vault)?;
        return tx.commit().map_err(|e| e.to_string());
    }
    let new_name = new_name.trim();
    if new_name.is_empty() { return Err("Speaker name cannot be empty".into()); }
    let changed = conn.execute(
        "UPDATE meeting_turns SET speaker_name = ?1 WHERE meeting_id = ?2 AND speaker_id = ?3",
        params![new_name, meeting_id, speaker_id],
    )
    .map_err(|e| format!("Failed to rename speaker: {}", e))?;
    if changed == 0 { return Err("That speaker is no longer in this meeting".into()); }

    if !update_vault {
        return Ok(());
    }
    let is_person: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM speaker_vault WHERE id=?1)", params![speaker_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if is_person {
        return rename_vault_speaker_in(conn, speaker_id, new_name);
    }
    // Voiceprints need the speaker recognition model (Settings → Models); without
    // it the rename applies to this meeting only.
    if !speaker_model_installed() {
        return Ok(());
    }

    // Channel, preferred clip and clip candidates of this speaker in this meeting.
    let info: Option<(i64, Option<String>, Option<String>)> = conn
        .query_row(
            r#"
            SELECT channel, snippet_path, candidate_snippets FROM meeting_turns
            WHERE meeting_id = ?1 AND speaker_id = ?2
            ORDER BY (snippet_path IS NULL), start_ms LIMIT 1
            "#,
            params![meeting_id, speaker_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    let Some((channel, snippet, candidates)) = info else { return Ok(()) };
    if channel == 0 {
        return Ok(()); // "You": not a caller voiceprint
    }

    let now = Utc::now().to_rfc3339();
    // A per-meeting caller: link to an existing person with that name, or enroll.
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM speaker_vault WHERE lower(name) = lower(?1) ORDER BY last_seen DESC, id LIMIT 1",
            params![new_name],
            |row| row.get(0),
        )
        .ok();
    let person_id = match existing {
        Some(id) => {
            let already_present: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM meeting_turns WHERE meeting_id=?1 AND speaker_id=?2)", params![meeting_id, id], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            record_vault_appearance(conn, &id, snippet.as_deref(), candidates.as_deref(), Some(meeting_id), !already_present)?;
            id
        }
        None => {
            let id = new_vault_id();
            let emb = snippet.as_deref().and_then(embedding_from_wav);
            let emb_json = emb.as_ref().map(|e| serde_json::to_string(e).unwrap_or_default());
            conn.execute(
                r#"
                INSERT INTO speaker_vault
                    (id, name, created_at, sample_count, meeting_count, last_seen, snippet_path, candidate_snippets, embedding_json)
                VALUES (?1, ?2, ?3, ?4, 1, ?3, ?5, ?6, ?7)
                "#,
                params![id, new_name, now, if emb_json.is_some() { 1 } else { 0 }, snippet, candidates, emb_json],
            )
            .map_err(|e| format!("Failed to enroll speaker: {}", e))?;
            if let Some(emb) = &emb {
                super::voice_bank::add_voiceprint(conn, &id, emb, snippet.as_deref(), Some(meeting_id))?;
            }
            id
        }
    };
    conn.execute(
        "UPDATE meeting_turns SET speaker_id = ?1 WHERE meeting_id = ?2 AND speaker_id = ?3",
        params![person_id, meeting_id, speaker_id],
    )
    .map_err(|e| format!("Failed to link speaker to vault: {}", e))?;
    recount_speakers(conn, meeting_id)?;
    Ok(())
}

// ── Speaker merges ──────────────────────────────────────────────────────────
//
// The separator can hear one person as two speakers. Merging moves every turn
// of one speaker to another in this meeting; the turns' previous speaker, name,
// channel and clips are kept in meeting_speaker_merges so the merge can be
// undone. Merges are undone newest first: a later merge may have moved the
// same turns again.

/// A merged turn as it was before the merge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct MergedTurn {
    turn_id: i64,
    speaker_id: String,
    speaker_name: String,
    channel: i64,
    snippet_path: Option<String>,
    candidate_snippets: Option<String>,
    current_snippet_idx: i64,
}

/// Merges the user can undo in a meeting, newest first.
fn speaker_merges(conn: &Connection, meeting_id: i64) -> Result<Vec<SpeakerMergeRecord>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, from_speaker_id, from_speaker_name, into_speaker_id, into_speaker_name, created_at \
             FROM meeting_speaker_merges WHERE meeting_id = ?1 ORDER BY id DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![meeting_id], |r| {
            Ok(SpeakerMergeRecord {
                id: r.get(0)?,
                from_speaker_id: r.get(1)?,
                from_speaker_name: r.get(2)?,
                into_speaker_id: r.get(3)?,
                into_speaker_name: r.get(4)?,
                created_at: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

pub(crate) fn recount_speakers(conn: &Connection, meeting_id: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE meetings SET speaker_count = (SELECT COUNT(DISTINCT speaker_id) FROM meeting_turns WHERE meeting_id = ?1 AND speaker_id != 'background') WHERE id = ?1",
        params![meeting_id],
    )
    .map(|_| ())
    .map_err(|e| format!("Failed to recount speakers: {}", e))
}

#[tauri::command]
pub async fn merge_meeting_speakers(
    meeting_id: i64,
    from_speaker_id: String,
    into_speaker_id: String,
) -> Result<SpeakerMergeRecord, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        merge_speakers_internal(&conn, meeting_id, &from_speaker_id, &into_speaker_id)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

/// Moves every turn of `from` to `into` (name, channel and clip included).
pub(crate) fn merge_speakers_internal(
    conn: &Connection,
    meeting_id: i64,
    from: &str,
    into: &str,
) -> Result<SpeakerMergeRecord, String> {
    if from == into {
        return Err("Pick two different speakers to merge".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let target: (String, i64, Option<String>, Option<String>, i64) = if into == "background" {
        ("Background".into(), 1, None, None, 0)
    } else { tx
        .query_row(
            "SELECT speaker_name, channel, snippet_path, candidate_snippets, COALESCE(current_snippet_idx, 0) FROM meeting_turns \
             WHERE meeting_id = ?1 AND speaker_id = ?2 ORDER BY (snippet_path IS NULL), start_ms LIMIT 1",
            params![meeting_id, into],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|_| "That speaker is no longer in this meeting".to_string())? };
    let moved: Vec<MergedTurn> = {
        let mut stmt = tx
            .prepare(
                "SELECT id, speaker_id, speaker_name, channel, snippet_path, candidate_snippets, COALESCE(current_snippet_idx, 0) \
                 FROM meeting_turns WHERE meeting_id = ?1 AND speaker_id = ?2 ORDER BY start_ms",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![meeting_id, from], |r| {
                Ok(MergedTurn {
                    turn_id: r.get(0)?,
                    speaker_id: r.get(1)?,
                    speaker_name: r.get(2)?,
                    channel: r.get(3)?,
                    snippet_path: r.get(4)?,
                    candidate_snippets: r.get(5)?,
                    current_snippet_idx: r.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    let Some(first) = moved.first() else {
        return Err("That speaker is no longer in this meeting".into());
    };
    let from_name = first.speaker_name.clone();
    let turns_json = serde_json::to_string(&moved).map_err(|e| e.to_string())?;
    let created_at = Utc::now().to_rfc3339();

    tx.execute(
        "INSERT INTO meeting_speaker_merges (meeting_id, from_speaker_id, from_speaker_name, into_speaker_id, into_speaker_name, turns_json, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![meeting_id, from, from_name, into, target.0, turns_json, created_at],
    )
    .map_err(|e| format!("Failed to record merge: {}", e))?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE meeting_turns SET speaker_id = ?1, speaker_name = ?2, channel = ?3, snippet_path = ?4, candidate_snippets = ?5, current_snippet_idx = ?6 \
         WHERE meeting_id = ?7 AND speaker_id = ?8",
        params![into, target.0, target.1, target.2, target.3, target.4, meeting_id, from],
    )
    .map_err(|e| format!("Failed to merge speakers: {}", e))?;
    recount_speakers(&tx, meeting_id)?;
    tx.commit().map_err(|e| format!("Failed to save merge: {}", e))?;
    Ok(SpeakerMergeRecord {
        id,
        from_speaker_id: from.to_string(),
        from_speaker_name: from_name,
        into_speaker_id: into.to_string(),
        into_speaker_name: target.0,
        created_at,
    })
}

#[tauri::command]
pub async fn undo_meeting_speaker_merge(meeting_id: i64, merge_id: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        undo_merge_internal(&conn, meeting_id, merge_id)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

/// Puts the turns of the newest merge back as they were.
pub(crate) fn undo_merge_internal(conn: &Connection, meeting_id: i64, merge_id: i64) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let newest: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, turns_json FROM meeting_speaker_merges WHERE meeting_id = ?1 ORDER BY id DESC LIMIT 1",
            params![meeting_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional().map_err(|e| e.to_string())?;
    let Some((newest_id, turns_json)) = newest else {
        return Err("Nothing to undo in this meeting".into());
    };
    if newest_id != merge_id {
        return Err("Undo the later merges first".into());
    }
    let turns: Vec<MergedTurn> = serde_json::from_str(&turns_json).map_err(|e| format!("Merge record is damaged: {}", e))?;
    for t in &turns {
        tx.execute(
            "UPDATE meeting_turns SET speaker_id = ?1, speaker_name = ?2, channel = ?3, snippet_path = ?4, candidate_snippets = ?5, current_snippet_idx = ?6 \
             WHERE id = ?7 AND meeting_id = ?8",
            params![t.speaker_id, t.speaker_name, t.channel, t.snippet_path, t.candidate_snippets, t.current_snippet_idx, t.turn_id, meeting_id],
        )
        .map_err(|e| format!("Failed to undo merge: {}", e))?;
    }
    tx.execute("DELETE FROM meeting_speaker_merges WHERE id = ?1", params![merge_id])
        .map_err(|e| format!("Failed to undo merge: {}", e))?;
    recount_speakers(&tx, meeting_id)?;
    tx.commit().map_err(|e| format!("Failed to undo merge: {}", e))
}

#[tauri::command]
pub async fn list_speaker_vault() -> Result<Vec<SpeakerVaultRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, name, created_at, sample_count, last_seen, snippet_path, candidate_snippets, meeting_count
                FROM speaker_vault ORDER BY last_seen DESC
                "#,
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                let cand_str: Option<String> = row.get(6)?;
                let candidate_snippets: Vec<String> = cand_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();

                let mut snippet_path: Option<String> = row.get(5)?;
                let mut candidate_snippets = candidate_snippets;
                let had_clip = snippet_path.is_some() || !candidate_snippets.is_empty();
                drop_missing_clips(&mut snippet_path, &mut candidate_snippets, None);
                Ok(SpeakerVaultRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    sample_count: row.get(3)?,
                    last_seen: row.get(4)?,
                    clip_missing: had_clip && snippet_path.is_none(),
                    snippet_path,
                    candidate_snippets,
                    meeting_count: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut results = Vec::new();
        for r in rows {
            if let Ok(rec) = r {
                results.push(rec);
            }
        }
        Ok(results)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
pub async fn delete_speaker_from_vault(speaker_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
        conn.execute("DELETE FROM speaker_vault WHERE id = ?1", params![speaker_id])
            .map_err(|e| format!("Failed to delete speaker: {}", e))?;
        super::voice_bank::forget_person_in(&conn, &speaker_id);
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
pub async fn cycle_speaker_turn_snippet(
    meeting_id: i64,
    speaker_id: String,
) -> Result<(String, usize), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;

        // Get candidate snippets and current idx for this speaker in this meeting
        let mut stmt = conn
            .prepare(
                r#"
                SELECT candidate_snippets, current_snippet_idx
                FROM meeting_turns
                WHERE meeting_id = ?1 AND speaker_id = ?2 AND candidate_snippets IS NOT NULL AND candidate_snippets != ''
                LIMIT 1
                "#,
            )
            .map_err(|e| e.to_string())?;

        let (cand_str, cur_idx): (String, i64) = stmt
            .query_row(params![meeting_id, speaker_id], |row| {
                Ok((row.get(0)?, row.get(1).unwrap_or(0)))
            })
            .map_err(|e| format!("No candidate snippets found for speaker: {}", e))?;

        let candidates: Vec<String> = serde_json::from_str(&cand_str)
            .map_err(|e| format!("Failed to parse candidate snippets: {}", e))?;

        let current = candidates.get(cur_idx.max(0) as usize).map(String::as_str);
        let (next_path, next_idx) = next_existing_snippet(&candidates, current)
            .ok_or_else(|| "No other voice samples are available for this speaker".to_string())?;

        // Update all turns for this speaker in this meeting
        conn.execute(
            r#"
            UPDATE meeting_turns
            SET snippet_path = ?1, current_snippet_idx = ?2
            WHERE meeting_id = ?3 AND speaker_id = ?4
            "#,
            params![next_path, next_idx as i64, meeting_id, speaker_id],
        )
        .map_err(|e| format!("Failed to update meeting turns: {}", e))?;

        // The chosen clip only changes what is shown/played for this person; the
        // voiceprint stays the average of all their samples.
        let _ = conn.execute(
            "UPDATE speaker_vault SET snippet_path = ?1 WHERE id = ?2",
            params![next_path, speaker_id],
        );

        Ok((next_path, next_idx))
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

/// The next voice sample after `current` in `candidates` whose file still
/// exists (clips can be deleted from disk). None when no other one is left.
fn next_existing_snippet(candidates: &[String], current: Option<&str>) -> Option<(String, usize)> {
    let start = current
        .and_then(|path| candidates.iter().position(|c| c == path))
        .map(|i| i + 1)
        .unwrap_or(0);
    (0..candidates.len())
        .map(|step| (start + step) % candidates.len())
        .find(|&i| Some(candidates[i].as_str()) != current && Path::new(&candidates[i]).exists())
        .map(|i| (candidates[i].clone(), i))
}

/// Shows the next stored voice sample for a Speaker Vault person.
fn cycle_vault_snippet_in(conn: &Connection, speaker_id: &str) -> Result<(String, usize), String> {
    let (cand_str, cur_snippet): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT candidate_snippets, snippet_path FROM speaker_vault WHERE id = ?1",
            params![speaker_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("Speaker not found in vault: {}", e))?;

    let candidates: Vec<String> = cand_str
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let (next_path, next_idx) = next_existing_snippet(&candidates, cur_snippet.as_deref())
        .ok_or_else(|| "No other voice samples are available for this person".to_string())?;

    // Display/playback clip only; the voiceprint stays the averaged embedding.
    conn.execute(
        "UPDATE speaker_vault SET snippet_path = ?1 WHERE id = ?2",
        params![next_path, speaker_id],
    )
    .map_err(|e| format!("Failed to update vault snippet: {}", e))?;
    Ok((next_path, next_idx))
}

#[tauri::command]
pub async fn cycle_vault_speaker_snippet(
    speaker_id: String,
) -> Result<(String, usize), String> {
    tauri::async_runtime::spawn_blocking(move || cycle_vault_snippet_in(&open_connection()?, &speaker_id))
        .await
        .map_err(|e| format!("Task failed: {}", e))?
}


#[tauri::command]
pub async fn export_meeting_notes(meeting_id: i64, format: String) -> Result<String, String> {
    let detail = get_meeting_detail(meeting_id).await?;
    render_meeting_notes(&detail, &format)
}

/// Writes a meeting's notes to `path`, the file chosen in the Save dialog.
/// (The webview cannot download a generated file itself on macOS.)
#[tauri::command]
pub async fn save_meeting_notes(meeting_id: i64, format: String, path: String) -> Result<(), String> {
    let detail = get_meeting_detail(meeting_id).await?;
    let notes = render_meeting_notes(&detail, &format)?;
    write_notes(Path::new(&path), &notes)
}

fn write_notes(path: &Path, notes: &str) -> Result<(), String> {
    std::fs::write(path, notes).map_err(|e| format!("Couldn't write {}: {}", path.display(), e))
}

/// " (@name)" for an action item's assignee, nothing when it has none.
fn assignee_suffix(assignee: &str, markdown: bool) -> String {
    match assignee.trim() {
        "" => String::new(),
        name if markdown => format!(" — *Assignee: @{name}*"),
        name => format!(" (@{name})"),
    }
}

/// A meeting's notes as JSON, plain text or Markdown (the default).
fn render_meeting_notes(detail: &MeetingDetailRecord, format: &str) -> Result<String, String> {
    let stamp = |ms: u64| format!("{:02}:{:02}", ms / 60000, (ms % 60000) / 1000);
    match format.to_lowercase().as_str() {
        "json" => serde_json::to_string_pretty(detail).map_err(|e| e.to_string()),
        "txt" | "text" => {
            let mut out = format!("TITLE: {}\nCATEGORY: {}\nDATE: {}\n\n", detail.title, detail.category, detail.created_at);
            out.push_str("SUMMARY:\n");
            for s in &detail.summary {
                out.push_str(&format!("• {}\n", s));
            }
            out.push_str("\nACTION ITEMS:\n");
            for a in &detail.action_items {
                out.push_str(&format!("[{}] {}{}\n", if a.status == "done" { "X" } else { " " }, a.task, assignee_suffix(&a.assignee, false)));
            }
            out.push_str("\nTRANSCRIPT:\n");
            for t in &detail.turns {
                out.push_str(&format!("[{}] {}: {}\n", stamp(t.start_ms), t.speaker_name, t.text));
            }
            Ok(out)
        }
        _ => {
            let mut md = format!("# {}\n\n**Category**: `{}` • **Date**: {}\n\n", detail.title, detail.category, detail.created_at);
            md.push_str("## 📋 Executive Summary\n\n");
            for s in &detail.summary {
                md.push_str(&format!("- {}\n", s));
            }
            md.push_str("\n## ✅ Action Items\n\n");
            for a in &detail.action_items {
                md.push_str(&format!("- [{}] **{}**{}\n", if a.status == "done" { "x" } else { " " }, a.task, assignee_suffix(&a.assignee, true)));
            }
            md.push_str("\n## 🎙️ Diarized Transcript\n\n");
            for t in &detail.turns {
                md.push_str(&format!("**{} {}**: {}\n\n", stamp(t.start_ms), t.speaker_name, t.text));
            }
            Ok(md)
        }
    }
}

// ── Meeting Detector Wrapper Commands ───────────────────────────────────────

#[tauri::command]
pub async fn scan_active_meetings(
    state: State<'_, AudioState>,
) -> Result<Vec<MeetingInfo>, String> {
    // Scanning blocks (browser accessibility walk, osascript): keep it off the
    // async runtime's workers.
    let detector = state.meeting_detector.clone();
    tauri::async_runtime::spawn_blocking(move || detector.scan())
        .await
        .map_err(|e| format!("scan_active_meetings task failed: {e}"))
}

#[tauri::command]
pub async fn get_meeting_detection_status(
    state: State<'_, AudioState>,
) -> Result<MeetingDetectionStatus, String> {
    Ok(state.meeting_detector.get_status())
}

#[tauri::command]
pub async fn start_meeting_detection(
    app: AppHandle,
    state: State<'_, AudioState>,
) -> Result<(), String> {
    state.meeting_detector.start_watching(app)
}

#[tauri::command]
pub async fn stop_meeting_detection(state: State<'_, AudioState>) -> Result<(), String> {
    state.meeting_detector.stop_watching();
    Ok(())
}

#[tauri::command]
pub async fn set_audio_source_mode(
    mode: String,
    state: State<'_, AudioState>,
) -> Result<(), String> {
    let mode_str = match mode.to_lowercase().as_str() {
        "dual_channel" | "dual" | "system" => "dual_channel",
        _ => "mic",
    };
    *state.audio_source_mode.lock().unwrap() = mode_str.to_string();
    println!("[INFO] Audio source mode set to: {}", mode_str);
    Ok(())
}

#[tauri::command]
pub async fn get_audio_source_mode(state: State<'_, AudioState>) -> Result<String, String> {
    Ok(state.audio_source_mode.lock().unwrap().clone())
}

#[tauri::command]
pub async fn set_auto_record_meetings(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AudioState>,
) -> Result<(), String> {
    if state.auto_record_meetings.swap(enabled, Ordering::SeqCst) != enabled {
        let _ = app.emit("auto-record-meetings-changed", enabled);
    }
    println!("[INFO] Auto-record meetings set to: {}", enabled);
    Ok(())
}

#[tauri::command]
pub async fn get_auto_record_meetings(state: State<'_, AudioState>) -> Result<bool, String> {
    Ok(state.auto_record_meetings.load(Ordering::SeqCst))
}

/// Minutes after stopping in which recording the same call again continues the
/// meeting (0 = always save a new meeting).
#[tauri::command]
pub fn get_meeting_continue_minutes() -> u64 {
    crate::meeting_continuation::window_minutes()
}

#[tauri::command]
pub fn set_meeting_continue_minutes(minutes: u64) -> u64 {
    crate::meeting_continuation::set_window_minutes(minutes)
}

/// Cosine threshold for recognising Speaker Vault people in new meetings.
#[tauri::command]
pub fn get_speaker_match_threshold() -> f32 {
    crate::speaker_embedding::match_threshold()
}

/// Returns the threshold actually applied (clamped).
#[tauri::command]
pub fn set_speaker_match_threshold(value: f32) -> f32 {
    crate::speaker_embedding::set_match_threshold(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_meeting_saves_source_timeline_and_capture_date_in_one_transaction() {
        let conn = Connection::open_in_memory().unwrap(); ensure_meetings_schema(&conn).unwrap();
        let source = crate::recording_import::RecordingSource {
            path: "/original/part.wav".into(), name: "part.wav".into(), duration_ms: 1000,
            recorded_start: Some("2026-10-08T12:00:00Z".into()), recorded_end: Some("2026-10-08T12:00:01Z".into()),
            timestamp_source: Some("Embedded recording timestamp".into()), file_created_at: None, file_modified_at: None, offset_ms: 0,
        };
        let id = insert_meeting_with_sources(&conn, "Imported", "Imported recording", "Taurscribe", "", 1000, None, "Speech", &[], &[source]).unwrap();
        let (created, json, parts): (String, String, i64) = conn.query_row("SELECT created_at, recording_sources, parts FROM meetings WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(created, "2026-10-08T12:00:00+00:00"); assert_eq!(parts, 1);
        let sources: Vec<crate::recording_import::RecordingSource> = serde_json::from_str(&json).unwrap();
        assert_eq!(sources[0].name, "part.wav"); assert_eq!(sources[0].offset_ms, 0);
    }

    #[test]
    fn test_legacy_wav_meetings_are_compressed() {
        let _serial = crate::meeting_audio::COMPRESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("taurscribe_legacy_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("meeting_legacy.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..48_000 * 3 {
            let t = i as f32 / 48_000.0;
            w.write_sample(0.1 * (t * 200.0 * 6.283).sin()).unwrap();
            w.write_sample(0.05 * (t * 300.0 * 6.283).sin()).unwrap();
        }
        w.finalize().unwrap();

        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let id = insert_completed_meeting(&conn, "Old", "meet", "Chrome", "", 3000, Some(&wav.to_string_lossy()), "hi", &[]).unwrap();

        assert_eq!(compress_legacy_meeting_audio(&conn), 1);
        let path: String = conn.query_row("SELECT audio_path FROM meetings WHERE id = ?1", params![id], |r| r.get(0)).unwrap();
        assert!(path.ends_with(".webm") && std::path::Path::new(&path).exists(), "{}", path);
        assert!(!wav.exists());
        assert_eq!(compress_legacy_meeting_audio(&conn), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_clips_are_dropped_and_counted() {
        let dir = std::env::temp_dir().join(format!("ts_clips_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("b.wav");
        std::fs::write(&kept, b"x").unwrap();
        let gone = dir.join("a.wav").to_string_lossy().to_string();
        let kept = kept.to_string_lossy().to_string();
        let mut snip = Some(gone.clone());
        let mut cands = vec![gone, kept.clone()];
        let mut idx = 0;
        assert_eq!(drop_missing_clips(&mut snip, &mut cands, Some(&mut idx)), 1);
        assert_eq!(snip.as_deref(), Some(kept.as_str()));
        assert_eq!((cands.len(), idx), (1, 0));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failed_audio_path_update_keeps_original_wav() {
        let dir = std::env::temp_dir().join(format!("taurscribe_conversion_failure_{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("meeting.wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for _ in 0..4_800 { writer.write_sample(0.1f32).unwrap(); }
        writer.finalize().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let id = insert_completed_meeting(&conn, "Old", "meet", "Chrome", "", 100, Some(&wav.to_string_lossy()), "hi", &[]).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_audio_update BEFORE UPDATE OF audio_path ON meetings BEGIN SELECT RAISE(ABORT, 'blocked'); END;").unwrap();

        assert!(convert_meeting_audio(&conn, id, &wav).is_err());
        let saved: String = conn.query_row("SELECT audio_path FROM meetings WHERE id = ?1", params![id], |row| row.get(0)).unwrap();
        assert_eq!(saved, wav.to_string_lossy());
        assert!(wav.exists());
        assert!(!wav.with_extension("webm").exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "failed conversion leaked output");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_audio_conversion_preserves_committed_playback() {
        let _serial = crate::meeting_audio::COMPRESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("taurscribe_stale_conversion_{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("meeting.wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for _ in 0..4_800 { writer.write_sample(0.1f32).unwrap(); }
        writer.finalize().unwrap();
        let webm = crate::meeting_audio::compress_for_playback(&wav).unwrap();
        let original = std::fs::read(&webm).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let id = insert_completed_meeting(&conn, "Saved", "meet", "Chrome", "", 100, Some(&webm.to_string_lossy()), "hi", &[]).unwrap();

        // A background worker still holds the old WAV path after another
        // worker committed its playback copy. The stale CAS must not delete it.
        assert!(convert_meeting_audio(&conn, id, &wav).is_err());
        assert_eq!(std::fs::read(&webm).expect("committed playback was deleted"), original);
        assert!(wav.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "stale output leaked");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_turn_insert_rolls_back_meeting() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_second_turn BEFORE INSERT ON meeting_turns WHEN NEW.text = 'reject' BEGIN SELECT RAISE(ABORT, 'blocked'); END;").unwrap();
        let mut turns = vec![turn("speaker_you", "You", 0, 0), turn("speaker_remote_1", "Alice", 1, 3000)];
        turns[1].text = "reject".into();
        assert!(insert_completed_meeting(&conn, "Test", "meet", "Chrome", "", 5000, None, "speech", &turns).is_err());
        let meetings: i64 = conn.query_row("SELECT count(*) FROM meetings", [], |row| row.get(0)).unwrap();
        let saved_turns: i64 = conn.query_row("SELECT count(*) FROM meeting_turns", [], |row| row.get(0)).unwrap();
        assert_eq!((meetings, saved_turns), (0, 0));
    }

    #[test]
    fn failed_review_update_rolls_back_speaker_rename() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let id = meeting_with(&conn, &[turn("speaker_you", "You", 0, 0)]);
        conn.execute_batch("CREATE TRIGGER reject_review BEFORE UPDATE ON meetings BEGIN SELECT RAISE(ABORT, 'blocked'); END;").unwrap();

        let renames = vec![SpeakerRename {
            speaker_id: "speaker_you".into(),
            new_name: "Abdullah".into(),
        }];
        assert!(save_meeting_review_internal(
            &conn, id, "Changed", "General", &[], &[], &renames,
        ).is_err());

        let name: String = conn.query_row(
            "SELECT speaker_name FROM meeting_turns WHERE meeting_id = ?1",
            params![id], |row| row.get(0),
        ).unwrap();
        assert_eq!(name, "You");
    }

    #[test]
    fn test_meetings_db_schema_and_crud() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();

        let turns = vec![
            DiarizedTurn {
                speaker_id: "speaker_you".to_string(),
                speaker_name: "You".to_string(),
                start_ms: 0,
                end_ms: 3000,
                channel: 0,
                text: "Hello everyone, welcome to the weekly team sync.".to_string(),
                snippet_path: None,
                candidate_snippets: Vec::new(),
                current_snippet_idx: 0,
            },
            DiarizedTurn {
                speaker_id: "speaker_remote_1".to_string(),
                speaker_name: "Alice".to_string(),
                start_ms: 3500,
                end_ms: 8000,
                channel: 1,
                text: "I will deploy the new meeting catalog to production.".to_string(),
                snippet_path: Some("/tmp/snippet_test.wav".to_string()),
                candidate_snippets: vec!["/tmp/snippet_test.wav".to_string()],
                current_snippet_idx: 0,
            },
        ];

        let meeting_id = insert_completed_meeting(
            &conn,
            "Team Standup",
            "meet",
            "Google Chrome",
            "https://meet.google.com/test",
            8000,
            Some("/tmp/meeting_test.wav"),
            "Hello everyone, welcome to the weekly team sync. I will deploy the new meeting catalog to production.",
            &turns,
        )
        .unwrap();

        assert!(meeting_id > 0);

        // Verify turns were inserted
        let mut stmt = conn
            .prepare("SELECT count(*) FROM meeting_turns WHERE meeting_id = ?1")
            .unwrap();
        let turn_count: i64 = stmt.query_row(params![meeting_id], |r| r.get(0)).unwrap();
        assert_eq!(turn_count, 2);

        // An unnamed caller's per-meeting label must not become a vault person.
        let vault_count: i64 = conn.query_row("SELECT count(*) FROM speaker_vault", [], |r| r.get(0)).unwrap();
        assert_eq!(vault_count, 0);
    }

    fn turn(speaker_id: &str, name: &str, channel: u8, start_ms: u64) -> DiarizedTurn {
        DiarizedTurn {
            speaker_id: speaker_id.to_string(),
            speaker_name: name.to_string(),
            start_ms,
            end_ms: start_ms + 2000,
            channel,
            text: format!("{} speaking", name),
            snippet_path: None,
            candidate_snippets: Vec::new(),
            current_snippet_idx: 0,
        }
    }

    fn meeting_with(conn: &Connection, turns: &[DiarizedTurn]) -> i64 {
        insert_completed_meeting(conn, "Sync", "meet", "Google Chrome", "", 6000, None, "text", turns).unwrap()
    }

    #[test]
    fn continuation_preserves_review_and_adds_only_new_suggestions() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let mut old = turn("speaker_you", "You", 0, 0);
        old.text = "I will send the proposal tomorrow.".into();
        let id = meeting_with(&conn, &[old.clone()]);
        let actions = vec![ActionItem { id: "action_1".into(), task: "Send proposal".into(), assignee: "Alice".into(), status: "done".into() }];
        save_meeting_review_internal(&conn, id, "Approved title", "Planning", &["Approved summary".into()], &actions, &[]).unwrap();
        let mut new = turn("speaker_you", "You", 0, 7000);
        new.text = "I will review the design tomorrow.".into();
        replace_meeting_content(&conn, id, 10000, None, "Continued speech", &[old, new]).unwrap();
        let (title, category, summary, saved_actions): (String, String, String, String) = conn.query_row(
            "SELECT title, category, summary, action_items FROM meetings WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
        assert_eq!(title, "Approved title");
        assert_eq!(category, "Planning");
        let summary: Vec<String> = serde_json::from_str(&summary).unwrap();
        assert_eq!(summary[0], "Approved summary");
        assert_eq!(summary.len(), 2);
        let saved: Vec<ActionItem> = serde_json::from_str(&saved_actions).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0], actions[0]);
        assert_ne!(saved[0].id, saved[1].id);
        assert!(saved[1].task.contains("review the design"));
    }

    #[test]
    fn unreviewed_continuation_updates_generated_notes() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let old = turn("speaker_you", "You", 0, 0);
        let id = meeting_with(&conn, &[old.clone()]);
        let mut new = turn("speaker_you", "You", 0, 7000);
        new.text = "I will fix the backend bug tomorrow.".into();
        replace_meeting_content(&conn, id, 10000, None, "Continued speech", &[old, new]).unwrap();
        let (category, reviewed): (String, bool) = conn.query_row("SELECT category, reviewed FROM meetings WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(category, "Engineering");
        assert!(!reviewed);
    }

    fn content_state(conn: &Connection, id: i64) -> (i64, i64, Option<String>, String, String, String, String, String, bool) {
        conn.query_row(
            "SELECT duration_ms, parts, audio_path, transcript_raw, title, summary, action_items, category, reviewed FROM meetings WHERE id=?1",
            [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
        ).unwrap()
    }

    #[test]
    fn failed_continuation_turn_insert_preserves_entire_meeting_and_audio() {
        let dir = std::env::temp_dir().join(format!("ts_continue_failure_{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<_> = ["old.webm", "old_clip.wav", "new.webm", "new_clip.wav"]
            .iter().map(|name| dir.join(name)).collect();
        for path in &paths { std::fs::write(path, b"retained audio").unwrap(); }
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let mut old = turn("speaker_you", "You", 0, 0);
        old.snippet_path = Some(paths[1].to_string_lossy().into_owned());
        let id = insert_completed_meeting(&conn, "Reviewed", "meet", "Chrome", "", 6000,
            Some(&paths[0].to_string_lossy()), "Original speech", &[old.clone()]).unwrap();
        save_meeting_review_internal(&conn, id, "Approved", "Planning", &["Approved note".into()], &[], &[]).unwrap();
        let before = content_state(&conn, id);
        let before_turns = stored_turns(&conn, id);
        let mut new = turn("speaker_remote_1", "Guest", 1, 6000);
        new.text = "reject".into();
        new.snippet_path = Some(paths[3].to_string_lossy().into_owned());
        conn.execute_batch("CREATE TRIGGER reject_continuation BEFORE INSERT ON meeting_turns WHEN NEW.text='reject' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();

        assert!(replace_meeting_content(&conn, id, 10000, Some(&paths[2].to_string_lossy()), "New speech", &[old, new]).is_err());
        assert_eq!(content_state(&conn, id), before);
        assert_eq!(stored_turns(&conn, id), before_turns);
        for path in &paths { assert_eq!(std::fs::read(path).unwrap(), b"retained audio"); }
        // Retrying after the failure must work on the same connection.
        conn.execute_batch("DROP TRIGGER reject_continuation;").unwrap();
        replace_meeting_content(&conn, id, 10000, Some(&paths[2].to_string_lossy()), "Retried speech", &[]).unwrap();
        assert!(!paths[0].exists());
        assert!(!paths[1].exists());
        assert!(paths[2].exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_review_json_blocks_continuation_without_erasing_saved_work() {
        for column in ["summary", "action_items"] {
            let conn = Connection::open_in_memory().unwrap();
            ensure_meetings_schema(&conn).unwrap();
            let old = turn("speaker_you", "You", 0, 0);
            let id = meeting_with(&conn, &[old.clone()]);
            conn.execute(&format!("UPDATE meetings SET reviewed=1, {column}='invalid JSON' WHERE id=?1"), [id]).unwrap();
            let before = content_state(&conn, id);
            let before_turns = stored_turns(&conn, id);
            assert!(replace_meeting_content(&conn, id, 10000, None, "New speech", &[old]).is_err());
            assert_eq!(content_state(&conn, id), before, "{column}");
            assert_eq!(stored_turns(&conn, id), before_turns);
        }
    }

    #[test]
    fn continuation_keeps_deleted_tasks_deleted_and_includes_exact_boundary() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let mut old = turn("speaker_you", "You", 0, 0);
        old.text = "I will send the proposal tomorrow.".into();
        let id = meeting_with(&conn, &[old.clone()]);
        save_meeting_review_internal(&conn, id, "Approved", "Planning", &[], &[], &[]).unwrap();
        let mut new = turn("speaker_you", "You", 0, 6000);
        new.text = "I will review the design tomorrow.".into();
        replace_meeting_content(&conn, id, 10000, None, "Continued speech", &[old.clone(), new.clone()]).unwrap();
        let actions: Vec<ActionItem> = serde_json::from_str(&content_state(&conn, id).6).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(actions[0].task.contains("review the design"));
        // A subsequent part with no new task must preserve the exact action,
        // including its stable ID, instead of resurrecting the deleted proposal.
        replace_meeting_content(&conn, id, 14000, None, "Continued speech", &[old, new]).unwrap();
        let after: Vec<ActionItem> = serde_json::from_str(&content_state(&conn, id).6).unwrap();
        assert_eq!(after, actions);
    }

    #[test]
    fn meetings_saved_after_the_migration_start_unreviewed() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        // A database from before the column existed, already holding a meeting.
        let legacy = meeting_with(&conn, &[turn("speaker_you", "You", 0, 0)]);
        conn.execute_batch("ALTER TABLE meetings DROP COLUMN reviewed;").unwrap();
        ensure_meetings_schema(&conn).unwrap();
        ensure_meetings_schema(&conn).unwrap(); // later launches must not re-mark rows
        let new = meeting_with(&conn, &[turn("speaker_you", "You", 0, 0)]);
        let reviewed = |id: i64| -> bool {
            conn.query_row("SELECT reviewed FROM meetings WHERE id=?1", [id], |r| r.get(0)).unwrap()
        };
        assert!(reviewed(legacy), "meetings from before the column existed keep their notes");
        assert!(!reviewed(new), "new meetings are not reviewed until the user saves a review");
    }

    #[test]
    fn legacy_meeting_reviews_are_preserved_by_migration() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let id = meeting_with(&conn, &[turn("speaker_you", "You", 0, 0)]);
        conn.execute_batch("ALTER TABLE meetings DROP COLUMN reviewed;").unwrap();
        conn.execute("UPDATE meetings SET summary='[\"Legacy approved summary\"]', category='Planning', action_items='[]' WHERE id=?1", [id]).unwrap();
        ensure_meetings_schema(&conn).unwrap();
        replace_meeting_content(&conn, id, 10000, None, "Continued speech", &[turn("speaker_you", "You", 0, 0)]).unwrap();
        let (summary, category): (String, String) = conn.query_row("SELECT summary,category FROM meetings WHERE id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(summary, "[\"Legacy approved summary\"]");
        assert_eq!(category, "Planning");
    }

    fn speakers_of(conn: &Connection, m: i64) -> Vec<(String, String, i64)> {
        let mut stmt = conn.prepare("SELECT speaker_id, speaker_name, channel FROM meeting_turns WHERE meeting_id = ?1 ORDER BY start_ms").unwrap();
        stmt.query_map([m], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect()
    }

    fn speaker_count_of(conn: &Connection, m: i64) -> i64 {
        conn.query_row("SELECT speaker_count FROM meetings WHERE id = ?1", [m], |r| r.get(0)).unwrap()
    }

    #[test]
    fn merging_speakers_moves_turns_and_undo_restores_them() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let mut a = turn("speaker_remote_1", "Speaker 1", 1, 0);
        a.snippet_path = Some("/clips/one.wav".into());
        let mut b = turn("speaker_remote_3", "Speaker 3", 1, 5000);
        b.snippet_path = Some("/clips/three.wav".into());
        let m = meeting_with(&conn, &[turn("speaker_you", "You", 0, 0), a, turn("speaker_remote_2", "Speaker 2", 1, 2500), b]);
        let before = speakers_of(&conn, m);
        assert_eq!(speaker_count_of(&conn, m), 4);

        let merge = merge_speakers_internal(&conn, m, "speaker_remote_3", "speaker_remote_1").unwrap();
        assert_eq!((merge.from_speaker_name.as_str(), merge.into_speaker_name.as_str()), ("Speaker 3", "Speaker 1"));
        let after = speakers_of(&conn, m);
        assert_eq!(after[3], ("speaker_remote_1".into(), "Speaker 1".into(), 1));
        assert_eq!(speaker_count_of(&conn, m), 3);
        let clip: Option<String> = conn.query_row("SELECT snippet_path FROM meeting_turns WHERE meeting_id = ?1 AND start_ms = 5000", [m], |r| r.get(0)).unwrap();
        assert_eq!(clip.as_deref(), Some("/clips/one.wav"));

        undo_merge_internal(&conn, m, merge.id).unwrap();
        assert_eq!(speakers_of(&conn, m), before);
        assert_eq!(speaker_count_of(&conn, m), 4);
        let clip: Option<String> = conn.query_row("SELECT snippet_path FROM meeting_turns WHERE meeting_id = ?1 AND start_ms = 5000", [m], |r| r.get(0)).unwrap();
        assert_eq!(clip.as_deref(), Some("/clips/three.wav"));
        assert!(speaker_merges(&conn, m).unwrap().is_empty());
    }

    #[test]
    fn merges_are_undone_newest_first() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let m = meeting_with(&conn, &[turn("speaker_remote_1", "Speaker 1", 1, 0), turn("speaker_remote_2", "Speaker 2", 1, 2500), turn("speaker_remote_3", "Speaker 3", 1, 5000)]);
        let before = speakers_of(&conn, m);
        let first = merge_speakers_internal(&conn, m, "speaker_remote_2", "speaker_remote_1").unwrap();
        let second = merge_speakers_internal(&conn, m, "speaker_remote_3", "speaker_remote_1").unwrap();
        assert_eq!(speaker_count_of(&conn, m), 1);
        assert_eq!(speaker_merges(&conn, m).unwrap().iter().map(|r| r.id).collect::<Vec<_>>(), [second.id, first.id]);

        assert!(undo_merge_internal(&conn, m, first.id).is_err());
        undo_merge_internal(&conn, m, second.id).unwrap();
        undo_merge_internal(&conn, m, first.id).unwrap();
        assert_eq!(speakers_of(&conn, m), before);
        assert!(undo_merge_internal(&conn, m, first.id).is_err());
    }

    #[test]
    fn merge_undo_preserves_a_vault_rename_made_after_the_merge() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        conn.execute("INSERT INTO speaker_vault (id,name,created_at,last_seen) VALUES ('person_alice','Alice','2026-10-19','2026-10-19')", []).unwrap();
        let m = meeting_with(&conn, &[
            turn("person_alice", "Alice", 1, 0),
            turn("speaker_remote_2", "Speaker 2", 1, 2500),
        ]);
        let merge = merge_speakers_internal(&conn, m, "person_alice", "speaker_remote_2").unwrap();
        // Alice no longer has a visible turn here, but the undo snapshot still references her.
        rename_vault_speaker_internal(&conn, "person_alice", "Alicia").unwrap();
        assert_eq!(speaker_merges(&conn, m).unwrap()[0].from_speaker_name, "Alicia");
        undo_merge_internal(&conn, m, merge.id).unwrap();
        assert_eq!(speakers_of(&conn, m)[0].1, "Alicia");
    }

    #[test]
    fn undo_merge_failure_rolls_back_restored_turns_and_keeps_the_undo_record() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let m = meeting_with(&conn, &[turn("a", "A", 1, 0), turn("b", "B", 1, 2500)]);
        let merge = merge_speakers_internal(&conn, m, "a", "b").unwrap();
        let before = speakers_of(&conn, m);
        conn.execute_batch("CREATE TRIGGER reject_undo BEFORE DELETE ON meeting_speaker_merges BEGIN SELECT RAISE(ABORT, 'blocked'); END;").unwrap();
        assert!(undo_merge_internal(&conn, m, merge.id).is_err());
        assert_eq!(speakers_of(&conn, m), before);
        assert_eq!(speaker_merges(&conn, m).unwrap().len(), 1);
        assert_eq!(speaker_count_of(&conn, m), 1);
    }

    #[test]
    fn merge_undo_keeps_a_meeting_only_alias_until_an_explicit_global_rename() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        conn.execute("INSERT INTO speaker_vault (id,name,created_at,last_seen) VALUES ('person_alice','Alice','2026-10-19','2026-10-19')", []).unwrap();
        let m = meeting_with(&conn, &[turn("person_alice", "Alice", 1, 0), turn("b", "B", 1, 2500)]);
        rename_speaker_internal(&conn, m, "person_alice", "Presenter", false).unwrap();
        let merge = merge_speakers_internal(&conn, m, "person_alice", "b").unwrap();
        assert_eq!(speaker_merges(&conn, m).unwrap()[0].from_speaker_name, "Presenter");
        undo_merge_internal(&conn, m, merge.id).unwrap();
        assert_eq!(speakers_of(&conn, m)[0].1, "Presenter");
        let merge = merge_speakers_internal(&conn, m, "person_alice", "b").unwrap();
        rename_vault_speaker_internal(&conn, "person_alice", "Alicia").unwrap();
        undo_merge_internal(&conn, m, merge.id).unwrap();
        assert_eq!(speakers_of(&conn, m)[0].1, "Alicia");
    }

    #[test]
    fn a_vault_rename_cannot_create_a_namesake_and_does_not_count_as_seen() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        conn.execute("INSERT INTO speaker_vault (id,name,created_at,last_seen) VALUES ('person_alice','Alice','2026-10-01','2026-10-01'),('person_bob','Bob','2026-10-01','2026-10-02')", []).unwrap();
        assert!(rename_vault_speaker_internal(&conn, "person_bob", "alice").is_err());
        let name: String = conn.query_row("SELECT name FROM speaker_vault WHERE id='person_bob'", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "Bob");
        // A different spelling of one's own name is fine, and is not an appearance.
        rename_vault_speaker_internal(&conn, "person_alice", "ALICE").unwrap();
        let (name, seen): (String, String) = conn.query_row("SELECT name, last_seen FROM speaker_vault WHERE id='person_alice'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((name.as_str(), seen.as_str()), ("ALICE", "2026-10-01"));
    }

    #[test]
    fn failed_undo_snapshot_rename_rolls_back_the_vault_and_visible_names() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        conn.execute("INSERT INTO speaker_vault (id,name,created_at,last_seen) VALUES ('person_alice','Alice','2026-10-19','2026-10-19')", []).unwrap();
        let m = meeting_with(&conn, &[turn("person_alice", "Alice", 1, 0), turn("b", "B", 1, 2500)]);
        let other = meeting_with(&conn, &[turn("person_alice", "Alice", 1, 0)]);
        let merge = merge_speakers_internal(&conn, m, "person_alice", "b").unwrap();
        conn.execute_batch("CREATE TRIGGER reject_snapshot BEFORE UPDATE ON meeting_speaker_merges BEGIN SELECT RAISE(ABORT, 'blocked'); END;").unwrap();
        assert!(rename_vault_speaker_internal(&conn, "person_alice", "Alicia").is_err());
        assert_eq!(vault_rows(&conn)[0].1, "Alice");
        assert_eq!(speakers_of(&conn, other)[0].1, "Alice");
        undo_merge_internal(&conn, m, merge.id).unwrap();
        assert_eq!(speakers_of(&conn, m)[0].1, "Alice");
    }

    #[test]
    fn merging_a_speaker_into_itself_or_a_missing_one_fails() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let m = meeting_with(&conn, &[turn("speaker_remote_1", "Speaker 1", 1, 0)]);
        assert!(merge_speakers_internal(&conn, m, "speaker_remote_1", "speaker_remote_1").is_err());
        assert!(merge_speakers_internal(&conn, m, "speaker_remote_9", "speaker_remote_1").is_err());
        assert!(merge_speakers_internal(&conn, m, "speaker_remote_1", "speaker_remote_9").is_err());
        assert!(speaker_merges(&conn, m).unwrap().is_empty());
    }

    #[test]
    fn deleting_a_meeting_removes_its_merges() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let m = meeting_with(&conn, &[turn("speaker_remote_1", "Speaker 1", 1, 0), turn("speaker_remote_2", "Speaker 2", 1, 2500)]);
        merge_speakers_internal(&conn, m, "speaker_remote_2", "speaker_remote_1").unwrap();
        delete_meeting_internal(&conn, m).unwrap();
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM meeting_speaker_merges", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    fn vault_rows(conn: &Connection) -> Vec<(String, String, i64)> {
        let mut stmt = conn.prepare("SELECT id, name, meeting_count FROM speaker_vault ORDER BY created_at").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn naming_a_caller_enrolls_a_person_and_links_the_meeting() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let m = meeting_with(&conn, &[turn("speaker_you", "You", 0, 0), turn("speaker_remote_1", "Remote Participant", 1, 2500)]);

        rename_speaker_internal(&conn, m, "speaker_remote_1", "Alice", true).unwrap();

        let vault = vault_rows(&conn);
        assert_eq!(vault.len(), 1);
        assert!(vault[0].0.starts_with("person_"), "vault ids are permanent, not per-meeting labels");
        assert_eq!((vault[0].1.as_str(), vault[0].2), ("Alice", 1));
        let linked: String = conn
            .query_row("SELECT speaker_id FROM meeting_turns WHERE meeting_id = ?1 AND channel = 1", params![m], |r| r.get(0))
            .unwrap();
        assert_eq!(linked, vault[0].0);
    }

    #[test]
    fn same_name_in_another_meeting_links_to_the_same_person() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let a = meeting_with(&conn, &[turn("speaker_remote_1", "Remote Participant", 1, 0)]);
        let b = meeting_with(&conn, &[turn("speaker_remote_1", "Remote Participant", 1, 0)]);
        rename_speaker_internal(&conn, a, "speaker_remote_1", "Alice", true).unwrap();
        rename_speaker_internal(&conn, b, "speaker_remote_1", "alice", true).unwrap();

        let vault = vault_rows(&conn);
        assert_eq!(vault.len(), 1, "no duplicate person for the same name");
        assert_eq!(vault[0].2, 2);
    }

    #[test]
    fn different_callers_with_the_same_per_meeting_label_stay_separate() {
        // Every meeting labels its first caller speaker_remote_1; that label must
        // never merge different people.
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let a = meeting_with(&conn, &[turn("speaker_remote_1", "Remote Participant", 1, 0)]);
        let b = meeting_with(&conn, &[turn("speaker_remote_1", "Remote Participant", 1, 0)]);
        rename_speaker_internal(&conn, a, "speaker_remote_1", "Alice", true).unwrap();
        rename_speaker_internal(&conn, b, "speaker_remote_1", "Bob", true).unwrap();

        let names: Vec<String> = vault_rows(&conn).into_iter().map(|v| v.1).collect();
        assert_eq!(names, vec!["Alice".to_string(), "Bob".to_string()]);
    }

    #[test]
    fn a_recognized_person_counts_once_per_meeting() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let a = meeting_with(&conn, &[turn("speaker_remote_1", "Remote Participant", 1, 0)]);
        rename_speaker_internal(&conn, a, "speaker_remote_1", "Alice", true).unwrap();
        let alice = vault_rows(&conn)[0].0.clone();

        // Next meeting: the diarizer recognized Alice in three turns.
        meeting_with(&conn, &[turn(&alice, "Alice", 1, 0), turn(&alice, "Alice", 1, 3000), turn(&alice, "Alice", 1, 6000)]);
        assert_eq!(vault_rows(&conn)[0].2, 2);
    }

    #[test]
    fn title_only_save_preserves_notes_actions_category_and_review_state() {
        let conn=Connection::open_in_memory().unwrap();ensure_meetings_schema(&conn).unwrap();
        let id=meeting_with(&conn,&[turn("you","You",0,0)]);
        conn.execute("UPDATE meetings SET summary='[\"New notes\"]',action_items='[\"New tasks\"]',category='Planning',reviewed=0 WHERE id=?1",[id]).unwrap();
        update_meeting_title_in(&conn,id,"  Correct title  ").unwrap();
        let saved:(String,String,String,String,bool)=conn.query_row("SELECT title,summary,action_items,category,reviewed FROM meetings WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(saved,("Correct title".into(),"[\"New notes\"]".into(),"[\"New tasks\"]".into(),"Planning".into(),false));
        assert!(update_meeting_title_in(&conn,id,"  ").is_err());
        assert!(update_meeting_title_in(&conn,id+100,"Missing").is_err());
    }

    #[test]
    fn meeting_rename_updates_all_linked_turns_and_rolls_back_any_failure() {
        let conn=Connection::open_in_memory().unwrap();ensure_meetings_schema(&conn).unwrap();
        let first=meeting_with(&conn,&[turn("remote","Speaker 1",1,0)]);
        rename_speaker_internal(&conn,first,"remote","Alice",true).unwrap();
        let person=vault_rows(&conn)[0].0.clone();
        let second=meeting_with(&conn,&[turn(&person,"Alice",1,0)]);
        rename_speaker_internal(&conn,first,&person,"Alicia",true).unwrap();
        assert_eq!(speakers_of(&conn,first)[0].1,"Alicia");assert_eq!(speakers_of(&conn,second)[0].1,"Alicia");
        assert_eq!(vault_rows(&conn)[0].1,"Alicia");
        conn.execute_batch("CREATE TRIGGER reject_person_name BEFORE UPDATE OF name ON speaker_vault BEGIN SELECT RAISE(ABORT,'blocked'); END;").unwrap();
        assert!(rename_speaker_internal(&conn,first,&person,"Lost name",true).is_err());
        assert_eq!(speakers_of(&conn,first)[0].1,"Alicia");assert_eq!(vault_rows(&conn)[0].1,"Alicia");
        conn.execute_batch("DROP TRIGGER reject_person_name;").unwrap();
        conn.execute_batch(&format!("CREATE TRIGGER reject_linked_name BEFORE UPDATE OF speaker_name ON meeting_turns WHEN NEW.meeting_id={second} BEGIN SELECT RAISE(ABORT,'blocked'); END;")).unwrap();
        assert!(rename_speaker_internal(&conn,first,&person,"Other lost name",true).is_err());
        assert_eq!(speakers_of(&conn,first)[0].1,"Alicia");assert_eq!(speakers_of(&conn,second)[0].1,"Alicia");assert_eq!(vault_rows(&conn)[0].1,"Alicia");
    }

    #[test]
    fn linking_two_groups_to_one_person_recounts_speakers_without_counting_the_meeting_twice() {
        let conn=Connection::open_in_memory().unwrap();ensure_meetings_schema(&conn).unwrap();
        let id=meeting_with(&conn,&[turn("one","Speaker 1",1,0),turn("two","Speaker 2",1,3000)]);
        rename_speaker_internal(&conn,id,"one","Alice",true).unwrap();
        rename_speaker_internal(&conn,id,"two","Alice",true).unwrap();
        assert_eq!(speaker_count_of(&conn,id),1);
        assert_eq!(vault_rows(&conn).len(),1);assert_eq!(vault_rows(&conn)[0].2,1);
        assert_eq!(speakers_of(&conn,id)[0].0,speakers_of(&conn,id)[1].0);
    }

    #[test]
    fn vault_rename_updates_all_linked_meetings() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let meeting_id = meeting_with(&conn, &[turn("speaker_remote_1", "Remote Participant", 1, 0)]);
        rename_speaker_internal(&conn, meeting_id, "speaker_remote_1", "Alice", true).unwrap();
        let person_id = vault_rows(&conn)[0].0.clone();
        meeting_with(&conn, &[turn(&person_id, "Alice", 1, 0)]);

        rename_vault_speaker_internal(&conn, &person_id, "Alicia").unwrap();
        assert_eq!(vault_rows(&conn)[0].1, "Alicia");
        let old_names: i64 = conn.query_row("SELECT count(*) FROM meeting_turns WHERE speaker_id = ?1 AND speaker_name != 'Alicia'", params![person_id], |row| row.get(0)).unwrap();
        assert_eq!(old_names, 0);
    }

    #[test]
    fn deleting_meeting_keeps_vault_referenced_clip() {
        let dir = std::env::temp_dir().join(format!("taurscribe_vault_clip_{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("kept.wav");
        let removed = dir.join("removed.wav");
        std::fs::write(&kept, b"clip").unwrap();
        std::fs::write(&removed, b"clip").unwrap();
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let mut speaker = turn("speaker_remote_1", "Alice", 1, 0);
        speaker.snippet_path = Some(kept.to_string_lossy().to_string());
        speaker.candidate_snippets = vec![kept.to_string_lossy().to_string(), removed.to_string_lossy().to_string()];
        let meeting_id = meeting_with(&conn, &[speaker]);
        conn.execute("INSERT INTO speaker_vault (id, name, created_at, last_seen, snippet_path) VALUES ('person_1', 'Alice', '', '', ?1)", params![kept.to_string_lossy()]).unwrap();

        delete_meeting_internal(&conn, meeting_id).unwrap();
        assert!(kept.exists());
        assert!(!removed.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_local_user_is_never_enrolled() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        let m = meeting_with(&conn, &[turn("speaker_you", "You", 0, 0)]);
        rename_speaker_internal(&conn, m, "speaker_you", "Abdullah", true).unwrap();
        assert!(vault_rows(&conn).is_empty());
    }

    #[test]
    fn voiceprints_are_averaged_not_overwritten() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        let mean = average_embedding(&a, 1, &b);
        assert!((mean[0] - mean[1]).abs() < 1e-6, "one old + one new sample weigh equally");
        let mean3 = average_embedding(&a, 3, &b);
        assert!(mean3[0] > mean3[1], "three earlier samples outweigh one new one");
    }
}

#[cfg(test)]
mod vault_and_export_tests {
    use super::*;
    use crate::meeting_summary::ActionItem;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ts_vault_{label}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn vault_with(conn: &Connection, current: Option<&str>, candidates: &[&str]) {
        conn.execute(
            "INSERT INTO speaker_vault (id, name, created_at, last_seen, snippet_path, candidate_snippets) VALUES ('p1', 'Alice', '', '', ?1, ?2)",
            params![current, serde_json::to_string(candidates).unwrap()],
        )
        .unwrap();
    }

    #[test]
    fn cycling_a_vault_sample_moves_to_the_next_clip_and_wraps() {
        let dir = scratch("cycle");
        let clips: Vec<String> = (0..3).map(|i| {
            let p = dir.join(format!("c{i}.wav"));
            std::fs::write(&p, b"RIFF").unwrap();
            p.to_string_lossy().into_owned()
        }).collect();
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        vault_with(&conn, Some(&clips[0]), &clips.iter().map(String::as_str).collect::<Vec<_>>());

        assert_eq!(cycle_vault_snippet_in(&conn, "p1").unwrap(), (clips[1].clone(), 1));
        assert_eq!(cycle_vault_snippet_in(&conn, "p1").unwrap(), (clips[2].clone(), 2));
        assert_eq!(cycle_vault_snippet_in(&conn, "p1").unwrap(), (clips[0].clone(), 0), "wraps around");
        let saved: String = conn.query_row("SELECT snippet_path FROM speaker_vault WHERE id='p1'", [], |r| r.get(0)).unwrap();
        assert_eq!(saved, clips[0], "the choice is saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleted_clips_are_skipped_and_a_lone_clip_says_there_is_nothing_else() {
        let dir = scratch("skip");
        let keep = dir.join("keep.wav");
        let other = dir.join("other.wav");
        std::fs::write(&keep, b"RIFF").unwrap();
        std::fs::write(&other, b"RIFF").unwrap();
        let gone = dir.join("deleted.wav");
        let (keep, other, gone) = (keep.to_string_lossy().into_owned(), other.to_string_lossy().into_owned(), gone.to_string_lossy().into_owned());
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        vault_with(&conn, Some(&keep), &[&keep, &gone, &other]);
        assert_eq!(cycle_vault_snippet_in(&conn, "p1").unwrap().0, other, "the deleted clip is skipped");

        std::fs::remove_file(&keep).unwrap();
        assert!(cycle_vault_snippet_in(&conn, "p1").unwrap_err().contains("No other voice samples"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cycling_with_no_samples_or_an_unknown_person_is_an_error_not_a_panic() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        assert!(cycle_vault_snippet_in(&conn, "nobody").unwrap_err().contains("not found"));
        vault_with(&conn, None, &[]);
        assert!(cycle_vault_snippet_in(&conn, "p1").is_err());
        conn.execute("UPDATE speaker_vault SET candidate_snippets = 'not json' WHERE id = 'p1'", []).unwrap();
        assert!(cycle_vault_snippet_in(&conn, "p1").is_err());
    }

    #[test]
    fn the_current_clip_missing_from_the_list_starts_at_the_first_existing_one() {
        let dir = scratch("first");
        let a = dir.join("a.wav");
        std::fs::write(&a, b"RIFF").unwrap();
        let a = a.to_string_lossy().into_owned();
        assert_eq!(next_existing_snippet(&[a.clone()], Some("/elsewhere/old.wav")), Some((a.clone(), 0)));
        assert_eq!(next_existing_snippet(&[a.clone()], None), Some((a, 0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sample_meeting() -> MeetingDetailRecord {
        let turn = |name: &str, start_ms: u64, text: &str| crate::diarization::DiarizedTurn {
            speaker_id: name.to_lowercase(), speaker_name: name.into(), start_ms, end_ms: start_ms + 1000,
            channel: 0, text: text.into(), snippet_path: None, candidate_snippets: Vec::new(), current_snippet_idx: 0,
        };
        MeetingDetailRecord {
            turn_ids: vec![], transcription_model: None,
            id: 7, session_id: "s".into(), title: "Budget review".into(), platform: "In person".into(),
            app_name: "".into(), url: "".into(), created_at: "2026-10-07T12:00:00Z".into(), duration_ms: 3_725_000,
            audio_path: None, transcript_raw: "".into(), summary: vec!["Budget approved".into()],
            action_items: vec![
                ActionItem { id: "a1".into(), task: "Send numbers".into(), assignee: "Sam".into(), status: "done".into() },
                ActionItem { id: "a2".into(), task: "Book room".into(), assignee: "  ".into(), status: "todo".into() },
            ],
            category: "Planning".into(), speaker_count: 2,
            turns: vec![turn("You", 5_000, "Let's start."), turn("Sam", 3_723_000, "Done, an hour later.")],
            audio_missing: false, clips_missing: 0, recording_sources: vec![], merges: vec![],
        }
    }

    #[test]
    fn notes_export_as_text_markdown_and_json() {
        let m = sample_meeting();
        let text = render_meeting_notes(&m, "TXT").unwrap();
        assert!(text.starts_with("TITLE: Budget review\nCATEGORY: Planning\n"));
        assert!(text.contains("[X] Send numbers (@Sam)\n"));
        assert!(text.contains("[ ] Book room\n"), "no empty (@) for an unassigned item: {text}");
        assert!(text.contains("[00:05] You: Let's start.\n"));
        assert!(text.contains("[62:03] Sam: Done, an hour later.\n"));

        let md = render_meeting_notes(&m, "markdown").unwrap();
        assert!(md.starts_with("# Budget review\n"));
        assert!(md.contains("- [x] **Send numbers** — *Assignee: @Sam*\n"));
        assert!(md.contains("- [ ] **Book room**\n"));
        assert!(md.contains("**00:05 You**: Let's start."));
        assert_eq!(render_meeting_notes(&m, "anything-else").unwrap(), md, "Markdown is the default");

        let json: serde_json::Value = serde_json::from_str(&render_meeting_notes(&m, "json").unwrap()).unwrap();
        assert_eq!(json["title"], "Budget review");
        assert_eq!(json["turns"].as_array().unwrap().len(), 2);
        assert_eq!(json["action_items"][1]["status"], "todo");
    }

    #[test]
    fn notes_are_written_to_the_chosen_file_and_a_bad_folder_is_reported() {
        let dir = scratch("notes");
        let file = dir.join("Budget_review_transcript.md");
        write_notes(&file, "# Budget review\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "# Budget review\n");
        let err = write_notes(&dir.join("missing-folder").join("x.md"), "x").unwrap_err();
        assert!(err.starts_with("Couldn't write"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_meeting_still_exports() {
        let mut m = sample_meeting();
        m.summary.clear();
        m.action_items.clear();
        m.turns.clear();
        for format in ["txt", "md", "json"] {
            assert!(!render_meeting_notes(&m, format).unwrap().is_empty(), "{format}");
        }
    }
}

pub(crate) fn turn_revision(conn: &Connection, meeting_id: i64) -> Result<String, String> {
    let mut stmt = conn.prepare("SELECT id,speaker_id,speaker_name,start_ms,end_ms,channel,text,snippet_path,candidate_snippets,COALESCE(current_snippet_idx,0) FROM meeting_turns WHERE meeting_id=?1 ORDER BY id").map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![meeting_id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?,r.get::<_,String>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,Option<String>>(8)?,r.get::<_,i64>(9)?))).map_err(|e|e.to_string())?;
    serde_json::to_string(&rows.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}

/// Commit re-separation only after all decoding succeeded. Existing manual
/// merges are obsolete once turns are replaced; vault identities stay untouched.
pub(crate) fn replace_separated_turns(conn: &Connection, meeting_id: i64, turns: &[DiarizedTurn], expected_revision: &str) -> Result<(), String> {
    if turns.is_empty() || turns.iter().all(|t| t.text.trim().is_empty()) { return Err("No speech was recovered; existing transcript is unchanged".into()); }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    if turn_revision(&tx,meeting_id)? != expected_revision { return Err("The meeting was edited during processing; existing changes were kept. Try re-separating again.".into()); }
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",params![meeting_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exists { return Err("Meeting was deleted during processing".into()); }
    tx.execute("DELETE FROM meeting_turns WHERE meeting_id=?1", params![meeting_id]).map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM meeting_speaker_merges WHERE meeting_id=?1", params![meeting_id]).map_err(|e| e.to_string())?;
    insert_turns(&tx, meeting_id, turns)?;
    let transcript = turns.iter().map(|t| t.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
    tx.execute("UPDATE meetings SET transcript_raw=?2 WHERE id=?1", params![meeting_id,transcript]).map_err(|e| e.to_string())?;
    recount_speakers(&tx,meeting_id)?;
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod reseparation_tests {
    use super::*;
    #[test]
    fn reprocessing_preserves_concurrent_edits_and_rolls_back_failed_writes() {
        let conn=Connection::open_in_memory().unwrap();ensure_meetings_schema(&conn).unwrap();
        conn.execute("INSERT INTO meetings(id,session_id,title,platform,app_name,url,created_at,duration_ms,transcript_raw,category,summary) VALUES(1,'m','Meeting','meet','','','',10000,'Original','Planning',?1)",[r#"["Keep notes"]"#]).unwrap();
        let t=DiarizedTurn {speaker_id:"a".into(),speaker_name:"Dana".into(),start_ms:0,end_ms:10000,channel:1,text:"Original".into(),snippet_path:None,candidate_snippets:vec![],current_snippet_idx:0};
        insert_turns(&conn,1,&[t.clone()]).unwrap();let original=turn_revision(&conn,1).unwrap();
        conn.execute("UPDATE meeting_turns SET speaker_name='Edited' WHERE meeting_id=1",[]).unwrap();
        let new=DiarizedTurn{text:"New".into(),..t};
        assert!(replace_separated_turns(&conn,1,&[new.clone()],&original).unwrap_err().contains("edited"));
        let edited=turn_revision(&conn,1).unwrap();
        assert!(replace_separated_turns(&conn,1,&[],&edited).is_err());
        conn.execute_batch("CREATE TRIGGER reject_turn BEFORE INSERT ON meeting_turns BEGIN SELECT RAISE(ABORT,'blocked'); END;").unwrap();
        assert!(replace_separated_turns(&conn,1,&[new.clone()],&edited).is_err());assert_eq!(turn_revision(&conn,1).unwrap(),edited);
        conn.execute_batch("DROP TRIGGER reject_turn;").unwrap();
        replace_separated_turns(&conn,1,&[new],&edited).unwrap();
        let (raw,notes):(String,String)=conn.query_row("SELECT transcript_raw,summary FROM meetings WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(raw,"New");assert_eq!(notes,r#"["Keep notes"]"#);
    }
}

#[derive(Debug, Serialize)]
pub struct DuplicateMeetingGroup {
    pub kind: String,
    pub meeting_ids: Vec<i64>,
}
#[derive(Debug, Serialize)]
pub struct MeetingDuplicateReport {
    pub groups: Vec<DuplicateMeetingGroup>,
    pub checked_meetings: usize,
    pub unavailable_audio: usize,
}

/// Read-only: identical stored recordings are certain; matching substantial
/// transcripts are only candidates (different encodings or repeated content).
#[tauri::command]
pub async fn check_duplicate_meetings() -> Result<MeetingDuplicateReport, String> {
    tauri::async_runtime::spawn_blocking(|| duplicate_meetings_in(&open_connection()?))
        .await.map_err(|e| format!("Duplicate check failed: {e}"))?
}

fn duplicate_meetings_in(conn: &Connection) -> Result<MeetingDuplicateReport, String> {
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, io::Read};
    let records = {
        let mut statement = conn.prepare("SELECT id, audio_path, transcript_raw, duration_ms FROM meetings ORDER BY id").map_err(|e| e.to_string())?;
        let rows = statement.query_map([], |row| Ok((row.get::<_,i64>(0)?, row.get::<_,Option<String>>(1)?, row.get::<_,String>(2)?, row.get::<_,i64>(3)?))).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?
    };
    let mut audio_groups = BTreeMap::<String, Vec<i64>>::new();
    let mut text_groups = BTreeMap::<String, Vec<(i64,i64)>>::new();
    let mut unavailable_audio = 0;
    for (id, path, transcript, duration) in &records {
        if let Some(path) = path {
            let hash = (|| -> std::io::Result<String> {
                let mut file = std::fs::File::open(path)?;
                let before = file.metadata()?;
                let mut hash = Sha256::new(); let mut buffer = [0u8; 64*1024];
                loop { let n=file.read(&mut buffer)?; if n==0 {break;} hash.update(&buffer[..n]); }
                let after=file.metadata()?;
                if before.len()!=after.len() || before.modified()?!=after.modified()? || after.len()==0 {
                    return Err(std::io::Error::other("Recording changed or is empty"));
                }
                Ok(format!("{:x}", hash.finalize()))
            })();
            match hash { Ok(key) => audio_groups.entry(key).or_default().push(*id), Err(_) => unavailable_audio+=1 }
        } else { unavailable_audio+=1; }
        let normalized = transcript.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if normalized.chars().count() >= 80 {
            text_groups.entry(format!("{:x}",Sha256::digest(normalized.as_bytes()))).or_default().push((*id,*duration));
        }
    }
    let mut groups: Vec<_> = audio_groups.into_values().filter(|ids| ids.len()>1)
        .map(|meeting_ids| DuplicateMeetingGroup { kind:"identical_recording".into(), meeting_ids }).collect();
    for candidates in text_groups.into_values() {
        // Require duration agreement as well as text; exact audio groups already
        // cover the strongest match and should not be shown twice.
        let mut remaining=candidates;
        while let Some((id,duration))=remaining.pop() {
            let mut ids=vec![id];
            remaining.retain(|(other,time)| {
                if (time-duration).abs() <= (duration.abs()/100).max(1000) {ids.push(*other);false} else {true}
            });
            if ids.len()>1 && !groups.iter().any(|group| ids.iter().all(|id| group.meeting_ids.contains(id))) {
                ids.sort_unstable(); groups.push(DuplicateMeetingGroup{kind:"matching_transcript".into(),meeting_ids:ids});
            }
        }
    }
    Ok(MeetingDuplicateReport {groups,checked_meetings:records.len(),unavailable_audio})
}

#[cfg(test)]
mod duplicate_check_tests {
    use super::*;
    #[test]
    fn duplicates_require_audio_or_substantial_matching_text_and_do_not_mutate_history() {
        let conn=Connection::open_in_memory().unwrap(); ensure_meetings_schema(&conn).unwrap();
        let dir=std::env::temp_dir().join(format!("taurscribe-dupe-test-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let a=dir.join("a.wav"); let b=dir.join("b.wav");
        std::fs::write(&a,b"identical fixture recording").unwrap(); std::fs::write(&b,b"identical fixture recording").unwrap();
        let long="substantial meeting text with specific agenda items and decisions rather than a common hello repeated";
        for (id,path,text,duration) in [(1,Some(a),"hello",5000),(2,Some(b),"different text",5000),(3,None,long,20000),(4,None,long,20100),(5,None,long,40000),(6,None,"hello",5000)] {
            conn.execute("INSERT INTO meetings(id,session_id,title,platform,app_name,url,created_at,duration_ms,audio_path,transcript_raw,category) VALUES (?1,?2,'same title','test','test','','now',?3,?4,?5,'')",
                params![id,format!("fixture-{id}"),duration,path.map(|p|p.to_string_lossy().into_owned()),text]).unwrap();
        }
        let report=duplicate_meetings_in(&conn).unwrap();
        assert_eq!(report.checked_meetings,6); assert_eq!(report.unavailable_audio,4);
        assert_eq!(report.groups.len(),2);
        assert!(report.groups.iter().any(|g|g.kind=="identical_recording"&&g.meeting_ids==vec![1,2]));
        assert!(report.groups.iter().any(|g|g.kind=="matching_transcript"&&g.meeting_ids==vec![3,4]));
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM meetings",[],|r|r.get::<_,i64>(0)).unwrap(),6);
        std::fs::remove_dir_all(dir).unwrap();
    }
}


#[tauri::command]
pub async fn get_meeting_tags(meeting_id: i64) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn=open_connection()?;
        let mut statement=conn.prepare("SELECT tag FROM meeting_tags WHERE meeting_id=?1 ORDER BY tag COLLATE NOCASE").map_err(|e|e.to_string())?;
        let rows=statement.query_map([meeting_id],|row|row.get::<_,String>(0)).map_err(|e|e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())
    }).await.map_err(|e|e.to_string())?
}

fn save_tags_in(conn: &Connection, meeting_id: i64, tags: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized=Vec::new();let mut seen=std::collections::HashSet::new();
    for tag in tags {
        let tag=tag.split_whitespace().collect::<Vec<_>>().join(" ");
        if tag.is_empty(){continue;}
        if tag.chars().count()>40 || tag.contains(',') || tag.chars().any(char::is_control) {return Err("Tags must be at most 40 characters, without commas or control characters".into());}
        if seen.insert(tag.to_lowercase()){normalized.push(tag);}
    }
    if normalized.len()>10 {return Err("Use at most 10 tags per meeting".into());}
    let tx=conn.unchecked_transaction().map_err(|e|e.to_string())?;
    let exists=tx.query_row("SELECT COUNT(*) FROM meetings WHERE id=?1",[meeting_id],|row|row.get::<_,i64>(0)).map_err(|e|e.to_string())?;
    if exists==0{return Err("Meeting no longer exists".into());}
    tx.execute("DELETE FROM meeting_tags WHERE meeting_id=?1",[meeting_id]).map_err(|e|e.to_string())?;
    for tag in &normalized {tx.execute("INSERT INTO meeting_tags(meeting_id,tag) VALUES (?1,?2)",params![meeting_id,tag]).map_err(|e|e.to_string())?;}
    tx.commit().map_err(|e|e.to_string())?;
    Ok(normalized)
}

#[tauri::command]
pub async fn set_meeting_tags(meeting_id: i64, tags: Vec<String>) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || save_tags_in(&open_connection()?,meeting_id,tags)).await.map_err(|e|e.to_string())?
}

#[cfg(test)]
mod history_persistence_tests {
    use super::*;
    #[test]
    fn tags_background_and_undo_survive_reopening_without_losing_transcript(){
        let root=std::env::temp_dir().join(format!("ts-history-persistence-{:016x}",rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();let db=root.join("history.db");
        let conn=Connection::open(&db).unwrap();ensure_meetings_schema(&conn).unwrap();
        let turns=vec![DiarizedTurn {speaker_id:"guest".into(),speaker_name:"Guest".into(),start_ms:0,end_ms:5000,channel:1,
            text:"Retained meeting transcript".into(),snippet_path:None,candidate_snippets:vec![],current_snippet_idx:0}];
        let id=insert_completed_meeting(&conn,"History fixture","test","test","",5000,None,"Retained meeting transcript",&turns).unwrap();
        assert_eq!(save_tags_in(&conn,id,vec!["  Project A ".into(),"project a".into(),"Review".into()]).unwrap(),vec!["Project A","Review"]);
        assert!(save_tags_in(&conn,id,vec!["x".repeat(41)]).is_err());
        let merged=merge_speakers_internal(&conn,id,"guest","background").unwrap();drop(conn);
        let conn=Connection::open(&db).unwrap();ensure_meetings_schema(&conn).unwrap();
        assert_eq!(conn.query_row("SELECT speaker_count FROM meetings WHERE id=?1",[id],|row|row.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(conn.query_row("SELECT transcript_raw FROM meetings WHERE id=?1",[id],|row|row.get::<_,String>(0)).unwrap(),"Retained meeting transcript");
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM meeting_tags WHERE meeting_id=?1",[id],|row|row.get::<_,i64>(0)).unwrap(),2);
        undo_merge_internal(&conn,id,merged.id).unwrap();
        assert_eq!(conn.query_row("SELECT speaker_id FROM meeting_turns WHERE meeting_id=?1",[id],|row|row.get::<_,String>(0)).unwrap(),"guest");
        drop(conn);std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod meeting_list_tests {
    use super::*;
    fn fixture(titles: &[&str]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        for (id, title) in titles.iter().enumerate() {
            conn.execute("INSERT INTO meetings (id,session_id,title,platform,app_name,url,created_at,duration_ms,transcript_raw,category,speaker_count) VALUES (?1,'s'||?1,?2,'zoom','','','2026-10-01T10:00:00+00:00',1000,'','General',1)", params![id as i64 + 1, title]).unwrap();
        }
        conn
    }
    fn found(conn: &Connection, search: &str) -> Vec<String> {
        list_meetings_in(conn, Some(search.into()), None, None, None, None).unwrap().into_iter().map(|m| m.title).collect()
    }
    #[test]
    fn search_text_is_matched_literally() {
        let conn = fixture(&["Hit 100% of target", "1000 signups", "file_list review", "Budget sync"]);
        assert_eq!(found(&conn, "100%"), ["Hit 100% of target"]);
        assert_eq!(found(&conn, "_"), ["file_list review"]);
        assert_eq!(found(&conn, "budget"), ["Budget sync"]);
        save_tags_in(&conn, 4, vec!["Q4_plan".into()]).unwrap();
        assert_eq!(found(&conn, "q4_"), ["Budget sync"]);
    }
}

#[cfg(test)]
mod manual_passage_tests {
    use super::*;
    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        for meeting in [1, 2] {
            conn.execute("INSERT INTO meetings (id,session_id,title,platform,app_name,url,created_at,duration_ms,transcript_raw,category,speaker_count) VALUES (?1,'s'||?1,'Title','','','','',10000,'Original','General',1)", [meeting]).unwrap();
            for start in [0, 5000] {
                conn.execute("INSERT INTO meeting_turns (meeting_id,speaker_id,speaker_name,start_ms,end_ms,channel,text) VALUES (?1,'speaker_1','Speaker 1',?2,?3,1,'Original words')", params![meeting,start,start+4000]).unwrap();
            }
        }
        conn
    }
    #[test]
    fn manual_passage_assignments_split_and_reuse_people_without_changing_other_meetings() {
        let conn = fixture();
        assign_meeting_turns_internal(&conn,1,&[1],None,Some("Alice")).unwrap();
        let alice: String = conn.query_row("SELECT speaker_id FROM meeting_turns WHERE id=1",[],|r|r.get(0)).unwrap();
        assert_eq!(conn.query_row("SELECT speaker_count FROM meetings WHERE id=1",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert_eq!(conn.query_row("SELECT speaker_name FROM meeting_turns WHERE id=2",[],|r|r.get::<_,String>(0)).unwrap(),"Speaker 1");
        assign_meeting_turns_internal(&conn,1,&[2],Some(&alice),None).unwrap();
        assert_eq!(conn.query_row("SELECT speaker_count FROM meetings WHERE id=1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(conn.query_row("SELECT speaker_name FROM meeting_turns WHERE id=3",[],|r|r.get::<_,String>(0)).unwrap(),"Speaker 1");
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM speaker_vault",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    #[test]
    fn stale_or_foreign_turns_roll_back_the_whole_assignment() {
        let conn = fixture();
        assert!(assign_meeting_turns_internal(&conn,1,&[1,3],None,Some("Alice")).is_err());
        assert_eq!(conn.query_row("SELECT speaker_name FROM meeting_turns WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"Speaker 1");
        assert!(assign_meeting_turns_internal(&conn,1,&[1],Some("missing"),None).is_err());
        assert!(assign_meeting_turns_internal(&conn,1,&[],None,Some("Alice")).is_err());
        assert!(assign_meeting_turns_internal(&conn,1,&[1],None,Some(" ")).is_err());
    }
    #[test]
    fn transcription_model_is_saved_without_guessing_for_old_meetings() {
        let conn=fixture();
        assert_eq!(conn.query_row("SELECT transcription_model FROM meetings WHERE id=1",[],|r|r.get::<_,Option<String>>(0)).unwrap(),None);
        save_transcription_model(&conn,1,Some("granite-speech-5-nc")).unwrap();
        assert_eq!(conn.query_row("SELECT transcription_model FROM meetings WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"granite-speech-5-nc");
    }
}

#[cfg(test)]
mod audit_retention_tests {
    use super::*;
    fn fixture() -> (Connection, PathBuf, String, String) {
        let root = std::env::temp_dir().join(format!("taurscribe-retention-{:016x}",rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let kept = root.join("speaker.wav").to_string_lossy().into_owned();
        let candidate = root.join("speaker_cand_1.wav").to_string_lossy().into_owned();
        for path in [PathBuf::from(&kept),PathBuf::from(&candidate),crate::diarization::voice_sample_path(&kept)] {std::fs::write(path,b"retained audio").unwrap();}
        let conn=Connection::open_in_memory().unwrap();ensure_meetings_schema(&conn).unwrap();
        conn.execute("INSERT INTO meetings(id,session_id,title,platform,app_name,url,created_at,duration_ms,transcript_raw,category) VALUES(1,'audit','Meeting','','','','',12000,'words','General')",[]).unwrap();
        conn.execute("INSERT INTO meeting_turns(meeting_id,speaker_id,speaker_name,start_ms,end_ms,channel,text,snippet_path,candidate_snippets) VALUES(1,'alice','Alice',0,6000,1,'words',?1,?2)",params![kept,serde_json::to_string(&vec![&kept,&candidate]).unwrap()]).unwrap();
        conn.execute("INSERT INTO speaker_vault(id,name,created_at,last_seen) VALUES('alice','Alice','','')",[]).unwrap();
        (conn,root,kept,candidate)
    }
    #[test]
    fn meeting_deletion_keeps_secondary_enrolled_audio_and_shared_voice_companion() {
        let (conn,root,kept,candidate)=fixture();
        conn.execute("INSERT INTO speaker_voiceprints(person_id,embedding_json,clip_path,created_at) VALUES('alice','[1]',?1,'')",[&kept]).unwrap();
        delete_meeting_internal(&conn,1).unwrap();
        assert!(Path::new(&kept).exists());
        assert!(crate::diarization::voice_sample_path(&kept).exists());
        assert!(!Path::new(&candidate).exists());
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM speaker_voiceprints",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn primary_vault_clip_keeps_its_voice_when_an_unenrolled_candidate_is_deleted() {
        let (conn,root,kept,candidate)=fixture();
        conn.execute("UPDATE speaker_vault SET snippet_path=?1",[&kept]).unwrap();
        delete_meeting_internal(&conn,1).unwrap();
        assert!(crate::diarization::voice_sample_path(&kept).exists());
        assert!(!Path::new(&candidate).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn continued_meeting_preserves_independently_enrolled_samples() {
        let (conn,root,kept,candidate)=fixture();
        conn.execute("INSERT INTO speaker_voiceprints(person_id,embedding_json,clip_path,created_at) VALUES('alice','[1]',?1,'')",[&kept]).unwrap();
        replace_meeting_content(&conn,1,16000,None,"new words",&[]).unwrap();
        assert!(Path::new(&kept).exists());
        assert!(crate::diarization::voice_sample_path(&kept).exists());
        assert!(!Path::new(&candidate).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn manual_assignment_previews_use_corrected_audio_ranges() {
        let conn=Connection::open_in_memory().unwrap();ensure_meetings_schema(&conn).unwrap();
        let root=std::env::temp_dir().join(format!("taurscribe-corrected-preview-{:016x}",rand::random::<u64>()));std::fs::create_dir_all(&root).unwrap();
        let audio=root.join("recording.wav");
        let mut writer=hound::WavWriter::create(&audio,hound::WavSpec{channels:1,sample_rate:16000,bits_per_sample:32,sample_format:hound::SampleFormat::Float}).unwrap();
        for i in 0..16000*12 {writer.write_sample(if i<16000*6 {0.2_f32}else{-0.2_f32}).unwrap();}writer.finalize().unwrap();
        conn.execute("INSERT INTO meetings(id,session_id,title,platform,app_name,url,created_at,duration_ms,audio_path,transcript_raw,category) VALUES(1,'preview','Meeting','','','','',12000,?1,'words','General')",[audio.to_string_lossy().as_ref()]).unwrap();
        for start in [0,6000] {conn.execute("INSERT INTO meeting_turns(meeting_id,speaker_id,speaker_name,start_ms,end_ms,channel,text,snippet_path) VALUES(1,'guest','Guest',?1,?2,1,'words','/old/group.wav')",params![start,start+6000]).unwrap();}
        assign_meeting_turns_internal(&conn,1,&[1],None,Some("Alice")).unwrap();
        let alice:String=conn.query_row("SELECT speaker_id FROM meeting_turns WHERE id=1",[],|r|r.get(0)).unwrap();
        for (speaker,expected) in [(&*alice,0.2_f32),("guest",-0.2_f32)] {
            let preview=super::super::voice_bank::generate_meeting_sample_in(&conn,1,speaker,&root.join("voicebank"),&[]).unwrap();
            let (samples,rate)=crate::audio_decode::decode_audio_mono_f32(Path::new(&preview)).unwrap();
            assert_eq!(rate,16000);assert_eq!(samples.len(),16000*6);
            assert!(samples.iter().all(|x|(*x-expected).abs()<0.00001),"preview contains another speaker's interval");
        }
        conn.execute("UPDATE meeting_turns SET end_ms=start_ms+2000 WHERE meeting_id=1",[]).unwrap();
        let short=super::super::voice_bank::generate_meeting_sample_in(&conn,1,&alice,&root.join("voicebank"),&[]).unwrap();
        assert_eq!(hound::WavReader::open(short).unwrap().duration(),16000*2,"a short corrected passage is still playable");
        std::fs::remove_dir_all(root).unwrap();
    }
}
