//! Read-only MCP (Model Context Protocol) server over stdio: lets an LLM app
//! (Claude Desktop, Claude Code, Cursor, LM Studio, …) search and read
//! Taurscribe's dictations, file transcripts, meetings and Speaker Vault names.
//!
//! Started as `taurscribe mcp` by the LLM app, not by the user. It opens the
//! database read-only and never writes. It only answers while "LLM access" is on
//! in Settings → App (`mcp_enabled` in settings.json); otherwise every tool call
//! explains how to turn it on.
//!
//! Protocol: JSON-RPC 2.0, one message per line on stdin/stdout
//! (MCP stdio transport). Implements initialize, ping, tools/list, tools/call.

use std::io::{BufRead, Write};

use rusqlite::{params_from_iter, Connection, OpenFlags};
use serde_json::{json, Value};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;

/// settings.json key the Settings toggle writes.
pub const ENABLED_KEY: &str = "mcp_enabled";

/// Runs the server until stdin closes.
pub fn run() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    // Notifications (no id) get no reply.
    let id = id?;
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
            ok(id, json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "taurscribe", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Taurscribe transcripts on this computer (read-only). Use `search` to find \
                    dictations, file transcripts and meetings by words, `list_meetings` / `list_transcripts` \
                    to browse by date, then `get_meeting` / `get_transcript` for full text. Dates are ISO \
                    (YYYY-MM-DD) in UTC.",
            }))
        }
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tool_list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let result = if !enabled() {
                Err("LLM access to Taurscribe is turned off. Turn on Settings → App → LLM access (MCP) in \
                     Taurscribe, then try again."
                    .to_string())
            } else {
                open_db().and_then(|db| call_tool(&db, name, &args))
            };
            match result {
                Ok(text) => ok(id, json!({ "content": [{ "type": "text", "text": text }] })),
                Err(e) => ok(id, json!({ "content": [{ "type": "text", "text": e }], "isError": true })),
            }
        }
        _ => error(id, -32601, &format!("method not found: {method}")),
    })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

// ── settings & database ─────────────────────────────────────────────────────

/// Tauri's store keeps settings.json in the app data dir (data dir / identifier).
pub fn settings_path() -> Option<std::path::PathBuf> {
    Some(dirs::data_dir()?.join("taurscribe").join("settings.json"))
}

fn enabled() -> bool {
    settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get(ENABLED_KEY).and_then(Value::as_bool))
        .unwrap_or(false)
}

fn open_db() -> Result<Connection, String> {
    let path = dirs::data_local_dir()
        .ok_or("no data directory")?
        .join("Taurscribe")
        .join("transcript_history.db");
    if !path.exists() {
        return Err("No Taurscribe transcripts yet.".into());
    }
    Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("could not open the Taurscribe database: {e}"))
}

// ── tools ───────────────────────────────────────────────────────────────────

fn tool_list() -> Value {
    let date = |what: &str| json!({ "type": "string", "description": format!("{what} date, YYYY-MM-DD (UTC), inclusive") });
    let limit = json!({ "type": "integer", "description": format!("Max results (default {DEFAULT_LIMIT}, max {MAX_LIMIT})") });
    let read_only = json!({ "readOnlyHint": true, "openWorldHint": false });
    json!([
        {
            "name": "search",
            "description": "Find dictations, file transcripts and meetings containing words. Returns ids, dates and \
                matching snippets; use get_meeting / get_transcript for the full text.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string", "description": "Words to find (all must appear, any case)" },
                "kind": { "type": "string", "enum": ["dictation", "file", "meeting"], "description": "Only this kind" },
                "from": date("Earliest"), "to": date("Latest"), "limit": limit.clone(),
            }, "required": ["query"] },
            "annotations": read_only.clone(),
        },
        {
            "name": "list_meetings",
            "description": "Recorded meetings, newest first: id, title, app, date, length, who spoke.",
            "inputSchema": { "type": "object", "properties": {
                "from": date("Earliest"), "to": date("Latest"),
                "person": { "type": "string", "description": "Only meetings where this speaker name appears" },
                "limit": limit.clone(),
            } },
            "annotations": read_only.clone(),
        },
        {
            "name": "get_meeting",
            "description": "One meeting in full: title, date, app, summary, action items and the speaker-labelled \
                transcript with times. 'You' is the person using Taurscribe.",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "integer", "description": "Meeting id from search or list_meetings" },
            }, "required": ["id"] },
            "annotations": read_only.clone(),
        },
        {
            "name": "list_transcripts",
            "description": "Dictations (spoken into the mic) and file transcripts (imported audio), newest first.",
            "inputSchema": { "type": "object", "properties": {
                "kind": { "type": "string", "enum": ["dictation", "file"] },
                "from": date("Earliest"), "to": date("Latest"), "limit": limit.clone(),
            } },
            "annotations": read_only.clone(),
        },
        {
            "name": "get_transcript",
            "description": "One dictation or file transcript in full.",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "integer", "description": "Transcript id from search or list_transcripts" },
            }, "required": ["id"] },
            "annotations": read_only.clone(),
        },
        {
            "name": "list_people",
            "description": "People in the Speaker Vault (named callers recognised by voice across meetings), with \
                how many meetings each was in and the latest ones.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": read_only,
        },
    ])
}

fn call_tool(db: &Connection, name: &str, args: &Value) -> Result<String, String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let id = || args.get("id").and_then(Value::as_i64).ok_or("`id` is required".to_string());
    match name {
        "search" => search(db, &s("query").ok_or("`query` is required")?, s("kind").as_deref(), s("from"), s("to"), limit),
        "list_meetings" => list_meetings(db, s("from"), s("to"), s("person"), limit),
        "get_meeting" => get_meeting(db, id()?),
        "list_transcripts" => list_transcripts(db, s("kind").as_deref(), s("from"), s("to"), limit),
        "get_transcript" => get_transcript(db, id()?),
        "list_people" => list_people(db),
        _ => Err(format!("unknown tool: {name}")),
    }
}

/// `created_at` bounds for ISO dates ("to" includes that whole day).
fn date_bounds(from: &Option<String>, to: &Option<String>) -> (String, String) {
    (from.clone().unwrap_or_default(), to.as_ref().map(|t| format!("{t}\u{10FFFF}")).unwrap_or_else(|| "\u{10FFFF}".into()))
}

/// Dictations are recorded from the mic; anything else came from a file.
/// Derived rather than read from the `kind` column: the server opens the DB
/// read-only and may meet one the app has not migrated yet.
const KIND_SQL: &str = crate::commands::KIND_FROM_SOURCE_SQL;

fn like_all(column: &str, words: usize) -> String {
    (0..words).map(|_| format!("{column} LIKE ? ESCAPE '\\'")).collect::<Vec<_>>().join(" AND ")
}

fn like_arg(word: &str) -> String {
    format!("%{}%", word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

/// ~160 characters around the first match.
fn snippet(text: &str, word: &str) -> String {
    let lower = text.to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let at = lower.find(&word.to_lowercase()).map(|b| lower[..b].chars().count()).unwrap_or(0);
    let start = at.saturating_sub(60);
    let end = (at + 100).min(chars.len());
    let mut s: String = chars[start..end].iter().collect();
    if start > 0 {
        s.insert(0, '…');
    }
    if end < chars.len() {
        s.push('…');
    }
    s.replace('\n', " ")
}

fn search(db: &Connection, query: &str, kind: Option<&str>, from: Option<String>, to: Option<String>, limit: i64) -> Result<String, String> {
    let words: Vec<&str> = query.split_whitespace().collect();
    let (lo, hi) = date_bounds(&from, &to);
    let mut out = Vec::new();

    if kind != Some("meeting") {
        let sql = format!(
            "SELECT id, created_at, {KIND_SQL} AS kind, transcript, audio_source FROM transcriptions \
             WHERE {} AND created_at >= ? AND created_at <= ? {} ORDER BY created_at DESC LIMIT ?",
            like_all("transcript", words.len()),
            match kind { Some("dictation") => format!("AND {KIND_SQL} = 'dictation'"), Some("file") => format!("AND {KIND_SQL} = 'file'"), _ => String::new() },
        );
        let mut args: Vec<rusqlite::types::Value> = words.iter().map(|w| like_arg(w).into()).collect();
        args.extend([lo.clone().into(), hi.clone().into(), limit.into()]);
        let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params_from_iter(args), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?)))
            .map_err(|e| e.to_string())?;
        for (id, at, k, text, src) in rows.filter_map(Result::ok) {
            let from_file = if k == "file" { format!(" · {}", src.unwrap_or_default()) } else { String::new() };
            out.push(format!("- {k} #{id} · {}{from_file}\n  {}", &at[..16.min(at.len())], snippet(&text, words[0])));
        }
    }

    if kind.is_none() || kind == Some("meeting") {
        // A meeting matches when all words appear in its title or transcript.
        let hay = "(title || ' ' || transcript_raw)";
        let sql = format!(
            "SELECT id, created_at, title, platform, transcript_raw FROM meetings \
             WHERE {} AND created_at >= ? AND created_at <= ? ORDER BY created_at DESC LIMIT ?",
            like_all(hay, words.len())
        );
        let mut args: Vec<rusqlite::types::Value> = words.iter().map(|w| like_arg(w).into()).collect();
        args.extend([lo.into(), hi.into(), limit.into()]);
        let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params_from_iter(args), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))
            .map_err(|e| e.to_string())?;
        for (id, at, title, platform, text) in rows.filter_map(Result::ok) {
            out.push(format!("- meeting #{id} · {} · {title} ({platform})\n  {}", &at[..16.min(at.len())], snippet(&text, words[0])));
        }
    }

    Ok(if out.is_empty() { format!("Nothing matched “{query}”.") } else { format!("{} result(s) for “{query}”:\n{}", out.len(), out.join("\n")) })
}

fn list_meetings(db: &Connection, from: Option<String>, to: Option<String>, person: Option<String>, limit: i64) -> Result<String, String> {
    let (lo, hi) = date_bounds(&from, &to);
    let person_sql = if person.is_some() {
        "AND id IN (SELECT meeting_id FROM meeting_turns WHERE lower(speaker_name) = lower(?))"
    } else {
        ""
    };
    let sql = format!(
        "SELECT id, created_at, title, platform, app_name, duration_ms FROM meetings \
         WHERE created_at >= ? AND created_at <= ? {person_sql} ORDER BY created_at DESC LIMIT ?"
    );
    let mut args: Vec<rusqlite::types::Value> = vec![lo.into(), hi.into()];
    if let Some(p) = &person {
        args.push(p.clone().into());
    }
    args.push(limit.into());
    let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, String, String, String, i64)> = stmt
        .query_map(params_from_iter(args), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    if rows.is_empty() {
        return Ok("No meetings found.".into());
    }
    let mut out = vec![format!("{} meeting(s):", rows.len())];
    for (id, at, title, platform, app, ms) in rows {
        out.push(format!(
            "- #{id} · {} · {title} · {platform} in {app} · {} · speakers: {}",
            &at[..16.min(at.len())],
            clock(ms),
            speakers(db, id).join(", ")
        ));
    }
    Ok(out.join("\n"))
}

fn speakers(db: &Connection, meeting_id: i64) -> Vec<String> {
    db.prepare("SELECT speaker_name FROM meeting_turns WHERE meeting_id = ? GROUP BY speaker_name ORDER BY MIN(start_ms)")
        .and_then(|mut s| s.query_map([meeting_id], |r| r.get::<_, String>(0)).map(|rows| rows.filter_map(Result::ok).collect()))
        .unwrap_or_default()
}

fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn get_meeting(db: &Connection, id: i64) -> Result<String, String> {
    let (at, title, platform, app, url, ms, summary, actions): (String, String, String, String, String, i64, Option<String>, Option<String>) = db
        .query_row(
            "SELECT created_at, title, platform, app_name, url, duration_ms, summary, action_items FROM meetings WHERE id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )
        .map_err(|_| format!("No meeting #{id}."))?;
    let mut out = vec![format!("# {title}"), format!("Meeting #{id} · {} UTC · {platform} in {app} · {}", &at[..16.min(at.len())], clock(ms))];
    if !url.is_empty() {
        out.push(format!("Link: {url}"));
    }
    out.push(format!("Speakers: {}", speakers(db, id).join(", ")));
    let points: Vec<String> = summary.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if !points.is_empty() {
        out.push("\n## Summary".into());
        out.extend(points.iter().map(|p| format!("- {p}")));
    }
    let items: Vec<Value> = actions.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if !items.is_empty() {
        out.push("\n## Action items".into());
        for a in items {
            let task = a.get("task").and_then(Value::as_str).unwrap_or("");
            let who = a.get("assignee").and_then(Value::as_str).filter(|w| !w.is_empty()).map(|w| format!(" ({w})")).unwrap_or_default();
            out.push(format!("- {task}{who}"));
        }
    }
    out.push("\n## Transcript".into());
    let mut stmt = db
        .prepare("SELECT start_ms, speaker_name, text FROM meeting_turns WHERE meeting_id = ? ORDER BY start_ms")
        .map_err(|e| e.to_string())?;
    let turns: Vec<(i64, String, String)> = stmt
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|t: &(i64, String, String)| !t.2.trim().is_empty())
        .collect();
    if turns.is_empty() {
        let raw: String = db.query_row("SELECT transcript_raw FROM meetings WHERE id = ?", [id], |r| r.get(0)).unwrap_or_default();
        out.push(raw);
    } else {
        out.extend(turns.into_iter().map(|(t, who, text)| format!("[{}] {who}: {}", clock(t), text.trim())));
    }
    Ok(out.join("\n"))
}

fn list_transcripts(db: &Connection, kind: Option<&str>, from: Option<String>, to: Option<String>, limit: i64) -> Result<String, String> {
    let (lo, hi) = date_bounds(&from, &to);
    let kind_sql = match kind {
        Some("dictation") => format!("AND {KIND_SQL} = 'dictation'"),
        Some("file") => format!("AND {KIND_SQL} = 'file'"),
        _ => String::new(),
    };
    let sql = format!(
        "SELECT id, created_at, {KIND_SQL}, audio_source, engine, transcript FROM transcriptions \
         WHERE created_at >= ? AND created_at <= ? {kind_sql} ORDER BY created_at DESC LIMIT ?"
    );
    let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, String, Option<String>, String, String)> = stmt
        .query_map(rusqlite::params![lo, hi, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    if rows.is_empty() {
        return Ok("No transcripts found.".into());
    }
    let mut out = vec![format!("{} transcript(s):", rows.len())];
    for (id, at, k, src, engine, text) in rows {
        let file = if k == "file" { format!(" · {}", src.unwrap_or_default()) } else { String::new() };
        out.push(format!("- {k} #{id} · {}{file} · {engine}\n  {}", &at[..16.min(at.len())], snippet(&text, "")));
    }
    Ok(out.join("\n"))
}

fn get_transcript(db: &Connection, id: i64) -> Result<String, String> {
    let (at, kind, src, engine, model, ms, text): (String, String, Option<String>, String, Option<String>, Option<i64>, String) = db
        .query_row(
            &format!("SELECT created_at, {KIND_SQL}, audio_source, engine, model_id, duration_ms, transcript FROM transcriptions WHERE id = ?"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .map_err(|_| format!("No transcript #{id}."))?;
    let source = if kind == "file" { format!("from file {}", src.unwrap_or_default()) } else { "dictated into the microphone".into() };
    let length = ms.map(|m| format!(" · {}", clock(m))).unwrap_or_default();
    Ok(format!(
        "Transcript #{id} · {} UTC · {source}{length} · {engine}{}\n\n{}",
        &at[..16.min(at.len())],
        model.map(|m| format!(" ({m})")).unwrap_or_default(),
        text.trim()
    ))
}

fn list_people(db: &Connection) -> Result<String, String> {
    let mut stmt = db
        .prepare("SELECT id, name, meeting_count, last_seen FROM speaker_vault ORDER BY last_seen DESC")
        .map_err(|e| e.to_string())?;
    let people: Vec<(String, String, i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    if people.is_empty() {
        return Ok("The Speaker Vault is empty (people are added when you name a speaker in a meeting).".into());
    }
    let mut out = vec![format!("{} people:", people.len())];
    for (pid, name, count, seen) in people {
        let recent: Vec<String> = db
            .prepare(
                "SELECT m.id, m.title FROM meetings m WHERE m.id IN \
                 (SELECT meeting_id FROM meeting_turns WHERE speaker_id = ?) ORDER BY m.created_at DESC LIMIT 3",
            )
            .and_then(|mut s| s.query_map([&pid], |r| Ok(format!("#{} {}", r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).map(|rows| rows.filter_map(Result::ok).collect()))
            .unwrap_or_default();
        out.push(format!(
            "- {name} · {count} meeting(s) · last seen {}{}",
            &seen[..10.min(seen.len())],
            if recent.is_empty() { String::new() } else { format!(" · recent: {}", recent.join("; ")) }
        ));
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&c).unwrap();
        c.execute_batch(
            "CREATE TABLE transcriptions (id INTEGER PRIMARY KEY, created_at TEXT, transcript TEXT, engine TEXT,
               duration_ms INTEGER, grammar_llm_used INTEGER, processing_time_ms INTEGER, model_id TEXT, audio_source TEXT);
             INSERT INTO transcriptions VALUES (1,'2026-09-20T10:00:00+00:00','Remember to email the budget to Sarah','qwen3',4000,0,0,NULL,'microphone');
             INSERT INTO transcriptions VALUES (2,'2026-09-21T10:00:00+00:00','Lecture about budget planning','whisper',60000,0,0,NULL,'lecture.mp3');
             INSERT INTO meetings (id, session_id, title, platform, app_name, url, created_at, duration_ms, transcript_raw, category, speaker_count)
               VALUES (7,'s','Budget sync','meet','Google Chrome','','2026-09-22T09:00:00+00:00',600000,'We agreed the budget is final',
               'general',2);
             INSERT INTO meeting_turns (meeting_id, speaker_id, speaker_name, start_ms, end_ms, channel, text)
               VALUES (7,'speaker_you','You',0,3000,0,'Is the budget final?'),(7,'person_1','Sarah',3500,6000,1,'Yes, the budget is final.');
             INSERT INTO speaker_vault (id, name, created_at, last_seen) VALUES ('person_1','Sarah','','2026-09-22');",
        )
        .unwrap();
        c
    }

    #[test]
    fn search_finds_every_kind_and_filters() {
        let d = db();
        let all = search(&d, "budget", None, None, None, 20).unwrap();
        assert!(all.contains("dictation #1") && all.contains("file #2") && all.contains("meeting #7"), "{all}");
        let files = search(&d, "budget", Some("file"), None, None, 20).unwrap();
        assert!(files.contains("file #2") && !files.contains("dictation #1") && !files.contains("meeting"), "{files}");
        let dated = search(&d, "budget", None, Some("2026-09-21".into()), Some("2026-09-21".into()), 20).unwrap();
        assert!(dated.contains("file #2") && !dated.contains("#1 ") && !dated.contains("meeting"), "{dated}");
        assert!(search(&d, "budget sarah", None, None, None, 20).unwrap().contains("dictation #1"));
        assert!(search(&d, "100%", None, None, None, 20).unwrap().starts_with("Nothing"));
    }

    #[test]
    fn meeting_reads_as_a_labelled_transcript() {
        let d = db();
        let m = get_meeting(&d, 7).unwrap();
        assert!(m.contains("# Budget sync") && m.contains("[0:03] Sarah: Yes, the budget is final."), "{m}");
        assert!(list_meetings(&d, None, None, Some("sarah".into()), 20).unwrap().contains("#7"));
        assert!(list_people(&d).unwrap().contains("Sarah · 0 meeting(s)"));
    }

    #[test]
    fn protocol_basics() {
        let init = handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        let tools = handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 6);
    }
}
