use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;
use sysinfo::System;
use tauri::Manager;

/// Shows the main window. Called by the frontend once it has finished its own
/// initialization so the user never sees a loading state when the window opens.
#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub fn show_overlay(app: tauri::AppHandle) {
    if let Some(overlay) = app.get_webview_window("overlay") {
        let monitor = cursor_monitor(&app)
            .or_else(|| overlay.primary_monitor().ok().flatten());

        if let Some(monitor) = monitor {
            let monitor_size = monitor.size();
            let monitor_pos  = monitor.position();
            let overlay_size = overlay.outer_size().unwrap_or(tauri::PhysicalSize::new(80, 80));
            let x = monitor_pos.x + ((monitor_size.width as i32 - overlay_size.width as i32) / 2);
            let bottom_margin = (120.0 * monitor.scale_factor()) as i32;
            let y = monitor_pos.y + monitor_size.height as i32 - overlay_size.height as i32 - bottom_margin;
            let _ = overlay.set_position(tauri::PhysicalPosition::new(x, y));
        }
        let _ = overlay.set_always_on_top(true);
        let _ = overlay.set_ignore_cursor_events(true);
        let _ = overlay.show();
    }
}

/// Returns the monitor the mouse cursor is currently on.
/// Uses GetCursorPos (Win32 FFI) on Windows; returns None on other platforms.
fn cursor_monitor(app: &tauri::AppHandle) -> Option<tauri::Monitor> {
    let (cx, cy) = cursor_pos()?;
    app.available_monitors().ok()?.into_iter().find(|m| {
        let pos  = m.position();
        let size = m.size();
        cx >= pos.x
            && cx < pos.x + size.width as i32
            && cy >= pos.y
            && cy < pos.y + size.height as i32
    })
}

#[cfg(target_os = "windows")]
fn cursor_pos() -> Option<(i32, i32)> {
    #[repr(C)]
    struct POINT { x: i32, y: i32 }
    extern "system" { fn GetCursorPos(lp: *mut POINT) -> i32; }
    let mut pt = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut pt) } != 0 { Some((pt.x, pt.y)) } else { None }
}

#[cfg(not(target_os = "windows"))]
fn cursor_pos() -> Option<(i32, i32)> {
    None
}

#[tauri::command]
pub fn hide_overlay(app: tauri::AppHandle) {
    if let Some(overlay) = app.get_webview_window("overlay") {
        let _ = overlay.set_ignore_cursor_events(false);
        let _ = overlay.hide();
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
    // We call [AVCaptureDevice authorizationStatusForMediaType:@"soun"] via objc runtime.
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
            2 | 1 => "denied".to_string(),
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
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => return "denied".to_string(),
    };
    let config = match device.default_input_config() {
        Ok(c) => c,
        Err(_) => return "denied".to_string(),
    };
    let config: cpal::StreamConfig = config.into();

    // Open a short-lived stream to trigger the macOS mic permission dialog
    let stream = device.build_input_stream(
        &config,
        move |_data: &[f32], _: &_| {},
        move |_err| {},
        None,
    );
    match stream {
        Ok(s) => {
            let _ = s.play();
            // Keep stream alive briefly so macOS registers the request
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(s);

            // Re-check permission status
            #[cfg(target_os = "macos")]
            { return check_microphone_permission_blocking(); }
            #[cfg(not(target_os = "macos"))]
            { return "granted".to_string(); }
        }
        Err(_) => "denied".to_string(),
    }
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
