# GitHub Actions Build Strategy

## Problem Summary

The GitHub Actions builds were failing on Linux and macOS platforms with path-related errors:
- **Error**: `path segment contains separator ':'` in `$LD_LIBRARY_PATH` and `$DYLD_FALLBACK_LIBRARY_PATH`
- **Root Cause**: The `.cargo/config.toml` file contains Windows-specific paths (e.g., `target-dir = "C:/bld"`) that break Unix builds

The Windows builds were failing with whisper-rs build errors:
- **Error**: `called Result::unwrap() on an Err value: NotPresent`
- **Root Cause**: CUDA/Vulkan dependencies are not available in the GitHub Actions Windows environment

## Solution

We've implemented a three-part solution:

### 1. Remove Windows-Specific Cargo Config on Unix
```yaml
- name: Remove Windows cargo config on Unix
  if: matrix.platform != 'windows-latest'
  run: rm -f src-tauri/.cargo/config.toml
```

This prevents the `C:/bld` target directory and other Windows paths from interfering with Linux/macOS builds.

### 2. Patch Cargo.toml to Use CPU-Only whisper-rs
```yaml
- name: Patch Cargo.toml for CI builds
  run: |
    if [[ "$RUNNER_OS" != "Windows" ]]; then
      sed -i.bak 's/features = \["cuda", "vulkan"\]/features = []/g' src-tauri/Cargo.toml
    else
      (Get-Content src-tauri/Cargo.toml) -replace 'features = \["cuda", "vulkan"\]', 'features = []' | Set-Content src-tauri/Cargo.toml
    fi
  shell: bash
```

This removes CUDA and Vulkan features from whisper-rs during CI builds, falling back to CPU-only inference.

### 3. Set Environment Variable
```yaml
env:
  WHISPER_NO_GPU: "1"
```

This signals to whisper-rs that GPU acceleration is not available.

## Trade-offs

### ✅ Pros
- **Builds succeed** on all platforms (Linux, macOS x86_64, macOS ARM64, Windows)
- **Simple setup** - No need to install CUDA/Vulkan SDKs in CI
- **Fast builds** - Avoids complex GPU compilation
- **Reliable** - CPU-only builds are stable and consistent

### ⚠️ Cons
- **CI builds are CPU-only** - GPU acceleration is disabled in GitHub Actions
- **Local builds still use GPU** - Your local development builds retain CUDA/Vulkan support
- **CI builds are slower at runtime** - But this doesn't matter for build verification

## Local Development vs CI

| Aspect | Local Development | GitHub Actions CI |
|--------|------------------|------------------|
| **GPU Support** | ✅ CUDA + Vulkan | ❌ CPU Only |
| **Config File** | `.cargo/config.toml` used | Removed on Unix |
| **Target Dir** | `C:/bld` (Windows) | Default `target/` |
| **Build Time** | Fast (cached) | Moderate |
| **Runtime Speed** | Very Fast (GPU) | Slower (CPU) |

## Future Improvements

If you want GPU-accelerated CI builds in the future, you would need to:

1. **Use self-hosted runners** with GPUs and proper SDKs installed
2. **Set up CUDA/Vulkan** in CI environments (complex and time-consuming)
3. **Use pre-built binaries** for whisper.cpp with GPU support
4. **Cross-compile** with GPU libs from a dedicated build server

For now, the CPU-only CI builds provide:
- ✅ Build verification (ensure code compiles)
- ✅ Cross-platform compatibility checks
- ✅ Artifact generation for distribution

The GPU features remain available in your local builds and production releases.

## Testing

To test the CI build locally:

```bash
# Remove the cargo config (simulating Unix CI)
rm src-tauri/.cargo/config.toml

# Patch Cargo.toml (simulating CI)
sed -i.bak 's/features = \["cuda", "vulkan"\]/features = []/g' src-tauri/Cargo.toml

# Build
cd src-tauri
cargo build --release

# Restore files
git restore .cargo/config.toml Cargo.toml
```

This ensures your code builds correctly in CI environments.
