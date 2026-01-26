fn main() {
    // Standard Tauri build process
    tauri_build::build();

    // CUSTOM: Set minimum macOS deployment target for ONNX Runtime
    #[cfg(target_os = "macos")]
    {
        // ONNX Runtime requires macOS 13.4+ on Apple Silicon
        // (also satisfies whisper.cpp C++17 std::filesystem requirement which needs 10.15+)
        println!("cargo:rustc-env=MACOSX_DEPLOYMENT_TARGET=13.4");
        std::env::set_var("MACOSX_DEPLOYMENT_TARGET", "13.4");
    }

    // CUSTOM: Force Clang for ARM64 Windows (whisper.cpp requirement)
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        // whisper.cpp requires Clang for ARM64 on Windows (MSVC not supported)
        println!("cargo:warning=Building for Windows ARM64 - Clang/LLVM required");
        std::env::set_var("CC", "clang-cl");
        std::env::set_var("CXX", "clang-cl");
        std::env::set_var("CMAKE_GENERATOR_TOOLSET", "ClangCL");
    }

    // CUSTOM: Add CUDA library search path to fix linker errors (Windows only)
    #[cfg(windows)]
    {
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
}
