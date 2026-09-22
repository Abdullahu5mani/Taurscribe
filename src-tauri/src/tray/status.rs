//! What the tray icon shows: one icon per app state, a hover tooltip with the
//! details, and short-lived states (done, nothing heard, errors) that return to
//! idle on their own.
//!
//! Icons live in icons/tray/ (scripts/make_tray_icons.swift):
//!   macOS          mac-<id>.png   SF Symbols; monochrome ones are template images
//!                                 (macOS tints them for light/dark menu bars)
//!   Windows/Linux  win-<id>-dark.png / -light.png (monochrome), win-<id>.png
//!                  (coloured); Material Symbols Rounded. Windows picks the variant
//!                  from the taskbar theme; Linux panels are assumed dark.
//! A detected call shows the meeting app's own icon (macOS) or a camera (elsewhere)
//! with a coloured dot.

use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::image::Image;
use tauri::{AppHandle, Manager};

use crate::state::AudioState;
use crate::types::AppState;

/// Status dot drawn on a meeting icon.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Dot {
    /// In a call, not recording.
    Green,
    /// Recording the call.
    Red,
    /// Recording, but the app's own audio could not be isolated (all system audio).
    Orange,
    /// Recording paused.
    Paused,
}

#[derive(Clone, Default)]
struct Meeting {
    platform: String,
    process: Option<String>,
    pid: Option<u32>,
}

struct Status {
    state: AppState,
    meeting: Option<Meeting>,
    detail: Option<String>,
    /// A short-lived state holds the icon until then; `Ready` requests wait.
    hold_until: Option<Instant>,
    generation: u64,
    recording_since: Option<chrono::DateTime<chrono::Local>>,
}

static STATUS: Mutex<Status> = Mutex::new(Status {
    state: AppState::Ready,
    meeting: None,
    detail: None,
    hold_until: None,
    generation: 0,
    recording_since: None,
});

/// How long a short-lived state stays before the tray returns to idle.
fn hold_for(state: AppState) -> Option<Duration> {
    match state {
        AppState::Done | AppState::NothingHeard | AppState::Cancelled => Some(Duration::from_millis(2500)),
        AppState::PasteFailed | AppState::Error | AppState::MicBlocked => Some(Duration::from_secs(8)),
        _ => None,
    }
}

/// The line shown at the top of the tray menu while an error-like state is up.
pub fn menu_status_line() -> Option<String> {
    let s = STATUS.lock().ok()?;
    let d = s.detail.clone()?;
    match s.state {
        AppState::PasteFailed | AppState::Error | AppState::MicBlocked | AppState::NothingHeard => Some(format!("⚠︎ {d}")),
        _ => None,
    }
}

/// Current call as (platform, process, pid).
pub fn current_meeting_full() -> Option<(String, Option<String>, Option<u32>)> {
    let s = STATUS.lock().ok()?;
    s.meeting.as_ref().map(|m| (m.platform.clone(), m.process.clone(), m.pid))
}

/// Sets the tray to `state`. `detail` is shown in the tooltip (and, for errors,
/// at the top of the menu). A `Ready` request while a short-lived state is
/// showing only updates the meeting; the state returns to idle when it expires.
pub fn set(
    app: &AppHandle,
    state: AppState,
    platform: Option<&str>,
    process: Option<&str>,
    pid: Option<u32>,
    detail: Option<String>,
) -> Result<(), String> {
    let meeting = platform.map(|p| Meeting { platform: p.to_string(), process: process.map(str::to_string), pid });
    let generation = {
        let mut s = STATUS.lock().map_err(|e| e.to_string())?;
        s.meeting = meeting;
        let held = s.hold_until.is_some_and(|t| Instant::now() < t);
        if state == AppState::Ready && held {
            return Ok(());
        }
        s.state = state;
        s.detail = detail;
        s.hold_until = hold_for(state).map(|d| Instant::now() + d);
        s.recording_since = match state {
            AppState::Recording | AppState::Paused => Some(s.recording_since.unwrap_or_else(chrono::Local::now)),
            _ => None,
        };
        s.generation += 1;
        s.generation
    };
    render(app)?;
    if let Some(d) = hold_for(state) {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(d);
            let meeting = {
                let mut s = match STATUS.lock() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                if s.generation != generation {
                    return; // something else happened meanwhile
                }
                s.state = AppState::Ready;
                s.detail = None;
                s.hold_until = None;
                s.generation += 1;
                s.meeting.clone()
            };
            let _ = render(&app);
            let loaded = app.state::<AudioState>().model_loaded.load(Ordering::Relaxed);
            let info = meeting.as_ref().map(|m| (m.platform.as_str(), m.pid.unwrap_or(0)));
            super::icons::update_tray_menu(&app, loaded, info, false);
        });
    }
    Ok(())
}

/// Redraws icon + tooltip from the stored status (e.g. after a model load).
pub fn render(app: &AppHandle) -> Result<(), String> {
    let (state, meeting, detail, since) = {
        let s = STATUS.lock().map_err(|e| e.to_string())?;
        (s.state, s.meeting.clone(), s.detail.clone(), s.recording_since)
    };
    let (icon, template) = icon_for(app, state, meeting.as_ref());
    let tooltip = tooltip_for(app, state, meeting.as_ref(), detail.as_deref(), since);
    let Some(tray) = app.tray_by_id("main-tray") else { return Ok(()) };
    tray.set_icon(Some(icon)).map_err(|e| format!("Failed to set tray icon: {e}"))?;
    #[cfg(target_os = "macos")]
    {
        tray.set_icon_as_template(template).map_err(|e| format!("Failed to set icon as template: {e}"))?;
        // Icon only; an empty title clears any old text (set_title(None) leaves it).
        let _ = tray.set_title(Some(""));
    }
    let _ = template;
    tray.set_tooltip(Some(format!("Taurscribe — {tooltip}"))).map_err(|e| format!("Failed to set tooltip: {e}"))?;
    println!("[TRAY] {:?}: {}", state, tooltip.replace('\n', " | "));
    Ok(())
}

// ── icons ───────────────────────────────────────────────────────────────────

/// What the idle state means right now.
fn idle_kind(app: &AppHandle) -> &'static str {
    match app.try_state::<AudioState>() {
        Some(s) if s.model_loaded.load(Ordering::Relaxed) => "ready",
        Some(s) if s.active_engine_has_downloaded_model() => "model-unloaded",
        Some(_) => "no-model",
        None => "ready",
    }
}

fn icon_id(app: &AppHandle, state: AppState) -> &'static str {
    match state {
        AppState::Ready => idle_kind(app),
        AppState::Recording => "dictating",
        AppState::Paused => "paused",
        AppState::Processing | AppState::ProcessingSpeech => "processing-speech",
        AppState::ProcessingMeeting => "processing-meeting",
        AppState::ProcessingFile => "processing-file",
        AppState::LoadingModel => "loading-model",
        AppState::Downloading => "downloading",
        AppState::Grammar => "grammar",
        AppState::Done => "done",
        AppState::NothingHeard => "nothing-heard",
        AppState::PasteFailed => "paste-failed",
        AppState::Error => "error",
        AppState::MicBlocked => "mic-blocked",
        AppState::Cancelled => "cancelled",
    }
}

fn icon_for(app: &AppHandle, state: AppState, meeting: Option<&Meeting>) -> (Image<'static>, bool) {
    if let Some(m) = meeting {
        let dot = match state {
            AppState::Ready => Some(Dot::Green),
            AppState::Recording if system_audio_fallback() => Some(Dot::Orange),
            AppState::Recording => Some(Dot::Red),
            AppState::Paused => Some(Dot::Paused),
            _ => None,
        };
        if let Some(dot) = dot {
            return meeting_icon(m, dot);
        }
    }
    status_icon(icon_id(app, state))
}

#[cfg(target_os = "macos")]
macro_rules! mac_icons {
    ($id:expr; $($name:literal),*) => {
        png(match $id { $($name => &include_bytes!(concat!("../../icons/tray/mac-", $name, ".png"))[..],)* _ => &include_bytes!("../../icons/tray/mac-ready.png")[..] })
    };
}

/// Decodes a bundled icon PNG (all are generated, so decoding cannot fail).
fn png(bytes: &[u8]) -> Image<'static> {
    Image::from_bytes(bytes).expect("bundled tray icon").to_owned()
}

/// Coloured icons keep their colour; everything else is monochrome.
const COLOURED: [&str; 7] = ["dictating", "paused", "done", "nothing-heard", "paste-failed", "error", "mic-blocked"];

#[cfg(target_os = "macos")]
fn status_icon(id: &str) -> (Image<'static>, bool) {
    let img = mac_icons!(id; "ready", "model-unloaded", "no-model", "downloading", "loading-model", "dictating",
        "paused", "processing-speech", "grammar", "done", "nothing-heard", "paste-failed", "error",
        "mic-blocked", "processing-meeting", "processing-file", "cancelled", "call");
    (img, !COLOURED.contains(&id))
}

#[cfg(not(target_os = "macos"))]
macro_rules! win_icons {
    ($id:expr, $light:expr; coloured: $($c:literal),*; mono: $($m:literal),*) => {
        match $id {
            $($c => png(&include_bytes!(concat!("../../icons/tray/win-", $c, ".png"))[..]),)*
            $($m => if $light { png(&include_bytes!(concat!("../../icons/tray/win-", $m, "-light.png"))[..]) }
                    else { png(&include_bytes!(concat!("../../icons/tray/win-", $m, "-dark.png"))[..]) },)*
            _ => png(&include_bytes!("../../icons/tray/win-ready-dark.png")[..]),
        }
    };
}

#[cfg(not(target_os = "macos"))]
fn status_icon(id: &str) -> (Image<'static>, bool) {
    let img = win_icons!(id, light_taskbar();
        coloured: "dictating", "paused", "done", "nothing-heard", "paste-failed", "error", "mic-blocked";
        mono: "ready", "model-unloaded", "no-model", "downloading", "loading-model", "processing-speech",
              "grammar", "processing-meeting", "processing-file", "cancelled", "call");
    (img, false)
}

fn meeting_icon(m: &Meeting, dot: Dot) -> (Image<'static>, bool) {
    #[cfg(target_os = "macos")]
    if let Some(img) = super::app_icon::meeting_app_icon(m.process.as_deref(), m.pid, dot) {
        return (img, false);
    }
    let _ = m;
    // Generic camera with the dot (non-template so the dot keeps its colour).
    let (base, _) = status_icon("call");
    let (w, h) = (base.width(), base.height());
    let mut rgba = base.rgba().to_vec();
    #[cfg(target_os = "macos")]
    for px in rgba.chunks_mut(4) {
        // The macOS camera is black (a template); draw it grey so it reads on both bars.
        if px[3] > 0 {
            px[0] = 142;
            px[1] = 142;
            px[2] = 147;
        }
    }
    draw_dot(&mut rgba, w, h, dot);
    (Image::new_owned(rgba, w, h), false)
}

/// A status dot with a white ring in the bottom-right corner.
pub fn draw_dot(rgba: &mut [u8], w: u32, h: u32, dot: Dot) {
    let (r, g, b) = match dot {
        Dot::Green => (48u8, 209u8, 88u8),
        Dot::Red => (255, 59, 48),
        Dot::Orange | Dot::Paused => (255, 149, 0),
    };
    let radius = w as f32 * 0.22;
    let (cx, cy) = (w as f32 - radius - 0.5, h as f32 - radius - 0.5);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let d = (fx * fx + fy * fy).sqrt();
            let px = ((y * w + x) * 4) as usize;
            if d <= radius - 2.5 {
                // Pause: two white bars inside the dot.
                let bar = dot == Dot::Paused
                    && fy.abs() <= radius * 0.42
                    && (fx.abs() >= radius * 0.14 && fx.abs() <= radius * 0.40);
                let c = if bar { [255, 255, 255, 255] } else { [r, g, b, 255] };
                rgba[px..px + 4].copy_from_slice(&c);
            } else if d <= radius {
                rgba[px..px + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
}

fn system_audio_fallback() -> bool {
    crate::audio_dual_channel::capture_diagnostics()
        .map(|d| d.active && (d.fell_back_to_system || !d.target.starts_with("Process")))
        .unwrap_or(false)
}

/// Windows: whether the taskbar uses the light theme (then icons are drawn black).
#[cfg(target_os = "windows")]
fn light_taskbar() -> bool {
    use std::os::windows::process::CommandExt;
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    if let Ok(c) = CACHE.lock() {
        if let Some((t, v)) = *c {
            if t.elapsed() < Duration::from_secs(10) {
                return v;
            }
        }
    }
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "/v", "SystemUsesLightTheme"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();
    let light = out
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().ends_with("0x1"))
        .unwrap_or(false);
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((Instant::now(), light));
    }
    light
}

/// Linux panels are dark on most desktops.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn light_taskbar() -> bool {
    false
}

// ── tooltip ─────────────────────────────────────────────────────────────────

pub(super) fn platform_display_name(p: &str) -> &str {
    match p.to_lowercase().as_str() {
        "meet" => "Meet",
        "zoom" => "Zoom",
        "teams" => "Teams",
        "slack" => "Slack",
        "discord" => "Discord",
        "webex" => "Webex",
        _ => p,
    }
}

fn engine_name(app: &AppHandle) -> &'static str {
    match app.try_state::<AudioState>().map(|s| *s.active_engine.lock().unwrap()) {
        Some(crate::types::ASREngine::Whisper) => "Whisper",
        Some(crate::types::ASREngine::Granite) => "Granite Speech 5",
        Some(crate::types::ASREngine::Qwen3) => "Qwen3-ASR",
        None => "the speech model",
    }
}

pub(super) fn idle_tooltip(app: &AppHandle) -> &'static str {
    match idle_kind(app) {
        "ready" => "Ready",
        "model-unloaded" => "Model not loaded (loads when you record, or choose Load Model)",
        _ => "No speech model downloaded (open Settings → Models)",
    }
}

fn app_label(m: &Meeting) -> String {
    #[cfg(target_os = "macos")]
    if let Some(name) = super::app_icon::app_display_name(m.process.as_deref(), m.pid) {
        return name;
    }
    m.process.clone().unwrap_or_else(|| "the meeting app".to_string())
}

fn tooltip_for(
    app: &AppHandle,
    state: AppState,
    meeting: Option<&Meeting>,
    detail: Option<&str>,
    since: Option<chrono::DateTime<chrono::Local>>,
) -> String {
    let started = since.map(|t| t.format("%H:%M").to_string()).unwrap_or_default();
    let engine = engine_name(app);
    match (state, meeting) {
        (AppState::Recording | AppState::Paused, Some(m)) => {
            let plat = platform_display_name(&m.platform);
            let label = app_label(m);
            let source = if system_audio_fallback() {
                "all system audio (the app's own audio could not be isolated)".to_string()
            } else {
                format!("{label}'s audio")
            };
            let verb = if state == AppState::Paused { "Paused recording" } else { "Recording" };
            format!("{verb} {plat} call ({label})\nYour microphone + {source}\nTranscribing with {engine} · since {started}")
        }
        (AppState::Recording, None) => format!("Recording dictation from your microphone\nTranscribing with {engine} · since {started}"),
        (AppState::Paused, None) => format!("Dictation paused · started {started}"),
        (AppState::Ready, Some(m)) => {
            let plat = platform_display_name(&m.platform);
            format!("In a {plat} call ({})\nNot recording · choose “Record {plat} Call” in this menu", app_label(m))
        }
        (AppState::Ready, None) => idle_tooltip(app).to_string(),
        (AppState::Processing | AppState::ProcessingSpeech, _) => format!("Processing spoken audio with {engine}"),
        (AppState::ProcessingMeeting, _) => format!("Processing meeting audio: transcribing with {engine} and separating speakers"),
        (AppState::ProcessingFile, _) => format!("Processing file audio with {engine}{}", detail.map(|d| format!("\n{d}")).unwrap_or_default()),
        (AppState::LoadingModel, _) => format!("Loading {engine}"),
        (AppState::Downloading, _) => detail.map(|d| format!("Downloading {d}")).unwrap_or_else(|| "Downloading a model".to_string()),
        (AppState::Grammar, _) => "Correcting grammar (FlowScribe)".to_string(),
        (AppState::Done, _) => detail.map(|d| format!("Done · {d}")).unwrap_or_else(|| "Done — transcript pasted".to_string()),
        (AppState::NothingHeard, _) => detail.unwrap_or("Nothing heard — no speech in the recording").to_string(),
        (AppState::PasteFailed, _) => detail.map(|d| format!("{d}\nThe transcript is in Taurscribe")).unwrap_or_else(|| "Couldn't paste — the transcript is in Taurscribe".to_string()),
        (AppState::Error, _) => detail.map(|d| format!("Error: {d}")).unwrap_or_else(|| "Something went wrong — open Taurscribe".to_string()),
        (AppState::MicBlocked, _) => "Microphone access is blocked\nAllow Taurscribe in System Settings → Privacy & Security → Microphone".to_string(),
        (AppState::Cancelled, _) => "Recording discarded".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_is_drawn_bottom_right() {
        let px = 44u32;
        let mut rgba = vec![0u8; (px * px * 4) as usize];
        draw_dot(&mut rgba, px, px, Dot::Red);
        let at = |x: u32, y: u32| rgba[((y * px + x) * 4) as usize..((y * px + x) * 4 + 4) as usize].to_vec();
        assert_eq!(at(px - 10, px - 10), vec![255, 59, 48, 255]);
        assert_eq!(at(4, 4), vec![0, 0, 0, 0]);
    }

    #[test]
    fn short_lived_states_expire() {
        assert!(hold_for(AppState::Done).is_some());
        assert!(hold_for(AppState::Error).unwrap() > hold_for(AppState::Done).unwrap());
        assert!(hold_for(AppState::Recording).is_none());
    }
}
