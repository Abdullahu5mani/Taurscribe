fn main() {
    // Standard Tauri build process
    tauri_build::build();

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    // CUSTOM: Set minimum macOS deployment target for ONNX Runtime
    if target_os == "macos" {
        // ONNX Runtime requires macOS 13.4+ on Apple Silicon
        // (also satisfies whisper.cpp C++17 std::filesystem requirement which needs 10.15+)
        println!("cargo:rustc-env=MACOSX_DEPLOYMENT_TARGET=13.4");
        std::env::set_var("MACOSX_DEPLOYMENT_TARGET", "13.4");

        // Also set CMAKE_OSX_DEPLOYMENT_TARGET for CMake-based dependencies (whisper-rs-sys)
        std::env::set_var("CMAKE_OSX_DEPLOYMENT_TARGET", "13.4");

        // Link AVFoundation so the ObjC runtime can resolve AVCaptureDevice
        // (used by check_microphone_permission to query mic authorization status).
        println!("cargo:rustc-link-lib=framework=AVFoundation");
    }

    // CUSTOM: Force Clang for ARM64 Windows (whisper.cpp requirement)
    if target_os == "windows" && target_arch == "aarch64" {
        // whisper.cpp requires Clang for ARM64 on Windows (MSVC not supported)
        println!("cargo:warning=Building for Windows ARM64 - Clang/LLVM required");
        std::env::set_var("CC", "clang-cl");
        std::env::set_var("CXX", "clang-cl");
        std::env::set_var("CMAKE_GENERATOR_TOOLSET", "ClangCL");
    }

    // CUSTOM: Add CUDA library search path to fix linker errors (Windows only)
    if target_os == "windows" {
        let mut found = false;

        // 1. Try CUDA_PATH environment variable
        if let Ok(cuda_path) = std::env::var("CUDA_PATH") {
            let cuda_path = std::path::PathBuf::from(cuda_path);
            let lib_path = cuda_path.join("lib").join("x64");

            if lib_path.exists() {
                println!("cargo:rustc-link-search=native={}", lib_path.display());
                println!(
                    "cargo:info=Found CUDA lib path via CUDA_PATH: {}",
                    lib_path.display()
                );
                found = true;
            }
        }

        // 2. Fallback: Check standard installation path (Hardcoded for v12.9 as seen on user system)
        if !found {
            let fallback_path = std::path::PathBuf::from(
                r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.9\lib\x64",
            );
            if fallback_path.exists() {
                println!("cargo:rustc-link-search=native={}", fallback_path.display());
                println!(
                    "cargo:info=Found CUDA lib path via Fallback: {}",
                    fallback_path.display()
                );
                found = true;
            }
        }

        if !found {
            // Only warn if we are on Windows and clearly trying to use CUDA (implied by this logic existing)
            // Ideally check features, but build.rs can't easily see enabled features of dependencies.
            println!(
                "cargo:warning=Could not find CUDA libraries in CUDA_PATH or standard locations."
            );
            println!(
                "cargo:warning=GPU builds will fail with LNK1181 if the linker cannot find cublas.lib"
            );
        }
    }

    // CUSTOM: Add CUDA library search path for Linux (both standard and headless stubs, target_os = "linux", target_arch = "x86_64")
    if target_os == "linux" && target_arch == "x86_64" {
        let mut candidates = Vec::new();
        if let Ok(p) = std::env::var("CUDA_PATH") {
            candidates.push(std::path::PathBuf::from(p));
        }
        candidates.push(std::path::PathBuf::from("/usr/local/cuda"));
        candidates.push(std::path::PathBuf::from("/usr/local/cuda-12.6"));
        candidates.push(std::path::PathBuf::from("/usr/local/cuda-12"));
        candidates.push(std::path::PathBuf::from("/opt/cuda"));

        if let Ok(entries) = std::fs::read_dir("/usr/local") {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && p.file_name().map(|n| n.to_string_lossy().starts_with("cuda")).unwrap_or(false) {
                    if !candidates.contains(&p) {
                        candidates.push(p);
                    }
                }
            }
        }

        for base in candidates {
            let stubs = base.join("lib64").join("stubs");
            if stubs.join("libcuda.so").exists() || stubs.join("libcuda.so.1").exists() {
                println!("cargo:rustc-link-search=native={}", stubs.display());
            }
            let lib64 = base.join("lib64");
            if lib64.exists() {
                println!("cargo:rustc-link-search=native={}", lib64.display());
                break;
            }
        }
    }
}
