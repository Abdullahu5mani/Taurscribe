//! Which GPUs this machine has, which GPU build of Taurscribe this is, and which
//! build would be fastest here.
//!
//! Every inference library is compiled for one vendor per build ("flavor"), so
//! each GPU gets its native stack instead of a lowest-common-denominator one:
//!
//! | flavor   | whisper.cpp | transcribe.cpp | llama.cpp | who gets it                     |
//! |----------|-------------|----------------|-----------|---------------------------------|
//! | apple    | Metal (+CoreML encoder) | Metal | Metal | every Mac                      |
//! | nvidia   | CUDA        | CUDA           | CUDA      | NVIDIA, Windows / Linux x64     |
//! | amd      | HIP (ROCm)  | ROCm           | ROCm      | Radeon RX / Pro cards, Linux    |
//! | vulkan   | Vulkan      | Vulkan         | Vulkan    | AMD on Windows, AMD APUs, Intel |
//! | adreno   | CPU         | CPU            | OpenCL    | Snapdragon (Windows / Linux)    |
//! | cpu      | CPU         | CPU            | CPU       | no usable GPU                   |
//!
//! Why AMD on Windows gets Vulkan: whisper-rs cannot build HIP on Windows, and
//! whisper.cpp and transcribe.cpp link ggml statically into one binary, so they
//! must share a backend. AMD's own Vulkan driver is the fastest path both have
//! there, without shipping ~1 GB of rocBLAS. Intel is Vulkan for the same reason:
//! only whisper.cpp has SYCL.
//!
//! ONNX Runtime (CAM++ voiceprints) uses CoreML on Apple Silicon and DirectML on
//! Windows in every flavor; elsewhere its tiny model runs on the CPU.

use serde::Serialize;

#[cfg(all(feature = "gpu-amd", windows))]
compile_error!("gpu-amd is Linux-only (whisper-rs cannot build HIP on Windows); build AMD on Windows with gpu-vulkan");
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
    Qualcomm,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuDevice {
    pub name: String,
    pub vendor: Vendor,
    /// A separate card (or Apple Silicon's GPU), as opposed to a weak integrated one.
    pub discrete: bool,
    pub vram_gb: Option<f32>,
}

/// The GPU flavor this binary was built as.
pub const fn build_flavor() -> &'static str {
    if cfg!(target_os = "macos") {
        "apple"
    } else if cfg!(any(feature = "gpu-nvidia", feature = "windows-nvidia")) {
        "nvidia"
    } else if cfg!(feature = "gpu-amd") {
        "amd"
    } else if cfg!(feature = "gpu-vulkan") {
        "vulkan"
    } else if cfg!(feature = "gpu-adreno") {
        "adreno"
    } else {
        "cpu"
    }
}

/// True when this build's whisper.cpp has a GPU backend.
pub const fn whisper_gpu_compiled() -> bool {
    cfg!(target_os = "macos") || cfg!(any(feature = "gpu-nvidia", feature = "windows-nvidia", feature = "gpu-amd", feature = "gpu-vulkan"))
}

/// Human-readable name of a flavor, for the UI.
pub fn flavor_label(flavor: &str) -> &'static str {
    match flavor {
        "apple" => "Apple (Metal)",
        "nvidia" => "NVIDIA (CUDA)",
        "amd" => "AMD (ROCm)",
        "vulkan" => "Vulkan (AMD on Windows, Intel, other GPUs)",
        "adreno" => "Snapdragon (OpenCL)",
        _ => "CPU",
    }
}

fn vendor_from_name(name: &str) -> Vendor {
    let n = name.to_lowercase();
    if n.contains("nvidia") || n.contains("geforce") || n.contains("quadro") || n.contains("rtx") || n.contains("tesla") {
        Vendor::Nvidia
    } else if n.contains("amd") || n.contains("radeon") || n.contains("ati ") || n.starts_with("ati") {
        Vendor::Amd
    } else if n.contains("intel") || n.contains("arc ") || n.contains("iris") || n.contains("uhd graphics") {
        Vendor::Intel
    } else if n.contains("apple") {
        Vendor::Apple
    } else if n.contains("qualcomm") || n.contains("adreno") || n.contains("snapdragon") {
        Vendor::Qualcomm
    } else {
        Vendor::Other
    }
}

/// Intel graphics are integrated except the Arc cards, whose names carry an
/// A- or B-series model ("Arc(TM) A770", "Arc B580"). Integrated Arc is just
/// "Arc(TM) Graphics" or a V-series part like "Arc 140V".
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn intel_is_discrete(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("arc")
        && n.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| {
            let b = t.as_bytes();
            b.len() == 4 && (b[0] == b'a' || b[0] == b'b') && b[1..].iter().all(u8::is_ascii_digit)
        })
}

/// All GPUs, detected once per process (detection shells out on some platforms).
pub fn devices() -> &'static [GpuDevice] {
    static DEVICES: OnceLock<Vec<GpuDevice>> = OnceLock::new();
    DEVICES.get_or_init(detect)
}

/// The GPU inference should run on: the strongest discrete one, else any.
pub fn primary() -> Option<&'static GpuDevice> {
    let rank = |d: &GpuDevice| {
        let v = match d.vendor {
            Vendor::Nvidia => 5,
            Vendor::Apple => 5,
            Vendor::Amd => 4,
            Vendor::Intel => 3,
            Vendor::Qualcomm => 2,
            Vendor::Other => 1,
        };
        (d.discrete as u8, v, (d.vram_gb.unwrap_or(0.0) * 10.0) as i64)
    };
    devices().iter().max_by_key(|d| rank(d))
}

/// The flavor that would be fastest on this machine.
pub fn recommended_flavor() -> &'static str {
    if cfg!(target_os = "macos") {
        return "apple";
    }
    let Some(gpu) = primary() else { return "cpu" };
    let arm = cfg!(target_arch = "aarch64");
    match gpu.vendor {
        Vendor::Nvidia if !arm => "nvidia",
        // ROCm covers discrete Radeon cards on Linux; APUs and Windows use AMD's Vulkan driver.
        Vendor::Amd if !arm && gpu.discrete && cfg!(target_os = "linux") => "amd",
        Vendor::Amd if !arm => "vulkan",
        Vendor::Qualcomm => "adreno",
        Vendor::Intel | Vendor::Other if !arm => "vulkan",
        _ => "cpu",
    }
}

/// False when the GPU would be slower than the CPU, so engines should not try it.
/// Today that is an Intel Mac with only Intel integrated graphics.
pub fn gpu_worth_using() -> bool {
    if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        return devices().iter().any(|d| d.discrete);
    }
    true
}

// ── detection ────────────────────────────────────────────────────────────────

fn detect() -> Vec<GpuDevice> {
    let mut out = platform_devices();
    // nvidia-smi knows the VRAM, which the OS listings often don't.
    if let Some(smi) = nvidia_smi() {
        for (name, vram) in smi {
            match out.iter_mut().find(|d| d.vendor == Vendor::Nvidia) {
                Some(d) if d.vram_gb.is_none() => d.vram_gb = Some(vram),
                Some(_) => {}
                None => out.push(GpuDevice { name, vendor: Vendor::Nvidia, discrete: true, vram_gb: Some(vram) }),
            }
        }
    }
    out
}

fn quiet(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn nvidia_smi() -> Option<Vec<(String, f32)>> {
    let out = quiet(std::process::Command::new("nvidia-smi").args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let list: Vec<(String, f32)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (name, mb) = l.split_once(',')?;
            Some((name.trim().to_string(), mb.trim().parse::<f32>().ok()? / 1024.0))
        })
        .collect();
    (!list.is_empty()).then_some(list)
}

#[cfg(target_os = "macos")]
fn platform_devices() -> Vec<GpuDevice> {
    let Ok(out) = std::process::Command::new("system_profiler").args(["SPDisplaysDataType", "-json"]).output() else {
        return Vec::new();
    };
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    v["SPDisplaysDataType"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|g| {
            let name = g["sppci_model"].as_str().unwrap_or("GPU").to_string();
            let vendor = if cfg!(target_arch = "aarch64") { Vendor::Apple } else { vendor_from_name(&name) };
            // Apple Silicon's GPU counts as "discrete": it is the fast path there.
            let discrete = cfg!(target_arch = "aarch64") || g["sppci_bus"].as_str() == Some("spdisplays_pcie_device") || vendor == Vendor::Amd;
            let vram_gb = g["spdisplays_vram"]
                .as_str()
                .or(g["_spdisplays_vram"].as_str())
                .and_then(|s| s.split_whitespace().next()?.parse::<f32>().ok())
                .map(|x| x / 1024.0);
            GpuDevice { name, vendor, discrete, vram_gb }
        })
        .collect()
}

#[cfg(windows)]
fn platform_devices() -> Vec<GpuDevice> {
    // wmic is gone from recent Windows 11 builds; CIM works everywhere.
    let Ok(out) = quiet(std::process::Command::new("powershell").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-CimInstance Win32_VideoController | Select-Object Name,AdapterCompatibility | ConvertTo-Json -Compress",
    ]))
    .output() else {
        return Vec::new();
    };
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let items = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(_) => vec![v],
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|g| {
            let name = g["Name"].as_str()?.trim().to_string();
            if name.to_lowercase().contains("basic display") || name.to_lowercase().contains("remote") {
                return None;
            }
            let vendor = match vendor_from_name(&name) {
                Vendor::Other => vendor_from_name(g["AdapterCompatibility"].as_str().unwrap_or("")),
                v => v,
            };
            let discrete = match vendor {
                Vendor::Nvidia => true,
                // "AMD Radeon(TM) Graphics" / "Radeon 780M" are APUs; cards carry an RX/Pro model.
                Vendor::Amd => { let n = name.to_lowercase(); n.contains(" rx ") || n.contains("rx ") || n.contains("pro w") || n.contains("radeon vii") }
                Vendor::Intel => intel_is_discrete(&name),
                _ => false,
            };
            Some(GpuDevice { name, vendor, discrete, vram_gb: None })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn platform_devices() -> Vec<GpuDevice> {
    // PCI vendor ids from sysfs; names from lspci when it is installed.
    let names: Vec<String> = std::process::Command::new("lspci")
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| { let l = l.to_lowercase(); l.contains("vga") || l.contains("3d controller") || l.contains("display controller") })
                .filter_map(|l| l.splitn(3, ':').nth(2).map(|s| s.trim().to_string()))
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return out };
    let mut seen = std::collections::HashSet::new();
    for e in dir.flatten() {
        let fname = e.file_name().to_string_lossy().to_string();
        if !fname.starts_with("card") || fname.contains('-') {
            continue;
        }
        let dev = e.path().join("device");
        let Ok(canon) = std::fs::canonicalize(&dev) else { continue };
        if !seen.insert(canon) {
            continue;
        }
        let id = std::fs::read_to_string(dev.join("vendor")).unwrap_or_default();
        let vendor = match id.trim() {
            "0x10de" => Vendor::Nvidia,
            "0x1002" => Vendor::Amd,
            "0x8086" => Vendor::Intel,
            "0x5143" => Vendor::Qualcomm,
            _ => Vendor::Other,
        };
        let name = names
            .iter()
            .find(|n| vendor_from_name(n) == vendor)
            .cloned()
            .unwrap_or_else(|| format!("{vendor:?} GPU"));
        // amdgpu reports dedicated VRAM; APUs only have a small carve-out.
        let vram_gb = std::fs::read_to_string(dev.join("mem_info_vram_total"))
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|b| (b / 1_073_741_824.0) as f32);
        let discrete = match vendor {
            Vendor::Nvidia => true,
            Vendor::Amd => vram_gb.is_some_and(|g| g >= 2.0),
            Vendor::Intel => intel_is_discrete(&name),
            _ => false,
        };
        out.push(GpuDevice { name, vendor, discrete, vram_gb });
    }
    out
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_devices() -> Vec<GpuDevice> {
    Vec::new()
}

// ── Tauri command ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct GpuReport {
    pub devices: Vec<GpuDevice>,
    pub primary: Option<GpuDevice>,
    pub build_flavor: &'static str,
    pub build_label: &'static str,
    pub recommended_flavor: &'static str,
    pub recommended_label: &'static str,
    /// True when another build would be faster on this machine.
    pub better_build_available: bool,
}

pub fn report() -> GpuReport {
    let rec = recommended_flavor();
    GpuReport {
        devices: devices().to_vec(),
        primary: primary().cloned(),
        build_flavor: build_flavor(),
        build_label: flavor_label(build_flavor()),
        recommended_flavor: rec,
        recommended_label: flavor_label(rec),
        better_build_available: rec != build_flavor() && rec != "cpu",
    }
}

#[tauri::command]
pub async fn get_gpu_report() -> Result<GpuReport, String> {
    tauri::async_runtime::spawn_blocking(report).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendors_from_names() {
        assert_eq!(vendor_from_name("NVIDIA GeForce RTX 4070"), Vendor::Nvidia);
        assert_eq!(vendor_from_name("AMD Radeon RX 7800 XT"), Vendor::Amd);
        assert_eq!(vendor_from_name("Intel(R) Arc(TM) A770 Graphics"), Vendor::Intel);
        assert_eq!(vendor_from_name("Intel(R) UHD Graphics 630"), Vendor::Intel);
        assert_eq!(vendor_from_name("Qualcomm(R) Adreno(TM) X1-85 GPU"), Vendor::Qualcomm);
        assert_eq!(vendor_from_name("Apple M4"), Vendor::Apple);
    }

    #[test]
    fn intel_arc_cards_are_discrete() {
        assert!(intel_is_discrete("Intel(R) Arc(TM) A770 Graphics"));
        assert!(intel_is_discrete("Intel(R) Arc(TM) B580 Graphics"));
        assert!(!intel_is_discrete("Intel(R) UHD Graphics 630"));
        assert!(!intel_is_discrete("Intel(R) Iris(R) Xe Graphics"));
    }

    #[test]
    fn this_build_has_a_flavor() {
        assert!(["apple", "nvidia", "amd", "vulkan", "adreno", "cpu"].contains(&build_flavor()));
    }
}
