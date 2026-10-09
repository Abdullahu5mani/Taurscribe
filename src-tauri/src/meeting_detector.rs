//! Meeting Detection Module for Taurscribe
//!
//! Provides automated, bot-free detection of live meetings (Zoom, Microsoft Teams,
//! Google Meet, Cisco Webex, Slack Huddle, Discord, etc.) across native desktop
//! applications and web browsers.
//!
//! On macOS and Windows, utilizes `meeting-record` to inspect OS-level CoreAudio
//! and WASAPI audio activity (microphone in use + sound playing) alongside window
//! titles and URLs. On Linux, provides process-based fallback inspection.

use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter, Manager};
use crate::AudioState;

#[cfg(target_os = "macos")]
#[path = "meeting_browser.rs"]
mod browser_activity;

fn is_browser_prejoin(pid: u32, app_name: &str, url: &str, platform: &str) -> bool {
    // Meet's call controls are read over Accessibility, which every macOS browser
    // exposes (an unreadable page stays unknown and suppresses nothing).
    #[cfg(target_os = "macos")]
    if platform == "meet" && (scriptable_browser(app_name).is_some() || app_name.to_lowercase().starts_with("firefox")) {
        return browser_activity::state(pid, url) == Some(false);
    }
    let _ = (pid, app_name, url, platform);
    false
}

/// How a browser's tabs are listed over AppleScript.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TabScript {
    /// Chrome and the browsers built on it: a tab has `title` and `URL`.
    Chromium,
    /// Safari: a tab has `name` and `URL`.
    Safari,
}

/// The AppleScript application name of a browser whose tabs can be listed, from
/// the app (or helper) process name. Longer names come first so Chrome Beta is
/// not taken for Chrome.
#[cfg(target_os = "macos")]
fn scriptable_browser(app_name: &str) -> Option<(&'static str, TabScript)> {
    const BROWSERS: &[(&str, &str, TabScript)] = &[
        ("google chrome canary", "Google Chrome Canary", TabScript::Chromium),
        ("google chrome beta", "Google Chrome Beta", TabScript::Chromium),
        ("google chrome dev", "Google Chrome Dev", TabScript::Chromium),
        ("google chrome", "Google Chrome", TabScript::Chromium),
        ("microsoft edge", "Microsoft Edge", TabScript::Chromium),
        ("brave browser", "Brave Browser", TabScript::Chromium),
        ("vivaldi", "Vivaldi", TabScript::Chromium),
        ("arc", "Arc", TabScript::Chromium),
        ("safari", "Safari", TabScript::Safari),
    ];
    let name = app_name.trim().to_lowercase();
    BROWSERS.iter().find_map(|(prefix, app, kind)| {
        // "Arc" must not match "Archive Utility": the prefix ends at a word.
        let rest = name.strip_prefix(prefix)?;
        (rest.is_empty() || rest.starts_with(' ')).then_some((*app, *kind))
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingInfo {
    pub pid: u32,
    pub app_name: String,
    pub title: String,
    pub url: String,
    pub platform: String,
    pub confidence: i32,
    pub should_record: bool,
    pub is_using_mic: bool,
    pub is_playing_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingDetectionStatus {
    pub is_watching: bool,
    pub supported: bool,
    pub active_meetings: Vec<MeetingInfo>,
}

/// Suppresses false alarms when a meeting app is merely in a pre-join lobby or
/// device check dialog (e.g. testing the microphone before entering the call).
pub fn is_prejoin_window_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    lower.contains("waiting room")
        || lower.contains("choose one meeting option")
        || lower == "joining..."
        || lower.starts_with("joining ")
        || lower.contains("pre-meeting")
        || lower.contains("preview audio")
        || lower.contains("ready to join?")
        || lower.contains("choose your audio")
        || lower.contains("join conversation")
        || lower.contains("meeting lobby")
        || lower == "google meet"
        || lower == "meet"
}

/// Maps a browser tab URL to the platform id of a live call, or `None` when the
/// tab is not a call (landing pages, chat views on other hosts, etc.).
pub fn classify_meeting_url(url: &str) -> Option<&'static str> {
    let url = url.to_lowercase();
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(&url);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split(':').next().unwrap_or(host);
    let path = format!("/{}", path);

    // Meet: only a room code (abc-defg-hij) is a call; /home, /landing are not.
    if host == "meet.google.com" {
        let code = path.trim_start_matches('/').split(['?', '/', '#']).next().unwrap_or("");
        let parts: Vec<&str> = code.split('-').collect();
        let is_room = parts.len() == 3
            && [3, 4, 3].iter().zip(&parts).all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_lowercase()));
        return is_room.then_some("meet");
    }
    // Teams work (teams.microsoft.com) and personal (teams.live.com). The v2 web
    // app keeps calls inside the same /v2/ single-page app, so the host is the signal.
    if host == "teams.microsoft.com" || host == "teams.live.com" || host.ends_with(".teams.microsoft.com") {
        return Some("teams");
    }
    // Zoom web client: /wc/<id>/..., /j/<id> (join), and app.zoom.us/wc.
    if host == "zoom.us" || host.ends_with(".zoom.us") {
        return (path.starts_with("/wc/") || path.starts_with("/j/") || path.starts_with("/s/")).then_some("zoom");
    }
    // Webex: personal rooms (/meet/...), the web meeting client (/wbxmjs/, /webappng/),
    // and web.webex.com meetings.
    if host == "webex.com" || host.ends_with(".webex.com") {
        let in_call = path.starts_with("/meet/")
            || path.contains("/wbxmjs/")
            || path.contains("/webappng/")
            || (host == "web.webex.com" && path.starts_with("/meeting"));
        return in_call.then_some("webex");
    }
    None
}

/// How long an app outside the catalogue must hold the mic while playing audio
/// before it is offered as a call. Skips a voice tool's brief capture or an
/// app's microphone check.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const OTHER_APP_CALL_MIN: std::time::Duration = std::time::Duration::from_secs(5);

/// Apps meeting-record classifies itself (conferencing clients and browsers), by
/// macOS bundle id or Windows executable name. Their mic use is the crate's to
/// judge; WebKit covers Safari's and our own web view's helper processes.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn is_catalogued_app(name: &str, bundle_id: &str) -> bool {
    const BUNDLES: &[&str] = &[
        "us.zoom.", "com.microsoft.teams", "com.cisco.webex", "com.webex.meetingmanager",
        "com.tinyspeck.slackmacgap", "com.hnc.discord", "com.google.chrome", "com.apple.safari",
        "com.microsoft.edgemac", "org.mozilla.firefox", "company.thebrowser.browser",
        "com.brave.browser", "com.vivaldi.vivaldi", "com.apple.webkit",
    ];
    const EXES: &[&str] = &[
        "zoom.exe", "cpthost.exe", "teams.exe", "ms-teams.exe", "msteams.exe", "webexmta.exe",
        "webex.exe", "atmgr.exe", "slack.exe", "discord.exe", "chrome.exe", "msedge.exe",
        "firefox.exe", "brave.exe", "arc.exe", "vivaldi.exe", "opera.exe", "msedgewebview2.exe",
    ];
    let bundle = bundle_id.to_lowercase();
    let exe = name.to_lowercase();
    (!bundle.is_empty() && BUNDLES.iter().any(|b| bundle.starts_with(b))) || EXES.contains(&exe.as_str())
}

/// The name to show for a call in an app outside the catalogue, or `None` when
/// the process is part of the OS rather than an app the user runs: on macOS
/// only `.app` bundles count (FaceTime and phone calls run in `avconferenced`),
/// on Windows anything outside the Windows directory.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn other_call_app_name(process_name: &str, app: &std::path::Path, exe: &std::path::Path) -> Option<String> {
    let stem = |p: &std::path::Path| p.file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.is_empty());
    if process_name == "avconferenced" {
        return Some("FaceTime".to_string());
    }
    if app.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) {
        return stem(app);
    }
    if exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
        let windir = std::env::var_os("SystemRoot").map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
        let lower = |p: &std::path::Path| p.to_string_lossy().to_lowercase();
        if lower(exe).starts_with(&lower(&windir)) {
            return None;
        }
        return stem(exe);
    }
    None
}

// ── macOS & Windows Implementation ──────────────────────────────────────────

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform_impl {
    use super::*;
    use meeting_record::{meetings, MeetingEvent};

    /// meeting-record attributes a browser's audio to a helper process and often
    /// cannot read a title or URL for it, so list every tab over AppleScript and
    /// pick the one that is a call.
    #[cfg(target_os = "macos")]
    fn resolve_browser_tab_info(app_name: &str, pid: u32, detected_title: &str, detected_url: &str) -> Option<(String, String, String, String)> {
        let (browser, kind) = super::scriptable_browser(app_name)?;
        let title_of = match kind {
            super::TabScript::Chromium => "title",
            super::TabScript::Safari => "name",
        };
        // `tell application` would launch a browser that is not running; this
        // one is, since it is the process doing the call's audio.
        let script = format!(
            r#"
        tell application "{browser}"
            set out to ""
            try
                repeat with w in windows
                    repeat with t in tabs of w
                        try
                            set out to out & ({title_of} of t) & "|||" & (URL of t) & linefeed
                        end try
                    end repeat
                end repeat
            end try
            return out
        end tell
        "#
        );
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let listing = String::from_utf8_lossy(&output.stdout);
        let mut candidates: Vec<_> = listing.lines().filter_map(|line| {
            let (title, url) = line.split_once("|||")?;
            let platform = super::classify_meeting_url(url.trim())?;
            Some((title.trim().to_string(), url.trim().to_string(), platform.to_string()))
        }).collect();
        // A preview tab can precede a live call in the tab order. Prefer a page
        // with actual call controls before an unknown or pre-join page.
        candidates.sort_by_key(|(_, url, platform)| {
            let room = url.split('/').nth(3).unwrap_or("").split(['?', '#']).next().unwrap_or("");
            let matches_source = url == detected_url || (!room.is_empty() && detected_title.contains(room));
            if platform == "meet" {
                match super::browser_activity::state(pid, url) { Some(true) => 0, _ if matches_source => 1, None => 2, Some(false) => 3 }
            } else if matches_source { 1 } else { 2 }
        });
        candidates.into_iter().next().map(|(title, url, platform)| (title, url, platform, browser.to_string()))
    }

    pub fn scan_meetings() -> Vec<MeetingInfo> {
        let detected = meetings::scan();
        let mut results = Vec::new();

        for m in &detected {
            let mut title = m.title.clone();
            let mut url = m.url.clone();
            let mut platform = m.platform.as_str().to_string();
            let mut app_name = m.app_name.clone();

            #[cfg(target_os = "macos")]
            if title.trim().is_empty() || platform == "browser" || platform == "other" || platform == "meet" {
                if let Some((tab_title, tab_url, tab_platform, browser)) = resolve_browser_tab_info(&app_name, m.pid, &title, &url) {
                    title = tab_title;
                    url = tab_url;
                    platform = tab_platform;
                    app_name = browser;
                }
            }

            // Apply pre-join suppression filter
            if is_prejoin_window_title(&title) || is_browser_prejoin(m.pid, &app_name, &url, &platform) {
                continue;
            }

            if title.trim().is_empty() {
                continue;
            }

            // Confidence & audio activity check: prevent idle browser tabs from being marked as meetings
            if !m.should_record && m.confidence < 70 && !m.is_using_mic && !m.is_playing_audio {
                continue;
            }

            results.push(MeetingInfo {
                pid: m.pid,
                app_name,
                title,
                url,
                platform,
                confidence: m.confidence,
                should_record: m.should_record,
                is_using_mic: m.is_using_mic,
                is_playing_audio: m.is_playing_audio,
            });
        }

        results.extend(other_app_calls(&detected));
        results
    }

    /// First time each app outside meeting-record's catalogue was seen holding
    /// the mic while playing audio, by app path.
    static OTHER_APP_SEEN: std::sync::LazyLock<Mutex<std::collections::HashMap<std::path::PathBuf, std::time::Instant>>> =
        std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

    /// Calls in apps meeting-record does not know (Signal, WhatsApp, FaceTime, a
    /// niche video app): any app that keeps the microphone open while playing
    /// audio for OTHER_APP_CALL_MIN. These are offered, never auto-recorded
    /// (`should_record` is false), since a DAW or a voice tool can look the same.
    fn other_app_calls(known: &[meeting_record::Meeting]) -> Vec<MeetingInfo> {
        use crate::audio_dual_channel::app_bundle_path;
        use std::collections::HashMap;
        use std::path::PathBuf;

        let mut sys = sysinfo::System::new();
        let mut app_of = |pid: u32| -> Option<(PathBuf, PathBuf)> {
            let spid = sysinfo::Pid::from_u32(pid);
            sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[spid]), true);
            let exe = sys.process(spid)?.exe()?.to_path_buf();
            Some((app_bundle_path(&exe), exe))
        };
        let own_app = std::env::current_exe().ok().map(|e| app_bundle_path(&e));
        // The crate's own detections, including ones filtered out above as a
        // lobby or pre-join page: those must not come back as a generic call.
        let known_apps: Vec<PathBuf> = known.iter().filter_map(|m| app_of(m.pid).map(|(app, _)| app)).collect();

        // (pid, name, using mic, playing audio) per app.
        let mut apps: HashMap<PathBuf, (u32, String, bool, bool)> = HashMap::new();
        for p in meeting_record::meetings::audio_processes() {
            if p.pid == std::process::id() || super::is_catalogued_app(&p.name, &p.bundle_id) {
                continue;
            }
            let Some((app, exe)) = app_of(p.pid) else { continue };
            if own_app.as_ref() == Some(&app) || known_apps.contains(&app) {
                continue;
            }
            let Some(name) = super::other_call_app_name(&p.name, &app, &exe) else { continue };
            let entry = apps.entry(app).or_insert((p.pid, name, false, false));
            entry.2 |= p.is_using_mic;
            entry.3 |= p.is_playing_audio;
        }

        let now = std::time::Instant::now();
        let mut seen = OTHER_APP_SEEN.lock().unwrap();
        seen.retain(|app, _| apps.get(app).is_some_and(|a| a.2 && a.3));
        let mut calls = Vec::new();
        for (app, (pid, name, mic, playing)) in apps {
            if !(mic && playing) {
                continue;
            }
            let since = *seen.entry(app).or_insert(now);
            if now.duration_since(since) < super::OTHER_APP_CALL_MIN {
                continue;
            }
            calls.push(MeetingInfo {
                pid,
                app_name: name.clone(),
                title: String::new(),
                url: String::new(),
                // The app's own name: what the banner, tray and saved meeting show.
                platform: name,
                confidence: 70,
                should_record: false,
                is_using_mic: true,
                is_playing_audio: true,
            });
        }
        calls
    }

    /// Background watcher guard handle
    pub struct WatcherHandle {
        _watcher: Option<meeting_record::Watcher>,
    }

    pub fn start_watcher(
        app_handle: AppHandle,
        active_cache: Arc<Mutex<Vec<MeetingInfo>>>,
    ) -> Result<WatcherHandle, String> {
        let app_handle_clone = app_handle.clone();
        let cache_clone = active_cache.clone();

        let watcher = meetings::watch(move |event, meeting| {
            let mut title = meeting.title.clone();
            let mut url = meeting.url.clone();
            let mut platform = meeting.platform.as_str().to_string();
            let mut app_name = meeting.app_name.clone();

            #[cfg(target_os = "macos")]
            if title.trim().is_empty() || platform == "browser" || platform == "other" || platform == "meet" {
                if let Some((tab_title, tab_url, tab_platform, browser)) = resolve_browser_tab_info(&app_name, meeting.pid, &title, &url) {
                    title = tab_title;
                    url = tab_url;
                    platform = tab_platform;
                    app_name = browser;
                }
            }

            let is_prejoin = is_prejoin_window_title(&title) || is_browser_prejoin(meeting.pid, &app_name, &url, &platform);
            let info = MeetingInfo {
                pid: meeting.pid,
                app_name,
                title,
                url,
                platform,
                confidence: meeting.confidence,
                should_record: meeting.should_record,
                is_using_mic: meeting.is_using_mic,
                is_playing_audio: meeting.is_playing_audio,
            };

            let mut cache = cache_clone.lock().unwrap();
            match event {
                MeetingEvent::Started => {
                    if is_prejoin {
                        return;
                    }
                    if !meeting.should_record && meeting.confidence < 70 && !meeting.is_using_mic && !meeting.is_playing_audio {
                        return;
                    }
                    println!(
                        "[MEETING DETECTED] Started: {} - '{}' (pid: {}, confidence: {}%)",
                        info.app_name, info.title, info.pid, info.confidence
                    );
                    if let Some(existing) = cache.iter_mut().find(|m| m.pid == info.pid) {
                        *existing = info.clone();
                    } else {
                        cache.push(info.clone());
                    }
                    let active_top = cache.first().cloned();
                    sync_detector_tray(&app_handle_clone, active_top.as_ref());
                    let _ = app_handle_clone.emit("meeting-detected", &info);
                }
                MeetingEvent::Updated => {
                    if is_prejoin {
                        // If window transitioned to pre-join or lobby, the active call has ended
                        if let Some(pos) = cache.iter().position(|m| m.pid == info.pid) {
                            let removed = cache.remove(pos);
                            println!("[MEETING ENDED] Transitioned to lobby/pre-join: {} - '{}'", removed.app_name, removed.title);
                            let active_top = cache.first().cloned();
                            sync_detector_tray(&app_handle_clone, active_top.as_ref());
                            let _ = app_handle_clone.emit("meeting-ended", &removed);
                        }
                        return;
                    }

                    if let Some(existing) = cache.iter_mut().find(|m| m.pid == info.pid) {
                        *existing = info.clone();
                        let active_top = cache.first().cloned();
                        sync_detector_tray(&app_handle_clone, active_top.as_ref());
                        let _ = app_handle_clone.emit("meeting-changed", &info);
                    } else if meeting.should_record || meeting.confidence >= 70 {
                        cache.push(info.clone());
                        let active_top = cache.first().cloned();
                        sync_detector_tray(&app_handle_clone, active_top.as_ref());
                        let _ = app_handle_clone.emit("meeting-detected", &info);
                    }
                }
                MeetingEvent::Ended => {
                    // Ended MUST never be suppressed by pre-join filter!
                    println!(
                        "[MEETING ENDED] Ended: {} - '{}' (pid: {})",
                        info.app_name, info.title, info.pid
                    );
                    cache.retain(|m| m.pid != info.pid);
                    let active_top = cache.first().cloned();
                    sync_detector_tray(&app_handle_clone, active_top.as_ref());
                    let _ = app_handle_clone.emit("meeting-ended", &info);
                }
            }
        })
        .map_err(|e| format!("Failed to start meeting watcher: {}", e))?;

        Ok(WatcherHandle {
            _watcher: Some(watcher),
        })
    }
}

/// Helper function to synchronously reflect active meeting state into the system tray icon & menu
pub fn sync_detector_tray(app: &AppHandle, meeting: Option<&MeetingInfo>) {
    if let Some(state) = app.try_state::<AudioState>() {
        let is_recording = state.recording_handle.lock().map(|h| h.is_some()).unwrap_or(false);
        let loaded = state.model_loaded.load(Ordering::Relaxed);
        let paused = state.recording_paused.load(Ordering::Relaxed);
        let app_state = if is_recording && paused {
            crate::types::AppState::Paused
        } else if is_recording {
            crate::types::AppState::Recording
        } else {
            crate::types::AppState::Ready
        };

        let plat = meeting.map(|m| m.platform.as_str());
        let pid = meeting.map(|m| m.pid);
        let proc = meeting.map(|m| m.app_name.as_str());

        let _ = crate::tray::update_tray_icon_with_meeting(app, app_state, plat, proc, pid);
        let meeting_info = plat.and_then(|p| pid.map(|pi| (p, pi)));
        crate::tray::update_tray_menu(app, loaded, meeting_info, is_recording);
    }
}

// ── Linux Fallback Implementation ───────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform_impl {
    use super::*;

    pub fn scan_meetings() -> Vec<MeetingInfo> {
        let mut results = Vec::new();
        let mut sys = sysinfo::System::new_all();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        for (pid, process) in sys.processes() {
            let name = process.name().to_string_lossy().to_lowercase();
            let platform = if name.contains("zoom") {
                Some(("Zoom", "zoom"))
            } else if name.contains("teams") {
                Some(("Microsoft Teams", "teams"))
            } else if name.contains("slack") {
                Some(("Slack", "slack"))
            } else if name.contains("discord") {
                Some(("Discord", "discord"))
            } else if name.contains("webex") {
                Some(("Cisco Webex", "webex"))
            } else {
                None
            };

            if let Some((app_name, platform_str)) = platform {
                results.push(MeetingInfo {
                    pid: pid.as_u32(),
                    app_name: app_name.to_string(),
                    title: "".to_string(),
                    url: "".to_string(),
                    platform: platform_str.to_string(),
                    confidence: 50,
                    should_record: true,
                    is_using_mic: true,
                    is_playing_audio: true,
                });
            }
        }

        results
    }

    pub struct WatcherHandle;

    pub fn start_watcher(
        _app_handle: AppHandle,
        _active_cache: Arc<Mutex<Vec<MeetingInfo>>>,
    ) -> Result<WatcherHandle, String> {
        Ok(WatcherHandle)
    }
}

// ── Public Manager ──────────────────────────────────────────────────────────

/// Reconciliation scans (2.5s apart) a cached meeting must be missing from
/// before it is reported as ended.
const REAPER_MISSES_BEFORE_END: u32 = 2;

pub struct MeetingDetectorManager {
    active_meetings: Arc<Mutex<Vec<MeetingInfo>>>,
    watcher_handle: Arc<Mutex<Option<platform_impl::WatcherHandle>>>,
    is_watching: Arc<AtomicBool>,
}

impl Default for MeetingDetectorManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MeetingDetectorManager {
    pub fn new() -> Self {
        Self {
            active_meetings: Arc::new(Mutex::new(Vec::new())),
            watcher_handle: Arc::new(Mutex::new(None)),
            is_watching: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Perform a synchronous scan of running audio processes and windows.
    pub fn scan(&self) -> Vec<MeetingInfo> {
        let detected = platform_impl::scan_meetings();
        let mut cache = self.active_meetings.lock().unwrap();
        let test_meetings: Vec<_> = cache.iter().filter(|m| m.pid == 99999).cloned().collect();
        *cache = detected.clone();
        for tm in test_meetings {
            if !cache.iter().any(|m| m.pid == tm.pid) {
                cache.push(tm.clone());
            }
        }
        cache.clone()
    }

    /// Start the background watcher to emit meeting events to the frontend.
    pub fn start_watching(&self, app_handle: AppHandle) -> Result<(), String> {
        if self.is_watching.load(Ordering::SeqCst) {
            return Ok(());
        }

        let handle = platform_impl::start_watcher(app_handle.clone(), self.active_meetings.clone())?;
        *self.watcher_handle.lock().unwrap() = Some(handle);
        self.is_watching.store(true, Ordering::SeqCst);
        println!("[INFO] Meeting detection background watcher active");

        // Spawn periodic background liveness reaper and scan reconciler
        let is_watching_reaper = self.is_watching.clone();
        let active_cache_reaper = self.active_meetings.clone();
        let app_handle_reaper = app_handle.clone();

        tauri::async_runtime::spawn(async move {
            let mut sys = sysinfo::System::new();
            // Consecutive reconciliation scans each cached meeting has been missing
            // from. A single scan can come back empty while windows change focus
            // (e.g. another browser window opening), so one miss is not an ending.
            let mut missed_scans: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            while is_watching_reaper.load(Ordering::SeqCst) {
                tokio::time::sleep(tokio::time::Duration::from_millis(2500)).await;
                if !is_watching_reaper.load(Ordering::SeqCst) {
                    break;
                }

                // 1. Check process liveness
                sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
                let mut dead_meetings = Vec::new();
                {
                    let cache = active_cache_reaper.lock().unwrap();
                    for m in cache.iter() {
                        if m.pid > 0 && m.pid != 99999 && sys.process(sysinfo::Pid::from_u32(m.pid)).is_none() {
                            dead_meetings.push(m.clone());
                        }
                    }
                }

                let mut changed = false;
                if !dead_meetings.is_empty() {
                    let mut cache = active_cache_reaper.lock().unwrap();
                    for dead in dead_meetings {
                        println!(
                            "[MEETING REAPER] Process PID {} exited, pruning meeting: {} - '{}'",
                            dead.pid, dead.app_name, dead.title
                        );
                        cache.retain(|m| m.pid != dead.pid);
                        let _ = app_handle_reaper.emit("meeting-ended", &dead);
                        changed = true;
                    }
                }

                // 2. Periodic reconciliation with real OS scan
                let scanned = platform_impl::scan_meetings();
                let mut closed_meetings = Vec::new();
                let mut adopted_meetings = Vec::new();
                {
                    let mut cache = active_cache_reaper.lock().unwrap();
                    cache.retain(|cached| {
                        let is_still_scanned = scanned.iter().any(|s| {
                            s.pid == cached.pid || (s.app_name == cached.app_name && s.platform == cached.platform && !cached.platform.is_empty())
                        });
                        // Synthetic test mock PIDs (pid == 99999) are kept until explicitly cleared
                        let is_synthetic_test = cached.pid == 99999;
                        if is_still_scanned || is_synthetic_test {
                            missed_scans.remove(&cached.pid);
                            return true;
                        }
                        let misses = missed_scans.entry(cached.pid).or_insert(0);
                        *misses += 1;
                        if *misses < REAPER_MISSES_BEFORE_END {
                            return true;
                        }
                        missed_scans.remove(&cached.pid);
                        closed_meetings.push(cached.clone());
                        false
                    });

                    // The watcher only reports transitions. If a meeting was pruned
                    // (or its Started event filtered) while the call carried on, it
                    // never fires Started again, so adopt live calls the scan sees.
                    for s in scanned.iter() {
                        let live = s.should_record || s.confidence >= 70;
                        let known = cache.iter().any(|c| {
                            c.pid == s.pid || (c.app_name == s.app_name && c.platform == s.platform)
                        });
                        if live && !known {
                            cache.push(s.clone());
                            adopted_meetings.push(s.clone());
                        }
                    }
                }

                for adopted in adopted_meetings {
                    println!(
                        "[MEETING REAPER] Adopted live call missed by watcher: {} - '{}' (pid: {})",
                        adopted.app_name, adopted.title, adopted.pid
                    );
                    let _ = app_handle_reaper.emit("meeting-detected", &adopted);
                    changed = true;
                }

                for closed in closed_meetings {
                    println!(
                        "[MEETING REAPER] Call closed (unscanned): {} - '{}' (pid: {})",
                        closed.app_name, closed.title, closed.pid
                    );
                    let _ = app_handle_reaper.emit("meeting-ended", &closed);
                    changed = true;
                }

                if changed {
                    let cache = active_cache_reaper.lock().unwrap();
                    let active_top = cache.first().cloned();
                    sync_detector_tray(&app_handle_reaper, active_top.as_ref());
                }
            }
        });

        Ok(())
    }

    /// Stop the background watcher.
    pub fn stop_watching(&self) {
        *self.watcher_handle.lock().unwrap() = None;
        self.is_watching.store(false, Ordering::SeqCst);
        println!("[INFO] Meeting detection background watcher stopped");
    }

    pub fn get_status(&self) -> MeetingDetectionStatus {
        let cache = self.active_meetings.lock().unwrap().clone();
        MeetingDetectionStatus {
            is_watching: self.is_watching.load(Ordering::SeqCst),
            supported: cfg!(any(target_os = "macos", target_os = "windows")),
            active_meetings: cache,
        }
    }

    /// Injects a simulated meeting for testing and emits `meeting-detected`
    pub fn inject_test_meeting(&self, app_handle: &AppHandle, info: MeetingInfo) {
        let mut cache = self.active_meetings.lock().unwrap();
        if let Some(existing) = cache.iter_mut().find(|m| m.pid == info.pid) {
            *existing = info.clone();
        } else {
            cache.push(info.clone());
        }
        sync_detector_tray(app_handle, Some(&info));
        let _ = app_handle.emit("meeting-detected", &info);
    }

    /// Clears simulated meetings for testing and emits `meeting-ended`
    pub fn clear_test_meetings(&self, app_handle: &AppHandle) {
        let mut cache = self.active_meetings.lock().unwrap();
        let ended = std::mem::take(&mut *cache);
        sync_detector_tray(app_handle, None);
        for m in ended {
            let _ = app_handle.emit("meeting-ended", &m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn catalogued_apps_are_left_to_meeting_record() {
        assert!(is_catalogued_app("Google Chrome Helper", "com.google.Chrome.helper"));
        assert!(is_catalogued_app("zoom.us", "us.zoom.xos"));
        assert!(is_catalogued_app("com.apple.WebKit.GPU", "com.apple.WebKit.GPU"));
        assert!(is_catalogued_app("ms-teams.exe", ""));
        assert!(is_catalogued_app("Chrome.exe", ""));
        assert!(!is_catalogued_app("Signal Helper (Renderer)", "org.whispersystems.signal-desktop.helper.Renderer"));
        assert!(!is_catalogued_app("WhatsApp.exe", ""));
    }

    #[test]
    fn other_apps_are_named_after_their_bundle_or_executable() {
        let signal = Path::new("/Applications/Signal.app");
        let helper = Path::new("/Applications/Signal.app/Contents/Frameworks/Signal Helper.app/Contents/MacOS/Signal Helper");
        assert_eq!(other_call_app_name("Signal Helper", signal, helper).as_deref(), Some("Signal"));
        let facetime = Path::new("/usr/libexec/avconferenced");
        assert_eq!(other_call_app_name("avconferenced", facetime, facetime).as_deref(), Some("FaceTime"));
        // OS daemons and bare executables on macOS are not apps the user runs.
        let daemon = Path::new("/System/Library/PrivateFrameworks/CoreSpeech.framework/corespeechd");
        assert_eq!(other_call_app_name("corespeechd", daemon, daemon), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_system_processes_are_not_offered() {
        let whatsapp = Path::new(r"C:\Program Files\WindowsApps\WhatsApp\WhatsApp.exe");
        assert_eq!(other_call_app_name("WhatsApp.exe", whatsapp, whatsapp).as_deref(), Some("WhatsApp"));
        let svchost = Path::new(r"C:\Windows\System32\svchost.exe");
        assert_eq!(other_call_app_name("svchost.exe", svchost, svchost), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn browsers_are_matched_by_name_for_tab_listing() {
        assert_eq!(scriptable_browser("Google Chrome"), Some(("Google Chrome", TabScript::Chromium)));
        assert_eq!(scriptable_browser("Google Chrome Helper (Renderer)"), Some(("Google Chrome", TabScript::Chromium)));
        assert_eq!(scriptable_browser("Google Chrome Beta"), Some(("Google Chrome Beta", TabScript::Chromium)));
        assert_eq!(scriptable_browser("Arc"), Some(("Arc", TabScript::Chromium)));
        assert_eq!(scriptable_browser("Microsoft Edge"), Some(("Microsoft Edge", TabScript::Chromium)));
        assert_eq!(scriptable_browser("Brave Browser"), Some(("Brave Browser", TabScript::Chromium)));
        assert_eq!(scriptable_browser("Safari"), Some(("Safari", TabScript::Safari)));
        assert_eq!(scriptable_browser("Archive Utility"), None);
        assert_eq!(scriptable_browser("firefox"), None);
        assert_eq!(scriptable_browser("zoom.us"), None);
    }

    #[test]
    fn test_is_prejoin_window_title() {
        assert!(is_prejoin_window_title("Zoom Waiting Room"));
        assert!(is_prejoin_window_title("Please wait, the meeting host will let you in soon - Waiting Room"));
        assert!(is_prejoin_window_title("Choose ONE Meeting Option"));
        assert!(is_prejoin_window_title("joining..."));
        assert!(is_prejoin_window_title("Joining Meeting"));
        assert!(is_prejoin_window_title("Preview Audio & Video"));

        // Live calls should NOT be filtered
        assert!(!is_prejoin_window_title("Weekly Team Standup (Zoom Meeting)"));
        assert!(!is_prejoin_window_title("Google Meet - Design Sync"));
        assert!(!is_prejoin_window_title("Microsoft Teams | Engineering Review"));
    }

    #[test]
    fn test_classify_meeting_url() {
        assert_eq!(classify_meeting_url("https://meet.google.com/msp-enhn-zci"), Some("meet"));
        assert_eq!(classify_meeting_url("https://meet.google.com/msp-enhn-zci?authuser=0"), Some("meet"));
        assert_eq!(classify_meeting_url("https://meet.google.com/home"), None);
        assert_eq!(classify_meeting_url("https://meet.google.com/landing"), None);
        assert_eq!(classify_meeting_url("https://teams.live.com/v2/"), Some("teams"));
        assert_eq!(classify_meeting_url("https://teams.live.com/meet/9343424118326?p=abc"), Some("teams"));
        assert_eq!(classify_meeting_url("https://teams.microsoft.com/v2/"), Some("teams"));
        assert_eq!(classify_meeting_url("https://app.zoom.us/wc/81234567890/join"), Some("zoom"));
        assert_eq!(classify_meeting_url("https://us05web.zoom.us/j/81234567890?pwd=x"), Some("zoom"));
        assert_eq!(classify_meeting_url("https://zoom.us/profile"), None);
        assert_eq!(classify_meeting_url("https://acme.webex.com/meet/jdoe"), Some("webex"));
        assert_eq!(classify_meeting_url("https://acme.webex.com/wbxmjs/joinservice/sites/acme/meeting"), Some("webex"));
        assert_eq!(classify_meeting_url("https://web.webex.com/sign-in"), None);
        assert_eq!(classify_meeting_url("https://www.youtube.com/watch?v=x"), None);
        assert_eq!(classify_meeting_url(""), None);
    }

    #[test]
    fn meeting_urls_ignore_lookalike_hosts_and_bad_codes() {
        assert_eq!(classify_meeting_url("https://meet.google.com.evil.com/abc-defg-hij"), None);
        assert_eq!(classify_meeting_url("https://notzoom.us/j/123"), None);
        assert_eq!(classify_meeting_url("https://teams.microsoft.com.example.org/v2/"), None);
        assert_eq!(classify_meeting_url("https://meet.google.com/ab-defg-hij"), None);
        assert_eq!(classify_meeting_url("https://meet.google.com/ab1-defg-hij"), None);
        assert_eq!(classify_meeting_url("https://meet.google.com/abc-defg-hij-klm"), None);
    }

    #[test]
    fn meeting_urls_tolerate_case_ports_and_missing_scheme() {
        assert_eq!(classify_meeting_url("HTTPS://MEET.GOOGLE.COM/ABC-DEFG-HIJ"), Some("meet"));
        assert_eq!(classify_meeting_url("meet.google.com/abc-defg-hij"), Some("meet"));
        assert_eq!(classify_meeting_url("https://zoom.us:443/j/123"), Some("zoom"));
        assert_eq!(classify_meeting_url("https://meet.google.com/abc-defg-hij#frag"), Some("meet"));
        assert_eq!(classify_meeting_url("https://web.webex.com/meeting/abc"), Some("webex"));
        assert_eq!(classify_meeting_url("https://acme.webex.com/"), None);
    }

    #[test]
    fn prejoin_titles_match_whole_title_for_bare_app_names() {
        assert!(is_prejoin_window_title("Google Meet"));
        assert!(is_prejoin_window_title("  meet".trim()));
        assert!(!is_prejoin_window_title("Meet with Sam"));
        assert!(!is_prejoin_window_title(""));
    }

    #[test]
    fn test_meeting_detector_scan() {
        let manager = MeetingDetectorManager::new();
        let status = manager.get_status();
        assert_eq!(status.supported, cfg!(any(target_os = "macos", target_os = "windows")));

        // Calling scan should not crash and should return a valid slice
        let meetings = manager.scan();
        println!("Scan returned {} active meetings:", meetings.len());
        for (i, m) in meetings.iter().enumerate() {
            println!(
                "  [{}] app='{}', platform='{}', title='{}', url='{}', pid={}, confidence={}%, should_record={}, is_using_mic={}, is_playing_audio={}",
                i + 1, m.app_name, m.platform, m.title, m.url, m.pid, m.confidence, m.should_record, m.is_using_mic, m.is_playing_audio
            );
        }
    }
}
