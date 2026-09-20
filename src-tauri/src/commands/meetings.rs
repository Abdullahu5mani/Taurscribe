use crate::meeting_detector::{MeetingDetectionStatus, MeetingInfo};
use crate::state::AudioState;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn scan_active_meetings(
    state: State<'_, AudioState>,
) -> Result<Vec<MeetingInfo>, String> {
    Ok(state.meeting_detector.scan())
}

#[tauri::command]
pub async fn get_meeting_detection_status(
    state: State<'_, AudioState>,
) -> Result<MeetingDetectionStatus, String> {
    Ok(state.meeting_detector.get_status())
}

#[tauri::command]
pub async fn start_meeting_detection(
    app: AppHandle,
    state: State<'_, AudioState>,
) -> Result<(), String> {
    state.meeting_detector.start_watching(app)
}

#[tauri::command]
pub async fn stop_meeting_detection(state: State<'_, AudioState>) -> Result<(), String> {
    state.meeting_detector.stop_watching();
    Ok(())
}

#[tauri::command]
pub async fn set_audio_source_mode(
    mode: String,
    state: State<'_, AudioState>,
) -> Result<(), String> {
    let mode_str = match mode.to_lowercase().as_str() {
        "dual_channel" | "dual" | "system" => "dual_channel",
        _ => "mic",
    };
    *state.audio_source_mode.lock().unwrap() = mode_str.to_string();
    println!("[INFO] Audio source mode set to: {}", mode_str);
    Ok(())
}

#[tauri::command]
pub async fn get_audio_source_mode(state: State<'_, AudioState>) -> Result<String, String> {
    Ok(state.audio_source_mode.lock().unwrap().clone())
}

#[tauri::command]
pub async fn set_auto_record_meetings(
    enabled: bool,
    state: State<'_, AudioState>,
) -> Result<(), String> {
    state.auto_record_meetings.store(enabled, Ordering::SeqCst);
    println!("[INFO] Auto-record meetings set to: {}", enabled);
    Ok(())
}

#[tauri::command]
pub async fn get_auto_record_meetings(state: State<'_, AudioState>) -> Result<bool, String> {
    Ok(state.auto_record_meetings.load(Ordering::SeqCst))
}
