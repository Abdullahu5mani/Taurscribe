//! Meeting capture has a separate macOS permission from microphone capture.

#[derive(serde::Serialize)]
pub struct MeetingPermissions {
    pub microphone: String,
    pub system_audio: String,
}

#[cfg(any(target_os = "macos", test))]
fn supports_audio_taps(version: &str) -> bool {
    let mut parts = version.trim().split('.');
    match (parts.next().and_then(|p| p.parse::<u32>().ok()), parts.next().and_then(|p| p.parse::<u32>().ok())) {
        (Some(major), minor) => major > 14 || (major == 14 && minor.unwrap_or(0) >= 2),
        _ => false,
    }
}

pub(crate) fn system_audio_permission() -> String {
    #[cfg(target_os = "macos")]
    {
        if !supports_audio_taps(&sysinfo::System::os_version().unwrap_or_default()) {
            return "unsupported".into();
        }
        match meeting_record::permissions::status(meeting_record::Permission::SystemAudio) {
            meeting_record::PermissionStatus::Granted => "granted",
            meeting_record::PermissionStatus::NotRequired => "not_required",
            meeting_record::PermissionStatus::Denied => "denied",
            meeting_record::PermissionStatus::Unknown => "undetermined",
        }.into()
    }
    #[cfg(target_os = "windows")]
    { "not_required".into() }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { "unsupported".into() }
}

#[tauri::command]
pub async fn check_meeting_permissions() -> Result<MeetingPermissions, String> {
    tauri::async_runtime::spawn_blocking(|| MeetingPermissions {
        microphone: super::misc::check_microphone_permission_blocking(),
        system_audio: system_audio_permission(),
    }).await.map_err(|e| format!("Could not check meeting permissions: {e}"))
}

#[tauri::command]
pub async fn request_system_audio_permission() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        #[cfg(target_os = "macos")]
        if system_audio_permission() == "undetermined" {
            meeting_record::permissions::request(meeting_record::Permission::SystemAudio)
                .map_err(|e| format!("Could not request system audio permission: {e}"))?;
        }
        Ok(system_audio_permission())
    }).await.map_err(|e| format!("System audio permission request failed: {e}"))?
}

#[tauri::command]
pub fn open_system_audio_settings() {
    #[cfg(target_os = "macos")]
    super::misc::open_macos_privacy_settings("Privacy_ScreenCapture");
}

/// Backstop for CLI, tray, and other starts that bypass the UI preflight. Run
/// before creating the WAV or opening either track; never substitute mic-only.
pub(crate) fn require_meeting_permissions() -> Result<(), String> {
    let system_audio = system_audio_permission();
    match system_audio.as_str() {
        "granted" | "not_required" => {},
        "unsupported" => return Err("Meeting audio capture is unsupported on this OS. On Mac, update to macOS 14.2 or later.".into()),
        _ => return Err("System audio permission is not granted. Allow Taurscribe in System Settings → Privacy & Security → Screen & System Audio Recording, then quit and reopen Taurscribe.".into()),
    }
    match super::misc::check_microphone_permission_blocking().as_str() {
        "granted" => Ok(()),
        _ => Err("Microphone permission denied. Allow Taurscribe in System Settings → Privacy & Security → Microphone.".into()),
    }
}

#[tauri::command]
pub fn retry_meeting_audio_capture() -> Result<(), String> {
    crate::audio_dual_channel::request_system_audio_retry()
}

#[cfg(test)]
mod tests {
    #[test]
    fn audio_taps_require_macos_14_2() {
        for v in ["13.6", "14.0", "14.1.9", "", "unknown"] {
            assert!(!super::supports_audio_taps(v), "{v}");
        }
        for v in ["14.2", "14.7.1", "15.0", "26.0", " 26.1\n"] {
            assert!(super::supports_audio_taps(v), "{v}");
        }
    }
}
