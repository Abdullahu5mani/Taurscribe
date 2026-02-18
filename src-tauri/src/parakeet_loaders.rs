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

// ─── Nemotron ────────────────────────────────────────────────────────────────

pub fn init_nemotron(path: &PathBuf) -> Result<(Nemotron, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        println!("[PARAKEET] macOS detected - explicitly forcing CPU for Nemotron");
        let m = try_cpu_nemotron(path.to_str().unwrap())?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(m) = try_gpu_nemotron(path.to_str().unwrap()) {
            println!("[PARAKEET] Loaded Nemotron with CUDA");
            return Ok((m, GpuBackend::Cuda));
        }
        if let Ok(m) = try_directml_nemotron(path.to_str().unwrap()) {
            println!("[PARAKEET] Loaded Nemotron with DirectML");
            return Ok((m, GpuBackend::DirectML));
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
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda);
        Nemotron::from_pretrained(_path, Some(config)).map_err(|e| format!("{}", e))
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
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::DirectML);
        Nemotron::from_pretrained(_path, Some(config)).map_err(|e| format!("{}", e))
    }
    #[cfg(not(target_os = "windows"))]
    Err("DirectML feature not enabled".to_string())
}

fn try_cpu_nemotron(path: &str) -> Result<Nemotron, String> {
    Nemotron::from_pretrained(path, None).map_err(|e| format!("{}", e))
}

// ─── CTC ─────────────────────────────────────────────────────────────────────

pub fn init_ctc(path: &PathBuf) -> Result<(Parakeet, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        println!("[PARAKEET] macOS detected - explicitly forcing CPU for CTC");
        let m = try_cpu_ctc(path.to_str().unwrap())?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(m) = try_gpu_ctc(path.to_str().unwrap()) {
            println!("[PARAKEET] Loaded CTC with CUDA");
            return Ok((m, GpuBackend::Cuda));
        }
        if let Ok(m) = try_directml_ctc(path.to_str().unwrap()) {
            println!("[PARAKEET] Loaded CTC with DirectML");
            return Ok((m, GpuBackend::DirectML));
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
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda);
        Parakeet::from_pretrained(_path, Some(config)).map_err(|e| format!("{}", e))
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
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::DirectML);
        Parakeet::from_pretrained(_path, Some(config)).map_err(|e| format!("{}", e))
    }
    #[cfg(not(target_os = "windows"))]
    Err("DirectML feature not enabled".to_string())
}

fn try_cpu_ctc(path: &str) -> Result<Parakeet, String> {
    Parakeet::from_pretrained(path, None).map_err(|e| format!("{}", e))
}

// ─── EOU ─────────────────────────────────────────────────────────────────────

pub fn init_eou(path: &PathBuf) -> Result<(ParakeetEOU, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        let m = try_cpu_eou(path.to_str().unwrap())?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(m) = try_gpu_eou(path.to_str().unwrap()) {
            return Ok((m, GpuBackend::Cuda));
        }
        if let Ok(m) = try_directml_eou(path.to_str().unwrap()) {
            return Ok((m, GpuBackend::DirectML));
        }
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
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda);
        ParakeetEOU::from_pretrained(_path, Some(config)).map_err(|e| format!("{}", e))
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
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::DirectML);
        ParakeetEOU::from_pretrained(_path, Some(config)).map_err(|e| format!("{}", e))
    }
    #[cfg(not(target_os = "windows"))]
    Err("DirectML feature not enabled".to_string())
}

fn try_cpu_eou(path: &str) -> Result<ParakeetEOU, String> {
    ParakeetEOU::from_pretrained(path, None).map_err(|e| format!("{}", e))
}

// ─── TDT ─────────────────────────────────────────────────────────────────────

pub fn init_tdt(path: &PathBuf) -> Result<(ParakeetTDT, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        let m = try_cpu_tdt(path.to_str().unwrap())?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(m) = try_gpu_tdt(path.to_str().unwrap()) {
            return Ok((m, GpuBackend::Cuda));
        }
        if let Ok(m) = try_directml_tdt(path.to_str().unwrap()) {
            return Ok((m, GpuBackend::DirectML));
        }
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
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda);
        ParakeetTDT::from_pretrained(_path, Some(config)).map_err(|e| format!("{}", e))
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
        use parakeet_rs::{ExecutionConfig, ExecutionProvider};
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::DirectML);
        ParakeetTDT::from_pretrained(_path, Some(config)).map_err(|e| format!("{}", e))
    }
    #[cfg(not(target_os = "windows"))]
    Err("DirectML feature not enabled".to_string())
}

fn try_cpu_tdt(path: &str) -> Result<ParakeetTDT, String> {
    ParakeetTDT::from_pretrained(path, None).map_err(|e| format!("{}", e))
}
