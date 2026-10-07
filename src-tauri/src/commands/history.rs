use chrono::Utc;
use dirs::data_local_dir;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
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
    // The dictation feed and Files panel can initialize separate connections
    // together. Serialize schema inspection/migration and publish it atomically.
    let transaction = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("Failed to begin history migration: {e}"))?;
    let conn = &transaction;
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
            kind                TEXT,
            source_path         TEXT,
            import_method       TEXT
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
    // Check first so repeat initialization is harmless, while real migration
    // failures are reported instead of silently disabling file history.
    for column in ["source_path", "import_method"] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('transcriptions') WHERE name = ?1)",
            [column], |row| row.get(0),
        ).map_err(|e| format!("Failed to inspect history schema: {e}"))?;
        if !exists {
            conn.execute(&format!("ALTER TABLE transcriptions ADD COLUMN {column} TEXT"), [])
                .map_err(|e| format!("Failed to migrate history {column}: {e}"))?;
        }
    }
    let _ = conn.execute(
        &format!("UPDATE transcriptions SET kind = {KIND_FROM_SOURCE_SQL} WHERE kind IS NULL"),
        [],
    );

    transaction.commit().map_err(|e| format!("Failed to commit history migration: {e}"))
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
    pub source_path: Option<String>,
    pub import_method: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileImportMethod {
    Drop,
    Picker,
}

impl FileImportMethod {
    fn as_str(self) -> &'static str {
        match self { Self::Drop => "drop", Self::Picker => "picker" }
    }
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
    source_path: Option<String>,
    import_method: Option<FileImportMethod>,
    completed_at: Option<String>,
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
            source_path,
            import_method,
            completed_at,
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
    source_path: Option<String>,
    import_method: Option<FileImportMethod>,
    completed_at: Option<String>,
) -> Result<(), String> {
    // Don't persist empty transcripts.
    if transcript.trim().is_empty() {
        return Ok(());
    }

    let created_at = match completed_at {
        Some(value) => chrono::DateTime::parse_from_rfc3339(&value)
            .map_err(|e| format!("Invalid transcription completion time: {e}"))?
            .with_timezone(&Utc).to_rfc3339(),
        None => Utc::now().to_rfc3339(),
    };
    let conn = ensure_history_db()?;
    insert_history_row_with_source(&conn, &created_at, &transcript, &engine, duration_ms, grammar_llm_used, processing_time_ms, model_id.as_deref(), audio_source.as_deref(), source_path.as_deref(), import_method.map(FileImportMethod::as_str))
        .map(|_| ())
}

/// Saves a finished dictation's transcript (as recognized, before the UI's
/// corrections) and returns its row id. `stop_recording` calls this before
/// deleting the recording, so the text is stored whatever happens next.
pub(crate) fn record_dictation(
    transcript: &str,
    engine: &str,
    duration_ms: Option<i64>,
    processing_time_ms: Option<i64>,
    model_id: Option<&str>,
) -> Result<i64, String> {
    let conn = ensure_history_db()?;
    insert_history_row(&conn, &Utc::now().to_rfc3339(), transcript, engine, duration_ms, false, processing_time_ms, model_id, Some("microphone"))
}

/// Replaces a dictation's text with what the UI finally produced (dictionary,
/// grammar correction, snippets) and records the total processing time.
#[tauri::command]
pub async fn update_transcript_history(
    id: i64,
    transcript: String,
    grammar_llm_used: bool,
    processing_time_ms: Option<i64>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = ensure_history_db()?;
        update_history_row(&conn, id, &transcript, grammar_llm_used, processing_time_ms)
    })
    .await
    .map_err(|e| format!("update_transcript_history task failed: {}", e))?
}

fn update_history_row(
    conn: &Connection,
    id: i64,
    transcript: &str,
    grammar_llm_used: bool,
    processing_time_ms: Option<i64>,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE transcriptions SET transcript = ?2, grammar_llm_used = ?3, processing_time_ms = COALESCE(?4, processing_time_ms) WHERE id = ?1",
            params![id, transcript, grammar_llm_used as i64, processing_time_ms],
        )
        .map_err(|e| format!("Failed to update history row: {}", e))?;
    if changed == 0 {
        return Err(format!("No transcript #{id} to update"));
    }
    Ok(())
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
) -> Result<i64, String> {
    insert_history_row_with_source(conn, created_at, transcript, engine, duration_ms, grammar_llm_used, processing_time_ms, model_id, audio_source, None, None)
}

#[allow(clippy::too_many_arguments)]
fn insert_history_row_with_source(
    conn: &Connection,
    created_at: &str,
    transcript: &str,
    engine: &str,
    duration_ms: Option<i64>,
    grammar_llm_used: bool,
    processing_time_ms: Option<i64>,
    model_id: Option<&str>,
    audio_source: Option<&str>,
    source_path: Option<&str>,
    import_method: Option<&str>,
) -> Result<i64, String> {
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
        "INSERT INTO transcriptions (created_at, transcript, engine, duration_ms, grammar_llm_used, processing_time_ms, model_id, audio_source, kind, source_path, import_method)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![created_at, transcript, engine, duration_ms, grammar_flag, processing_time_ms, model_id, audio_source, if source_path.is_some() { "file" } else { kind_for_source(audio_source) }, source_path, import_method],
    )
    .map_err(|e| {
        eprintln!("[HISTORY] Failed to insert history row: {}", e);
        format!("Failed to insert history row: {}", e)
    })?;

    Ok(conn.last_insert_rowid())
}

/// List recent transcription history, newest first.
///
/// macOS fix: Async with spawn_blocking to avoid blocking the AppKit main thread.
#[tauri::command]
pub async fn list_transcript_history(
    limit: Option<u32>,
    offset: Option<u32>,
    kind: Option<String>,
) -> Result<Vec<TranscriptRecord>, String> {
    tauri::async_runtime::spawn_blocking(move || list_transcript_history_blocking(limit, offset, kind))
        .await
        .map_err(|e| format!("list_transcript_history task failed: {}", e))?
}

fn list_transcript_history_blocking(
    limit: Option<u32>,
    offset: Option<u32>,
    kind: Option<String>,
) -> Result<Vec<TranscriptRecord>, String> {
    let conn = ensure_history_db()?;
    let limit = limit.unwrap_or(50) as i64;
    let offset = offset.unwrap_or(0) as i64;
    let out = query_history(&conn, limit, offset, kind.as_deref())?;

    println!(
        "[HISTORY] list_transcript_history: limit={}, offset={}, rows={}",
        limit,
        offset,
        out.len()
    );

    Ok(out)
}

/// `kind` ("dictation" / "file") filters before the limit, so a page of one
/// kind isn't emptied by newer rows of the other.
fn query_history(conn: &Connection, limit: i64, offset: i64, kind: Option<&str>) -> Result<Vec<TranscriptRecord>, String> {

    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, created_at, transcript, engine, duration_ms, grammar_llm_used, processing_time_ms, model_id, audio_source, kind, source_path, import_method
             FROM transcriptions
             WHERE ?3 IS NULL OR COALESCE(kind, {KIND_FROM_SOURCE_SQL}) = ?3
             ORDER BY julianday(created_at) DESC, id DESC
             LIMIT ?1 OFFSET ?2"
        ))
        .map_err(|e| {
            eprintln!("[HISTORY] Failed to prepare history query: {}", e);
            format!("Failed to prepare history query: {}", e)
        })?;

    let rows = stmt
        .query_map(params![limit, offset, kind], |row| {
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
                source_path: row.get(10)?,
                import_method: row.get(11)?,
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
        let rows = query_history(&conn, 10, 0, None).unwrap();
        let texts: Vec<&str> = rows.iter().map(|r| r.transcript.as_str()).collect();
        assert_eq!(texts, ["third", "second", "first", "older"]);
        let page: Vec<String> = query_history(&conn, 2, 1, None).unwrap().into_iter().map(|r| r.transcript).collect();
        assert_eq!(page, ["second", "first"]);
    }

    #[test]
    fn completion_order_is_preserved_when_an_older_file_is_saved_later_in_the_same_second() {
        let conn = mem_db();
        // A retry of an older completed file can have a higher insertion id.
        insert_history_row(&conn, "2026-10-07T12:00:00.900+00:00", "newer completion", "granite", None, false, None, None, Some("newer.wav")).unwrap();
        insert_history_row(&conn, "2026-10-07T12:00:00.100+00:00", "older completion saved later", "granite", None, false, None, None, Some("older.wav")).unwrap();
        let rows = query_history(&conn, 2, 0, Some("file")).unwrap();
        println!("[HISTORY_TEST] subsecond order: {:?}", rows.iter().map(|r| (&r.created_at, &r.transcript)).collect::<Vec<_>>());
        assert_eq!(rows[0].transcript, "newer completion");
    }

    #[test]
    fn subsecond_history_pages_follow_completion_time_across_offsets_and_timezones() {
        let conn = mem_db();
        let mut expected = Vec::new();
        for i in 0..40 {
            let millis = (i * 137) % 1000;
            let stamp = if i % 2 == 0 {
                format!("2026-10-07T12:00:00.{millis:03}+00:00")
            } else {
                format!("2026-10-07T08:00:00.{millis:03}-04:00")
            };
            let id = insert_history_row(&conn, &stamp, &format!("synthetic row {i}"), "whisper", None, false, None, None, None).unwrap();
            expected.push((chrono::DateTime::parse_from_rfc3339(&stamp).unwrap().timestamp_millis(), id));
        }
        expected.sort_by(|a, b| b.cmp(a));
        let mut actual = Vec::new();
        for offset in (0..40).step_by(7) {
            actual.extend(query_history(&conn, 7, offset, None).unwrap().iter().map(|row| row.id));
        }
        println!("[HISTORY_TEST] paged time order: {actual:?}");
        assert_eq!(actual, expected.iter().map(|(_, id)| *id).collect::<Vec<_>>());
    }

    #[test]
    fn kind_filter_applies_before_the_limit() {
        let conn = mem_db();
        insert_history_row(&conn, "2026-09-26T10:00:00+00:00", "dictated", "whisper", None, false, None, None, None).unwrap();
        for i in 0..5 {
            insert_history_row(&conn, &format!("2026-09-27T10:00:0{i}+00:00"), "file", "whisper", None, false, None, None, Some("talk.mp3")).unwrap();
        }
        // A row whose kind was never backfilled still counts by its source.
        conn.execute("INSERT INTO transcriptions (created_at, transcript, engine, grammar_llm_used) VALUES ('2026-09-25T10:00:00+00:00', 'legacy', 'whisper', 0)", []).unwrap();
        let dictations: Vec<String> = query_history(&conn, 2, 0, Some("dictation")).unwrap().into_iter().map(|r| r.transcript).collect();
        assert_eq!(dictations, ["dictated", "legacy"]);
        assert_eq!(query_history(&conn, 10, 0, Some("file")).unwrap().len(), 5);
        assert_eq!(query_history(&conn, 10, 0, None).unwrap().len(), 7);
    }

    #[test]
    fn a_saved_dictation_is_updated_with_the_final_text() {
        let conn = mem_db();
        let id = insert_history_row(&conn, "2026-09-27T10:00:00+00:00", "raw words", "granite", Some(2000), false, Some(150), None, Some("microphone")).unwrap();
        update_history_row(&conn, id, "Raw words.", true, Some(900)).unwrap();
        let r = &query_history(&conn, 1, 0, None).unwrap()[0];
        assert_eq!((r.id, r.transcript.as_str(), r.grammar_llm_used, r.processing_time_ms), (id, "Raw words.", true, Some(900)));
        // Without a new time the backend's measurement stays.
        update_history_row(&conn, id, "Raw words!", false, None).unwrap();
        assert_eq!(query_history(&conn, 1, 0, None).unwrap()[0].processing_time_ms, Some(900));
        assert!(update_history_row(&conn, id + 1, "x", false, None).is_err());
    }

    #[test]
    fn inserted_rows_round_trip() {
        let conn = mem_db();
        insert_history_row(&conn, "2026-09-27T10:00:00+00:00", "hello", "qwen3", Some(1200), true, Some(300), Some("qwen3-asr-0.6b"), Some("talk.mp3")).unwrap();
        let r = &query_history(&conn, 1, 0, None).unwrap()[0];
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
        let r = &query_history(&conn, 5, 0, None).unwrap()[0];
        assert_eq!((r.transcript.as_str(), r.kind.as_str(), r.model_id.as_deref()), ("old", "dictation", None));
        assert!(r.source_path.is_none());
        assert!(r.import_method.is_none());
    }

    #[test]
    fn file_history_survives_reopening_and_removing_original_audio() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!("taurscribe-history-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let db = directory.join("history.db");
        let audio = directory.join("Example speech 音声.wav");
        std::fs::write(&audio, b"original audio stays here").unwrap();
        {
            let conn = Connection::open(&db).unwrap();
            init_history_schema(&conn).unwrap();
            for method in ["drop", "picker"] {
                insert_history_row_with_source(&conn, "2026-09-30T14:01:02+00:00", "Saved words.\nSecond line.", "qwen3", Some(5200), false, Some(310), Some("qwen3-asr-0.6b"), Some("Example speech 音声.wav"), audio.to_str(), Some(method)).unwrap();
            }
        }
        assert_eq!(std::fs::read(&audio).unwrap(), b"original audio stays here");
        // History is useful after the user's original file has gone away.
        std::fs::remove_file(&audio).unwrap();
        {
            let conn = Connection::open(&db).unwrap();
            init_history_schema(&conn).unwrap();
            let rows = query_history(&conn, 25, 0, Some("file")).unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].import_method.as_deref(), Some("picker"));
            assert_eq!(rows[1].import_method.as_deref(), Some("drop"));
            for row in rows {
                assert_eq!(row.source_path.as_deref(), audio.to_str());
                assert_eq!(row.transcript, "Saved words.\nSecond line.");
                assert_eq!(row.created_at, "2026-09-30T14:01:02+00:00");
                assert_eq!(row.model_id.as_deref(), Some("qwen3-asr-0.6b"));
                assert_eq!(row.audio_source.as_deref(), Some("Example speech 音声.wav"));
                assert_eq!((row.duration_ms, row.processing_time_ms), (Some(5200), Some(310)));
            }
        }
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1, "Only the transcript database is retained");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn upgrading_existing_file_history_preserves_unknown_source_details() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE transcriptions (id INTEGER PRIMARY KEY, created_at TEXT NOT NULL, transcript TEXT NOT NULL, engine TEXT NOT NULL, duration_ms INTEGER, grammar_llm_used INTEGER NOT NULL, processing_time_ms INTEGER, model_id TEXT, audio_source TEXT, kind TEXT);
            INSERT INTO transcriptions VALUES (1, '2026-09-29T10:00:00+00:00', 'Existing file transcript', 'whisper', 1200, 0, 300, 'base.en', 'old.mp3', 'file');").unwrap();
        init_history_schema(&conn).unwrap();
        init_history_schema(&conn).unwrap();
        let row = &query_history(&conn, 25, 0, Some("file")).unwrap()[0];
        assert_eq!(row.id, 1);
        assert_eq!(row.transcript, "Existing file transcript");
        assert_eq!(row.audio_source.as_deref(), Some("old.mp3"));
        assert_eq!(row.model_id.as_deref(), Some("base.en"));
        assert!(row.source_path.is_none());
        assert!(row.import_method.is_none());
    }

    #[test]
    fn import_method_accepts_only_known_values() {
        assert!(matches!(serde_json::from_str::<FileImportMethod>("\"drop\"").unwrap(), FileImportMethod::Drop));
        assert!(matches!(serde_json::from_str::<FileImportMethod>("\"picker\"").unwrap(), FileImportMethod::Picker));
        assert!(serde_json::from_str::<FileImportMethod>("\"made_up\"").is_err());
    }

    #[test]
    fn concurrent_history_initialization_migrates_once_without_losing_rows() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!("taurscribe-history-race-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("history.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE transcriptions (id INTEGER PRIMARY KEY, created_at TEXT NOT NULL, transcript TEXT NOT NULL, engine TEXT NOT NULL, duration_ms INTEGER, grammar_llm_used INTEGER NOT NULL);
                INSERT INTO transcriptions VALUES (1, '2026-09-29T10:00:00+00:00', 'Keep existing words', 'whisper', NULL, 0);").unwrap();
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let workers: Vec<_> = (0..4).map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let conn = Connection::open(path).unwrap();
                barrier.wait();
                init_history_schema(&conn).unwrap();
                let rows = query_history(&conn, 25, 0, None).unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].transcript, "Keep existing words");
                assert!(rows[0].source_path.is_none());
                assert!(rows[0].import_method.is_none());
            })
        }).collect();
        for worker in workers { worker.join().unwrap(); }
        std::fs::remove_dir_all(directory).unwrap();
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
