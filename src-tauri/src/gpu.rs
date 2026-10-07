//! Which GPUs this machine has, which GPU build of Taurscribe this is, and which
//! build would be fastest here.
//!
//! Every inference library is compiled for one vendor per build ("flavor"), so
//! each GPU gets its native stack instead of a lowest-common-denominator one:
//!
//! | flavor   | whisper.cpp | transcribe.cpp | llama.cpp | who gets it                     |
//! |----------|-------------|----------------|-----------|---------------------------------|
//! | apple    | Metal (+CoreML encoder) | Metal | Metal | every Mac                      |
//! | nvidia   | CUDA+Vulkan | CUDA+Vulkan    | CUDA+Vulkan | NVIDIA, Windows / Linux x64   |
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
//! Releases build the NVIDIA flavor with `gpu-nvidia,gpu-vulkan`: the user can
//! then pick an integrated GPU, or Vulkan instead of CUDA, in Settings → Models
//! (`compute_devices`). CUDA stays the default ("auto": ggml lists it first).
//!
//! ONNX Runtime (CAM++ voiceprints) uses CoreML on Apple Silicon; elsewhere its
//! tiny model runs on the CPU (DirectML fails it for many input lengths).

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
        Vendor::Intel if !arm => "vulkan",
        Vendor::Other if !arm && gpu.discrete => "vulkan",
        _ => "cpu",
    }
}

/// False when engines should not try the GPU: the GPU would be slower than the
/// CPU (an Intel Mac with only integrated graphics), or this build's GPU isn't
/// in the machine (a CUDA build on a PC without NVIDIA). ggml would fall back
/// to the CPU anyway, but the app would report the GPU backend as active.
pub fn gpu_worth_using() -> bool {
    // Nothing to probe where the answer can't depend on the hardware (Apple
    // Silicon always has its GPU; a CPU build has no GPU backend): detection
    // shells out, and this runs while an engine lock is held.
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) || build_flavor() == "cpu" {
        return true;
    }
    worth_using(build_flavor(), devices())
}

fn worth_using(flavor: &str, devs: &[GpuDevice]) -> bool {
    if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        return devs.iter().any(|d| d.discrete);
    }
    // Detection found nothing at all: trust the build rather than a failed probe.
    if devs.is_empty() {
        return true;
    }
    let has = |v: Vendor| devs.iter().any(|d| d.vendor == v);
    match flavor {
        "nvidia" => has(Vendor::Nvidia),
        "amd" => has(Vendor::Amd),
        "adreno" => has(Vendor::Qualcomm),
        // Any real GPU has a Vulkan driver; an unknown integrated adapter is
        // usually a VM's display device.
        "vulkan" => devs.iter().any(|d| d.vendor != Vendor::Other || d.discrete),
        _ => true,
    }
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

// ── device choice ────────────────────────────────────────────────────────────
//
// whisper.cpp and transcribe.cpp share one statically linked ggml, so one
// device registry serves both. A device is named "<backend>:<description>"
// (e.g. "vulkan:AMD Radeon(TM) Graphics"): registry indices are not stable
// across runs, names are. llama.cpp has its own registry (DLLs); it is matched
// by the same name.

/// A device the speech engines can run on, or an NPU they can't use yet.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ComputeDevice {
    /// "auto", "cpu", or "<backend>:<description>"; what Settings stores.
    pub id: String,
    pub name: String,
    /// "CUDA", "Vulkan", "Metal", "ROCm", "CPU" or "NPU".
    pub backend: String,
    /// "gpu", "igpu", "cpu" or "npu".
    pub kind: &'static str,
    pub memory_gb: Option<f32>,
    /// False for hardware no engine in this build can run on.
    pub usable: bool,
    pub note: Option<String>,
}

pub const AUTO_DEVICE: &str = "auto";
pub const CPU_DEVICE: &str = "cpu";
/// Settings key of the chosen device.
pub const DEVICE_SETTING: &str = "compute_device";

fn backend_label(kind: &str) -> String {
    match kind {
        "cuda" => "CUDA".into(),
        "vulkan" => "Vulkan".into(),
        "metal" => "Metal".into(),
        "rocm" => "ROCm".into(),
        "sycl" => "SYCL".into(),
        other => other.to_uppercase(),
    }
}

fn device_id(kind: &str, description: &str) -> String {
    format!("{}:{}", kind.to_lowercase(), description.trim())
}

/// GPUs in ggml's registry, in registry order (whisper.cpp's `gpu_device`
/// counts in this order).
fn registry_gpus() -> Vec<transcribe_cpp::Device> {
    use transcribe_cpp::DeviceType;
    transcribe_cpp::devices()
        .into_iter()
        .filter(|d| matches!(d.device_type, DeviceType::Gpu | DeviceType::Igpu))
        .collect()
}

/// GPUs best first: separate cards before integrated ones, otherwise in
/// registry order. The first is what "auto" runs on. ggml's Vulkan backend
/// lists integrated GPUs too, in the driver's order, which can put an APU
/// ahead of the Radeon card in the same PC.
fn ranked_gpus() -> Vec<transcribe_cpp::Device> {
    let mut gpus = registry_gpus();
    gpus.sort_by_key(|d| d.device_type == transcribe_cpp::DeviceType::Igpu);
    gpus
}

/// Every device the user can pick: GPUs best first, then the CPU, then NPUs.
pub fn compute_devices() -> Vec<ComputeDevice> {
    let mut out: Vec<ComputeDevice> = if gpu_worth_using() {
        ranked_gpus()
            .into_iter()
            .map(|d| ComputeDevice {
                id: device_id(&d.kind, &d.description),
                name: d.description.trim().to_string(),
                backend: backend_label(&d.kind),
                kind: if d.device_type == transcribe_cpp::DeviceType::Igpu { "igpu" } else { "gpu" },
                memory_gb: (d.memory_total > 0).then(|| d.memory_total as f32 / 1_073_741_824.0),
                usable: true,
                note: None,
            })
            .collect()
    } else {
        Vec::new()
    };
    out.push(ComputeDevice {
        id: CPU_DEVICE.into(),
        name: "CPU".into(),
        backend: "CPU".into(),
        kind: "cpu",
        memory_gb: None,
        usable: true,
        note: None,
    });
    out.extend(npus().into_iter().map(|name| ComputeDevice {
        id: format!("npu:{name}"),
        name,
        backend: "NPU".into(),
        kind: "npu",
        memory_gb: None,
        usable: false,
        note: Some(NPU_NOTE.into()),
    }));
    out
}

const NPU_NOTE: &str = "Whisper, Granite, Qwen3 and FlowScribe run on ggml, which has no backend for this NPU yet";

/// The saved choice: "auto", "cpu" or a device id.
static PREFERRED: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

pub fn preferred_device() -> String {
    PREFERRED.read().unwrap().clone().unwrap_or_else(|| AUTO_DEVICE.into())
}

pub fn set_preferred_device(id: &str) {
    *PREFERRED.write().unwrap() = Some(id.to_string());
}

/// Reads the saved choice from settings.json; called once at startup.
pub fn load_preferred_device() {
    let saved = crate::mcp_server::settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get(DEVICE_SETTING)?.as_str().map(str::to_string));
    if let Some(id) = saved {
        set_preferred_device(&id);
    }
}

/// What an engine should load on. `force_cpu` is the engine's own GPU/CPU
/// switch (file jobs and the LLM pass it); otherwise the saved device applies.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Cpu,
    /// The best GPU (ggml's first).
    Auto,
    Gpu(String),
}

impl Target {
    pub fn resolve(force_cpu: bool) -> Self {
        if force_cpu || !gpu_worth_using() {
            return Target::Cpu;
        }
        Self::from_id(&preferred_device())
    }

    fn from_id(id: &str) -> Self {
        match id {
            CPU_DEVICE => Target::Cpu,
            AUTO_DEVICE | "" => Target::Auto,
            other => Target::Gpu(other.to_string()),
        }
    }

    /// The id stored with a loaded model, so a reload is skipped only when
    /// the device would not change.
    pub fn key(&self) -> String {
        match self {
            Target::Cpu => CPU_DEVICE.into(),
            Target::Auto => AUTO_DEVICE.into(),
            Target::Gpu(id) => id.clone(),
        }
    }

    /// The id of the GPU this target runs on: the chosen one if present,
    /// else the best one. None on the CPU or without a GPU.
    fn gpu_id(&self) -> Option<String> {
        let best = || ranked_gpus().first().map(|d| device_id(&d.kind, &d.description));
        match self {
            Target::Cpu => None,
            Target::Auto => best(),
            Target::Gpu(id) => {
                if registry_gpus().iter().any(|d| &device_id(&d.kind, &d.description) == id) {
                    Some(id.clone())
                } else {
                    // An eGPU unplugged or a driver gone since it was chosen.
                    println!("[GPU] Chosen device '{id}' not found; using the best GPU");
                    best()
                }
            }
        }
    }

    /// The registry device to load transcribe.cpp models on.
    pub fn transcribe_device(&self) -> Option<transcribe_cpp::Device> {
        let id = self.gpu_id()?;
        registry_gpus().into_iter().find(|d| device_id(&d.kind, &d.description) == id)
    }

    /// whisper.cpp's `gpu_device`: the position among GPUs in the registry.
    pub fn whisper_gpu_index(&self) -> i32 {
        let Some(id) = self.gpu_id() else { return 0 };
        registry_gpus()
            .iter()
            .position(|d| device_id(&d.kind, &d.description) == id)
            .map_or(0, |i| i as i32)
    }

    /// The matching llama.cpp device index (its own registry), if any.
    pub fn llama_device(&self) -> Option<usize> {
        let id = self.gpu_id()?;
        llama_cpp_2::list_llama_ggml_backend_devices()
            .into_iter()
            .find(|d| device_id(&d.backend, &d.description) == id)
            .map(|d| d.index)
    }
}

// NPUs, from the OS. None of the engines can use them; they are listed so the
// choice is visible rather than silently missing. (Apple's Neural Engine is not
// listed: Core ML already uses it for the Whisper encoder when the .mlmodelc
// companion is downloaded.)
#[cfg(windows)]
fn npus() -> Vec<String> {
    let Ok(out) = quiet(std::process::Command::new("powershell").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-PnpDevice -Class ComputeAccelerator -PresentOnly -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FriendlyName",
    ]))
    .output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

#[cfg(target_os = "linux")]
fn npus() -> Vec<String> {
    // amdxdna (AMD Ryzen AI), intel_vpu (Intel AI Boost), qaic …
    let Ok(dir) = std::fs::read_dir("/sys/class/accel") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let driver = std::fs::read_link(e.path().join("device/driver")).ok()?;
            let driver = driver.file_name()?.to_string_lossy().to_string();
            Some(match driver.as_str() {
                "amdxdna" => "AMD Ryzen AI NPU".to_string(),
                "intel_vpu" => "Intel AI Boost NPU".to_string(),
                other => format!("NPU ({other})"),
            })
        })
        .collect()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn npus() -> Vec<String> {
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
    /// Devices to choose from (see `compute_devices`).
    pub compute_devices: Vec<ComputeDevice>,
    /// The saved choice: "auto", "cpu" or a device id.
    pub preferred_device: String,
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
        compute_devices: compute_devices(),
        preferred_device: preferred_device(),
    }
}

/// Saves the device choice for the next model load (the frontend reloads the
/// model and persists the setting itself).
#[tauri::command]
pub fn set_compute_device(id: String) {
    println!("[GPU] Compute device set to '{id}'");
    set_preferred_device(&id);
}

#[tauri::command]
pub async fn get_gpu_report() -> Result<GpuReport, String> {
    tauri::async_runtime::spawn_blocking(report).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(vendor: Vendor, discrete: bool) -> GpuDevice {
        GpuDevice { name: String::new(), vendor, discrete, vram_gb: None }
    }

    #[test]
    #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
    fn gpu_only_used_when_the_builds_vendor_is_present() {
        let vm = [dev(Vendor::Other, false)];
        assert!(!worth_using("nvidia", &vm));
        assert!(!worth_using("amd", &vm));
        assert!(!worth_using("vulkan", &vm));
        assert!(worth_using("cpu", &vm));
        assert!(worth_using("nvidia", &[dev(Vendor::Intel, false), dev(Vendor::Nvidia, true)]));
        assert!(!worth_using("nvidia", &[dev(Vendor::Amd, true)]));
        assert!(worth_using("vulkan", &[dev(Vendor::Intel, false)]));
        assert!(worth_using("adreno", &[dev(Vendor::Qualcomm, false)]));
        // A failed probe must not switch the GPU off.
        assert!(worth_using("nvidia", &[]));
    }

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
    fn device_choice_maps_to_targets() {
        assert_eq!(Target::from_id("auto"), Target::Auto);
        assert_eq!(Target::from_id(""), Target::Auto);
        assert_eq!(Target::from_id("cpu"), Target::Cpu);
        assert_eq!(Target::from_id("vulkan:AMD Radeon(TM) Graphics"), Target::Gpu("vulkan:AMD Radeon(TM) Graphics".into()));
        assert_eq!(Target::Gpu("cuda:X".into()).key(), "cuda:X");
        assert_eq!(device_id("Vulkan", " NVIDIA GeForce RTX 5070 "), "vulkan:NVIDIA GeForce RTX 5070");
        // A device that isn't present falls back to the best GPU, as Auto does.
        let missing = Target::Gpu("cuda:no such card".into());
        assert_eq!(missing.whisper_gpu_index(), Target::Auto.whisper_gpu_index());
        assert_eq!(missing.transcribe_device(), Target::Auto.transcribe_device());
        assert!(Target::Cpu.transcribe_device().is_none());
    }

    #[test]
    fn the_cpu_is_always_a_choice() {
        let devices = compute_devices();
        assert!(devices.iter().any(|d| d.id == CPU_DEVICE && d.usable));
        assert!(devices.iter().filter(|d| d.kind == "npu").all(|d| !d.usable));
    }

    #[test]
    fn this_build_has_a_flavor() {
        assert!(["apple", "nvidia", "amd", "vulkan", "adreno", "cpu"].contains(&build_flavor()));
    }
}
