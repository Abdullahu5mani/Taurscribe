//! Voice bank: several voiceprints per Speaker Vault person instead of one
//! blurred average, and the user's yes / no answers about who is who.
//!
//! * Every voice sample the user confirms (a meeting speaker named as the
//!   person, a "Find more samples" suggestion added, an "Is this Dana?" yes)
//!   is kept as its own voiceprint in `speaker_voiceprints`.
//! * A person matches a new speaker by their CLOSEST voiceprint, so a laptop
//!   mic, a phone and a room each get their own reference.
//! * "Not them" answers are kept in `speaker_rejections`: that speaker is never
//!   suggested for the person again, and a later speaker who sounds more like a
//!   rejected sample than like the person is not labelled as them.
//!
//! Candidates come from the voice files the diarizer writes for each meeting
//! speaker (their speech back to back, from the full-quality recording), so a
//! suggestion is one speaker in one past meeting.

use super::meetings::{average_embedding, open_connection, speaker_model_installed};
use crate::speaker_embedding::{cosine_similarity, VaultSpeakerEmbedding};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

/// Below this a "Find more samples" candidate is not worth listening to.
pub const SUGGEST_FLOOR: f32 = 0.40;
/// Below this "Is this Dana?" is not asked after a meeting.
pub const ASK_FLOOR: f32 = 0.45;

pub(crate) fn ensure_voice_bank_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS speaker_voiceprints (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            person_id       TEXT NOT NULL,
            embedding_json  TEXT NOT NULL,
            clip_path       TEXT,
            meeting_id      INTEGER,
            created_at      TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_voiceprints_person_clip
            ON speaker_voiceprints(person_id, clip_path);
        CREATE INDEX IF NOT EXISTS idx_voiceprints_person
            ON speaker_voiceprints(person_id);

        CREATE TABLE IF NOT EXISTS speaker_rejections (
            person_id       TEXT NOT NULL,
            meeting_id      INTEGER NOT NULL,
            speaker_id      TEXT NOT NULL,
            embedding_json  TEXT,
            created_at      TEXT NOT NULL,
            PRIMARY KEY (person_id, meeting_id, speaker_id)
        );

        -- Voiceprints of meeting speakers' voice files, computed once.
        CREATE TABLE IF NOT EXISTS voice_file_prints (
            path            TEXT PRIMARY KEY,
            embedding_json  TEXT NOT NULL
        );

        -- People enrolled before the voice bank: their averaged voiceprint
        -- becomes their first sample.
        INSERT OR IGNORE INTO speaker_voiceprints (person_id, embedding_json, clip_path, created_at)
            SELECT v.id, v.embedding_json, v.snippet_path, v.created_at FROM speaker_vault v
            WHERE v.embedding_json IS NOT NULL AND v.embedding_json != ''
              AND NOT EXISTS (SELECT 1 FROM speaker_voiceprints p WHERE p.person_id = v.id);
        "#,
    )
    .map_err(|e| format!("Failed to initialize the voice bank: {e}"))
}

fn parse(json: &str) -> Option<Vec<f32>> {
    serde_json::from_str::<Vec<f32>>(json).ok().filter(|e| !e.is_empty())
}

/// Keeps one more voiceprint for a person (a clip already kept is ignored).
pub(crate) fn add_voiceprint(conn: &Connection, person_id: &str, embedding: &[f32], clip: Option<&str>, meeting_id: Option<i64>) {
    let Ok(json) = serde_json::to_string(embedding) else { return };
    let _ = conn.execute(
        "INSERT OR IGNORE INTO speaker_voiceprints (person_id, embedding_json, clip_path, meeting_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![person_id, json, clip, meeting_id, Utc::now().to_rfc3339()],
    );
}

/// Every voiceprint of every person (one entry per sample), for matching.
pub(crate) fn load_voiceprints(conn: &Connection) -> Vec<VaultSpeakerEmbedding> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT p.person_id, v.name, p.embedding_json, p.clip_path FROM speaker_voiceprints p JOIN speaker_vault v ON v.id = p.person_id",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?))
    });
    let Ok(rows) = rows else { return Vec::new() };
    rows.flatten()
        .filter_map(|(id, name, json, clip)| {
            parse(&json).map(|embedding| VaultSpeakerEmbedding { speaker_id: id, display_name: name, embedding, snippet_path: clip })
        })
        .collect()
}

/// Rejected samples per person: (person id, voiceprint).
pub(crate) fn load_rejections(conn: &Connection) -> Vec<(String, Vec<f32>)> {
    let Ok(mut stmt) = conn.prepare("SELECT person_id, embedding_json FROM speaker_rejections WHERE embedding_json IS NOT NULL") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))) else {
        return Vec::new();
    };
    rows.flatten().filter_map(|(p, json)| parse(&json).map(|e| (p, e))).collect()
}

/// The voiceprint of a meeting speaker's voice file, computed once and cached.
fn voice_file_print(conn: &Connection, voice_file: &Path) -> Option<Vec<f32>> {
    let key = voice_file.to_string_lossy().to_string();
    if let Some(json) = conn
        .query_row("SELECT embedding_json FROM voice_file_prints WHERE path = ?1", params![key], |r| r.get::<_, String>(0))
        .optional()
        .ok()
        .flatten()
    {
        return parse(&json);
    }
    if !voice_file.exists() {
        return None;
    }
    let emb = crate::speaker_embedding::embedding_from_wav(&key)?;
    if let Ok(json) = serde_json::to_string(&emb) {
        let _ = conn.execute("INSERT OR REPLACE INTO voice_file_prints (path, embedding_json) VALUES (?1, ?2)", params![key, json]);
    }
    Some(emb)
}

/// One meeting speaker not linked to anyone in the vault.
struct MeetingSpeaker {
    meeting_id: i64,
    meeting_title: String,
    created_at: String,
    speaker_id: String,
    speaker_name: String,
    clip: Option<String>,
    speech_ms: i64,
    preview: String,
}

/// Unnamed speakers (not you, not a vault person) in `meeting` or in every meeting.
fn unnamed_speakers(conn: &Connection, meeting: Option<i64>) -> Result<Vec<MeetingSpeaker>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT t.meeting_id, m.title, m.created_at, t.speaker_id, t.speaker_name,
                   MAX(t.snippet_path), SUM(t.end_ms - t.start_ms),
                   (SELECT text FROM meeting_turns x WHERE x.meeting_id = t.meeting_id AND x.speaker_id = t.speaker_id
                      ORDER BY (x.end_ms - x.start_ms) DESC LIMIT 1)
            FROM meeting_turns t JOIN meetings m ON m.id = t.meeting_id
            WHERE t.channel = 1 AND t.speaker_id != 'speaker_you'
              AND t.speaker_id NOT IN (SELECT id FROM speaker_vault)
              AND (?1 IS NULL OR t.meeting_id = ?1)
            GROUP BY t.meeting_id, t.speaker_id
            ORDER BY m.created_at DESC
            "#,
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![meeting], |r| {
            Ok(MeetingSpeaker {
                meeting_id: r.get(0)?,
                meeting_title: r.get(1)?,
                created_at: r.get(2)?,
                speaker_id: r.get(3)?,
                speaker_name: r.get(4)?,
                clip: r.get(5)?,
                speech_ms: r.get(6)?,
                preview: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

fn rejected(conn: &Connection, person_id: &str, meeting_id: i64, speaker_id: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM speaker_rejections WHERE person_id = ?1 AND meeting_id = ?2 AND speaker_id = ?3",
        params![person_id, meeting_id, speaker_id],
        |_| Ok(()),
    )
    .is_ok()
}

fn require_speaker_model() -> Result<(), String> {
    if speaker_model_installed() {
        Ok(())
    } else {
        Err("The voice bank needs the Meeting speakers download (Settings → Models).".into())
    }
}

/// A past-meeting speaker who may be the person: listen, then add or reject.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VoiceSampleSuggestion {
    pub meeting_id: i64,
    pub meeting_title: String,
    pub created_at: String,
    pub speaker_id: String,
    pub speaker_name: String,
    /// A 3 s clip to play, when one exists.
    pub clip_path: Option<String>,
    pub speech_ms: i64,
    /// Their longest line in that meeting.
    pub preview: String,
    pub similarity: f32,
}

/// Links a meeting speaker to a vault person: their turns get the person's
/// name, and their voice becomes one more voiceprint of the person.
pub(crate) fn accept_voice_sample_in(conn: &Connection, person_id: &str, meeting_id: i64, speaker_id: &str) -> Result<(), String> {
    let name: String = conn
        .query_row("SELECT name FROM speaker_vault WHERE id = ?1", params![person_id], |r| r.get(0))
        .map_err(|_| "That person is no longer in the Speaker Vault.".to_string())?;
    let clip: Option<String> = conn
        .query_row(
            "SELECT MAX(snippet_path) FROM meeting_turns WHERE meeting_id = ?1 AND speaker_id = ?2",
            params![meeting_id, speaker_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let changed = conn
        .execute(
            "UPDATE meeting_turns SET speaker_id = ?1, speaker_name = ?2 WHERE meeting_id = ?3 AND speaker_id = ?4",
            params![person_id, name, meeting_id, speaker_id],
        )
        .map_err(|e| format!("Couldn't label the speaker: {e}"))?;
    if changed == 0 {
        return Err("That speaker is no longer in the meeting.".into());
    }
    let _ = conn.execute(
        "DELETE FROM speaker_rejections WHERE person_id = ?1 AND meeting_id = ?2 AND speaker_id = ?3",
        params![person_id, meeting_id, speaker_id],
    );
    if let Some(emb) = clip.as_deref().and_then(|c| voice_file_print(conn, &crate::diarization::voice_sample_path(c))) {
        add_voiceprint(conn, person_id, &emb, clip.as_deref(), Some(meeting_id));
        refresh_average(conn, person_id)?;
    }
    let _ = conn.execute(
        "UPDATE speaker_vault SET meeting_count = meeting_count + 1, last_seen = ?2 WHERE id = ?1",
        params![person_id, Utc::now().to_rfc3339()],
    );
    Ok(())
}

pub(crate) fn reject_voice_sample_in(conn: &Connection, person_id: &str, meeting_id: i64, speaker_id: &str) -> Result<(), String> {
    let clip: Option<String> = conn
        .query_row(
            "SELECT MAX(snippet_path) FROM meeting_turns WHERE meeting_id = ?1 AND speaker_id = ?2",
            params![meeting_id, speaker_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let emb_json = clip
        .as_deref()
        .and_then(|c| voice_file_print(conn, &crate::diarization::voice_sample_path(c)))
        .and_then(|e| serde_json::to_string(&e).ok());
    conn.execute(
        "INSERT OR REPLACE INTO speaker_rejections (person_id, meeting_id, speaker_id, embedding_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![person_id, meeting_id, speaker_id, emb_json, Utc::now().to_rfc3339()],
    )
    .map_err(|e| format!("Couldn't save the answer: {e}"))?;
    Ok(())
}

/// Keeps `speaker_vault.embedding_json` (the person's overall voiceprint) and
/// `sample_count` in step with their samples.
fn refresh_average(conn: &Connection, person_id: &str) -> Result<(), String> {
    let prints: Vec<Vec<f32>> = load_voiceprints(conn).into_iter().filter(|p| p.speaker_id == person_id).map(|p| p.embedding).collect();
    let mut mean: Vec<f32> = Vec::new();
    for (n, e) in prints.iter().enumerate() {
        mean = average_embedding(&mean, n as i64, e);
    }
    let json = (!mean.is_empty()).then(|| serde_json::to_string(&mean).unwrap_or_default());
    conn.execute(
        "UPDATE speaker_vault SET embedding_json = COALESCE(?2, embedding_json), sample_count = ?3 WHERE id = ?1",
        params![person_id, json, prints.len() as i64],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// One stored voice sample of a person.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Voiceprint {
    pub id: i64,
    pub clip_path: Option<String>,
    pub meeting_id: Option<i64>,
    pub meeting_title: Option<String>,
    pub created_at: String,
}

pub(crate) fn list_voiceprints_in(conn: &Connection, person_id: &str) -> Result<Vec<Voiceprint>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT p.id, p.clip_path, p.meeting_id, m.title, p.created_at
            FROM speaker_voiceprints p LEFT JOIN meetings m ON m.id = p.meeting_id
            WHERE p.person_id = ?1 ORDER BY p.created_at DESC, p.id DESC
            "#,
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![person_id], |r| {
            let clip: Option<String> = r.get(1)?;
            Ok(Voiceprint {
                id: r.get(0)?,
                clip_path: clip.filter(|c| Path::new(c).exists()),
                meeting_id: r.get(2)?,
                meeting_title: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

pub(crate) fn delete_voiceprint_in(conn: &Connection, person_id: &str, voiceprint_id: i64) -> Result<(), String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM speaker_voiceprints WHERE person_id = ?1", params![person_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if count <= 1 {
        return Err("This is their only voice sample. Delete the person instead.".into());
    }
    let removed = conn
        .execute("DELETE FROM speaker_voiceprints WHERE id = ?1 AND person_id = ?2", params![voiceprint_id, person_id])
        .map_err(|e| e.to_string())?;
    if removed == 0 {
        return Err("That voice sample is already gone.".into());
    }
    refresh_average(conn, person_id)
}

/// Removes everything the voice bank holds about a person.
pub(crate) fn forget_person_in(conn: &Connection, person_id: &str) {
    let _ = conn.execute("DELETE FROM speaker_voiceprints WHERE person_id = ?1", params![person_id]);
    let _ = conn.execute("DELETE FROM speaker_rejections WHERE person_id = ?1", params![person_id]);
}

/// "Is this Dana?" after a meeting: an unnamed speaker whose voice is close to
/// a vault person, but not close enough to be labelled automatically.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SpeakerNameSuggestion {
    pub speaker_id: String,
    pub speaker_name: String,
    pub person_id: String,
    pub person_name: String,
    pub similarity: f32,
    pub clip_path: Option<String>,
}

