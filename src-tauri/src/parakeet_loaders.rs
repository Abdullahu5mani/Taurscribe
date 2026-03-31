/// GPU/CPU loader helpers for Parakeet models.
///
/// Each model type (Nemotron, CTC, EOU, TDT) needs three loader variants:
///   - `try_gpu_*`      — CUDA (Linux / Windows x86_64)
///   - `try_directml_*` — DirectML (Windows only)
///   - `try_cpu_*`      — CPU fallback (all platforms)
///
/// The `init_*` functions run the platform-appropriate sequence and return
/// the loaded model together with the `GpuBackend` that was used.
use parakeet_rs::{Nemotron, Parakeet, ParakeetEOU, ParakeetTDT};
use std::path::PathBuf;

use crate::parakeet::GpuBackend;

// ─── Session config helpers ───────────────────────────────────────────────────

/// Number of intra-op threads: half the physical cores, clamped to [2, 6].
/// Parakeet runs chunks continuously alongside the audio capture thread,
/// so we leave headroom rather than saturating all cores.
fn intra_thread_count() -> usize {
    (std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        / 2)
    .max(2)
    .min(6)
}

/// Build an `ExecutionConfig` for CPU inference with tuned session options.
///
/// - `intra_threads` — parallelism within a single ONNX operator
/// - `inter_threads(1)` — Conformer/RNN-T is sequential; parallel inter-op adds sync overhead
/// `parakeet-rs` 0.3.0 only exposes thread counts and execution provider selection
/// through `ExecutionConfig`, so we keep the tuning here within that supported API.
#[cfg(not(target_os = "macos"))]
fn cpu_config() -> parakeet_rs::ExecutionConfig {
    use parakeet_rs::ExecutionConfig;
    ExecutionConfig::new()
        .with_intra_threads(intra_thread_count())
        .with_inter_threads(1)
}

/// Build an `ExecutionConfig` for CUDA inference.
/// Same session options as `cpu_config` plus the CUDA execution provider.
#[cfg(any(target_os = "linux", all(target_os = "windows", target_arch = "x86_64")))]
fn cuda_config() -> parakeet_rs::ExecutionConfig {
    use parakeet_rs::{ExecutionConfig, ExecutionProvider};
    ExecutionConfig::new()
        .with_execution_provider(ExecutionProvider::Cuda)
        .with_intra_threads(intra_thread_count())
        .with_inter_threads(1)
}

/// Build an `ExecutionConfig` for DirectML inference (Windows GPU/NPU).
#[cfg(target_os = "windows")]
fn directml_config() -> parakeet_rs::ExecutionConfig {
    use parakeet_rs::{ExecutionConfig, ExecutionProvider};
    ExecutionConfig::new()
        .with_execution_provider(ExecutionProvider::DirectML)
        .with_intra_threads(intra_thread_count())
        .with_inter_threads(1)
}

// ─── Nemotron ────────────────────────────────────────────────────────────────

pub fn init_nemotron(path: &PathBuf, force_cpu: bool) -> Result<(Nemotron, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        println!("[PARAKEET] macOS detected - explicitly forcing CPU for Nemotron");
        let m = try_cpu_nemotron(path.to_str().unwrap())?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if force_cpu {
            println!("[PARAKEET] CPU-only mode selected for Nemotron");
            let m = try_cpu_nemotron(path.to_str().unwrap())?;
            return Ok((m, GpuBackend::Cpu));
        }
        match try_gpu_nemotron(path.to_str().unwrap()) {
            Ok(m) => return Ok((m, GpuBackend::Cuda)),
            Err(e) => eprintln!("[PARAKEET] CUDA failed for Nemotron: {e}"),
        }
        match try_directml_nemotron(path.to_str().unwrap()) {
            Ok(m) => return Ok((m, GpuBackend::DirectML)),
            Err(e) => eprintln!("[PARAKEET] DirectML failed for Nemotron: {e}"),
        }
        println!("[PARAKEET] Fallback to CPU for Nemotron");
        let m = try_cpu_nemotron(path.to_str().unwrap())?;
        Ok((m, GpuBackend::Cpu))
    }
}

#[allow(dead_code)]
fn try_gpu_nemotron(_path: &str) -> Result<Nemotron, String> {
    #[cfg(any(
        target_os = "linux",
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    {
        Nemotron::from_pretrained(_path, Some(cuda_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(not(any(
        target_os = "linux",
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    Err("CUDA feature not enabled".to_string())
}

#[allow(dead_code)]
fn try_directml_nemotron(_path: &str) -> Result<Nemotron, String> {
    #[cfg(target_os = "windows")]
    {
        Nemotron::from_pretrained(_path, Some(directml_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(not(target_os = "windows"))]
    Err("DirectML feature not enabled".to_string())
}

fn try_cpu_nemotron(path: &str) -> Result<Nemotron, String> {
    #[cfg(not(target_os = "macos"))]
    {
        Nemotron::from_pretrained(path, Some(cpu_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(target_os = "macos")]
    {
        Nemotron::from_pretrained(path, None).map_err(|e| format!("{}", e))
    }
}

// ─── CTC ─────────────────────────────────────────────────────────────────────

pub fn init_ctc(path: &PathBuf, force_cpu: bool) -> Result<(Parakeet, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        println!("[PARAKEET] macOS detected - explicitly forcing CPU for CTC");
        let m = try_cpu_ctc(path.to_str().unwrap())?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if force_cpu {
            println!("[PARAKEET] CPU-only mode selected for CTC");
            let m = try_cpu_ctc(path.to_str().unwrap())?;
            return Ok((m, GpuBackend::Cpu));
        }
        match try_gpu_ctc(path.to_str().unwrap()) {
            Ok(m) => return Ok((m, GpuBackend::Cuda)),
            Err(e) => eprintln!("[PARAKEET] CUDA failed for CTC: {e}"),
        }
        match try_directml_ctc(path.to_str().unwrap()) {
            Ok(m) => return Ok((m, GpuBackend::DirectML)),
            Err(e) => eprintln!("[PARAKEET] DirectML failed for CTC: {e}"),
        }
        println!("[PARAKEET] Fallback to CPU for CTC");
        let m = try_cpu_ctc(path.to_str().unwrap())?;
        Ok((m, GpuBackend::Cpu))
    }
}

#[allow(dead_code)]
fn try_gpu_ctc(_path: &str) -> Result<Parakeet, String> {
    #[cfg(any(
        target_os = "linux",
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    {
        Parakeet::from_pretrained(_path, Some(cuda_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(not(any(
        target_os = "linux",
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    Err("CUDA feature not enabled".to_string())
}

#[allow(dead_code)]
fn try_directml_ctc(_path: &str) -> Result<Parakeet, String> {
    #[cfg(target_os = "windows")]
    {
        Parakeet::from_pretrained(_path, Some(directml_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(not(target_os = "windows"))]
    Err("DirectML feature not enabled".to_string())
}

fn try_cpu_ctc(path: &str) -> Result<Parakeet, String> {
    #[cfg(not(target_os = "macos"))]
    {
        Parakeet::from_pretrained(path, Some(cpu_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(target_os = "macos")]
    {
        Parakeet::from_pretrained(path, None).map_err(|e| format!("{}", e))
    }
}

// ─── EOU ─────────────────────────────────────────────────────────────────────

pub fn init_eou(path: &PathBuf, force_cpu: bool) -> Result<(ParakeetEOU, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        let m = try_cpu_eou(path.to_str().unwrap())?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if force_cpu {
            println!("[PARAKEET] CPU-only mode selected for EOU");
            let m = try_cpu_eou(path.to_str().unwrap())?;
            return Ok((m, GpuBackend::Cpu));
        }
        match try_gpu_eou(path.to_str().unwrap()) {
            Ok(m) => return Ok((m, GpuBackend::Cuda)),
            Err(e) => eprintln!("[PARAKEET] CUDA failed for EOU: {e}"),
        }
        match try_directml_eou(path.to_str().unwrap()) {
            Ok(m) => return Ok((m, GpuBackend::DirectML)),
            Err(e) => eprintln!("[PARAKEET] DirectML failed for EOU: {e}"),
        }
        println!("[PARAKEET] Fallback to CPU for EOU");
        let m = try_cpu_eou(path.to_str().unwrap())?;
        Ok((m, GpuBackend::Cpu))
    }
}

#[allow(dead_code)]
fn try_gpu_eou(_path: &str) -> Result<ParakeetEOU, String> {
    #[cfg(any(
        target_os = "linux",
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    {
        ParakeetEOU::from_pretrained(_path, Some(cuda_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(not(any(
        target_os = "linux",
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    Err("CUDA feature not enabled".to_string())
}

#[allow(dead_code)]
fn try_directml_eou(_path: &str) -> Result<ParakeetEOU, String> {
    #[cfg(target_os = "windows")]
    {
        ParakeetEOU::from_pretrained(_path, Some(directml_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(not(target_os = "windows"))]
    Err("DirectML feature not enabled".to_string())
}

fn try_cpu_eou(path: &str) -> Result<ParakeetEOU, String> {
    #[cfg(not(target_os = "macos"))]
    {
        ParakeetEOU::from_pretrained(path, Some(cpu_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(target_os = "macos")]
    {
        ParakeetEOU::from_pretrained(path, None).map_err(|e| format!("{}", e))
    }
}

// ─── TDT ─────────────────────────────────────────────────────────────────────

pub fn init_tdt(path: &PathBuf, force_cpu: bool) -> Result<(ParakeetTDT, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        let m = try_cpu_tdt(path.to_str().unwrap())?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if force_cpu {
            println!("[PARAKEET] CPU-only mode selected for TDT");
            let m = try_cpu_tdt(path.to_str().unwrap())?;
            return Ok((m, GpuBackend::Cpu));
        }
        match try_gpu_tdt(path.to_str().unwrap()) {
            Ok(m) => return Ok((m, GpuBackend::Cuda)),
            Err(e) => eprintln!("[PARAKEET] CUDA failed for TDT: {e}"),
        }
        match try_directml_tdt(path.to_str().unwrap()) {
            Ok(m) => return Ok((m, GpuBackend::DirectML)),
            Err(e) => eprintln!("[PARAKEET] DirectML failed for TDT: {e}"),
        }
        println!("[PARAKEET] Fallback to CPU for TDT");
        let m = try_cpu_tdt(path.to_str().unwrap())?;
        Ok((m, GpuBackend::Cpu))
    }
}

#[allow(dead_code)]
fn try_gpu_tdt(_path: &str) -> Result<ParakeetTDT, String> {
    #[cfg(any(
        target_os = "linux",
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    {
        ParakeetTDT::from_pretrained(_path, Some(cuda_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(not(any(
        target_os = "linux",
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    Err("CUDA feature not enabled".to_string())
}

#[allow(dead_code)]
fn try_directml_tdt(_path: &str) -> Result<ParakeetTDT, String> {
    #[cfg(target_os = "windows")]
    {
        ParakeetTDT::from_pretrained(_path, Some(directml_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(not(target_os = "windows"))]
    Err("DirectML feature not enabled".to_string())
}

fn try_cpu_tdt(path: &str) -> Result<ParakeetTDT, String> {
    #[cfg(not(target_os = "macos"))]
    {
        ParakeetTDT::from_pretrained(path, Some(cpu_config())).map_err(|e| format!("{}", e))
    }
    #[cfg(target_os = "macos")]
    {
        ParakeetTDT::from_pretrained(path, None).map_err(|e| format!("{}", e))
    }
}

/// One-line summary after load. parakeet-rs registers GPU EP first then CPU without `error_on_failure`
/// on CUDA, so a bad cuDNN path can still “load” but run on CPU — Cohere uses stricter ORT options.
pub fn log_parakeet_backend_resolution(model_type: &str, backend: &GpuBackend, force_cpu: bool) {
    if force_cpu {
        println!("[PARAKEET] {} — CPU-only (forced).", model_type);
        return;
    }
    match backend {
        GpuBackend::Cuda => println!(
            "[PARAKEET] {} — CUDA EP loaded (CPU EP also registered as ORT fallback; use Task Manager or nvidia-smi during transcribing to confirm GPU if unsure).",
            model_type
        ),
        GpuBackend::DirectML => println!(
            "[PARAKEET] {} — DirectML EP loaded (CPU EP registered as ORT fallback).",
            model_type
        ),
        GpuBackend::Cpu => println!(
            "[PARAKEET] {} — CPU EP only (GPU paths failed).",
            model_type
        ),
    }
}
