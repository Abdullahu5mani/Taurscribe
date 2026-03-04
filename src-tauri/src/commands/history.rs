use chrono::Utc;
use dirs::data_local_dir;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;

fn get_history_db_path() -> Result<PathBuf, String> {
    let app_data = data_local_dir().ok_or("Could not find AppData directory")?;
    let base = app_data.join("Taurscribe");
    if let Err(e) = std::fs::create_dir_all(&base) {
        return Err(format!("Failed to create Taurscribe data directory: {}", e));
    }
    Ok(base.join("transcript_history.db"))
}

fn ensure_history_db() -> Result<Connection, String> {
    let path = get_history_db_path()?;
    let conn = Connection::open(&path)
        .map_err(|e| format!("Failed to open history DB at {}: {}", path.display(), e))?;

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS transcriptions (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at       TEXT NOT NULL,
            transcript       TEXT NOT NULL,
            engine           TEXT NOT NULL,
            duration_ms      INTEGER,
            grammar_llm_used INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_transcriptions_created_at
            ON transcriptions(created_at DESC);
        "#,
    )
    .map_err(|e| {
        eprintln!("[HISTORY] Failed to initialize history DB: {}", e);
        format!("Failed to initialize history DB: {}", e)
    })?;

    Ok(conn)
}

#[derive(Serialize)]
pub struct TranscriptRecord {
    pub id: i64,
    pub created_at: String,
    pub transcript: String,
    pub engine: String,
    pub duration_ms: Option<i64>,
    pub grammar_llm_used: bool,
}

/// Save a single transcription entry to the history database.
///
/// `grammar_llm_used` indicates whether the FlowScribe grammar LLM processed this transcript.
#[tauri::command]
pub fn save_transcript_history(
    transcript: String,
    engine: String,
    duration_ms: Option<i64>,
    grammar_llm_used: bool,
) -> Result<(), String> {
    // Don't persist empty transcripts.
    if transcript.trim().is_empty() {
        return Ok(());
    }

    let conn = ensure_history_db()?;
    let created_at = Utc::now().to_rfc3339();
    let grammar_flag: i64 = if grammar_llm_used { 1 } else { 0 };

    println!(
        "[HISTORY] Saving transcript: engine={}, len={}, grammar_llm_used={}",
        engine,
        transcript.len(),
        grammar_llm_used
    );

    conn.execute(
        "INSERT INTO transcriptions (created_at, transcript, engine, duration_ms, grammar_llm_used)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![created_at, transcript, engine, duration_ms, grammar_flag],
    )
    .map_err(|e| {
        eprintln!("[HISTORY] Failed to insert history row: {}", e);
        format!("Failed to insert history row: {}", e)
    })?;

    Ok(())
}

/// List recent transcription history, newest first.
#[tauri::command]
pub fn list_transcript_history(
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<TranscriptRecord>, String> {
    let conn = ensure_history_db()?;
    let limit = limit.unwrap_or(50) as i64;
    let offset = offset.unwrap_or(0) as i64;

    let mut stmt = conn
        .prepare(
            "SELECT id, created_at, transcript, engine, duration_ms, grammar_llm_used
             FROM transcriptions
             ORDER BY datetime(created_at) DESC
             LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| {
            eprintln!("[HISTORY] Failed to prepare history query: {}", e);
            format!("Failed to prepare history query: {}", e)
        })?;

    let rows = stmt
        .query_map(params![limit, offset], |row| {
            let grammar_int: i64 = row.get(5)?;
            Ok(TranscriptRecord {
                id: row.get(0)?,
                created_at: row.get(1)?,
                transcript: row.get(2)?,
                engine: row.get(3)?,
                duration_ms: row.get(4)?,
                grammar_llm_used: grammar_int != 0,
            })
        })
        .map_err(|e| {
            eprintln!("[HISTORY] Failed to query history rows: {}", e);
            format!("Failed to query history rows: {}", e)
        })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| {
            eprintln!("[HISTORY] Failed to read history row: {}", e);
            format!("Failed to read history row: {}", e)
        })?);
    }

    println!(
        "[HISTORY] list_transcript_history: limit={}, offset={}, rows={}",
        limit,
        offset,
        out.len()
    );

    Ok(out)
}

/// Delete a single transcription entry by its primary key.
#[tauri::command]
pub fn delete_transcript_history(id: i64) -> Result<(), String> {
    let conn = ensure_history_db()?;
    let affected = conn
        .execute("DELETE FROM transcriptions WHERE id = ?1", params![id])
        .map_err(|e| {
            eprintln!("[HISTORY] Failed to delete history row {}: {}", id, e);
            format!("Failed to delete history row: {}", e)
        })?;
    println!("[HISTORY] Deleted {} row(s) for id={}", affected, id);
    Ok(())
}
