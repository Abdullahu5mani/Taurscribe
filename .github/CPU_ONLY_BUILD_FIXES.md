# CPU-Only Build Fixes for GitHub Actions

## Overview
This document summarizes the fixes applied to enable successful CPU-only builds across all platforms (macOS, Linux, Windows) in GitHub Actions.

## Issues Identified and Fixed

### 1. **macOS (Both Intel & Apple Silicon)**
**Problem:** Build failed with C++ filesystem API errors
```
error: 'path' is unavailable: introduced in macOS 10.15
```

**Root Cause:** whisper.cpp uses C++17 filesystem APIs that require macOS 10.15+, but CMake was hardcoding the deployment target to macOS 10.13 (`-mmacosx-version-min=10.13`)

**Fix:** Set `MACOSX_DEPLOYMENT_TARGET=10.15` environment variable in the build action's env section
- Added to the `tauri-apps/tauri-action@v0` step's `env` section
- This ensures CMake picks up the correct deployment target when building whisper-rs-sys
- Uses conditional: `${{ matrix.platform == 'macos-latest' && '10.15' || '' }}`


### 2. **Linux (Ubuntu 22.04)**
**Problem:** Build failed with missing ALSA library
```
The system library `alsa` required by crate `alsa-sys` was not found.
The file `alsa.pc` needs to be installed
```

**Root Cause:** `cpal` (audio library used by the app) requires ALSA development headers, but they weren't installed in CI

**Fix:** Added `libasound2-dev` to Linux dependencies
```bash
sudo apt-get install -y ... libasound2-dev
```

### 3. **Windows**
**Problem:** Build failed with missing libclang
```
Unable to find libclang: "couldn't find any valid shared libraries matching: ['clang.dll', 'libclang.dll']"
```

**Root Cause:** `bindgen` (used by whisper-rs-sys) requires libclang to generate Rust bindings from C headers

**Fix:** Install LLVM via Chocolatey and set `LIBCLANG_PATH`
```powershell
choco install llvm -y
echo "LIBCLANG_PATH=C:\Program Files\LLVM\bin" >> $env:GITHUB_ENV
```

## Summary of Changes

### `.github/workflows/build.yml`

1. **Line 60**: Added `libasound2-dev` to Linux dependencies for ALSA audio support
2. **Lines 62-67**: Added LLVM installation step for Windows to provide libclang
3. **Lines 101-102**: Added `MACOSX_DEPLOYMENT_TARGET: 10.15` to build action's env section for macOS compatibility

## Build Strategy

The workflow now:
1. ✅ Removes GPU features (cuda, vulkan) from whisper-rs to simplify CI builds
2. ✅ Installs all required system dependencies per platform
3. ✅ Sets correct environment variables for compatibility
4. ✅ Builds with CPU-only whisper.cpp (still uses Accelerate on macOS for performance)

## Testing

After these changes, all platforms should build successfully:
- **macOS (Intel)**: CPU build with Accelerate framework
- **macOS (Apple Silicon)**: CPU build with Accelerate framework  
- **Linux**: CPU build with basic BLAS
- **Windows**: CPU build

## Next Steps

1. Commit these changes
2. Push to trigger GitHub Actions
3. Verify all 4 builds complete successfully
4. If successful, the build artifacts will be uploaded for each platform

## Notes

- The CPU-only builds will be slower than GPU-accelerated builds, but they are sufficient for CI/CD testing
- Local development can still use CUDA/Vulkan features
- The `src-tauri/.cargo/config.toml` file (with Windows-specific paths) is automatically removed on Unix platforms to prevent conflicts
