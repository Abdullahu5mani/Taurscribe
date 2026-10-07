//! Local endpoint for the `taurscribe` command-line tool (see `cli.rs`).
//!
//! Security model:
//! 1. **Bind** — `127.0.0.1` on a random port, never an external interface.
//! 2. **Token** — a fresh random token per app launch, written with the port to
//!    `cli-endpoint.json` in the app data dir (owner-only on Unix). Every request
//!    must carry it, so only processes that can read the user's app data can connect.
//! 3. **Surface** — status, dictation start/stop (the same events the hotkey emits,
//!    so the frontend's guards still apply) and file transcription. Nothing else.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::state::AudioState;
use crate::types::ASREngine;

/// One request per connection: a single JSON line.
#[derive(Deserialize)]
pub struct CliRequest {
    pub token: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Value,
}

/// One reply per request: a single JSON line.
#[derive(Serialize, Deserialize)]
pub struct CliResponse {
    pub ok: bool,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub error: Option<String>,
}

/// Where the running app advertises its port and token.
#[derive(Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

/// Same folder as settings.json (Tauri's app data dir for the `taurscribe` identifier).
pub fn endpoint_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("taurscribe").join("cli-endpoint.json"))
}

const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// Set once the frontend has registered its hotkey listeners; before that a
/// dictation event from the CLI would be emitted to nobody.
static UI_LISTENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The frontend is loading a model or finishing the last dictation (transcript,
/// clean-up, paste), when it ignores a dictation start. True until the
/// frontend first reports it.
static UI_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[tauri::command]
pub fn cli_ui_ready() {
    UI_LISTENING.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
pub fn cli_ui_busy(busy: bool) {
    UI_BUSY.store(busy, std::sync::atomic::Ordering::Relaxed);
}

/// The UI can take a dictation event right now. A recording starting or
/// stopping counts as busy too: it covers the moment between a stop and the
/// frontend reporting that it is processing the transcript.
fn ui_ready(state: &AudioState) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    UI_LISTENING.load(Relaxed) && !UI_BUSY.load(Relaxed) && !state.recording_transition_active()
}

#[tauri::command]
pub fn cli_install_status() -> crate::cli::InstallStatus {
    crate::cli::install_status()
}

#[tauri::command]
pub async fn install_cli() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(crate::cli::install)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn uninstall_cli() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(crate::cli::uninstall)
        .await
        .map_err(|e| e.to_string())?
}

fn new_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_endpoint(ep: &Endpoint) -> Result<(), String> {
    let path = endpoint_path().ok_or("no data directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string(ep).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        // An older file may predate the 0600 mode; tighten it either way.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(())
}

/// Removes the endpoint file if it still belongs to this process.
pub fn remove_endpoint() {
    let Some(path) = endpoint_path() else { return };
    let mine = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Endpoint>(&s).ok())
        .is_some_and(|ep| ep.pid == std::process::id());
    if mine {
        let _ = std::fs::remove_file(path);
    }
}

pub fn spawn_cli_server(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[CLI] Could not open the command-line endpoint: {e}");
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(a) => a.port(),
            Err(e) => {
                eprintln!("[CLI] Could not read the endpoint address: {e}");
                return;
            }
        };
        let token = new_token();
        if let Err(e) = write_endpoint(&Endpoint { port, token: token.clone(), pid: std::process::id() }) {
            eprintln!("[CLI] Could not write the endpoint file: {e}");
            return;
        }
        println!("[CLI] Command-line endpoint on 127.0.0.1:{port}");
        loop {
            let Ok((sock, _)) = listener.accept().await else { continue };
            let app = app.clone();
            let token = token.clone();
            tauri::async_runtime::spawn(async move {
                let (rd, mut wr) = sock.into_split();
                let mut line = String::new();
                let mut reader = BufReader::new(tokio::io::AsyncReadExt::take(rd, MAX_REQUEST_BYTES));
                let reply = match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => match serde_json::from_str::<CliRequest>(&line) {
                        Ok(req) if crate::control_server::constant_time_eq(req.token.as_bytes(), token.as_bytes()) => {
                            // A disconnect may be observed after the worker has
                            // exited and another client has started the same path.
                            let job_id = format!("cli-{:032x}", rand::random::<u128>());
                            let work = handle(&app, &req.cmd, &req.args, &job_id);
                            tokio::pin!(work);
                            let mut rd = reader.into_inner().into_inner();
                            // Polling the job first registers its cancel flag before a
                            // closed connection is noticed.
                            tokio::select! {
                                biased;
                                reply = &mut work => reply,
                                () = client_gone(&mut rd) => {
                                    abandon(&app, &req, &job_id, work).await;
                                    return;
                                }
                            }
                        }
                        Ok(_) => Err("invalid token".to_string()),
                        Err(e) => Err(format!("bad request: {e}")),
                    },
                };
                let resp = match reply {
                    Ok(data) => CliResponse { ok: true, data, error: None },
                    Err(e) => CliResponse { ok: false, data: Value::Null, error: Some(e) },
                };
                if let Ok(mut s) = serde_json::to_string(&resp) {
                    s.push('\n');
                    let _ = wr.write_all(s.as_bytes()).await;
                }
            });
        }
    });
}

/// Resolves when the CLI closes its end (it exited, e.g. on Ctrl-C).
async fn client_gone(rd: &mut tokio::net::tcp::OwnedReadHalf) {
    let mut buf = [0u8; 64];
    while let Ok(n) = tokio::io::AsyncReadExt::read(rd, &mut buf).await {
        if n == 0 {
            return;
        }
    }
}

/// The CLI went away mid-request: cancel its file job instead of transcribing
/// for nobody (and holding up the next job and dictation), then finish the
/// batch it was part of so "unload after each use" still unloads.
async fn abandon(app: &AppHandle, req: &CliRequest, job_id: &str, work: std::pin::Pin<&mut impl std::future::Future<Output = Result<Value, String>>>) {
    if req.cmd != "transcribe" {
        let _ = work.await;
        return;
    }
    if let Some(path) = req.args.get("path").and_then(Value::as_str) {
        let _ = crate::commands::cancel_file_transcription(path.to_string()).await;
    }
    let _ = work.await;
    crate::commands::maybe_unload_after_file_batch(app, &app.state::<AudioState>());
}

fn engine_name(e: &ASREngine) -> &'static str {
    match e {
        ASREngine::Whisper => "whisper",
        ASREngine::Granite => "granite",
        ASREngine::Qwen3 => "qwen3",
    }
}

fn is_recording(state: &AudioState) -> bool {
    state.recording_handle.lock().map(|h| h.is_some()).unwrap_or(false)
}

fn status(app: &AppHandle) -> Value {
    use std::sync::atomic::Ordering;
    let state = app.state::<AudioState>();
    let engine = state.active_engine.lock().map(|e| e.clone()).unwrap_or(ASREngine::Whisper);
    let (model, backend) = match engine {
        ASREngine::Whisper => {
            let w = state.whisper_snapshot();
            (w.model, w.backend)
        }
        other => match state.gguf_snapshot(other) {
            Some(g) if g.loaded => (g.model_id, g.backend),
            Some(g) => (None, g.backend),
            None => (None, String::new()),
        },
    };
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "recording": is_recording(&state),
        // A meeting recording, call or in person (saved to Meetings, not dictation history).
        "meeting": is_recording(&state)
            && (state.last_recording_is_dual_channel.load(Ordering::SeqCst)
                || state.last_recording_is_in_person.load(Ordering::SeqCst)),
        "paused": state.recording_paused.load(Ordering::Relaxed),
        "engine": engine_name(&engine),
        "model": model,
        "backend": if backend.is_empty() { Value::Null } else { Value::String(backend) },
        "engine_loading": state.engine_loading.load(Ordering::Relaxed),
        "ui_ready": ui_ready(&state),
    })
}

/// One value from the frontend's settings.json.
fn setting(key: &str) -> Option<Value> {
    crate::mcp_server::settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|mut v| v.get_mut(key).map(Value::take))
}

/// The Settings → Models "GPU / CPU" choice, as the frontend passes it to file jobs.
fn use_gpu_setting() -> bool {
    setting("asr_backend").and_then(|v| v.as_str().map(|s| s != "cpu")).unwrap_or(true)
}

/// The model a CLI file job should use: the one loaded now, else the one
/// selected in Settings. Without one, a job after auto-unload would load a
/// default model instead of the user's.
fn cli_model(state: &AudioState, engine: ASREngine) -> Option<String> {
    let loaded = match engine {
        ASREngine::Whisper => state.whisper_snapshot().model,
        other => state.gguf_snapshot(other).filter(|s| s.loaded).and_then(|s| s.model_id),
    };
    let key = match engine {
        ASREngine::Whisper => "whisper_model",
        ASREngine::Granite => "granite_model",
        ASREngine::Qwen3 => "qwen3_model",
    };
    loaded.or_else(|| setting(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string))
}

async fn handle(app: &AppHandle, cmd: &str, args: &Value, job_id: &str) -> Result<Value, String> {
    match cmd {
        "status" => Ok(status(app)),
        "benchmark" => {
            let result = crate::commands::benchmark_compute_devices(app.state::<AudioState>(), app.clone()).await?;
            match (result.data, result.error) {
                (Some(report), _) => serde_json::to_value(report).map_err(|e| e.to_string()),
                (None, Some(e)) => Err(e.message),
                (None, None) => Err("the speed test failed".into()),
            }
        }
        "dictate" => {
            if !ui_ready(&app.state::<AudioState>()) {
                return Err("Taurscribe is busy (starting, loading a model, or finishing the last dictation); try again in a moment".into());
            }
            let action = args.get("action").and_then(Value::as_str).unwrap_or("toggle");
            let recording = is_recording(&app.state::<AudioState>());
            // start_recording refuses during a file job; say so now rather than
            // leaving the CLI to wait for a recording that never starts.
            if !recording && action != "stop" && crate::commands::file_job_active() {
                return Err("Taurscribe is transcribing a file; try again when it finishes".into());
            }
            let event = match action {
                "start" if recording => return Err("already recording".into()),
                "stop" if !recording => return Err("not recording".into()),
                "start" => "hotkey-start-recording",
                "stop" => "hotkey-stop-recording",
                "toggle" => "hotkey-toggle-recording",
                other => return Err(format!("unknown dictate action: {other}")),
            };
            app.emit(event, ()).map_err(|e| e.to_string())?;
            Ok(json!({ "sent": event, "was_recording": recording }))
        }
        "transcribe" => {
            let path = args.get("path").and_then(Value::as_str).ok_or("`path` is required")?.to_string();
            if !std::path::Path::new(&path).is_file() {
                return Err(format!("no such file: {path}"));
            }
            let state = app.state::<AudioState>();
            let engine = state.active_engine.lock().map(|e| *e).unwrap_or(ASREngine::Whisper);
            let model = cli_model(&state, engine);
            // More files follow in this CLI batch: don't auto-unload in between.
            let defer_unload = args.get("defer_unload").and_then(Value::as_bool).unwrap_or(false);
            let res = crate::commands::transcribe_file(
                app.clone(),
                state,
                path,
                Some(engine_name(&engine).to_string()),
                model,
                Some(job_id.to_string()),
                Some(defer_unload),
                Some(use_gpu_setting()),
            )
            .await?;
            serde_json::to_value(res).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown command: {other}")),
    }
}
