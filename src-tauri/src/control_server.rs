//! Localhost-only control server for deterministic development, simulation and E2E testing.
//!
//! Security model:
//! 1. **Bind** — strictly `127.0.0.1`. Never exposed on external network interfaces.
//! 2. **Test Mode** — the server only starts with `TAURSCRIBE_TEST_MODE=1`.
//! 3. **Bearer Token** — every route requires an explicitly configured token.

use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode, Request},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

use crate::meeting_detector::MeetingInfo;
use crate::state::AudioState;

#[derive(Clone)]
pub struct ServerState {
    pub app_handle: AppHandle,
}

pub fn is_test_mode() -> bool {
    std::env::var("TAURSCRIBE_TEST_MODE").as_deref() == Ok("1")
}

pub fn control_token() -> Option<String> {
    std::env::var("TAURSCRIBE_CONTROL_TOKEN").ok().filter(|token| !token.is_empty())
}

pub fn control_port() -> u16 {
    std::env::var("TAURSCRIBE_CONTROL_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8766)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (l, r)| acc | (l ^ r))
        == 0
}

fn authorize_control(headers: &HeaderMap) -> Option<(StatusCode, &'static str)> {
    if !is_test_mode() {
        return Some((StatusCode::FORBIDDEN, "TAURSCRIBE_TEST_MODE=1 is required"));
    }

    let Some(expected) = control_token() else {
        return Some((StatusCode::FORBIDDEN, "TAURSCRIBE_CONTROL_TOKEN is required"));
    };
    authorize_bearer(headers, &expected)
}

fn authorize_bearer(headers: &HeaderMap, expected: &str) -> Option<(StatusCode, &'static str)> {
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or_default();

    if presented.is_empty() {
        return Some((StatusCode::UNAUTHORIZED, "missing bearer token"));
    }
    if !constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        return Some((StatusCode::UNAUTHORIZED, "invalid bearer token"));
    }
    None
}

async fn require_control_token(request: Request<axum::body::Body>, next: Next) -> impl IntoResponse {
    if let Some((status, message)) = authorize_control(request.headers()) {
        return (status, message).into_response();
    }
    next.run(request).await
}

#[derive(Serialize)]
struct ActionResponse {
    success: bool,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

fn success_response(detail: impl Into<String>, data: Option<serde_json::Value>) -> (StatusCode, Json<ActionResponse>) {
    (
        StatusCode::OK,
        Json(ActionResponse {
            success: true,
            detail: detail.into(),
            data,
        }),
    )
}

fn error_response(status: StatusCode, detail: impl Into<String>) -> (StatusCode, Json<ActionResponse>) {
    (
        status,
        Json(ActionResponse {
            success: false,
            detail: detail.into(),
            data: None,
        }),
    )
}

// ── Read Handlers ────────────────────────────────────────────────────────────

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "app": "Taurscribe",
        "version": env!("CARGO_PKG_VERSION"),
        "test_mode": is_test_mode(),
        "mutations_enabled": is_test_mode(),
        "port": control_port(),
        "pid": std::process::id(),
    }))
}

async fn status_handler(State(state): State<ServerState>) -> impl IntoResponse {
    let audio_state = state.app_handle.state::<AudioState>();
    let is_recording = audio_state.recording_handle.lock().unwrap().is_some();
    let is_paused = audio_state.recording_paused.load(Ordering::Relaxed);
    let active_engine = format!("{:?}", *audio_state.active_engine.lock().unwrap());
    let is_dual_channel = audio_state.last_recording_is_dual_channel.load(Ordering::SeqCst);
    let detector = audio_state.meeting_detector.get_status();
    let scanned = audio_state.meeting_detector.scan();

    Json(serde_json::json!({
        "recording": {
            "is_recording": is_recording,
            "is_paused": is_paused,
            "active_engine": active_engine,
            "is_dual_channel": is_dual_channel,
        },
        "detector": {
            "is_watching": detector.is_watching,
            "active_meetings": detector.active_meetings,
            "scanned_meetings": scanned,
        },
        "capture": crate::audio_dual_channel::capture_diagnostics(),
        "processing": crate::meeting_audio::last_processing_diagnostics(),
        "voiceprints": {
            // "neural": people are recognised by voice; "fallback": by name only.
            "engine": if crate::speaker_embedding::get_speaker_engine().lock().map(|e| e.uses_neural_model()).unwrap_or(false) { "neural" } else { "fallback" },
        },
    }))
}

async fn scan_handler(State(state): State<ServerState>) -> impl IntoResponse {
    let audio_state = state.app_handle.state::<AudioState>();
    let scanned = audio_state.meeting_detector.scan();
    Json(serde_json::json!({
        "scanned_meetings": scanned,
    }))
}

async fn meetings_handler() -> impl IntoResponse {
    match crate::commands::meetings::list_meetings(None, None, None, Some(100), Some(0)).await {
        Ok(res) => (StatusCode::OK, Json(serde_json::json!(res))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))),
    }
}

async fn meeting_detail_handler(Path(id): Path<i64>) -> impl IntoResponse {
    match crate::commands::meetings::get_meeting_detail(id).await {
        Ok(res) => (StatusCode::OK, Json(serde_json::json!(res))),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": e }))),
    }
}

async fn vault_handler() -> impl IntoResponse {
    match crate::commands::meetings::list_speaker_vault().await {
        Ok(speakers) => (StatusCode::OK, Json(serde_json::json!(speakers))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))),
    }
}

// ── Mutation Handlers ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SimulateEventBody {
    title: Option<String>,
    platform: Option<String>,
    app_name: Option<String>,
    url: Option<String>,
}

async fn simulate_event_handler(
    Path(event): Path<String>,
    headers: HeaderMap,
    State(state): State<ServerState>,
    body: Option<Json<SimulateEventBody>>,
) -> impl IntoResponse {
    if let Some((status, msg)) = authorize_control(&headers) {
        return error_response(status, msg);
    }

    let audio_state = state.app_handle.state::<AudioState>();
    match event.as_str() {
        "meeting-join" => {
            let b = body.map(|b| b.0).unwrap_or(SimulateEventBody {
                title: None,
                platform: None,
                app_name: None,
                url: None,
            });
            let info = MeetingInfo {
                pid: 99999,
                app_name: b.app_name.unwrap_or_else(|| "Google Chrome".into()),
                title: b.title.unwrap_or_else(|| "Architecture Review - Google Meet".into()),
                url: b.url.unwrap_or_else(|| "https://meet.google.com/test-sync-123".into()),
                platform: b.platform.unwrap_or_else(|| "meet".into()),
                confidence: 95,
                should_record: true,
                is_using_mic: true,
                is_playing_audio: true,
            };
            audio_state.meeting_detector.inject_test_meeting(&state.app_handle, info.clone());
            success_response("Simulated meeting joined", Some(serde_json::to_value(info).unwrap()))
        }
        "meeting-leave" => {
            audio_state.meeting_detector.clear_test_meetings(&state.app_handle);
            success_response("Simulated meeting left", None)
        }
        "mute" => success_response("Simulated mute applied", None),
        "unmute" => success_response("Simulated unmute applied", None),
        other => error_response(
            StatusCode::BAD_REQUEST,
            format!("Unknown event '{}'; expected meeting-join, meeting-leave, mute, unmute", other),
        ),
    }
}

#[derive(Deserialize)]
struct CaptureStartBody {
    audio_source: Option<String>,
    denoise: Option<bool>,
}

async fn capture_start_handler(
    headers: HeaderMap,
    State(state): State<ServerState>,
    body: Option<Json<CaptureStartBody>>,
) -> impl IntoResponse {
    if let Some((status, msg)) = authorize_control(&headers) {
        return error_response(status, msg);
    }

    let audio_source = body
        .as_ref()
        .and_then(|b| b.audio_source.clone())
        .or_else(|| Some("dual_channel".into()));
    let denoise = body.as_ref().and_then(|b| b.denoise);

    let audio_state = state.app_handle.state::<AudioState>();
    match crate::commands::start_recording(state.app_handle.clone(), audio_state, denoise, audio_source).await {
        Ok(res) => {
            if res.ok {
                success_response("Capture started", res.data.map(|d| serde_json::json!(d)))
            } else {
                error_response(StatusCode::BAD_REQUEST, res.error.map(|e| e.message).unwrap_or_default())
            }
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn capture_stop_handler(headers: HeaderMap, State(state): State<ServerState>) -> impl IntoResponse {
    if let Some((status, msg)) = authorize_control(&headers) {
        return error_response(status, msg);
    }

    let audio_state = state.app_handle.state::<AudioState>();
    match crate::commands::stop_recording(audio_state, state.app_handle.clone()).await {
        Ok(res) => {
            if res.ok {
                success_response("Capture stopped", res.data.map(|d| serde_json::json!(d)))
            } else {
                error_response(StatusCode::BAD_REQUEST, res.error.map(|e| e.message).unwrap_or_default())
            }
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
struct FeedMeetingBody {
    wav_path: String,
    transcript: String,
    title: Option<String>,
    platform: Option<String>,
    app_name: Option<String>,
}

async fn feed_meeting_handler(
    headers: HeaderMap,
    State(state): State<ServerState>,
    Json(body): Json<FeedMeetingBody>,
) -> impl IntoResponse {
    if let Some((status, msg)) = authorize_control(&headers) {
        return error_response(status, msg);
    }

    let src_path = std::path::Path::new(&body.wav_path);
    if !src_path.exists() {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Audio file '{}' does not exist", body.wav_path),
        );
    }

    let meetings_dir = match crate::commands::meetings::get_meetings_dir() {
        Ok(d) => d,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };

    let meeting_timestamp = chrono::Utc::now().timestamp_millis();
    let dest_filename = format!("meeting_{}.wav", meeting_timestamp);
    let dest_path = meetings_dir.join("audio").join(dest_filename);

    if let Err(e) = std::fs::copy(src_path, &dest_path) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to copy audio: {}", e),
        );
    }

    let snippets_dir = meetings_dir.join("snippets");
    let turns = crate::diarization::diarize_meeting_recording(
        &dest_path,
        &body.transcript,
        &snippets_dir,
        meeting_timestamp,
    );

    let duration_ms = if let Ok(reader) = hound::WavReader::open(&dest_path) {
        let spec = reader.spec();
        let total_samples = reader.duration();
        ((total_samples as f64 / spec.sample_rate as f64) * 1000.0) as i64
    } else {
        turns.last().map(|t| t.end_ms as i64).unwrap_or(1000)
    };

    let audio_path = crate::meeting_audio::keep_playback_copy(&dest_path);

    let title = body.title.unwrap_or_else(|| format!("Simulated Meeting {}", meeting_timestamp));
    let platform = body.platform.unwrap_or_else(|| "meet".into());
    let app_name = body.app_name.unwrap_or_else(|| "Google Chrome".into());

    let conn = match crate::commands::meetings::open_connection() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };

    match crate::commands::meetings::insert_completed_meeting(
        &conn,
        &title,
        &platform,
        &app_name,
        "",
        duration_ms,
        Some(&audio_path.to_string_lossy()),
        &body.transcript,
        &turns,
    ) {
        Ok(id) => {
            use tauri::Emitter;
            let _ = state.app_handle.emit(
                "meeting-processing-complete",
                serde_json::json!({
                    "meeting_id": id,
                    "transcript": body.transcript,
                }),
            );
            success_response(
                "Meeting processed and saved",
                Some(serde_json::json!({
                    "meeting_id": id,
                    "duration_ms": duration_ms,
                    "turns": turns,
                })),
            )
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
struct CycleTurnBody {
    meeting_id: i64,
    speaker_id: String,
}

async fn cycle_turn_snippet_handler(
    headers: HeaderMap,
    Json(body): Json<CycleTurnBody>,
) -> impl IntoResponse {
    if let Some((status, msg)) = authorize_control(&headers) {
        return error_response(status, msg);
    }

    match crate::commands::meetings::cycle_speaker_turn_snippet(body.meeting_id, body.speaker_id).await {
        Ok((path, idx)) => success_response(
            "Cycled turn snippet",
            Some(serde_json::json!({ "snippet_path": path, "candidate_index": idx })),
        ),
        Err(e) => error_response(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Deserialize)]
struct CycleVaultBody {
    speaker_id: String,
}

async fn cycle_vault_snippet_handler(
    headers: HeaderMap,
    Json(body): Json<CycleVaultBody>,
) -> impl IntoResponse {
    if let Some((status, msg)) = authorize_control(&headers) {
        return error_response(status, msg);
    }

    match crate::commands::meetings::cycle_vault_speaker_snippet(body.speaker_id).await {
        Ok((path, idx)) => success_response(
            "Cycled vault snippet",
            Some(serde_json::json!({ "snippet_path": path, "candidate_index": idx })),
        ),
        Err(e) => error_response(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Deserialize)]
struct RenameSpeakerBody {
    meeting_id: i64,
    #[serde(alias = "old_name")]
    speaker_id: String,
    new_name: String,
    update_vault: Option<bool>,
}

async fn rename_speaker_handler(
    headers: HeaderMap,
    Json(body): Json<RenameSpeakerBody>,
) -> impl IntoResponse {
    if let Some((status, msg)) = authorize_control(&headers) {
        return error_response(status, msg);
    }

    let update_vault = body.update_vault.unwrap_or(true);
    match crate::commands::meetings::rename_speaker(
        body.meeting_id,
        body.speaker_id,
        body.new_name,
        update_vault,
    )
    .await
    {
        Ok(()) => success_response("Speaker renamed successfully", None),
        Err(e) => error_response(StatusCode::BAD_REQUEST, e),
    }
}

async fn reset_handler(headers: HeaderMap, State(state): State<ServerState>) -> impl IntoResponse {
    if let Some((status, msg)) = authorize_control(&headers) {
        return error_response(status, msg);
    }

    let audio_state = state.app_handle.state::<AudioState>();
    audio_state.meeting_detector.clear_test_meetings(&state.app_handle);
    success_response("Reset completed", None)
}

// ── Server Bootstrap ─────────────────────────────────────────────────────────

pub fn build_router(app_handle: AppHandle) -> Router {
    let server_state = ServerState { app_handle };

    let mut app = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/status", get(status_handler))
        .route("/api/scan", get(scan_handler))
        .route("/api/meetings", get(meetings_handler))
        .route("/api/meetings/{id}", get(meeting_detail_handler))
        .route("/api/vault", get(vault_handler));

    if is_test_mode() {
        app = app
            .route("/api/simulate/{event}", post(simulate_event_handler))
            .route("/api/capture/start", post(capture_start_handler))
            .route("/api/capture/stop", post(capture_stop_handler))
            .route("/api/simulate/feed-meeting", post(feed_meeting_handler))
            .route("/api/simulate/cycle-turn-snippet", post(cycle_turn_snippet_handler))
            .route("/api/simulate/cycle-vault-snippet", post(cycle_vault_snippet_handler))
            .route("/api/simulate/rename-speaker", post(rename_speaker_handler))
            .route("/api/reset", post(reset_handler));
    }

    app.with_state(server_state)
        .layer(middleware::from_fn(require_control_token))
}

pub fn spawn_control_server(app_handle: AppHandle) {
    if !is_test_mode() {
        return;
    }
    if control_token().is_none() {
        eprintln!("[control-server] TAURSCRIBE_CONTROL_TOKEN is required in test mode; server disabled");
        return;
    }
    tauri::async_runtime::spawn(async move {
        let app = build_router(app_handle);
        let port = control_port();
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));

        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[control-server] Failed to bind to 127.0.0.1:{}: {}", port, e);
                return;
            }
        };

        println!(
            "[control-server] listening on http://127.0.0.1:{} (mutations {})",
            port,
            "enabled"
        );

        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("[control-server] Server error: {}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"proof-token", b"proof-token"));
        assert!(!constant_time_eq(b"proof-token", b"wrong-token"));
        assert!(!constant_time_eq(b"short", b"longer-token"));
        assert!(!constant_time_eq(b"", b"token"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_authorize_control_missing_token() {
        let headers = HeaderMap::new();
        let res = authorize_bearer(&headers, "test-token");
        assert!(res.is_some());
        let (status, msg) = res.unwrap();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(msg, "missing bearer token");
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer wrong-token".parse().unwrap());
        assert_eq!(authorize_bearer(&headers, "test-token"), Some((StatusCode::UNAUTHORIZED, "invalid bearer token")));
        headers.insert("authorization", "Bearer test-token".parse().unwrap());
        assert!(authorize_bearer(&headers, "test-token").is_none());
    }
}
