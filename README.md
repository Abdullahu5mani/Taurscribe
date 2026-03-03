# Taurscribe 🎙️

![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)
![Tauri](https://img.shields.io/badge/Tauri-24C8D5?style=for-the-badge&logo=tauri&logoColor=white)
![React](https://img.shields.io/badge/React-20232A?style=for-the-badge&logo=react&logoColor=61DAFB)
![CUDA](https://img.shields.io/badge/CUDA-76B900?style=for-the-badge&logo=nvidia&logoColor=white)
![TypeScript](https://img.shields.io/badge/TypeScript-007ACC?style=for-the-badge&logo=typescript&logoColor=white)

> **Private, Offline, GPU-Accelerated Speech-to-Text Application**

**Taurscribe** (*Tauri* + *Transcribe*) is a state-of-the-art desktop application designed to bring powerful AI transcription models directly to your local machine. By running entirely offline, it guarantees 100% privacy while leveraging your hardware's full potential for real-time performance.

---

<p align="center">
  <table>
    <tr>
      <td align="center">
        <img src="assets/screenshots/UI.png" width="100%" />
        <br />
        <b>Modern, Minimalist Interface</b>
      </td>
      <td align="center">
        <img src="assets/screenshots/live-transcription.png" width="100%" />
        <br />
        <b>Real-Time Streaming Transcription</b>
      </td>
    </tr>
    <tr>
      <td colspan="2" align="center">
        <img src="assets/screenshots/Settings.png" width="100%" />
        <br />
        <b>Comprehensive Settings & Model Management</b>
      </td>
    </tr>
  </table>
</p>

## 🚀 Key Technical Achievements

This project demonstrates advanced systems programming and machine learning integration techniques:

*   **Dual-Engine Architecture**: Seamlessly switches between **OpenAI Whisper** (for high-accuracy batch processing) and **NVIDIA Parakeet** (for ultra-low latency streaming).
*   **Intelligent Hardware Acceleration**: Implements a dynamic backend selection system that automatically utilizes **CUDA** (NVIDIA), **DirectML** (Windows NPU/AMD), or **Metal** (macOS) for optimal inference speeds.
*   **Zero-Copy Audio Pipeline**: Built with Rust's ownership model to manage high-throughput audio streams (48kHz stereo → 16kHz mono) without memory leaks or data races.
*   **Custom Fine-Tuned LLM**: Features a specialized, fine-tuned Language Model (LLM) optimized for local inference. This model is trained to reconstruct punctuation, capitalization, and sentence structure from raw ASR output, delivering professional-grade readability on consumer hardware without cloud dependencies.
*   **Custom Voice Activity Detection (VAD)**: Energy-based VAD algorithm to filter silence and optimize compute resources, reducing idle CPU usage by ~45%.

## ⚙️ Audio Processing Pipeline

Taurscribe employs two distinct architectural strategies to balance speed and accuracy:

```
  🎯 WHISPER ARCHITECTURE (Buffered)
  ┌──────────────┐     ┌─────────────┐     ┌────┐              ┌──────────────┐
  │  Audio Input │ ──► │ Accumulator │ ──► │VAD?│ ──(Yes)───► │   Whisper    │
  └──────────────┘     └─────────────┘     └────┘              │   Encoder    │
                              ▲               │ (No)           └──────┬───────┘
                              └───────────────┘                       │
                                   (Wait 6s)                          ▼
                                                              ┌──────────────┐
                                                              │   Seq2Seq    │
                                                              │   Decoder    │
                                                              └──────────────┘
```

```
  ⚡ PARAKEET ARCHITECTURE (Streaming Ring Buffer)
  
       Microphone (Write Ptr)
             │
             ▼
      ┌──────┴──────┐
      │  R I N G    │ ◄── Circular Buffer (Lock-Free)
      │  B U F F E R│
      └──────┬──────┘
             │
             ▼
        (Read Ptr) ──► ┌─────────────┐      ┌──────────────┐
                       │  Parakeet   │ ──►  │  CTC Search  │ ──► "Hello..."
                       │   Engine    │      │   Decoding   │
                       └─────────────┘      └──────────────┘
                                                  ▲
                                                  │
                                            (0.5s Latency)
```

> **Note on Circular Buffer**: The Parakeet engine utilizes a lock-free ring buffer to handle audio samples. As the microphone writes data (`write_ptr`), the inference engine chases it (`read_ptr`) with millisecond precision, ensuring zero buffer bloat.

## ✨ Features

- **Offline Privacy**: No data ever leaves your device. No API keys, no subscriptions, no tracking.
- **Real-Time Transcription**: See words appear instantly as you speak.
- **Grammar Correction**: Integrated LLM automatically fixes punctuation, capitalization, and grammar on the fly.
- **Model Management**: Support for GGUF and ONNX formats with easy switching.
- **Cross-Platform Core**: Designed with a Rust backend that compiles to native binaries for Windows, macOS, and Linux.
- **Global Hotkeys**: Control recording from any application using `Ctrl+Win`, perfect for capturing quick thoughts without switching windows.
- **System Tray Integration**: Runs unobtrusively in the background with dynamic status icons (Ready/Recording/Processing).
- **Download Manager**: Integrated tool to download models directly from the app with progress tracking, automatic extraction (zip/CoreML), and cryptographic integrity verification.

## 🔐 Model Integrity & SHA Verification

Taurscribe implements cryptographic verification to ensure every downloaded model is authentic and uncorrupted. This protects against man-in-the-middle attacks, partial downloads, and file corruption.

### How It Works

| Step | Description |
|------|-------------|
| **1. Registry** | Each model in the [model registry](src-tauri/src/commands/model_registry.rs) has a hardcoded **SHA-1 hash** compiled into the binary. These hashes are sourced from the official upstream repositories (e.g. [ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp)). |
| **2. Download** | Files are streamed from Hugging Face / GitHub with real-time progress events sent to the UI. |
| **3. Verify** | After download, the `verify_model_hash` command reads each file in **8 KB chunks**, computing a streaming SHA-1 digest. The result is compared against the hardcoded hash. |
| **4. Marker** | On success, a `.verified` marker file is written alongside the model so the app can skip re-verification on subsequent launches. |
| **5. Mismatch** | If the hash doesn't match, the user receives an explicit error (`Hash mismatch for <file>: Expected <x>, Got <y>`) and the model is not loaded. |

### Verified Models

All **Whisper GGML models** (tiny through large-v3-turbo, including quantized variants) ship with hardcoded SHA-1 hashes:

```
ggml-tiny.bin          → bd577a113a864445d4c299885e0cb97d4ba92b5f
ggml-base.bin          → 465707469ff3a37a2b9b8d8f89f2f99de7299dac
ggml-small.bin         → 55356645c2b361a969dfd0ef2c5a50d530afd8d5
ggml-medium.bin        → fd9727b6e1217c2f614f9b698455c4ffd82463b4
ggml-large-v3.bin      → ad82bf6a9043ceed055076d0fd39f5f186ff8062
ggml-large-v3-turbo.bin→ 4af2b29d7ec73d781377bfd1758ca957a807e941
...and all quantized variants (q5_0, q5_1, q8_0)
```

> **Note**: CoreML encoders, Parakeet ONNX models, LLM weights, and the spell-check dictionary currently download without SHA verification (`sha1: ""`). Their integrity relies on HTTPS transport security. Adding SHA-256 hashes for these models is on the roadmap.

## 🧠 Custom Intelligence Engine

Taurscribe goes beyond simple transcription by integrating a **custom fine-tuned LLM** specifically designed for text refinement.

*   **Task-Specific Optimization**: Fine-tuned on high-quality datasets to master the specific task of converting raw, unformatted speech into grammatically correct, punctuated text.
*   **Edge-Optimized**: Quantized and pruned to run efficiently on local CPUs and consumer GPUs, ensuring low-latency performance alongside the transcription engine.
*   **Context Awareness**: Understands sentence boundaries and proper noun formatting significantly better than generic small-parameter models.

## 🛠️ Architecture Overview

The application follows a modular architecture separating the high-performance backend from the reactive frontend:

| Component | Technology | Responsibility |
|-----------|------------|----------------|
| **Frontend** | React + TypeScript | Reactive UI, Real-time Visualizations, State Management |
| **Backend Core** | Rust (Tauri) | Application State, IPC Bridge, System Integration |
| **Audio Engine** | `cpal` + `ringbuf` | Low-latency Audio Capture, Ring Buffer Management |
| **Inference** | `candle-rs` | Tensor Operations, Model Loading, GPU/CPU Execution |
| **Post-Process** | Custom Logic | Tokenization, Spell Checking, Text Formatting |

*For a deep dive into the code structure, see [ARCHITECTURE.md](./ARCHITECTURE.md).*

## 📦 Getting Started

### Prerequisites

- **Rust**: [Install Rust](https://www.rust-lang.org/tools/install)
- **Node.js**: [Install Node.js](https://nodejs.org/) (or Bun)
- **Build Tools**:
  - **Windows**: Visual Studio C++ Build Tools & CMake
  - **macOS**: Xcode Command Line Tools
  - **Linux**: `libwebkit2gtk-4.0-dev`, `build-essential`, `libssl-dev`

### Installation

1.  **Clone the repository**
    ```bash
    git clone https://github.com/Abdullahu5mani/Taurscribe.git
    cd Taurscribe
    ```

2.  **Install frontend dependencies**
    ```bash
    npm install
    # or
    bun install
    ```

3.  **Run in Development Mode**
    This will start the Vite server and the Tauri backend with hot-reload enabled.
    ```bash
    npm run tauri dev
    ```

## 🧪 Hardware Acceleration Setup

Taurscribe automatically detects available hardware. To ensure GPU support:

- **NVIDIA**: Ensure latest drivers and CUDA toolkit are installed. The app checks for `nvidia-smi`.
- **AMD/Intel**: DirectML is used automatically on Windows.
- **macOS**: Metal is used automatically on Apple Silicon.

## 📝 Roadmap

- [x] Core Whisper Integration
- [x] Real-time VAD Implementation
- [x] Local LLM Grammar Correction
- [x] UI Sound Effects (Success/Fail/Too Short)
- [x] Customizable Hotkeys
- [x] MacOS CoreML optimization (Whisper.cpp)
- [x] Nice Installer Screen (Custom Tauri Customization)
- [x] Audio Lead-in & Tail Silence Padding
- [ ] Brand Refresh (New Logo)
- [ ] Toggle Recording Mode (Click-to-Record vs Hold)
 - [x] Text Snippets / Macros
 - [x] Custom Dictionary (Proper Nouns)
- [ ] Smart Fallback for Short Audio
- [ ] Transcription History (Save & revisit past transcriptions)
- [ ] Recording Duration Timer (Live `00:12` counter while recording)
- [ ] Multilingual / Language Selector (Whisper supports 90+ languages)
- [ ] Auto-Start on Boot
 - [x] Minimize to System Tray
- [ ] Live Transcript Display (Show streaming text while recording)
- [ ] Export Options (`.txt`, `.srt`, `.md`)
- [ ] Audio Waveform Visualizer
 - [x] Extended Keyboard Shortcuts
- [ ] Recording Playback (Listen back to saved `.wav` files)
- [ ] Speaker Diarization (Label transcript by speaker)
- [ ] File / Audio Import (Drag-and-drop `.wav`, `.mp3`, `.m4a`)
 - [x] Smart Punctuation & Formatting
- [ ] Auto-Update Mechanism (Tauri updater plugin)
- [ ] Accessibility (Screen reader, keyboard navigation, high-contrast)
- [ ] First Launch & Guide Screen (Onboarding flow)
- [ ] Overlay Recording Mode (Press and hold to record, release to stop)
- [ ] Context-aware Paste (Text/Image detection on clipboard)
 - [x] Visual Listening Overlay

---

<p align="center">
  Built with ❤️ using <strong>Rust</strong> and <strong>Tauri</strong>.
</p>
