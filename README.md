<!--
Logo and tagline section
-->
<div align="center">
  <img src="public/logos/taurscribe-logo.svg" width="120" alt="Taurscribe Logo" />
  <h1>Taurscribe</h1>
  <strong>Local speech-to-text that respects your privacy</strong>

<br/>

**Private • Offline • GPU-Accelerated • Instant**

![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)
![TypeScript](https://img.shields.io/badge/TypeScript-007ACC?style=for-the-badge&logo=typescript&logoColor=white)
![React](https://img.shields.io/badge/React-20232A?style=for-the-badge&logo=react&logoColor=61DAFB)
![Tauri](https://img.shields.io/badge/Tauri-24C8D5?style=for-the-badge&logo=tauri&logoColor=white)

</div>

---

<p align="center">
  <img src="assets/screenshots/UI.png" width="85%" alt="Taurscribe Interface" />
  <br/>
  <i>Clean, focused interface. Status and settings at a glance.</i>
</p>

<p align="center">
  <img src="assets/screenshots/live-transcription.png" width="85%" alt="Live Transcription" />
  <br/>
  <i>Real-time streaming with instant visual feedback and live output.</i>
</p>

<p align="center">
  <img src="assets/screenshots/Settings.png" width="85%" alt="Settings" />
  <br/>
  <i>Model management, hotkeys, and post-processing — all in one place.</i>
</p>

---

## What is Taurscribe?

Taurscribe is a desktop application for local, offline speech-to-text transcription. Unlike cloud-based solutions, everything runs on your machine — your audio never leaves your computer.

Built on Tauri (Rust + React), Taurscribe gives you the speed and accuracy of cloud services with the privacy and control of local software. Choose your transcription engine, enable post-processing, and get publication-ready text instantly.

* **Local-first** — No cloud APIs, no tracking, no surprises
* **Three engines** — Whisper (accuracy), Parakeet (speed), Cohere (alternative)
* **Smart post-processing** — AI-powered grammar correction and spell-checking
* **Cross-platform** — Windows, macOS, Linux
* **GPU-accelerated** — Automatic hardware detection (NVIDIA CUDA, Apple Metal, AMD Vulkan)

---

## Features

### Transcription

| Feature | Details |
|---------|---------|
| **Multiple engines** | Switch engines on-the-fly; mutually exclusive to save VRAM |
| **Real-time streaming** | Parakeet delivers sub-500ms latency; see words appear as you speak |
| **Batch processing** | Drag audio/video files into the app for high-accuracy offline transcription |
| **Global hotkey** | Press Ctrl+Win anywhere to record, even behind other windows |
| **File support** | All common audio and video codecs |

### Text quality

| Feature | Details |
|---------|---------|
| **Grammar & tone correction** | FlowScribe LLM v2 — 0.5B parameters, runs in <100ms on CPU |
| **Tone styles** | Casual, Verbatim, Enthusiastic, Software Dev, Professional |
| **Spell checking** | SymSpell dictionary with custom word-list support |
| **Personalization** | User dictionary for consistent technical term transcription |

### Control & integration

| Feature | Details |
|---------|---------|
| **First-run wizard** | Hardware detection and engine onboarding |
| **Quick settings** | One-click toggles for quality, tone, and spell-check |
| **System tray** | Minimal background presence with LED status signaling |
| **Model management** | Download, verify, and switch models without restarting |

---

## FlowScribe LLM v2: Local text refinement

Raw ASR output is often rough. [FlowScribe v2](https://huggingface.co/Abdullahu5mani/flowscribe-qwen2.5-0.5b-v2) is a fine-tuned, locally-hosted language model that runs in under 100ms:

```
Raw:       "im going to the coffee shop tomorrow at two"
Refined:   "I'm going to the coffee shop tomorrow at 2 PM."
```

Handles punctuation, capitalization, contractions, tone adaptation, and technical term consistency — no cloud round-trip, no latency penalty.

---

## Architecture

### Two transcription strategies

**Whisper — Buffered accuracy-first**

Accumulates ~6 seconds of audio with voice activity detection, then sends to the encoder when speech is detected.

```
Microphone (16kHz mono)
  ▼
Ring Buffer (6s accumulation)
  ▼
Voice Activity Detector
  ├─ Silence → wait
  └─ Speech → send to encoder
  ▼
Whisper Encoder → Output
```

**Parakeet — Lock-free streaming**

Uses a non-blocking ring buffer for sub-500ms latency. The inference process chases the write pointer, producing output continuously.

```
Microphone (48kHz stereo)
  ▼
Resampler (16kHz mono)
  ▼
Lock-Free Ring Buffer (write → read)
  ▼
Parakeet Engine (continuous inference)
  ▼
CTC Decoding → Output stream
```

Whisper optimizes for accuracy — best for meetings, interviews, archival. Parakeet optimizes for responsiveness — best for real-time note-taking and live captions.

### Transcription engines

| Engine | Latency | Primary use | Format |
|--------|---------|-------------|--------|
| **Whisper** | 2–10s | High-accuracy transcription | GGUF |
| **Parakeet** | <500ms | Real-time streaming | ONNX |
| **Cohere** | Varies | Alternative backbone | ONNX |

Engines are **mutually exclusive** — switching unloads the previous one to free VRAM.

---

## Hardware acceleration

| OS | Whisper | Parakeet/ORT | Grammar LLM |
|----|---------|--------------|-------------|
| Windows x64 | CUDA, Vulkan | CUDA, DirectML, TensorRT | CUDA |
| macOS | Accelerate | XNNPACK | Metal |
| Linux x64 | CUDA, Vulkan | CUDA, TensorRT | CUDA |
| Windows ARM64 | CPU | DirectML, XNNPACK | CPU |

NVIDIA GPUs try CUDA first, falling back to DirectML to avoid reshape failures.

---

## Technical stack

| Layer | Tools | Responsibility |
|-------|-------|----------------|
| **Frontend** | React, TypeScript | UI, model switching, transcription display |
| **IPC** | Tauri, Serde | Frontend ↔ Backend messaging |
| **Audio** | CPAL, RingBuf (Rust) | Microphone capture, multi-threaded pipeline |
| **Inference** | whisper.rs, parakeet.rs, ORT | Model loading, GPU dispatch, streaming |
| **Post-process** | llama-cpp-2, SymSpell | Grammar LLM, spell-check |
| **Platform** | Native APIs | Hotkeys, tray, file dialogs, permissions |

### Engineering highlights

* **Zero-copy audio pipeline** — Rust ownership prevents leaks
* **Lock-free ring buffer** — Sub-millisecond Parakeet latency from wait-free structures
* **Custom VAD** — Energy-based voice activity detection, ~45% idle CPU reduction
* **Dynamic backend selection** — CUDA → Vulkan → CPU routing
* **Quantized LLM** — FlowScribe v2 at 0.5B parameters, <100ms inference
* **SHA-256 model verification** — 8KB chunk-by-chunk integrity checks on every download

---

## Getting started

### System requirements

* GPU optional but recommended (NVIDIA/AMD/Metal for <1s latency)
* 4GB+ RAM per active engine
* 2GB+ disk per model

### Installation

1. Download from [Releases](https://github.com/Abdullahu5mani/Taurscribe/releases)
2. Run installer (Windows, macOS, or Linux)
3. Launch — Setup Wizard guides hardware and engine selection
4. Download a model (smallest: `ggml-tiny.bin` ~75MB)
5. Test: Press **Ctrl+Win**

---

## Development

```bash
# Install dependencies
npm install

# Start dev server with live reload
npm run tauri dev

# Build for production
npm run tauri build

# Quick Rust check (faster than full build)
cd src-tauri && cargo check

# Run test suite
cd src-tauri && cargo test
```

See [TESTING.md](./TESTING.md) for integration tests and the LibriSpeech accuracy eval pipeline.

---

## Acknowledgments

Built with:
* **whisper.rs** — ggerganov/whisper.cpp
* **ONNX Runtime** — Microsoft
* **llama.cpp** — ggerganov
* **Tauri** — Desktop framework
* **React 19** — UI framework

---

## Support & contributions

* Issues: [GitHub Issues](https://github.com/Abdullahu5mani/Taurscribe/issues)
* Discussions: [GitHub Discussions](https://github.com/Abdullahu5mani/Taurscribe/discussions)

---

**Taurscribe: Speech-to-text on your terms.**
