# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Install frontend dependencies
npm install

# Development (starts Vite + Tauri with hot-reload)
npm run tauri dev

# Production build
npm run tauri build

# Rust checks only (faster than full build)
cd src-tauri && cargo check

# Run Rust tests
cd src-tauri && cargo test
```

No frontend test framework is configured. No linting scripts are defined in package.json beyond what Tauri scaffolds.

## Architecture

Taurscribe is a **Tauri 2 desktop app** (React + TypeScript frontend, Rust backend) for local, offline speech-to-text transcription.

### IPC Bridge

All frontend↔backend communication uses Tauri's IPC:
- Frontend calls Rust via `invoke()` from `@tauri-apps/api/core`
- Backend emits events (`hotkey-start-recording`, `hotkey-stop-recording`, `transcription-chunk`, `models-changed`) that the frontend listens to via `listen()`
- Persistent settings are stored via `@tauri-apps/plugin-store` → `settings.json`

### Rust Backend (`src-tauri/src/`)

- **`lib.rs`** — Entry point: initializes WhisperManager, VADManager, ParakeetManager; builds the Tauri app; registers all commands; spawns hotkey listener and file watcher threads
- **`state.rs`** — `AudioState` global state struct, held by Tauri's managed state. All engines are `Arc<Mutex<...>>`. Whisper and Parakeet are **mutually exclusive** (switching to one unloads the other to save VRAM)
- **`commands/`** — Tauri command handlers split by domain: `recording.rs`, `models.rs`, `llm.rs`, `spellcheck.rs`, `settings.rs`, `misc.rs`, `downloader.rs`, `model_registry.rs`
- **`whisper.rs`** — `WhisperManager`: loads GGUF models via `whisper-rs`; processes buffered 16kHz mono audio after VAD triggers
- **`parakeet.rs` + `parakeet_loaders.rs`** — `ParakeetManager`: streaming CTC inference via `parakeet-rs` + ONNX Runtime (`ort`); uses a lock-free ring buffer for real-time audio
- **`vad.rs`** — Energy-based Voice Activity Detection; filters silence before sending audio to Whisper
- **`llm.rs`** — `LLMEngine`: loads a fine-tuned Qwen 2.5 0.5B GGUF model via `llama-cpp-2` for grammar/punctuation correction; loaded on demand
- **`spellcheck.rs`** — `SpellChecker` using `symspell`; loaded on demand
- **`audio.rs`** — `cpal`-based microphone capture; splits into two parallel streams: raw WAV file writer + AI transcription pipeline
- **`hotkeys/listener.rs`** — `rdev`-based global hotkey listener (`Ctrl+Win`)
- **`tray/`** — System tray with dynamic status icons (Ready/Recording/Processing)
- **`watcher.rs`** — `notify`-based file watcher on the models directory; emits `models-changed` event when files change
- **`utils.rs`** — Path helpers: `get_recordings_dir()` → `%LOCALAPPDATA%\Taurscribe\temp\`, `get_models_dir()` → `%LOCALAPPDATA%\Taurscribe\models\`
- **`types.rs`** — Shared enums: `ASREngine` (Whisper/Parakeet), `AppState` (Ready/Recording/Processing)

### React Frontend (`src/`)

`App.tsx` is the root component. All major logic is extracted into five custom hooks:

| Hook | Responsibility |
|------|---------------|
| `useHeaderStatus` | Status message with optional timeout and processing indicator |
| `useModels` | Whisper + Parakeet model lists; `refreshModels()` re-invokes both list commands |
| `usePostProcessing` | LLM grammar correction + SymSpell spell-check toggle; auto-loads/unloads engines on toggle |
| `useEngineSwitch` | Active engine state, loading state, engine-switch handlers with mutual-exclusion logic |
| `useRecording` | `handleStartRecording` / `handleStopRecording`; orchestrates the full post-processing pipeline (spell check → grammar LLM → type_text) |

`App.tsx` wires the hooks together and owns the UI rendering. Refs (e.g., `activeEngineRef`, `enableGrammarLMRef`) are used alongside state so async handlers always read the latest values without stale closures.

### Model File Locations

| Model type | Path |
|---|---|
| Whisper GGUF | `%LOCALAPPDATA%\Taurscribe\models\ggml-*.bin` |
| Parakeet ONNX | `%LOCALAPPDATA%\Taurscribe\models\<subdirectory>\` |
| Grammar LLM GGUF | `taurscribe-runtime\models\qwen_finetuned_gguf\model_q4_k_m.gguf` (hardcoded dev path), falls back to `GRAMMAR_LLM_DIR` env var, then `%LOCALAPPDATA%\Taurscribe\models\qwen_finetuned_gguf\` |
| Recordings (temp WAV) | `%LOCALAPPDATA%\Taurscribe\temp\` |

### Hardware Acceleration (Cargo features by platform)

| Platform | Whisper | Parakeet/ORT | LLM |
|---|---|---|---|
| Windows x86_64 | CUDA + Vulkan | CUDA + DirectML + TensorRT | CUDA |
| macOS | default | XNNPACK | Metal (auto) |
| Linux x86_64 | CUDA + Vulkan | CUDA + TensorRT | CUDA |
| Windows ARM64 | default | DirectML + XNNPACK | default |

### Recording Data Flow

1. `cpal` captures microphone at native sample rate (typically 48kHz stereo)
2. Two parallel channels: raw samples → WAV file writer thread; processed samples → AI transcription thread
3. Audio resampled to 16kHz mono for AI engines
4. Whisper: buffered approach with VAD (accumulates ~6s, sends on voice activity)
5. Parakeet: streaming ring-buffer approach (≤0.5s latency, continuous CTC decoding)
6. After `stop_recording`, the final transcript goes through: optional SymSpell → optional grammar LLM → `type_text` (Enigo keyboard automation to paste into active window)

### Key Constraints

- **Whisper and Parakeet are mutually exclusive** in VRAM; switching unloads the other
- **`MIN_RECORDING_MS = 1500`** — recordings shorter than 1.5s are rejected
- The grammar LLM GGUF path is currently hardcoded to a local dev path in `llm.rs:18`; this must be updated before distributing builds
- Adding new Tauri commands requires registering them in the `invoke_handler!` macro in `lib.rs`
