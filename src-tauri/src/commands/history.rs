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

/// SQL expression deriving `kind` from `audio_source` (used for backfill and by
/// the MCP server as a fallback for databases it cannot migrate).
pub const KIND_FROM_SOURCE_SQL: &str =
    "CASE WHEN audio_source IS NULL OR audio_source = 'microphone' THEN 'dictation' ELSE 'file' END";

fn kind_for_source(audio_source: Option<&str>) -> &'static str {
    match audio_source {
        None | Some("microphone") => "dictation",
        Some(_) => "file",
    }
}

fn ensure_history_db() -> Result<Connection, String> {
    let path = get_history_db_path()?;
    let conn = Connection::open(&path)
        .map_err(|e| format!("Failed to open history DB at {}: {}", path.display(), e))?;
    init_history_schema(&conn)?;
    Ok(conn)
}

fn init_history_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS transcriptions (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at          TEXT NOT NULL,
            transcript          TEXT NOT NULL,
            engine              TEXT NOT NULL,
            duration_ms         INTEGER,
            grammar_llm_used    INTEGER NOT NULL,
            processing_time_ms  INTEGER,
            model_id            TEXT,
            audio_source        TEXT,
            kind                TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_transcriptions_created_at
            ON transcriptions(created_at DESC);
        "#,
    )
    .map_err(|e| {
        eprintln!("[HISTORY] Failed to initialize history DB: {}", e);
        format!("Failed to initialize history DB: {}", e)
    })?;

    // Migrate existing DBs that predate newer columns.
    let _ = conn.execute(
        "ALTER TABLE transcriptions ADD COLUMN processing_time_ms INTEGER",
        [],
    );
    let _ = conn.execute("ALTER TABLE transcriptions ADD COLUMN model_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE transcriptions ADD COLUMN audio_source TEXT",
        [],
    );
    // `kind` = "dictation" (microphone) or "file" (dropped audio file). Rows from
    // before the column existed are backfilled from audio_source.
    let _ = conn.execute("ALTER TABLE transcriptions ADD COLUMN kind TEXT", []);
    let _ = conn.execute(
        &format!("UPDATE transcriptions SET kind = {KIND_FROM_SOURCE_SQL} WHERE kind IS NULL"),
        [],
    );

    Ok(())
}

#[derive(Serialize)]
pub struct TranscriptRecord {
    pub id: i64,
    pub created_at: String,
    pub transcript: String,
    pub engine: String,
    pub duration_ms: Option<i64>,
    pub grammar_llm_used: bool,
    pub processing_time_ms: Option<i64>,
    pub model_id: Option<String>,
    pub audio_source: Option<String>,
    pub kind: String,
}

/// Save a single transcription entry to the history database.
///
/// `grammar_llm_used` indicates whether the FlowScribe grammar LLM processed this transcript.
///
/// macOS fix: Made async with spawn_blocking because SQLite I/O would otherwise
/// block the macOS AppKit main thread (Tauri 2 dispatches sync commands there).
#[tauri::command]
pub async fn save_transcript_history(
    transcript: String,
    engine: String,
    duration_ms: Option<i64>,
    grammar_llm_used: bool,
    processing_time_ms: Option<i64>,
    model_id: Option<String>,
    audio_source: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        save_transcript_history_blocking(
            transcript,
            engine,
            duration_ms,
            grammar_llm_used,
            processing_time_ms,
            model_id,
            audio_source,
        )
    })
    .await
    .map_err(|e| format!("save_transcript_history task failed: {}", e))?
}

fn save_transcript_history_blocking(
    transcript: String,
    engine: String,
    duration_ms: Option<i64>,
    grammar_llm_used: bool,
    processing_time_ms: Option<i64>,
    model_id: Option<String>,
    audio_source: Option<String>,
) -> Result<(), String> {
    // Don't persist empty transcripts.
    if transcript.trim().is_empty() {
        return Ok(());
    }

    let conn = ensure_history_db()?;
    insert_history_row(&conn, &Utc::now().to_rfc3339(), &transcript, &engine, duration_ms, grammar_llm_used, processing_time_ms, model_id.as_deref(), audio_source.as_deref())
}

#[allow(clippy::too_many_arguments)]
fn insert_history_row(
    conn: &Connection,
    created_at: &str,
    transcript: &str,
    engine: &str,
    duration_ms: Option<i64>,
    grammar_llm_used: bool,
    processing_time_ms: Option<i64>,
    model_id: Option<&str>,
    audio_source: Option<&str>,
) -> Result<(), String> {
    let grammar_flag: i64 = if grammar_llm_used { 1 } else { 0 };

    println!(
        "[HISTORY] Saving transcript: engine={}, model={:?}, source={:?}, len={}, grammar_llm_used={}, processing_time_ms={:?}",
        engine,
        model_id,
        audio_source,
        transcript.len(),
        grammar_llm_used,
        processing_time_ms
    );

    conn.execute(
        "INSERT INTO transcriptions (created_at, transcript, engine, duration_ms, grammar_llm_used, processing_time_ms, model_id, audio_source, kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![created_at, transcript, engine, duration_ms, grammar_flag, processing_time_ms, model_id, audio_source, kind_for_source(audio_source)],
    )
    .map_err(|e| {
        eprintln!("[HISTORY] Failed to insert history row: {}", e);
        format!("Failed to insert history row: {}", e)
    })?;

    Ok(())
}

/// List recent transcription history, newest first.
///
/// macOS fix: Async with spawn_blocking to avoid blocking the AppKit main thread.
#[tauri::command]
pub async fn list_transcript_history(
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<TranscriptRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || list_transcript_history_blocking(limit, offset))
        .await
        .map_err(|e| format!("list_transcript_history task failed: {}", e))?
}

fn list_transcript_history_blocking(
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<TranscriptRecord>, String> {
    let conn = ensure_history_db()?;
    let limit = limit.unwrap_or(50) as i64;
    let offset = offset.unwrap_or(0) as i64;
    let out = query_history(&conn, limit, offset)?;

    println!(
        "[HISTORY] list_transcript_history: limit={}, offset={}, rows={}",
        limit,
        offset,
        out.len()
    );

    Ok(out)
}

fn query_history(conn: &Connection, limit: i64, offset: i64) -> Result<Vec<TranscriptRecord>, String> {

    let mut stmt = conn
        .prepare(
            "SELECT id, created_at, transcript, engine, duration_ms, grammar_llm_used, processing_time_ms, model_id, audio_source, kind
             FROM transcriptions
             ORDER BY datetime(created_at) DESC, id DESC
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
                processing_time_ms: row.get(6)?,
                model_id: row.get(7)?,
                audio_source: row.get(8)?,
                kind: row.get::<_, Option<String>>(9)?.unwrap_or_else(|| "dictation".into()),
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
    Ok(out)
}

/// Delete a single transcription entry by its primary key.
///
/// macOS fix: Async with spawn_blocking to avoid blocking the AppKit main thread.
#[tauri::command]
pub async fn delete_transcript_history(id: i64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || delete_transcript_history_blocking(id))
        .await
        .map_err(|e| format!("delete_transcript_history task failed: {}", e))?
}

fn delete_transcript_history_blocking(id: i64) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_history_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn rows_from_the_same_second_come_back_newest_first() {
        let conn = mem_db();
        for text in ["first", "second", "third"] {
            insert_history_row(&conn, "2026-09-27T10:00:00.5+00:00", text, "whisper", None, false, None, None, None).unwrap();
        }
        insert_history_row(&conn, "2026-09-26T10:00:00+00:00", "older", "whisper", None, false, None, None, None).unwrap();
        let rows = query_history(&conn, 10, 0).unwrap();
        let texts: Vec<&str> = rows.iter().map(|r| r.transcript.as_str()).collect();
        assert_eq!(texts, ["third", "second", "first", "older"]);
        let page: Vec<String> = query_history(&conn, 2, 1).unwrap().into_iter().map(|r| r.transcript).collect();
        assert_eq!(page, ["second", "first"]);
    }

    #[test]
    fn inserted_rows_round_trip() {
        let conn = mem_db();
        insert_history_row(&conn, "2026-09-27T10:00:00+00:00", "hello", "qwen3", Some(1200), true, Some(300), Some("qwen3-asr-0.6b"), Some("talk.mp3")).unwrap();
        let r = &query_history(&conn, 1, 0).unwrap()[0];
        assert_eq!((r.engine.as_str(), r.duration_ms, r.grammar_llm_used), ("qwen3", Some(1200), true));
        assert_eq!((r.processing_time_ms, r.model_id.as_deref()), (Some(300), Some("qwen3-asr-0.6b")));
        assert_eq!((r.audio_source.as_deref(), r.kind.as_str()), (Some("talk.mp3"), "file"));
    }

    #[test]
    fn schema_migrates_an_old_table_and_backfills_kind() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE transcriptions (id INTEGER PRIMARY KEY AUTOINCREMENT, created_at TEXT NOT NULL, transcript TEXT NOT NULL,
               engine TEXT NOT NULL, duration_ms INTEGER, grammar_llm_used INTEGER NOT NULL);
             INSERT INTO transcriptions (created_at, transcript, engine, grammar_llm_used) VALUES ('2025-01-01T00:00:00+00:00', 'old', 'whisper', 0);",
        )
        .unwrap();
        init_history_schema(&conn).unwrap();
        init_history_schema(&conn).unwrap(); // idempotent
        let r = &query_history(&conn, 5, 0).unwrap()[0];
        assert_eq!((r.transcript.as_str(), r.kind.as_str(), r.model_id.as_deref()), ("old", "dictation", None));
    }

    #[test]
    fn kind_backfill_matches_insert_rule() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE transcriptions (id INTEGER PRIMARY KEY, audio_source TEXT);
             INSERT INTO transcriptions (audio_source) VALUES (NULL), ('microphone'), ('talk.mp3');
             ALTER TABLE transcriptions ADD COLUMN kind TEXT;",
        )
        .unwrap();
        conn.execute(&format!("UPDATE transcriptions SET kind = {KIND_FROM_SOURCE_SQL} WHERE kind IS NULL"), [])
            .unwrap();
        let rows: Vec<(Option<String>, String)> = conn
            .prepare("SELECT audio_source, kind FROM transcriptions ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (source, kind) in rows {
            assert_eq!(kind, kind_for_source(source.as_deref()));
        }
        assert_eq!(kind_for_source(Some("talk.mp3")), "file");
    }
}
