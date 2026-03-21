use crate::state::AudioState;
use crate::types::{ASREngine, AppState, HotkeyBinding};
use cpal::traits::{DeviceTrait, HostTrait};
use dirs::data_local_dir;
use serde::Serialize;
use std::fs;
use std::path::Path;
use sysinfo::System;
use tauri::{Emitter, Manager};

/// Shows the main window. Called by the frontend once it has finished its own
/// initialization so the user never sees a loading state when the window opens.
#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        // macOS: appear on all Spaces so the window can be focused from any Space.
        let _ = window.set_visible_on_all_workspaces(true);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn show_overlay(app: tauri::AppHandle) {
    crate::overlay::show(&app);
}

#[tauri::command]
pub fn hide_overlay(app: tauri::AppHandle) {
    crate::overlay::hide(&app);
}

/// Updates the overlay phase from the frontend.
/// macOS  → updates native NSPanel + egui context directly.
/// Win/Linux → emits "overlay-state" to the WebView overlay window.
#[tauri::command]
pub fn set_overlay_state(
    app: tauri::AppHandle,
    phase: String,
    text: Option<String>,
    ms: Option<u64>,
    engine: Option<String>,
) {
    crate::overlay::set_state(
        &app,
        crate::overlay::OverlayStatePayload {
            phase,
            text,
            ms,
            engine,
        },
    );
}

/// Forwards an action from the overlay HUD back to the main application UI.
#[tauri::command]
pub fn request_overlay_action(app: tauri::AppHandle, action: String) -> Result<(), String> {
    match action.as_str() {
        "pause" | "resume" | "cancel" => {
            app.emit("overlay-action", action.clone())
                .map_err(|e| format!("Failed to emit overlay action: {}", e))?;
            crate::overlay::restore_focus(&app);
            Ok(())
        }
        _ => Err(format!("Unknown overlay action: {}", action)),
    }
}

/// Returns the names of all available audio input devices on this machine.
///
/// macOS fix: Async with spawn_blocking because cpal device enumeration
/// touches CoreAudio, which can block and freeze the AppKit main thread.
#[tauri::command]
pub async fn list_input_devices() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(|| {
        let host = cpal::default_host();
        host.input_devices()
            .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

/// Returns the name of the microphone that will actually be used for the next recording.
/// If the user has selected a specific device, returns that; otherwise returns the system default.
#[tauri::command]
pub async fn get_active_input_device(state: tauri::State<'_, crate::state::AudioState>) -> Result<String, String> {
    let preferred = state.selected_input_device.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        if let Some(name) = preferred {
            // Verify the preferred device still exists
            if let Ok(devices) = host.input_devices() {
                if devices.into_iter().any(|d| d.name().ok().as_deref() == Some(name.as_str())) {
                    return Ok(name);
                }
            }
        }
        // Fall back to system default
        host.default_input_device()
            .and_then(|d| d.name().ok())
            .ok_or_else(|| "No microphone found".to_string())
    })
    .await
    .map_err(|e| format!("{}", e))?
}

// Simple test command to see if Rust is working
#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
pub fn get_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    { "macos" }
    #[cfg(target_os = "windows")]
    { "windows" }
    #[cfg(target_os = "linux")]
    { "linux" }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { "unknown" }
}

/// Returns true when running on macOS with an Apple Silicon chip (M-series, aarch64).
#[tauri::command]
pub fn is_apple_silicon() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    { true }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    { false }
}

#[derive(Serialize)]
pub struct SystemInfo {
    pub cpu_name: String,
    pub cpu_cores: usize,
    pub ram_total_gb: f32,
    pub gpu_name: String,
    pub cuda_available: bool,
    pub vram_gb: Option<f32>,
    pub backend_hint: String,
}

/// Returns CPU, RAM, and GPU info for the first-launch setup screen.
///
/// macOS fix: Async with spawn_blocking because sysinfo + GPU detection
/// shell out to external commands and can block the AppKit main thread.
#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, String> {
    tauri::async_runtime::spawn_blocking(get_system_info_blocking)
        .await
        .map_err(|e| format!("get_system_info task failed: {}", e))
}

fn get_system_info_blocking() -> SystemInfo {
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_else(|| "Unknown CPU".to_string());

    let cpu_cores = sys.cpus().len();

    let ram_total_gb = sys.total_memory() as f32 / 1_073_741_824.0; // bytes → GB

    let (gpu_name, cuda_available, vram_gb) = detect_gpu();

    let backend_hint = if cuda_available {
        "CUDA".to_string()
    } else {
        #[cfg(target_os = "macos")]
        { "Metal".to_string() }
        #[cfg(not(target_os = "macos"))]
        {
            if gpu_name != "Unknown" {
                #[cfg(target_os = "windows")]
                { "DirectML / Vulkan".to_string() }
                #[cfg(not(target_os = "windows"))]
                { "Vulkan".to_string() }
            } else {
                "CPU".to_string()
            }
        }
    };

    SystemInfo {
        cpu_name,
        cpu_cores,
        ram_total_gb,
        gpu_name,
        cuda_available,
        vram_gb,
        backend_hint,
    }
}

// ── System audio mute / unmute ────────────────────────────────────────────────

/// macOS fix: Async with spawn_blocking — system audio control
/// may involve CoreAudio calls that block.
#[tauri::command]
pub async fn mute_system_audio() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| crate::system_audio::mute())
        .await
        .map_err(|e| format!("mute_system_audio task failed: {}", e))?
}

/// macOS fix: Async with spawn_blocking — system audio control
/// may involve CoreAudio calls that block.
#[tauri::command]
pub async fn unmute_system_audio() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| crate::system_audio::unmute())
        .await
        .map_err(|e| format!("unmute_system_audio task failed: {}", e))?
}

// ── Microphone permission check ───────────────────────────────────────────────

/// Returns "granted", "denied", or "undetermined" for microphone access.
/// On macOS, this queries AVCaptureDevice authorization status.
/// On non-macOS platforms, always returns "granted" (no permission gate).
#[tauri::command]
pub async fn check_microphone_permission() -> String {
    tauri::async_runtime::spawn_blocking(check_microphone_permission_blocking)
        .await
        .unwrap_or_else(|_| "denied".to_string())
}

#[cfg(target_os = "macos")]
fn check_microphone_permission_blocking() -> String {
    // AVAuthorizationStatus: 0 = notDetermined, 1 = restricted, 2 = denied, 3 = authorized
    // [AVCaptureDevice authorizationStatusForMediaType:] is a pure read — never shows a dialog.
    use std::ffi::CStr;

    extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn objc_msgSend(receiver: *mut std::ffi::c_void, sel: *mut std::ffi::c_void, ...) -> i64;
    }

    unsafe {
        let cls = objc_getClass(
            CStr::from_bytes_with_nul_unchecked(b"AVCaptureDevice\0").as_ptr(),
        );
        if cls.is_null() {
            return "undetermined".to_string();
        }
        let sel = sel_registerName(
            CStr::from_bytes_with_nul_unchecked(b"authorizationStatusForMediaType:\0").as_ptr(),
        );

        // AVMediaTypeAudio = @"soun" — create an NSString via stringWithUTF8String:
        let nsstring_cls = objc_getClass(
            CStr::from_bytes_with_nul_unchecked(b"NSString\0").as_ptr(),
        );
        let string_sel = sel_registerName(
            CStr::from_bytes_with_nul_unchecked(b"stringWithUTF8String:\0").as_ptr(),
        );
        let media_audio: *mut std::ffi::c_void = std::mem::transmute(
            (std::mem::transmute::<_, extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *const std::ffi::c_char) -> *mut std::ffi::c_void>(
                objc_msgSend as *const () as *mut std::ffi::c_void
            ))(nsstring_cls, string_sel, b"soun\0".as_ptr() as *const std::ffi::c_char)
        );

        let status = (std::mem::transmute::<_, extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void) -> i64>(
            objc_msgSend as *const () as *mut std::ffi::c_void
        ))(cls, sel, media_audio);

        match status {
            3 => "granted".to_string(),
            2 => "denied".to_string(),
            1 => "restricted".to_string(), // MDM / parental controls — user cannot change this
            _ => "undetermined".to_string(),
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn check_microphone_permission_blocking() -> String {
    "granted".to_string()
}

/// macOS only: Requests microphone permission by briefly opening an audio input
/// stream. This triggers the system permission dialog if status is "undetermined".
/// Returns the updated permission status after the attempt.
#[tauri::command]
pub async fn request_microphone_permission() -> String {
    tauri::async_runtime::spawn_blocking(request_microphone_permission_blocking)
        .await
        .unwrap_or_else(|_| "denied".to_string())
}

fn request_microphone_permission_blocking() -> String {
    // If already determined, skip the prompt entirely.
    #[cfg(target_os = "macos")]
    {
        let current = check_microphone_permission_blocking();
        if current != "undetermined" {
            return current;
        }
    }

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            // No device found — return actual status rather than assuming "denied"
            #[cfg(target_os = "macos")]
            return check_microphone_permission_blocking();
            #[cfg(not(target_os = "macos"))]
            return "granted".to_string();
        }
    };
    let config = match device.default_input_config() {
        Ok(c) => c,
        Err(_) => {
            #[cfg(target_os = "macos")]
            return check_microphone_permission_blocking();
            #[cfg(not(target_os = "macos"))]
            return "granted".to_string();
        }
    };
    let config: cpal::StreamConfig = config.into();

    // Attempting to open an audio input stream on macOS 10.14+ with status
    // "notDetermined" triggers the AVFoundation permission dialog. The build
    // call may fail immediately (before the user responds) — that is expected;
    // we must NOT return "denied" here. The frontend polls every 1.5 s so it
    // will pick up the granted/denied state once the user taps Allow/Deny.
    let stream = device.build_input_stream(
        &config,
        move |_data: &[f32], _: &_| {},
        move |_err| {},
        None,
    );
    if let Ok(s) = stream {
        let _ = s.play();
        // Keep alive briefly so CoreAudio registers the session
        std::thread::sleep(std::time::Duration::from_millis(300));
        drop(s);
    }
    // Always return the live status — never hard-code "denied"
    #[cfg(target_os = "macos")]
    return check_microphone_permission_blocking();
    #[cfg(not(target_os = "macos"))]
    return "granted".to_string();
}

// ── GPU detection ─────────────────────────────────────────────────────────────

fn detect_gpu() -> (String, bool, Option<f32>) {
    // nvidia-smi works cross-platform wherever NVIDIA drivers are installed
    if let Some((name, vram)) = try_nvidia_smi() {
        return (name, true, Some(vram));
    }

    // Platform fallbacks for non-NVIDIA or when nvidia-smi isn't in PATH
    #[cfg(target_os = "windows")]
    if let Some(name) = try_wmic_gpu() {
        let is_nvidia = name.to_lowercase().contains("nvidia");
        return (name, is_nvidia, None);
    }

    #[cfg(target_os = "macos")]
    if let Some(name) = try_macos_gpu() {
        return (name, false, None); // macOS uses Metal, not CUDA
    }

    #[cfg(target_os = "linux")]
    if let Some(name) = try_lspci_gpu() {
        let is_nvidia = name.to_lowercase().contains("nvidia");
        return (name, is_nvidia, None);
    }

    ("Unknown".to_string(), false, None)
}

fn try_nvidia_smi() -> Option<(String, f32)> {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?;
    let mut parts = line.splitn(2, ',');
    let name = parts.next()?.trim().to_string();
    let vram_mb: f32 = parts.next()?.trim().parse().ok()?;

    Some((name, vram_mb / 1024.0))
}

#[cfg(target_os = "windows")]
fn try_wmic_gpu() -> Option<String> {
    let out = std::process::Command::new("wmic")
        .args(["path", "win32_VideoController", "get", "name"])
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1) // skip "Name" header
        .map(|l| l.trim().to_string())
        .find(|l| !l.is_empty())
}

#[cfg(target_os = "macos")]
fn try_macos_gpu() -> Option<String> {
    let out = std::process::Command::new("system_profiler")
        .args(["SPDisplaysDataType"])
        .output()
        .ok()?;

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.trim_start().starts_with("Chipset Model:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .map(|s| s.trim().to_string())
}

#[cfg(target_os = "linux")]
fn try_lspci_gpu() -> Option<String> {
    let out = std::process::Command::new("lspci").output().ok()?;

    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .find(|l| l.to_lowercase().contains("vga") || l.to_lowercase().contains("3d controller"))?;

    // "01:00.0 VGA compatible controller: NVIDIA Corporation GeForce ..."
    // We want everything after the second ':'
    let after_addr = line.splitn(2, ' ').nth(1)?;
    after_addr.splitn(2, ':').nth(1).map(|s| s.trim().to_string())
}

// ── Accessibility / Input Monitoring permission ───────────────────────────────

/// Returns true if this process is trusted for Accessibility (and therefore
/// Input Monitoring) on macOS. On all other platforms returns true immediately.
#[tauri::command]
pub fn check_accessibility_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_accessibility_trusted_query()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Opens the macOS System Settings pane for Privacy & Security → Accessibility
/// so the user can grant permission without hunting for it. No-op on other OSes.
#[tauri::command]
pub fn open_accessibility_settings() {
    #[cfg(target_os = "macos")]
    {
        // x-apple.systempreferences: deep-link opens directly to the right pane.
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

/// Relaunches the application process. Used after the user grants a permission
/// that requires a restart (e.g. Accessibility on macOS) to take effect.
#[tauri::command]
pub fn relaunch_app(app: tauri::AppHandle) {
    app.restart();
}

fn factory_reset_marker_path() -> Result<std::path::PathBuf, String> {
    let app_data = data_local_dir().ok_or_else(|| "Could not find app data directory".to_string())?;
    Ok(app_data.join("taurscribe-factory-reset-pending"))
}

fn remove_path_with_retries(path: &Path) -> Result<(), String> {
    const MAX_ATTEMPTS: u32 = 5;

    if !path.exists() {
        return Ok(());
    }

    let is_dir = path.is_dir();
    let kind = if is_dir { "directory" } else { "file" };
    let mut last_error = None;

    for attempt in 1..=MAX_ATTEMPTS {
        let result = if is_dir {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };

        match result {
            Ok(()) => return Ok(()),
            Err(_) if !path.exists() => return Ok(()),
            Err(err) => {
                last_error = Some(err);
                if attempt < MAX_ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_millis(150 * attempt as u64));
                }
            }
        }
    }

    let message = last_error
        .map(|err| err.to_string())
        .unwrap_or_else(|| "unknown error".to_string());
    Err(format!(
        "Failed to remove {kind} at {}: {}",
        path.display(),
        message
    ))
}

fn clear_app_data_root(base: &Path) -> Result<(), String> {
    if !base.exists() {
        return Ok(());
    }

    remove_path_with_retries(base)
}

fn clear_app_data_root_runtime_safe(base: &Path) -> Result<(), String> {
    if !base.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(base)
        .map_err(|e| format!("Failed to read app data directory {}: {}", base.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| {
            format!(
                "Failed to inspect app data directory {}: {}",
                base.display(),
                e
            )
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // On Windows WebView2 keeps EBWebView locked while the app is running.
        // We skip it in dev-mode resets and let the page reload handle the rest.
        if name.to_ascii_lowercase().starts_with("ebwebview") {
            continue;
        }

        remove_path_with_retries(&path)?;
    }

    Ok(())
}

pub fn perform_pending_factory_reset_on_startup() -> Result<(), String> {
    let marker_path = factory_reset_marker_path()?;
    if !marker_path.exists() {
        return Ok(());
    }

    let app_data = data_local_dir().ok_or_else(|| "Could not find app data directory".to_string())?;
    println!("[RESET] Pending factory reset detected. Clearing app data before startup...");

    for name in ["Taurscribe", "taurscribe"] {
        clear_app_data_root(&app_data.join(name))?;
    }

    remove_path_with_retries(&marker_path)?;
    println!("[RESET] Factory reset completed successfully.");
    Ok(())
}

/// Deletes all persisted app data (models, settings, history, temp) and relaunches.
/// This is a full "factory reset".
#[tauri::command]
pub async fn factory_reset_app_data(
    app: tauri::AppHandle,
    state: tauri::State<'_, AudioState>,
) -> Result<bool, String> {
    if state.recording_handle.lock().unwrap().is_some() {
        return Err("Stop the current recording before running a factory reset.".to_string());
    }

    if let Ok(mut whisper) = state.whisper.lock() {
        whisper.unload();
    }
    if let Ok(mut parakeet) = state.parakeet.lock() {
        parakeet.unload();
    }
    if let Ok(mut granite) = state.granite_speech.lock() {
        granite.unload();
    }
    if let Ok(mut llm) = state.llm.lock() {
        *llm = None;
    }
    if let Ok(mut denoiser) = state.denoiser.lock() {
        *denoiser = None;
    }
    if let Ok(mut transcript) = state.session_transcript.lock() {
        transcript.clear();
    }
    if let Ok(mut last_recording_path) = state.last_recording_path.lock() {
        *last_recording_path = None;
    }
    if let Ok(mut app_state) = state.current_app_state.lock() {
        *app_state = AppState::Ready;
    }
    if let Ok(mut active_engine) = state.active_engine.lock() {
        *active_engine = ASREngine::Whisper;
    }
    if let Ok(mut selected_input_device) = state.selected_input_device.lock() {
        *selected_input_device = None;
    }
    if let Ok(mut hotkey_config) = state.hotkey_config.lock() {
        *hotkey_config = HotkeyBinding::default();
    }
    if let Ok(mut close_behavior) = state.close_behavior.lock() {
        *close_behavior = "tray".to_string();
    }
    state
        .hotkey_suppressed
        .store(false, std::sync::atomic::Ordering::Relaxed);
    state
        .recording_paused
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let _ = crate::system_audio::force_unmute();
    let _ = crate::tray::update_tray_icon(&app, AppState::Ready);
    crate::overlay::hide(&app);

    if cfg!(debug_assertions) {
        let app_data = data_local_dir().ok_or_else(|| "Could not find app data directory".to_string())?;
        for name in ["Taurscribe", "taurscribe"] {
            clear_app_data_root_runtime_safe(&app_data.join(name))?;
        }
        return Ok(false);
    }

    let marker_path = factory_reset_marker_path()?;
    fs::write(&marker_path, b"pending")
        .map_err(|e| format!("Failed to create factory reset marker at {}: {}", marker_path.display(), e))?;

    app.restart();
}

/// Non-prompting accessibility check used by the command (no dialog pop-up).
#[cfg(target_os = "macos")]
fn macos_accessibility_trusted_query() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    extern "C" {
        fn AXIsProcessTrustedWithOptions(
            options: core_foundation::base::CFTypeRef,
        ) -> bool;
    }

    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::false_value(); // do NOT prompt here — wizard controls when to prompt
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);

    unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef()) }
}
