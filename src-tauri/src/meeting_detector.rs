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

// ── macOS & Windows Implementation ──────────────────────────────────────────

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform_impl {
    use super::*;
    use meeting_record::{meetings, MeetingEvent};

    /// meeting-record attributes Chrome's audio to a helper process and often
    /// cannot read a title or URL for it, so list every tab over AppleScript and
    /// pick the one that is a call.
    #[cfg(target_os = "macos")]
    fn resolve_browser_tab_info(app_name: &str) -> Option<(String, String, String)> {
        if !app_name.to_lowercase().contains("chrome") {
            return None;
        }
        let script = r#"
        tell application "Google Chrome"
            set out to ""
            try
                repeat with w in windows
                    repeat with t in tabs of w
                        set out to out & (title of t) & "|||" & (URL of t) & linefeed
                    end repeat
                end repeat
            end try
            return out
        end tell
        "#;
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let listing = String::from_utf8_lossy(&output.stdout);
        listing.lines().find_map(|line| {
            let (title, url) = line.split_once("|||")?;
            let platform = super::classify_meeting_url(url.trim())?;
            Some((title.trim().to_string(), url.trim().to_string(), platform.to_string()))
        })
    }

    pub fn scan_meetings() -> Vec<MeetingInfo> {
        let detected = meetings::scan();
        let mut results = Vec::new();

        for m in detected {
            let mut title = m.title.clone();
            let mut url = m.url.clone();
            let mut platform = m.platform.as_str().to_string();
            let mut app_name = m.app_name.clone();

            #[cfg(target_os = "macos")]
            if (title.trim().is_empty() || platform == "browser" || platform == "other") && app_name.to_lowercase().contains("chrome") {
                if let Some((tab_title, tab_url, tab_platform)) = resolve_browser_tab_info(&app_name) {
                    title = tab_title;
                    url = tab_url;
                    platform = tab_platform;
                    app_name = "Google Chrome".to_string();
                }
            }

            // Apply pre-join suppression filter
            if is_prejoin_window_title(&title) {
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

        results
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
            if (title.trim().is_empty() || platform == "browser" || platform == "other") && app_name.to_lowercase().contains("chrome") {
                if let Some((tab_title, tab_url, tab_platform)) = resolve_browser_tab_info(&app_name) {
                    title = tab_title;
                    url = tab_url;
                    platform = tab_platform;
                    app_name = "Google Chrome".to_string();
                }
            }

            let is_prejoin = is_prejoin_window_title(&title);
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
