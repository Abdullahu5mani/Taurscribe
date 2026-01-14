# Whisper.cpp Binaries

This directory contains pre-built Whisper.cpp binaries for Windows with different backend support.

## Directory Structure

- **whisper-cpu/** - CPU-only build (baseline, works everywhere)
- **whisper-cuda/** - CUDA-accelerated build (NVIDIA GPUs)
- **whisper-vulkan/** - Vulkan-accelerated build (AMD/NVIDIA GPUs)

## Essential Files (per backend)

Each directory contains the **minimum required files** to run Whisper transcription:

### Executables
- `whisper-cli.exe` (~470 KB) - Main CLI tool for transcription

### Required DLLs
- `whisper.dll` (~1.3 MB) - Core Whisper library with AI models
- `ggml.dll` (~67 KB) - GGML core library
- `ggml-base.dll` (~525 KB) - GGML base operations
- `ggml-cpu.dll` (~704 KB) - CPU backend (in all builds)
- `ggml-cuda.dll` (~26 MB) - CUDA backend (CUDA build only)
- `ggml-vulkan.dll` (~50 MB) - Vulkan backend (Vulkan build only)

**⚠️ Important:** You CANNOT run `whisper-cli.exe` without the DLLs - they contain the actual transcription logic!

## Total Sizes

- **CPU Build:** ~3.1 MB
- **CUDA Build:** ~29 MB (due to CUDA library)
- **Vulkan Build:** ~53 MB (due to embedded Vulkan shaders)

## Usage in Taurscribe

The application automatically selects the appropriate binary based on:
1. Available GPU hardware
2. Installed drivers (CUDA/Vulkan)
3. Falls back to CPU if GPU acceleration unavailable

See `src-tauri/src/whisper_manager.rs` for backend selection logic.

## Rebuilding

If you need to rebuild these binaries from the whisper.cpp source:

```bash
# CPU build
cmake -B build-cpu -DGGML_CUDA=OFF -DGGML_VULKAN=OFF
cmake --build build-cpu --config Release

# CUDA build
cmake -B build-cuda -DGGML_CUDA=ON -DGGML_VULKAN=OFF
cmake --build build-cuda --config Release

# Vulkan build
cmake -B build-vulkan -DGGML_CUDA=OFF -DGGML_VULKAN=ON
cmake --build build-vulkan --config Release
```

Then copy only the essential files from `build-*/bin/Release/`:
- `whisper-cli.exe`
- All `*.dll` files

## Git LFS

The large DLL files (especially `ggml-cuda.dll` and `ggml-vulkan.dll`) are tracked with Git LFS to avoid bloating the repository.
