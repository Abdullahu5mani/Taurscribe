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

/// How many of a person's closest samples are averaged into their match
/// score. With few samples the closest one decides (a laptop and a phone
/// sample are both "them", so they must not be averaged together); with more,
/// one odd sample (a wrong add, crosstalk) can't produce a match on its own.
pub fn samples_averaged(sample_count: usize) -> usize {
    match sample_count {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    }
}

/// Mean of the `samples_averaged` highest similarities.
fn person_score(mut sims: Vec<f32>) -> f32 {
    sims.sort_by(|a, b| b.total_cmp(a));
    let k = samples_averaged(sims.len()).min(sims.len()).max(1);
    sims.iter().take(k).sum::<f32>() / k as f32
}

/// How alike `embedding` is to each person: the average similarity of their
/// closest samples (see [`samples_averaged`]). People whose rejected samples
/// are at least as close are left out. Best first.
pub fn rank_people(embedding: &[f32], prints: &[VaultSpeakerEmbedding], rejected: &[(String, Vec<f32>)]) -> Vec<(String, String, f32)> {
    let mut people: Vec<(String, String, Vec<f32>)> = Vec::new();
    for p in prints {
        let sim = cosine_similarity(embedding, &p.embedding);
        match people.iter_mut().find(|(id, _, _)| *id == p.speaker_id) {
            Some(entry) => entry.2.push(sim),
            None => people.push((p.speaker_id.clone(), p.display_name.clone(), vec![sim])),
        }
    }
    let mut best: Vec<(String, String, f32)> = people.into_iter().map(|(id, name, sims)| (id, name, person_score(sims))).collect();
    best.retain(|(id, _, sim)| {
        !rejected.iter().any(|(person, rej)| person == id && cosine_similarity(embedding, rej) >= *sim)
    });
    best.sort_by(|a, b| b.2.total_cmp(&a.2));
    best
}

/// The person `embedding` belongs to, when their closest voiceprint reaches `threshold`.
pub fn best_person(embedding: &[f32], prints: &[VaultSpeakerEmbedding], rejected: &[(String, Vec<f32>)], threshold: f32) -> Option<(String, String, f32)> {
    rank_people(embedding, prints, rejected).into_iter().next().filter(|(_, _, sim)| *sim >= threshold)
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
            WHERE t.channel = 1 AND t.speaker_id NOT IN ('speaker_you', 'background')
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

pub(crate) fn find_voice_samples_in(conn: &Connection, person_id: &str, limit: usize) -> Result<Vec<VoiceSampleSuggestion>, String> {
    let prints: Vec<VaultSpeakerEmbedding> = load_voiceprints(conn).into_iter().filter(|p| p.speaker_id == person_id).collect();
    if prints.is_empty() {
        return Err("This person has no voice sample yet.".into());
    }
    let own_rejections: Vec<(String, Vec<f32>)> = load_rejections(conn).into_iter().filter(|(p, _)| p == person_id).collect();
    let mut out = Vec::new();
    for s in unnamed_speakers(conn, None)? {
        if rejected(conn, person_id, s.meeting_id, &s.speaker_id) {
            continue;
        }
        let Some(clip) = s.clip.as_deref() else { continue };
        let Some(emb) = voice_file_print(conn, &crate::diarization::voice_sample_path(clip)) else { continue };
        let Some((_, _, similarity)) = rank_people(&emb, &prints, &own_rejections).into_iter().next() else { continue };
        if similarity < SUGGEST_FLOOR {
            continue;
        }
        out.push(VoiceSampleSuggestion {
            meeting_id: s.meeting_id,
            meeting_title: s.meeting_title,
            created_at: s.created_at,
            speaker_id: s.speaker_id,
            speaker_name: s.speaker_name,
            clip_path: Path::new(clip).exists().then(|| clip.to_string()),
            speech_ms: s.speech_ms,
            preview: s.preview.chars().take(160).collect(),
            similarity,
        });
    }
    out.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
    out.truncate(limit);
    Ok(out)
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

pub(crate) fn suggest_speaker_names_in(conn: &Connection, meeting_id: i64) -> Result<Vec<SpeakerNameSuggestion>, String> {
    let prints = load_voiceprints(conn);
    if prints.is_empty() {
        return Ok(Vec::new());
    }
    let rejections = load_rejections(conn);
    let mut out: Vec<SpeakerNameSuggestion> = Vec::new();
    for s in unnamed_speakers(conn, Some(meeting_id))? {
        let Some(clip) = s.clip.as_deref() else { continue };
        let Some(emb) = voice_file_print(conn, &crate::diarization::voice_sample_path(clip)) else { continue };
        let candidate = rank_people(&emb, &prints, &rejections)
            .into_iter()
            .find(|(person, _, _)| !rejected(conn, person, meeting_id, &s.speaker_id));
        if let Some((person_id, person_name, similarity)) = candidate {
            if similarity >= ASK_FLOOR {
                out.push(SpeakerNameSuggestion {
                    speaker_id: s.speaker_id,
                    speaker_name: s.speaker_name,
                    person_id,
                    person_name,
                    similarity,
                    clip_path: Path::new(clip).exists().then(|| clip.to_string()),
                });
            }
        }
    }
    // One question per person: the speaker who sounds most like them.
    out.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
    let mut asked = std::collections::HashSet::new();
    out.retain(|s| asked.insert(s.person_id.clone()));
    Ok(out)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce(&Connection) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(&open_connection()?))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

fn sample_operations() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    &LOCK
}

/// Produce a genuinely new clip from a confirmed person's retained meeting
/// speech. It is only enrolled after the user listens and confirms the identity.
#[tauri::command]
pub async fn generate_vault_voice_sample(person_id: String, excluded_paths: Option<Vec<String>>) -> Result<String, String> {
    blocking(move |conn| {
        let _samples = sample_operations().lock().map_err(|e| e.to_string())?;
        generate_vault_sample_excluding(conn, &person_id, &super::meetings::get_meetings_dir()?.join("voicebank"), &excluded_paths.unwrap_or_default())

    }).await
}

#[cfg(test)]
fn generate_vault_sample_in(conn: &Connection, person_id: &str, dir: &Path) -> Result<String, String> {
    generate_vault_sample_excluding(conn, person_id, dir, &[])
}
fn generate_vault_sample_excluding(conn: &Connection, person_id: &str, dir: &Path, excluded: &[String]) -> Result<String, String> {
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM speaker_vault WHERE id=?1)", params![person_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !exists { return Err("This person is no longer in the vault".into()); }
        let paths: Vec<String> = {
            let mut stmt = conn.prepare("SELECT DISTINCT snippet_path FROM meeting_turns WHERE speaker_id=?1 AND snippet_path IS NOT NULL ORDER BY meeting_id DESC").map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![person_id], |r| r.get(0)).map_err(|e| e.to_string())?;
            rows.collect::<Result<_,_>>().map_err(|e| e.to_string())?
        };
        generate_sample_from_paths(conn, person_id, paths, dir, excluded)
}

fn generate_sample_from_paths(conn: &Connection, person_id: &str, paths: Vec<String>, dir: &Path, excluded: &[String]) -> Result<String, String> {
        if excluded.len() > 32 { return Err("Close some previews before generating more".into()); }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for path in paths {
            let voice = crate::diarization::voice_sample_path(&path);
            if !voice.exists() { continue; }
            let (samples, rate) = match crate::audio_decode::decode_audio_mono_f32(&voice) { Ok(audio) => audio, Err(_) => continue };
            let pcm = crate::audio_preprocess::resample_mono_to_16k(&samples, rate)?;
            if let Some(clip) = sample_from_pcm(conn, person_id, &pcm, dir, excluded)? { return Ok(clip); }
        }
        Err("No new retained speech is available for this person. Use Find more to confirm them in another meeting, or add a recording of just their voice.".into())
}

fn sample_from_pcm(conn: &Connection, person_id: &str, pcm: &[f32], dir: &Path, excluded: &[String]) -> Result<Option<String>, String> {
    if excluded.len() > 32 { return Err("Close some previews before generating more".into()); }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            for chunk in pcm.chunks(16000 * 15) {
                if chunk.len() < 16000 * 5 { continue; }
                let mut hasher = sha2::Sha256::new();
                use sha2::Digest;
                hasher.update(person_id.as_bytes());
                for sample in chunk { hasher.update(sample.to_le_bytes()); }
                let name = format!("sample-{:x}.wav", hasher.finalize());
                let clip = dir.join(name).to_string_lossy().to_string();
                let enrolled: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM speaker_voiceprints WHERE person_id=?1 AND clip_path=?2)", params![person_id, clip], |r| r.get(0)).map_err(|e| e.to_string())?;
                if enrolled || excluded.contains(&clip) { continue; }
                let spec = hound::WavSpec { channels: 1, sample_rate: 16000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
                let mut writer = hound::WavWriter::create(&clip, spec).map_err(|e| e.to_string())?;
                for &sample in chunk { writer.write_sample(sample).map_err(|e| e.to_string())?; }
                writer.finalize().map_err(|e| e.to_string())?;
                return Ok(Some(clip));
            }
    Ok(None)
}

pub(super) fn generate_meeting_sample_in(conn: &Connection, meeting_id: i64, speaker_id: &str, dir: &Path, excluded: &[String]) -> Result<String, String> {
    let paths: Vec<String> = {
        let mut stmt = conn.prepare("SELECT DISTINCT snippet_path FROM meeting_turns WHERE meeting_id=?1 AND speaker_id=?2 AND snippet_path IS NOT NULL ORDER BY start_ms").map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![meeting_id,speaker_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<_,_>>().map_err(|e| e.to_string())?
    };
    let identity = format!("meeting-{meeting_id}-{speaker_id}");
    if paths.iter().any(|p| crate::diarization::voice_sample_path(p).exists()) {
        return generate_sample_from_paths(conn, &identity, paths, dir, excluded);
    }
    // A manual correction invalidates old group clips. Reconstruct only this
    // person's corrected intervals from the retained recording on demand.
    let audio: Option<String> = conn.query_row("SELECT audio_path FROM meetings WHERE id=?1", [meeting_id], |r| r.get(0)).map_err(|e| e.to_string())?;
    let audio = audio.ok_or("No retained recording is available for this meeting")?;
    let mut stmt = conn.prepare("SELECT start_ms,end_ms FROM meeting_turns WHERE meeting_id=?1 AND speaker_id=?2 ORDER BY start_ms,end_ms").map_err(|e| e.to_string())?;
    let ranges = stmt.query_map(params![meeting_id,speaker_id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    if ranges.is_empty() { return Err("This speaker is no longer in the meeting".into()); }
    let (samples, rate, channels) = crate::audio_decode::decode_audio_interleaved_strict(Path::new(&audio))?;
    let mono: Vec<f32> = samples.chunks_exact(channels as usize).map(|frame| frame.iter().sum::<f32>() / channels as f32).collect();
    let mut speech = Vec::new();
    let mut previous_end = 0;
    for (start, end) in ranges {
        let start = ((start.max(0) as u128 * rate as u128 / 1000).min(mono.len() as u128) as usize).max(previous_end);
        let end = (end.max(0) as u128 * rate as u128 / 1000).min(mono.len() as u128) as usize;
        if end > start { speech.extend_from_slice(&mono[start..end]); previous_end = end; }
    }
    let pcm = crate::audio_preprocess::resample_mono_to_16k(&speech, rate)?;
    sample_from_pcm(conn, &identity, &pcm, dir, excluded)?.ok_or_else(|| "No new retained speech is available for this speaker".into())
}

/// A listening preview from another retained section of this meeting speaker.
/// This does not add a person or voiceprint to the vault.
#[tauri::command]
pub async fn generate_meeting_speaker_preview(meeting_id: i64, speaker_id: String, excluded_paths: Option<Vec<String>>) -> Result<String, String> {
    blocking(move |conn| {
        let _samples = sample_operations().lock().map_err(|e| e.to_string())?;
        generate_meeting_sample_in(conn, meeting_id, &speaker_id, &super::meetings::get_meetings_dir()?.join("voicebank"), &excluded_paths.unwrap_or_default())
    }).await
}

/// Delete only an unconfirmed generated preview, never an enrolled voiceprint
/// or a file chosen by the user outside the app's derived-clip folder.
#[tauri::command]
pub async fn discard_generated_voice_sample(path: String) -> Result<(), String> {
    blocking(move |conn| {
        let _samples = sample_operations().lock().map_err(|e| e.to_string())?;
        let dir = super::meetings::get_meetings_dir()?.join("voicebank");
        let file = Path::new(&path);
        if file.parent() != Some(dir.as_path()) || !file.file_name().is_some_and(|n| n.to_string_lossy().starts_with("sample-")) { return Err("This is not a generated sample preview".into()); }
        let enrolled: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM speaker_voiceprints WHERE clip_path=?1)",params![path],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !enrolled && file.exists() { std::fs::remove_file(file).map_err(|e|e.to_string())?; }
        Ok(())
    }).await
}

/// Explicit confirmation enrolls the audio itself, not merely its preview path.
#[tauri::command]
pub async fn add_vault_voice_sample(person_id: String, path: String) -> Result<(), String> {
    blocking(move |conn| {
        let _samples = sample_operations().lock().map_err(|e| e.to_string())?;
        if !speaker_model_installed() { return Err("Download Meeting speakers in Settings → Models first".into()); }
        add_vault_sample_in(conn, &person_id, &path, &super::meetings::get_meetings_dir()?.join("voicebank"), &mut |pcm| {
            let mut engine = crate::speaker_embedding::get_speaker_engine().lock().map_err(|e| e.to_string())?;
            if !engine.uses_neural_model() { return Err("Speaker recognition model is unavailable".into()); }
            engine.compute_embedding(pcm, 16000)
        })

    }).await
}

fn add_vault_sample_in(conn: &Connection, person_id: &str, path: &str, dir: &Path, compute: &mut dyn FnMut(&[f32]) -> Result<Vec<f32>, String>) -> Result<(), String> {
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM speaker_vault WHERE id=?1)", params![person_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !exists { return Err("This person is no longer in the vault".into()); }
        // Streaming bounds prevent an accidental full meeting upload allocating
        // an unbounded buffer. A voice reference needs one isolated short clip.
        let mut samples = Vec::new();
        let (_, rate) = crate::audio_decode::decode_audio_mono_stream_strict(Path::new(&path), |part, rate, _, _| {
            if samples.len() + part.len() > rate as usize * 60 { return Err("Choose a clip of 5–60 seconds containing only this person".into()); }
            samples.extend_from_slice(part); Ok(())
        })?;
        let speech = crate::diarization::detect_channel_speech_segments(&samples, rate, 0, 0.015);
        let samples: Vec<f32> = speech.iter().flat_map(|s| samples[s.start_sample.min(samples.len())..s.end_sample.min(samples.len())].iter().copied()).collect();
        if samples.len() < rate as usize * 5 { return Err("At least 5 seconds of clear speech from this person are needed".into()); }
        let pcm = crate::audio_preprocess::resample_mono_to_16k(&samples, rate)?;
        let emb = compute(&pcm)?;
        if emb.is_empty() || emb.iter().any(|v| !v.is_finite()) { return Err("Could not extract a usable voiceprint".into()); }
        use sha2::Digest;
        let mut hash = sha2::Sha256::new(); hash.update(person_id.as_bytes());
        for sample in &pcm { hash.update(sample.to_le_bytes()); }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let is_generated = Path::new(&path).parent() == Some(dir) && Path::new(&path).file_name().is_some_and(|n| n.to_string_lossy().starts_with("sample-"));
        let clip = if is_generated { path.to_string() } else { dir.join(format!("voice-{:x}.wav", hash.finalize())).to_string_lossy().to_string() };
        if !Path::new(&clip).exists() {
            let spec = hound::WavSpec { channels: 1, sample_rate: 16000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
            let mut writer = hound::WavWriter::create(&clip, spec).map_err(|e| e.to_string())?;
            for &sample in &pcm { writer.write_sample(sample).map_err(|e| e.to_string())?; }
            writer.finalize().map_err(|e| e.to_string())?;
        }
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let still_exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM speaker_vault WHERE id=?1)",params![person_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !still_exists { return Err("This person was removed while the sample was processing".into()); }
        tx.execute("INSERT OR IGNORE INTO speaker_voiceprints(person_id, embedding_json, clip_path, created_at) VALUES(?1,?2,?3,?4)", params![person_id, serde_json::to_string(&emb).map_err(|e| e.to_string())?,clip,Utc::now().to_rfc3339()]).map_err(|e| e.to_string())?;
        refresh_average(&tx, &person_id)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
}

#[tauri::command]
pub async fn find_voice_samples(person_id: String, limit: Option<usize>) -> Result<Vec<VoiceSampleSuggestion>, String> {
    require_speaker_model()?;
    blocking(move |c| find_voice_samples_in(c, &person_id, limit.unwrap_or(10).clamp(1, 100))).await
}

#[tauri::command]
pub async fn accept_voice_sample(person_id: String, meeting_id: i64, speaker_id: String) -> Result<(), String> {
    require_speaker_model()?;
    blocking(move |c| accept_voice_sample_in(c, &person_id, meeting_id, &speaker_id)).await
}

#[tauri::command]
pub async fn reject_voice_sample(person_id: String, meeting_id: i64, speaker_id: String) -> Result<(), String> {
    blocking(move |c| reject_voice_sample_in(c, &person_id, meeting_id, &speaker_id)).await
}

#[tauri::command]
pub async fn list_voiceprints(person_id: String) -> Result<Vec<Voiceprint>, String> {
    blocking(move |c| list_voiceprints_in(c, &person_id)).await
}

#[tauri::command]
pub async fn delete_voiceprint(person_id: String, voiceprint_id: i64) -> Result<(), String> {
    blocking(move |c| delete_voiceprint_in(c, &person_id, voiceprint_id)).await
}

#[tauri::command]
pub async fn suggest_speaker_names(meeting_id: i64) -> Result<Vec<SpeakerNameSuggestion>, String> {
    if !speaker_model_installed() {
        return Ok(Vec::new());
    }
    blocking(move |c| suggest_speaker_names_in(c, meeting_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::meetings::ensure_meetings_schema;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_meetings_schema(&conn).unwrap();
        conn
    }
    fn json(v: &[f32]) -> String { serde_json::to_string(v).unwrap() }
    fn person(conn: &Connection, id: &str, name: &str, print: &[f32]) {
        conn.execute(
            "INSERT INTO speaker_vault (id, name, created_at, sample_count, last_seen, embedding_json) VALUES (?1, ?2, '2026-10-01', 1, '2026-10-01', ?3)",
            params![id, name, json(print)],
        ).unwrap();
        ensure_voice_bank_schema(conn).unwrap(); // seeds their first voiceprint
    }
    fn meeting(conn: &Connection, id: i64, title: &str) {
        conn.execute(
            "INSERT INTO meetings (id, session_id, title, platform, app_name, url, created_at, duration_ms, transcript_raw, category) VALUES (?1, ?2, ?3, 'meet', '', '', ?4, 60000, '', 'General')",
            params![id, format!("s{id}"), title, format!("2026-10-0{id}T12:00:00Z")],
        ).unwrap();
    }
    /// A meeting speaker with a voice file whose voiceprint is `print` (cached,
    /// so no speaker model is needed).
    fn speaker(conn: &Connection, meeting_id: i64, speaker_id: &str, channel: i64, print: &[f32], text: &str) {
        let clip = format!("/clips/snippet_{meeting_id}_{speaker_id}.wav");
        conn.execute(
            "INSERT INTO meeting_turns (meeting_id, speaker_id, speaker_name, start_ms, end_ms, channel, text, snippet_path) VALUES (?1, ?2, ?3, 0, 4000, ?4, ?5, ?6)",
            params![meeting_id, speaker_id, speaker_id.replace("speaker_remote_", "Speaker "), channel, text, clip],
        ).unwrap();
        let voice = crate::diarization::voice_sample_path(&clip);
        conn.execute("INSERT OR REPLACE INTO voice_file_prints (path, embedding_json) VALUES (?1, ?2)", params![voice.to_string_lossy(), json(print)]).unwrap();
    }
    fn prints_of(conn: &Connection, id: &str) -> usize {
        load_voiceprints(conn).iter().filter(|p| p.speaker_id == id).count()
    }

    const DANA: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const DANA_PHONE: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
    const SAM: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

    #[test]
    fn people_enrolled_before_the_voice_bank_get_their_voiceprint_as_a_first_sample() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        assert_eq!(prints_of(&conn, "p_dana"), 1);
        ensure_voice_bank_schema(&conn).unwrap();
        assert_eq!(prints_of(&conn, "p_dana"), 1, "seeding happens once");
    }

    #[test]
    fn a_person_matches_by_their_closest_sample_not_a_blurred_average() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        add_voiceprint(&conn, "p_dana", &DANA_PHONE, Some("/clips/phone.wav"), None);
        let prints = load_voiceprints(&conn);
        // Her phone voice matches her exactly; the average of laptop + phone would score ~0.71.
        let (id, _, sim) = best_person(&DANA_PHONE, &prints, &[], 0.6).unwrap();
        assert_eq!(id, "p_dana");
        assert!((sim - 1.0).abs() < 1e-5);
        assert!(best_person(&SAM, &prints, &[], 0.6).is_none());
    }

    #[test]
    fn with_many_samples_one_odd_sample_cannot_match_a_stranger_alone() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        for (i, v) in [[0.98, 0.0, 0.2, 0.0], [0.97, 0.0, 0.0, 0.24], [0.96, 0.0, 0.14, 0.24]].iter().enumerate() {
            add_voiceprint(&conn, "p_dana", v, Some(&format!("/clips/d{i}.wav")), None);
        }
        // A wrongly added sample that is really Sam.
        add_voiceprint(&conn, "p_dana", &SAM, Some("/clips/oops.wav"), None);
        let prints = load_voiceprints(&conn);
        assert_eq!(samples_averaged(5), 2);
        assert!(best_person(&SAM, &prints, &[], 0.6).is_none(), "the odd sample alone scores ~0.5 with two averaged");
        let (id, _, sim) = best_person(&[0.99, 0.05, 0.1, 0.0], &prints, &[], 0.6).unwrap();
        assert_eq!(id, "p_dana");
        assert!(sim > 0.95, "Dana still matches strongly: {sim}");
    }

    #[test]
    fn averaging_grows_with_the_number_of_samples() {
        assert_eq!((samples_averaged(1), samples_averaged(3), samples_averaged(4), samples_averaged(7), samples_averaged(8), samples_averaged(500)), (1, 1, 2, 2, 3, 3));
        assert_eq!(person_score(vec![0.2, 0.9, 0.5]), 0.9, "few samples: the closest decides");
        assert!((person_score(vec![0.9, 0.1, 0.8, 0.2, 0.0]) - 0.85).abs() < 1e-6);
    }

    #[test]
    fn a_rejected_sound_alike_is_not_labelled_as_the_person() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        let prints = load_voiceprints(&conn);
        let sound_alike = [0.9, 0.3, 0.0, 0.0];
        assert!(best_person(&sound_alike, &prints, &[], 0.6).is_some());
        let rejected = vec![("p_dana".to_string(), vec![0.88, 0.32, 0.0, 0.0])];
        assert!(best_person(&sound_alike, &prints, &rejected, 0.6).is_none(), "closer to the rejected sample than to Dana");
        assert!(best_person(&DANA, &prints, &rejected, 0.6).is_some(), "Dana herself still matches");
    }

    #[test]
    fn find_more_samples_lists_unnamed_speakers_closest_first() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        person(&conn, "p_sam", "Sam", &SAM);
        meeting(&conn, 1, "Standup");
        meeting(&conn, 2, "Planning");
        speaker(&conn, 1, "speaker_remote_1", 1, &[0.8, 0.6, 0.0, 0.0], "Morning all");    // 0.80
        speaker(&conn, 2, "speaker_remote_2", 1, &[0.95, 0.31, 0.0, 0.0], "Budget is fine"); // ~0.95
        speaker(&conn, 2, "speaker_remote_3", 1, &[0.1, 0.99, 0.0, 0.0], "Nope");          // ~0.10, under the floor
        speaker(&conn, 2, "speaker_you", 0, &DANA, "Me");                                   // the user
        speaker(&conn, 1, "p_sam", 1, &DANA, "Already Sam");                                // already a vault person
        let found = find_voice_samples_in(&conn, "p_dana", 10).unwrap();
        let keys: Vec<_> = found.iter().map(|f| (f.meeting_id, f.speaker_id.as_str())).collect();
        assert_eq!(keys, [(2, "speaker_remote_2"), (1, "speaker_remote_1")]);
        assert_eq!(found[0].meeting_title, "Planning");
        assert_eq!(found[0].preview, "Budget is fine");
        assert!(found[0].similarity > found[1].similarity);
        assert_eq!(find_voice_samples_in(&conn, "p_dana", 1).unwrap().len(), 1, "limit");
    }

    #[test]
    fn adding_a_sample_labels_that_speaker_and_keeps_another_voiceprint() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        meeting(&conn, 2, "Planning");
        speaker(&conn, 2, "speaker_remote_2", 1, &[0.95, 0.31, 0.0, 0.0], "Budget is fine");
        accept_voice_sample_in(&conn, "p_dana", 2, "speaker_remote_2").unwrap();

        let (id, name): (String, String) = conn.query_row("SELECT speaker_id, speaker_name FROM meeting_turns WHERE meeting_id = 2", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((id.as_str(), name.as_str()), ("p_dana", "Dana"));
        assert_eq!(prints_of(&conn, "p_dana"), 2);
        let (samples, meetings): (i64, i64) = conn.query_row("SELECT sample_count, meeting_count FROM speaker_vault WHERE id = 'p_dana'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((samples, meetings), (2, 1));
        assert!(find_voice_samples_in(&conn, "p_dana", 10).unwrap().is_empty(), "no longer a suggestion");
        assert!(accept_voice_sample_in(&conn, "p_dana", 2, "speaker_remote_2").unwrap_err().contains("no longer in the meeting"));
        assert!(accept_voice_sample_in(&conn, "p_gone", 2, "speaker_remote_9").unwrap_err().contains("no longer in the Speaker Vault"));
    }

    #[test]
    fn not_them_is_remembered() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        meeting(&conn, 1, "Standup");
        speaker(&conn, 1, "speaker_remote_1", 1, &[0.8, 0.6, 0.0, 0.0], "Morning all");
        assert_eq!(find_voice_samples_in(&conn, "p_dana", 10).unwrap().len(), 1);
        reject_voice_sample_in(&conn, "p_dana", 1, "speaker_remote_1").unwrap();
        assert!(find_voice_samples_in(&conn, "p_dana", 10).unwrap().is_empty());
        assert!(suggest_speaker_names_in(&conn, 1).unwrap().is_empty(), "not asked again after the meeting either");
        assert_eq!(load_rejections(&conn).len(), 1, "kept with its voiceprint for later matching");
    }

    #[test]
    fn after_a_meeting_each_person_is_asked_about_one_close_speaker() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        person(&conn, "p_sam", "Sam", &SAM);
        meeting(&conn, 3, "Review");
        speaker(&conn, 3, "speaker_remote_1", 1, &[0.7, 0.71, 0.0, 0.0], "a");   // ~0.70 Dana, ~0.71 Sam
        speaker(&conn, 3, "speaker_remote_2", 1, &[0.9, 0.0, 0.43, 0.0], "b");   // ~0.90 Dana
        speaker(&conn, 3, "speaker_remote_3", 1, &[0.0, 0.0, 0.0, 1.0], "c");    // nobody
        let asks = suggest_speaker_names_in(&conn, 3).unwrap();
        let pairs: Vec<_> = asks.iter().map(|a| (a.speaker_id.as_str(), a.person_name.as_str())).collect();
        assert_eq!(pairs, [("speaker_remote_2", "Dana"), ("speaker_remote_1", "Sam")]);
    }

    #[test]
    fn samples_can_be_removed_but_never_the_last_one() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        add_voiceprint(&conn, "p_dana", &DANA_PHONE, Some("/clips/phone.wav"), Some(4));
        let list = list_voiceprints_in(&conn, "p_dana").unwrap();
        assert_eq!(list.len(), 2);
        let phone = list.iter().find(|v| v.meeting_id == Some(4)).unwrap().id;
        delete_voiceprint_in(&conn, "p_dana", phone).unwrap();
        let avg: String = conn.query_row("SELECT embedding_json FROM speaker_vault WHERE id = 'p_dana'", [], |r| r.get(0)).unwrap();
        assert!(cosine_similarity(&parse(&avg).unwrap(), &DANA) > 0.999, "overall voiceprint recomputed");
        let last = list_voiceprints_in(&conn, "p_dana").unwrap()[0].id;
        assert!(delete_voiceprint_in(&conn, "p_dana", last).unwrap_err().contains("only voice sample"));
    }

    #[test]
    fn forgetting_a_person_removes_their_samples_and_answers() {
        let conn = db();
        person(&conn, "p_dana", "Dana", &DANA);
        meeting(&conn, 1, "Standup");
        speaker(&conn, 1, "speaker_remote_1", 1, &SAM, "x");
        reject_voice_sample_in(&conn, "p_dana", 1, "speaker_remote_1").unwrap();
        forget_person_in(&conn, "p_dana");
        assert_eq!(prints_of(&conn, "p_dana"), 0);
        assert!(load_rejections(&conn).is_empty());
    }
}

#[cfg(test)]
mod generated_sample_tests {
    use super::*;
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self { let path=std::env::temp_dir().join(format!("ts_voice_sample_{:016x}",rand::random::<u64>())); std::fs::create_dir_all(&path).unwrap(); Self(path) }
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TempDir { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
    #[test]
    fn generate_confirm_and_regenerate_use_real_audio_and_keep_distinct_samples() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::meetings::ensure_meetings_schema(&conn).unwrap();
        conn.execute("INSERT INTO speaker_vault(id,name,created_at,last_seen,embedding_json) VALUES('p','Dana','','','[1.0,0.0]')",[]).unwrap();
        ensure_voice_bank_schema(&conn).unwrap();
        let root = TempDir::new(); let clip=root.path().join("preview.wav");
        let voice = crate::diarization::voice_sample_path(&clip.to_string_lossy());
        let spec=hound::WavSpec {channels:1,sample_rate:16000,bits_per_sample:32,sample_format:hound::SampleFormat::Float};
        let mut writer=hound::WavWriter::create(&voice,spec).unwrap();
        for n in 0..16000*30 { let amp=if n<16000*15 {0.3} else {0.5}; writer.write_sample(amp*(n as f32*0.04).sin()).unwrap(); }
        writer.finalize().unwrap();
        conn.execute("INSERT INTO meetings(id,session_id,title,platform,app_name,url,created_at,duration_ms,transcript_raw,category) VALUES(1,'m','Meeting','meet','','','',30000,'','General')",[]).unwrap();
        conn.execute("INSERT INTO meeting_turns(meeting_id,speaker_id,speaker_name,start_ms,end_ms,channel,text,snippet_path) VALUES(1,'p','Dana',0,30000,1,'Speech',?1)",params![clip.to_string_lossy()]).unwrap();
        let dir=root.path().join("bank");
        let first=generate_vault_sample_in(&conn,"p",&dir).unwrap();
        let count=|c:&Connection| c.query_row("SELECT COUNT(*) FROM speaker_voiceprints WHERE person_id='p'",[],|r|r.get::<_,i64>(0)).unwrap();
        assert_eq!(count(&conn),1,"preview does not enroll");
        add_vault_sample_in(&conn,"p",&first,&dir,&mut |_| Ok(vec![0.8,0.2])).unwrap();
        assert_eq!(count(&conn),2);
        add_vault_sample_in(&conn,"p",&first,&dir,&mut |_| Ok(vec![0.8,0.2])).unwrap();
        assert_eq!(count(&conn),2,"same clip never adds twice");
        let second=generate_vault_sample_in(&conn,"p",&dir).unwrap(); assert_ne!(first,second);
        add_vault_sample_in(&conn,"p",&second,&dir,&mut |_| Ok(vec![0.9,0.1])).unwrap();
        assert_eq!(count(&conn),3);
        assert!(generate_vault_sample_in(&conn,"p",&dir).unwrap_err().contains("No new retained speech"));
        let stored:i64=conn.query_row("SELECT sample_count FROM speaker_vault WHERE id='p'",[],|r|r.get(0)).unwrap();assert_eq!(stored,3);
        assert!(generate_vault_sample_in(&conn,"missing",&dir).is_err());
    }
    #[test]
    fn silence_and_short_clips_do_not_add_a_voiceprint() {
        let conn=Connection::open_in_memory().unwrap();super::super::meetings::ensure_meetings_schema(&conn).unwrap();
        conn.execute("INSERT INTO speaker_vault(id,name,created_at,last_seen) VALUES('p','Dana','','')",[]).unwrap();
        let root=TempDir::new();let path=root.path().join("silence.wav");
        let spec=hound::WavSpec{channels:1,sample_rate:16000,bits_per_sample:32,sample_format:hound::SampleFormat::Float};
        let mut w=hound::WavWriter::create(&path,spec).unwrap();for _ in 0..16000*10 {w.write_sample(0.0f32).unwrap();}w.finalize().unwrap();
        let err=add_vault_sample_in(&conn,"p",&path.to_string_lossy(),root.path(),&mut |_| panic!("silence must not reach embedding model")).unwrap_err();
        assert!(err.contains("5 seconds"));
        assert_eq!(list_voiceprints_in(&conn,"p").unwrap().len(),0);
    }
}
