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
use tauri::{AppHandle, Emitter};

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
}

// ── macOS & Windows Implementation ──────────────────────────────────────────

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform_impl {
    use super::*;
    use meeting_record::{meetings, MeetingEvent};

    pub fn scan_meetings() -> Vec<MeetingInfo> {
        let detected = meetings::scan();
        let mut results = Vec::new();

        for m in detected {
            // Apply pre-join suppression filter
            if is_prejoin_window_title(&m.title) {
                continue;
            }

            results.push(MeetingInfo {
                pid: m.pid,
                app_name: m.app_name,
                title: m.title,
                url: m.url,
                platform: m.platform.as_str().to_string(),
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
            if is_prejoin_window_title(&meeting.title) {
                return;
            }

            let info = MeetingInfo {
                pid: meeting.pid,
                app_name: meeting.app_name.clone(),
                title: meeting.title.clone(),
                url: meeting.url.clone(),
                platform: meeting.platform.as_str().to_string(),
                confidence: meeting.confidence,
                should_record: meeting.should_record,
                is_using_mic: meeting.is_using_mic,
                is_playing_audio: meeting.is_playing_audio,
            };

            let mut cache = cache_clone.lock().unwrap();
            match event {
                MeetingEvent::Started => {
                    println!(
                        "[MEETING DETECTED] Started: {} - '{}' (pid: {}, confidence: {}%)",
                        info.app_name, info.title, info.pid, info.confidence
                    );
                    if !cache.iter().any(|m| m.pid == info.pid) {
                        cache.push(info.clone());
                    }
                    let _ = app_handle_clone.emit("meeting-detected", &info);
                }
                MeetingEvent::Updated => {
                    if let Some(existing) = cache.iter_mut().find(|m| m.pid == info.pid) {
                        *existing = info.clone();
                    }
                    let _ = app_handle_clone.emit("meeting-changed", &info);
                }
                MeetingEvent::Ended => {
                    println!(
                        "[MEETING ENDED] Ended: {} - '{}' (pid: {})",
                        info.app_name, info.title, info.pid
                    );
                    cache.retain(|m| m.pid != info.pid);
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

// ── Linux Fallback Implementation ───────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform_impl {
    use super::*;

    pub fn scan_meetings() -> Vec<MeetingInfo> {
        let mut results = Vec::new();
        let mut sys = sysinfo::System::new_all();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All);

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
        *cache = detected.clone();
        detected
    }

    /// Start the background watcher to emit meeting events to the frontend.
    pub fn start_watching(&self, app_handle: AppHandle) -> Result<(), String> {
        if self.is_watching.load(Ordering::SeqCst) {
            return Ok(());
        }

        let handle = platform_impl::start_watcher(app_handle, self.active_meetings.clone())?;
        *self.watcher_handle.lock().unwrap() = Some(handle);
        self.is_watching.store(true, Ordering::SeqCst);
        println!("[INFO] Meeting detection background watcher active");
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
    fn test_meeting_detector_scan() {
        let manager = MeetingDetectorManager::new();
        let status = manager.get_status();
        assert_eq!(status.supported, cfg!(any(target_os = "macos", target_os = "windows")));

        // Calling scan should not crash and should return a valid slice
        let meetings = manager.scan();
        println!("Scan returned {} active meetings", meetings.len());
    }
}
