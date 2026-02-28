use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;
use sysinfo::System;

/// Returns the names of all available audio input devices on this machine.
#[tauri::command]
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
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
#[tauri::command]
pub fn get_system_info() -> SystemInfo {
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
                "Vulkan / DirectML".to_string()
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
