# Project: Taurscribe Platform Performance & Cross-Platform Stabilization

## Architecture
Taurscribe is a desktop speech-to-text and AI dictation application built on Tauri v2 (Rust backend, web frontend).
The platform runs multi-engine local ASR (Whisper via `whisper-rs` on CoreML/Metal/CUDA, Parakeet Conformer via MLX or ONNX Runtime, Granite Speech via MLX or ONNX Runtime) and local LLM grammar correction (`llama-cpp-2`).

The architecture interfaces with platform-native OS features:
- **macOS**: Apple Silicon Metal compute pipeline, CoreML ANE execution for Whisper encoder, AppKit/Accessibility text injection, AVFoundation audio capture, dynamic dylib bundling.
- **Windows**: Multi-threaded audio worker pipeline, Intel hybrid architecture P/E-core affinity, AVX2/AVX-VNNI/AVX-512 runtime SIMD dispatch, DirectML / CUDA acceleration, Windows SendInput text injection.
- **Linux**: Dual display server support (X11 & Wayland), multi-tier text injection (`/dev/uinput`, `ydotool`, `wtype`, FreeDesktop RemoteDesktop Portal, `enigo`), dual audio backend (ALSA & native PipeWire), dynamic CUDA driver stub discovery on headless CI runners.

---

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| 1 | Metal Shader Cache Warmup | Startup pre-warming routine for MLX Metal compute pipelines to eliminate first-chunk JIT compilation stutter | M1 | ORIGINAL_REQUEST §R1 |
| 2 | Model Registry Quantization | Register and support compatible 8-bit quantized model weights with SHA256 checksums in registry and downloader | M1 | ORIGINAL_REQUEST §R1 |
| 3 | CoreML Whisper Preservation | Preserve Whisper execution on CoreML ANE encoder as-is with zero MLX involvement | M1 | ORIGINAL_REQUEST §R1 |
| 4 | CoreML ANE Decoder Graph Investigation | Investigate CoreML computation graphs for Whisper decoder, stateful KV-cache, and ANE offloading feasibility | M2 | ORIGINAL_REQUEST §R2 |
| 5 | CoreML Decoder Generation & Benchmarks | Offline CoreML decoder conversion script, limitation documentation, and benchmark matrix | M2 | ORIGINAL_REQUEST §R2 |
| 6 | Intel macOS CI Release Matrix | Add `x86_64-apple-darwin` to `.github/workflows/release.yml` with `macos-latest` cross-compilation | M3 | ORIGINAL_REQUEST §R3 |
| 7 | Intel macOS Dylib Bundling | Run `dylibbundler` to rewrite `@rpath` and bundle x86_64 dylibs into `Contents/Frameworks/` | M3 | ORIGINAL_REQUEST §R3 |
| 8 | Taurscribe_x64.dmg Release Artifact | Automate production and upload of `Taurscribe_x64.dmg` alongside `aarch64` build | M3 | ORIGINAL_REQUEST §R3 |
| 9 | Windows Hybrid P-Core Thread Affinity | Implement thread affinity pinning (`SetThreadAffinityMask`) for Intel hybrid P-cores via `GetLogicalProcessorInformationEx` | M4 | ORIGINAL_REQUEST §R4 |
| 10 | Windows SIMD Runtime Dispatch | Add runtime CPU feature detection (`is_x86_feature_detected!`) for AVX-512, AVX-VNNI, and AVX2 | M4 | ORIGINAL_REQUEST §R4 |
| 11 | Windows GPU LLM Retention | Retain existing GPU configurations for LLM grammar correction without regression | M4 | ORIGINAL_REQUEST §R4 |
| 12 | Linux Wayland Input Injection | Multi-tier text injection via `/dev/uinput`, `ydotool`, `wtype`, FreeDesktop Portal, and `enigo` | M5 | ORIGINAL_REQUEST §R5 |
| 13 | Linux PipeWire Audio Pipeline | Native PipeWire audio negotiation alongside ALSA without device contention | M5 | ORIGINAL_REQUEST §R5 |
| 14 | Linux CI Build Job Re-enablement | Re-enable Linux in `release.yml` with dynamic `libcuda.so` discovery and stub configuration | M5 | ORIGINAL_REQUEST §R5 |
| 15 | Cross-Platform Build & Test Validation | Clean CI workflow validation, local `cargo check` across targets, and test verification | M6 | ORIGINAL_REQUEST §Acceptance Criteria |
| 16 | Leftover Items Hardware Audit | Itemized audit report of items requiring specialized physical hardware verification | M6 | ORIGINAL_REQUEST §Acceptance Criteria |

---

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | Apple Silicon Metal Pre-warm & Model Quantization (R1) | MLX warmup routine in `parakeet_mlx/engine.rs`, background pre-warming, model registry & downloader 8-bit quantized weights, preserve Whisper CoreML | none | DONE |
| M2 | CoreML Whisper Decoder Feasibility & Graph Investigation (R2) | Offline CoreML stateful decoder generator script, limitation documentation (KV-cache, ANE residency), benchmark matrix | M1 | DONE |
| M3 | Intel macOS CI Release Matrix Expansion (R3) | `.github/workflows/release.yml` x86_64 target, dylibbundler bundling, `Taurscribe_x64.dmg` artifact production | none | DONE |
| M4 | Windows P/E-Core Affinity & SIMD Runtime Dispatch (R4) | Windows hybrid P-core affinity pinning (`GetLogicalProcessorInformationEx`), AVX-512/AVX-VNNI/AVX2 detection, GPU LLM retention | none | DONE |
| M5 | Linux Wayland Input & PipeWire Audio Capture (R5) | Wayland multi-tier text injection, PipeWire audio negotiation, Linux CI build re-enablement with dynamic CUDA stubs | M3 | DONE |
| M6 | Final Verification & Leftovers Audit | Comprehensive validation across CI syntax, cross-platform cargo checks, test pass, and physical hardware audit | M1, M2, M3, M4, M5 | DONE |
| M7 | Appium macOS E2E UI Automation & Multi-Engine Benchmarks | Full automated UI test suite across native file picker, CoreML file transcription (50.2x RT), engine picker popover, settings navigation, and multi-model benchmark matrix | M1, M2 | DONE |

---

## Interface Contracts

### 1. MLX Metal Warmup Interface (`src-tauri/src/parakeet_mlx/engine.rs`)
```rust
impl ParakeetNemotronMlx {
    /// Warms up Metal compute pipelines and JIT kernels with a dummy chunk (560ms of silence).
    /// Calls self.reset() afterwards to restore clean state.
    pub fn warmup(&mut self) -> Result<(), ParakeetMlxError>;
}
```

### 2. Windows CPU Affinity & SIMD Interface (`src-tauri/src/cpu_features.rs` / `src-tauri/src/platform_tuning.rs`)
```rust
#[cfg(target_os = "windows")]
pub fn get_performance_core_affinity_mask() -> Option<usize>;

#[cfg(target_os = "windows")]
pub fn apply_thread_performance_affinity();

pub struct SimdCapabilities {
    pub has_avx2: bool,
    pub has_fma: bool,
    pub has_avx512f: bool,
    pub has_avx512vnni: bool,
    pub has_avxvnni: bool,
}

impl SimdCapabilities {
    pub fn detect() -> Self;
}
```

### 3. Linux Text Injection Interface (`src-tauri/src/text_injection.rs`)
```rust
pub enum TextInjectionBackend {
    UInput,
    Ydotool,
    Wtype,
    RemoteDesktopPortal,
    Enigo,
}

pub fn inject_text_or_paste(text: &str) -> Result<TextInjectionBackend, String>;
```

---

## Code Layout
- `src-tauri/src/parakeet_mlx/`: MLX fast conformer implementation (Apple Silicon).
- `src-tauri/src/granite_mlx/`: MLX Granite speech implementation.
- `src-tauri/src/whisper.rs`: Whisper ASR backend (CoreML ANE encoder + GGML Metal/CPU/CUDA decoder).
- `src-tauri/src/commands/model_registry.rs`: Model metadata, download URLs, SHA256 hashes, quantization types.
- `src-tauri/src/commands/recording.rs`: Recording lifecycle, worker threads (`transcriber_thread`), text injection.
- `src-tauri/src/commands/downloader.rs`: Downloader pipeline and zip extractor.
- `src-tauri/src/platform_tuning.rs`: Thread affinity, priority, hybrid P/E-core detection.
- `src-tauri/src/cpu_features.rs`: SIMD runtime detection (`is_x86_feature_detected!`).
- `src-tauri/src/text_injection.rs`: Cross-platform text injection (Wayland `/dev/uinput`, `ydotool`, Portal, X11, macOS, Windows).
- `src-tauri/build.rs`: Native library linking, dynamic CUDA search paths for Linux/Windows.
- `.github/workflows/release.yml`: Release matrix configuration (macOS arm64/x64, Windows x64/arm64, Linux x64).
- `scripts/`: Platform helper scripts (`bundle-macos-dylibs.sh`, `bundle-linux-solibs.sh`, `export_whisper_decoder_coreml.py`).
- `docs/`: Technical feasibility reports (`docs/whisper_coreml_decoder_feasibility.md`).
