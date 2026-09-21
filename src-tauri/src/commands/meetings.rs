use crate::diarization::DiarizedTurn;
use crate::meeting_detector::{MeetingDetectionStatus, MeetingInfo};
use crate::meeting_summary::{extract_summary_heuristics, ActionItem, MeetingSummaryOutput};
use crate::state::AudioState;
use chrono::Utc;
use dirs::data_local_dir;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, State};

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
            speaker_count       INTEGER NOT NULL DEFAULT 1
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
        "#,
    )
    .map_err(|e| format!("Failed to initialize meetings schema: {}", e))?;

    // Incremental column migrations for existing databases
    let _ = conn.execute("ALTER TABLE meeting_turns ADD COLUMN candidate_snippets TEXT", []);
    let _ = conn.execute("ALTER TABLE meeting_turns ADD COLUMN current_snippet_idx INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE speaker_vault ADD COLUMN embedding_json TEXT", []);
    let _ = conn.execute("ALTER TABLE speaker_vault ADD COLUMN candidate_snippets TEXT", []);
    // Number of meetings a vault person appeared in (sample_count = voice clips averaged).
    let _ = conn.execute("ALTER TABLE speaker_vault ADD COLUMN meeting_count INTEGER NOT NULL DEFAULT 0", []);
    // Recordings joined into this meeting (restarting the same call continues it).
    let _ = conn.execute("ALTER TABLE meetings ADD COLUMN parts INTEGER NOT NULL DEFAULT 1", []);

    Ok(())
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
fn record_vault_appearance(conn: &Connection, vault_id: &str, snippet: Option<&str>, candidates_json: Option<&str>) {
    let now = Utc::now().to_rfc3339();
    let existing: Option<(Option<String>, i64)> = conn
        .query_row(
            "SELECT embedding_json, sample_count FROM speaker_vault WHERE id = ?1",
            params![vault_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let Some((emb_json, samples)) = existing else { return };
    let new_emb = snippet.and_then(embedding_from_wav);
    let (emb_json, samples) = match (new_emb, emb_json.and_then(|j| serde_json::from_str::<Vec<f32>>(&j).ok())) {
        (Some(new), Some(cur)) => (Some(average_embedding(&cur, samples, &new)), samples + 1),
        (Some(new), None) => (Some(new), 1),
        (None, cur) => (cur, samples),
    };
    let _ = conn.execute(
        r#"
        UPDATE speaker_vault SET
            meeting_count = meeting_count + 1,
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
            candidates_json
        ],
    );
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
    let webm = crate::meeting_audio::compress_for_playback(wav)?;
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
    /// The meeting has an audio path but the file is gone (deleted or moved).
    /// The transcript is still complete.
    #[serde(default)]
    pub audio_missing: bool,
    /// Speaker clips that are gone; they are left out of `turns`.
    #[serde(default)]
    pub clips_missing: usize,
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
fn record_vault_appearances(tx: &Connection, turns: &[DiarizedTurn], already_counted: &std::collections::HashSet<String>) {
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
            record_vault_appearance(tx, &t.speaker_id, t.snippet_path.as_deref(), cand_json.as_deref());
        }
    }
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
    let old_turns = stored_turns(conn, meeting_id);
    let old_audio: Option<String> = conn
        .query_row("SELECT audio_path FROM meetings WHERE id = ?1", params![meeting_id], |r| r.get(0))
        .map_err(|e| format!("Meeting {meeting_id} not found: {e}"))?;
    let title: String = conn
        .query_row("SELECT title FROM meetings WHERE id = ?1", params![meeting_id], |r| r.get(0))
        .unwrap_or_default();

    let tx = conn.unchecked_transaction().map_err(|e| format!("Failed to begin meeting transaction: {}", e))?;
    let summary_output = extract_summary_heuristics(turns, &title);
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
    insert_turns(&tx, meeting_id, turns)?;
    let counted: std::collections::HashSet<String> = old_turns.iter().map(|t| t.0.clone()).collect();
    record_vault_appearances(&tx, turns, &counted);
    tx.commit().map_err(|e| format!("Failed to commit meeting: {}", e))?;

    // Files the old version used and neither the new one nor the vault uses.
    let mut keep: std::collections::HashSet<String> = turns
        .iter()
        .flat_map(|t| t.snippet_path.iter().chain(t.candidate_snippets.iter()).cloned())
        .collect();
    if let Ok(mut stmt) = conn.prepare("SELECT snippet_path, candidate_snippets FROM speaker_vault") {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))) {
            for (sel, cands) in rows.filter_map(Result::ok) {
                keep.extend(sel);
                keep.extend(cands.and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok()).unwrap_or_default());
            }
        }
    }
    for (_, _, _, _, _, snip, cands) in &old_turns {
        let mut paths: Vec<String> = cands.as_deref().and_then(|c| serde_json::from_str(c).ok()).unwrap_or_default();
        paths.extend(snip.clone());
        for p in paths.into_iter().filter(|p| !keep.contains(p)) {
            let _ = std::fs::remove_file(crate::diarization::voice_sample_path(&p));
            let _ = std::fs::remove_file(&p);
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
    let tx = conn.unchecked_transaction().map_err(|e| format!("Failed to begin meeting transaction: {}", e))?;
    // Random suffix: two meetings saved within the same millisecond must not collide.
    let session_id = format!(
        "call_{}_{}_{:06x}",
        Utc::now().timestamp_millis(),
        std::process::id(),
        rand::random::<u32>() & 0xff_ffff
    );
    let created_at = Utc::now().to_rfc3339();

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
            audio_path, transcript_raw, summary, action_items, category, speaker_count
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
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

    insert_turns(&tx, meeting_id, turns)?;
    record_vault_appearances(&tx, turns, &std::collections::HashSet::new());

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
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;
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
                query.push_str(" AND (title LIKE ? OR transcript_raw LIKE ?)");
                let pattern = format!("%{}%", s.trim());
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
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
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
                       candidate_snippets, current_snippet_idx
                FROM meeting_turns WHERE meeting_id = ?1 ORDER BY start_ms ASC
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

                Ok(DiarizedTurn {
                    speaker_id: row.get(0)?,
                    speaker_name: row.get(1)?,
                    start_ms: row.get::<_, i64>(2)? as u64,
                    end_ms: row.get::<_, i64>(3)? as u64,
                    channel: row.get::<_, i64>(4)? as u8,
                    text: row.get(5)?,
                    snippet_path: row.get(6)?,
                    candidate_snippets: cand_vec,
                    current_snippet_idx: cur_idx.max(0) as usize,
                })
            })
            .map_err(|e| format!("Failed to read turns: {}", e))?;

        let mut turns = Vec::new();
        let mut clips_missing = 0;
        for t in turn_rows {
            if let Ok(mut turn) = t {
                clips_missing += drop_missing_clips(&mut turn.snippet_path, &mut turn.candidate_snippets, Some(&mut turn.current_snippet_idx));
                turns.push(turn);
            }
        }
        let audio_missing = meeting.8.is_some() && !file_exists(&meeting.8);
        if audio_missing || clips_missing > 0 {
            println!("[MEETINGS] Meeting #{}: audio missing={audio_missing}, {clips_missing} speaker clip(s) missing", meeting.0);
        }

        Ok(MeetingDetailRecord {
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
            audio_missing,
            clips_missing,
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
            SET title = ?1, category = ?2, summary = ?3, action_items = ?4
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
        "UPDATE meetings SET title = ?1, category = ?2, summary = ?3, action_items = ?4 WHERE id = ?5",
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

        let mut vault_stmt = conn.prepare("SELECT snippet_path, candidate_snippets FROM speaker_vault")
            .map_err(|e| e.to_string())?;
        let vault_rows = vault_stmt.query_map([], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?))
        }).map_err(|e| e.to_string())?;
        let mut vault_clips = std::collections::HashSet::new();
        for row in vault_rows {
            let (selected, candidates) = row.map_err(|e| e.to_string())?;
            if let Some(path) = selected { vault_clips.insert(path); }
            if let Some(json) = candidates {
                vault_clips.extend(serde_json::from_str::<Vec<String>>(&json).unwrap_or_default());
            }
        }
        drop(vault_stmt);

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM meeting_turns WHERE meeting_id = ?1", params![meeting_id])
            .map_err(|e| format!("Failed to delete turns: {}", e))?;
        tx.execute("DELETE FROM meetings WHERE id = ?1", params![meeting_id])
            .map_err(|e| format!("Failed to delete meeting: {}", e))?;
        tx.commit().map_err(|e| format!("Failed to commit meeting deletion: {}", e))?;

        if let Some(path) = audio_path { let _ = std::fs::remove_file(path); }
        crate::meeting_continuation::forget_meeting(meeting_id);
        for path in meeting_clips.difference(&vault_clips) {
            let _ = std::fs::remove_file(crate::diarization::voice_sample_path(path));
            let _ = std::fs::remove_file(path);
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
    let changed = tx.execute("UPDATE speaker_vault SET name = ?1 WHERE id = ?2", params![name, speaker_id])
        .map_err(|e| e.to_string())?;
    if changed == 0 { return Err("Speaker not found in vault".into()); }
    tx.execute("UPDATE meeting_turns SET speaker_name = ?1 WHERE speaker_id = ?2", params![name, speaker_id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
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
    conn.execute(
        "UPDATE meeting_turns SET speaker_name = ?1 WHERE meeting_id = ?2 AND speaker_id = ?3",
        params![new_name, meeting_id, speaker_id],
    )
    .map_err(|e| format!("Failed to rename speaker: {}", e))?;

    if !update_vault {
        return Ok(());
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
    let is_person: bool = conn
        .query_row("SELECT 1 FROM speaker_vault WHERE id = ?1", params![speaker_id], |_| Ok(true))
        .unwrap_or(false);
    if is_person {
        conn.execute(
            "UPDATE speaker_vault SET name = ?1, last_seen = ?2 WHERE id = ?3",
            params![new_name, now, speaker_id],
        )
        .map_err(|e| format!("Failed to rename vault speaker: {}", e))?;
        return Ok(());
    }

    // A per-meeting caller: link to an existing person with that name, or enroll.
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM speaker_vault WHERE lower(name) = lower(?1) LIMIT 1",
            params![new_name],
            |row| row.get(0),
        )
        .ok();
    let person_id = match existing {
        Some(id) => {
            record_vault_appearance(conn, &id, snippet.as_deref(), candidates.as_deref());
            id
        }
        None => {
            let id = new_vault_id();
            let emb_json = snippet
                .as_deref()
                .and_then(embedding_from_wav)
                .map(|e| serde_json::to_string(&e).unwrap_or_default());
            conn.execute(
                r#"
                INSERT INTO speaker_vault
                    (id, name, created_at, sample_count, meeting_count, last_seen, snippet_path, candidate_snippets, embedding_json)
                VALUES (?1, ?2, ?3, ?4, 1, ?3, ?5, ?6, ?7)
                "#,
                params![id, new_name, now, if emb_json.is_some() { 1 } else { 0 }, snippet, candidates, emb_json],
            )
            .map_err(|e| format!("Failed to enroll speaker: {}", e))?;
            id
        }
    };
    conn.execute(
        "UPDATE meeting_turns SET speaker_id = ?1 WHERE meeting_id = ?2 AND speaker_id = ?3",
        params![person_id, meeting_id, speaker_id],
    )
    .map_err(|e| format!("Failed to link speaker to vault: {}", e))?;
    Ok(())
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

        if candidates.is_empty() {
            return Err("No candidate snippets available".to_string());
        }

        let next_idx = (cur_idx as usize + 1) % candidates.len();
        let next_path = candidates[next_idx].clone();

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

#[tauri::command]
pub async fn cycle_vault_speaker_snippet(
    speaker_id: String,
) -> Result<(String, usize), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_connection()?;

        let mut stmt = conn
            .prepare(
                r#"
                SELECT candidate_snippets, snippet_path
                FROM speaker_vault
                WHERE id = ?1
                "#,
            )
            .map_err(|e| e.to_string())?;

        let (cand_str, cur_snippet): (Option<String>, Option<String>) = stmt
            .query_row(params![speaker_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("Speaker not found in vault: {}", e))?;

        let candidates: Vec<String> = cand_str
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        if candidates.is_empty() {
            return Err("No candidate snippets available for this vault speaker".to_string());
        }

        let cur_idx = cur_snippet
            .as_ref()
            .and_then(|path| candidates.iter().position(|c| c == path))
            .unwrap_or(0);

        let next_idx = (cur_idx + 1) % candidates.len();
        let next_path = candidates[next_idx].clone();

        // Display/playback clip only; the voiceprint stays the averaged embedding.
        conn.execute(
            "UPDATE speaker_vault SET snippet_path = ?1 WHERE id = ?2",
            params![next_path, speaker_id],
        )
        .map_err(|e| format!("Failed to update vault snippet: {}", e))?;

        Ok((next_path, next_idx))
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
pub async fn generate_meeting_summary(
    meeting_id: i64,
    state: State<'_, AudioState>,
) -> Result<MeetingSummaryOutput, String> {
    let detail = get_meeting_detail(meeting_id).await?;

    // Try local LLM if loaded
    let llm_handle = state.llm.clone();
    let has_llm = {
        let guard = llm_handle.lock().unwrap();
        guard.is_some()
    };

    if has_llm {
        // Construct prompt with diarized conversation turns
        let mut conversation = String::new();
        for t in &detail.turns {
            conversation.push_str(&format!("{}: {}\n", t.speaker_name, t.text));
        }

        let system_prompt = "You are an executive meeting assistant. Output valid JSON only with keys: title (string), category (string), summary (array of bullet strings), action_items (array of objects with task, assignee, status).";
        let user_prompt = format!("Transcript:\n{}\n\nGenerate JSON meeting summary:", conversation);
        let combined = format!("System: {}\nUser: {}\nAssistant: ", system_prompt, user_prompt);

        let llm_res: Option<String> = {
            let mut guard = llm_handle.lock().unwrap();
            if let Some(engine) = guard.as_mut() {
                engine.run_with_options(&combined, 512, 0.3).ok()
            } else {
                None
            }
        };

        if let Some(text) = llm_res {
            let fallback = extract_summary_heuristics(&detail.turns, &detail.title);
            let parsed = crate::meeting_summary::parse_llm_summary_json(&text, fallback);
            let _ = update_meeting(
                meeting_id,
                parsed.title.clone(),
                parsed.category.clone(),
                parsed.summary.clone(),
                parsed.action_items.clone(),
            )
            .await;
            return Ok(parsed);
        }
    }

    // Fallback heuristic output
    let output = extract_summary_heuristics(&detail.turns, &detail.title);
    let _ = update_meeting(
        meeting_id,
        output.title.clone(),
        output.category.clone(),
        output.summary.clone(),
        output.action_items.clone(),
    )
    .await;

    Ok(output)
}

#[tauri::command]
pub async fn export_meeting_notes(meeting_id: i64, format: String) -> Result<String, String> {
    let detail = get_meeting_detail(meeting_id).await?;

    match format.to_lowercase().as_str() {
        "json" => serde_json::to_string_pretty(&detail).map_err(|e| e.to_string()),
        "txt" | "text" => {
            let mut out = format!("TITLE: {}\nCATEGORY: {}\nDATE: {}\n\n", detail.title, detail.category, detail.created_at);
            out.push_str("SUMMARY:\n");
            for s in &detail.summary {
                out.push_str(&format!("• {}\n", s));
            }
            out.push_str("\nACTION ITEMS:\n");
            for a in &detail.action_items {
                out.push_str(&format!("[{}] {} (@{})\n", if a.status == "done" { "X" } else { " " }, a.task, a.assignee));
            }
            out.push_str("\nTRANSCRIPT:\n");
            for t in &detail.turns {
                out.push_str(&format!("[{:02}:{:02}] {}: {}\n", t.start_ms / 60000, (t.start_ms % 60000) / 1000, t.speaker_name, t.text));
            }
            Ok(out)
        }
        _ => {
            // Markdown default
            let mut md = format!("# {}\n\n**Category**: `{}` • **Date**: {}\n\n", detail.title, detail.category, detail.created_at);
            md.push_str("## 📋 Executive Summary\n\n");
            for s in &detail.summary {
                md.push_str(&format!("- {}\n", s));
            }
            md.push_str("\n## ✅ Action Items\n\n");
            for a in &detail.action_items {
                md.push_str(&format!("- [{}] **{}** — *Assignee: @{}*\n", if a.status == "done" { "x" } else { " " }, a.task, a.assignee));
            }
            md.push_str("\n## 🎙️ Diarized Transcript\n\n");
            for t in &detail.turns {
                md.push_str(&format!("**{:02}:{:02} {}**: {}\n\n", t.start_ms / 60000, (t.start_ms % 60000) / 1000, t.speaker_name, t.text));
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
    Ok(state.meeting_detector.scan())
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
    state: State<'_, AudioState>,
) -> Result<(), String> {
    state.auto_record_meetings.store(enabled, Ordering::SeqCst);
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
        let _ = std::fs::remove_dir_all(dir);
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
