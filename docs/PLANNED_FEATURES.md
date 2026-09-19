# Taurscribe: Planned Features & Architectural Roadmap
### Ordered Strictly from Easiest to Hardest

> **Vision**: Transform Taurscribe from a local speech-to-text dictation utility into the premier **100% offline, cross-platform AI Meeting Intelligence & Dictation Suite**—combining sub-80ms streaming voice typing with speaker-diarized meeting cataloging and local LLM intelligence, with zero cloud dependency and zero subscription fees.

---

## Master Ranking & Complexity Overview

| Rank | Feature | Estimated Time | Complexity | Core Dependency & Notes |
|:---:|:---|:---:|:---:|:---|
| **#1** | [1-Click Whisper CoreML ANE Auto-Downloader](#rank-1-1-click-whisper-coreml-ane-auto-downloader) | **~1–2 hours** | ⭐ | Existing Downloader & Model Registry |
| **#2** | [Custom Vocabulary & Context Jargon Injection](#rank-2-custom-vocabulary--context-jargon-injection) | **~2–3 hours** | ⭐⭐ | Existing `DictionaryTab.tsx` + Whisper `initial_prompt` |
| **#3** | [Automated Meeting Categorization with User Confirmation](#rank-3-automated-meeting-categorization-with-user-confirmation) | **~1 day** | ⭐⭐⭐ | Local LLM Zero-Shot Structured JSON Inference |
| **#4** | [Searchable Meeting Catalog & Action Item Hub](#rank-4-searchable-meeting-catalog--action-item-hub) | **~1–2 days** | ⭐⭐⭐ | SQLite / JSON Store + React Catalog View |
| **#5** | [Qwen3-ASR Engine Integration (Open ASR SOTA)](#rank-5-qwen3-asr-engine-integration-open-asr-leaderboard-sota) | **~1–2 days** | ⭐⭐⭐⭐ | Existing `ort` (CoreML/DirectML/CUDA) + Model Runtime |
| **#6** | [Speaker Diarization with Voiceprint Vault & Audio Snippets](#rank-6-speaker-diarization-with-isolated-voiceprint-enrollment--memory) | **~2–3 days** | ⭐⭐⭐⭐ | ONNX `pyannote` + `CAM++` Embedding Pipeline |
| **#7** | [Live Floating Capsule with Real-Time Audio Waveform](#rank-7-live-floating-capsule-with-real-time-audio-waveform) | **~2–3 days** | ⭐⭐⭐⭐ | Non-activating NSPanel/WebView + Caret Tracker |
| **#8** | [Dual-Channel System Loopback & Mic Meeting Recorder](#rank-8-dual-channel-system-loopback--mic-meeting-recorder) | **~3–4 days** | ⭐⭐⭐⭐⭐ | ScreenCaptureKit (Mac), WASAPI Loopback (Win), PipeWire (Linux) |
| **#9** | [Voice Transformation Commands (Specialized Model Fine-Tuning)](#rank-9-voice-transformation-commands-specialized-model-fine-tuning) | **Deferred** | ⭐⭐⭐⭐⭐ | **DEFERRED**: Requires dataset curation, LoRA/fine-tuning, GGUF export |

---

## Core Architectural Guarantees

| Principle | Taurscribe Guarantee |
|:---|:---|
| **100% On-Device Privacy** | No audio, transcript, voiceprint, or meeting metadata ever touches an external server or cloud API. |
| **True Cross-Platform Acceleration** | Native acceleration across macOS (Apple Silicon Metal + ANE), Windows 11 (NVIDIA CUDA, DirectML, Snapdragon ARM64), and Linux (CUDA, Vulkan, NEON). |
| **Dual-Mode AI Engine** | **Parakeet** for instant sub-80ms real-time voice typing; **Whisper / Qwen3-ASR** for long-form contextual accuracy. |
| **Zero Python Dependency** | All models execute in pure compiled native Rust via `ort` (ONNX Runtime), `whisper-rs` (C++), `mlx-rs` (Metal), and `llama-cpp-2` (C++). |

---

## [RANK 1] 1-Click Whisper CoreML ANE Auto-Downloader
* **Difficulty:** ⭐ (Easiest — Quick Win)
* **Estimated Effort:** ~1–2 hours
* **Target Platforms:** macOS (Apple Silicon M-Series)

### Problem & Opportunity
Whisper runs at **38x real-time** on Apple Silicon Metal GPU, but jumps to **85x real-time** (2.2x faster) with zero fan noise and almost zero battery draw when the 30-second mel encoder graph is offloaded to the **Apple Neural Engine (ANE)** via a companion `ggml-{model}-encoder.mlmodelc` bundle. Currently, users must manually locate, download, and extract these bundles.

### The Solution
1. **Model Manager UI**:
   - Detect `is_apple_silicon()`.
   - Display an **"⚡ ANE Accelerated"** badge on supported Whisper models (Tiny, Base, Small, Medium, Large-v3-Turbo).
2. **Automated Companion Download**:
   - In [`model_registry.rs`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src-tauri/src/commands/model_registry.rs), attach companion CoreML `.zip` URLs from Hugging Face (`ggerganov/whisper.cpp`).
   - When the user clicks "Download Model", the existing [`downloader.rs`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src-tauri/src/commands/downloader.rs) automatically pulls both `ggml-{model}.bin` and `ggml-{model}-encoder.mlmodelc.zip`.
   - Extract the `.mlmodelc` folder alongside the binary.
   - [`whisper.rs`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src-tauri/src/whisper.rs) automatically detects the companion directory and activates the `CoreML` backend.

---

## [RANK 2] Custom Vocabulary & Context Jargon Injection
* **Difficulty:** ⭐⭐ (Easy–Moderate)
* **Estimated Effort:** ~2–3 hours
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Problem
General ASR models stumble on proprietary developer syntax (`useCallback`, `tauri-plugin-store`, `x86_64`), medical/legal jargon, and company names.

### The Solution
1. **User Custom Vocabulary List**:
   - Leverage the existing [`DictionaryTab.tsx`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src/components/settings/AboutTab.tsx) store.
   - Users maintain a list of custom words, names, acronyms, and technical symbols.
2. **Whisper Decoder Logit Biasing**:
   - Pass the custom word list into Whisper's `params.set_initial_prompt(...)`.
   - Tells Whisper’s autoregressive decoder to strongly favor the tokens making up those specific words during beam search.
3. **Dynamic Active-Window Context Detection**:
   - When the recording hotkey is pressed, inspect the active window title:
     - Active in VS Code / Terminal ──► Automatically append language/syntax keywords to prompt.
     - Active in Medical EHR / Law practice app ──► Bias toward domain vocabulary.

---

## [RANK 3] Automated Meeting Categorization with User Confirmation
* **Difficulty:** ⭐⭐⭐ (Moderate)
* **Estimated Effort:** ~1 day
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Overview
Immediately after a meeting ends, the embedded local LLM processes the transcript, automatically categorizes the meeting, generates a concise descriptive title, extracts action items with assigned owners, and presents an interactive confirmation modal for 1-click approval.

### Interactive Confirmation Modal UI
```
┌─────────────────────────────────────────────────────────────────────────────┐
│ 🎙️ Meeting Processed (24m 18s)                              [Auto-Categorized]│
├─────────────────────────────────────────────────────────────────────────────┤
│ Title:    [ Sprint Planning: CoreML ANE Integration                  ] ✏️   │
│ Category: [ 🛠️ Engineering / Sprint ▾ ]  (AI Suggested · Click to change)   │
│ Tags:     [#backend] [#coreml] [#sprint-42]                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│ 👥 Speakers Detected (2):                                                   │
│   ✓ Abdullah   [94% Match · Enrolled Voiceprint]   (Speaks 54% of call)    │
│   ❓ Speaker 2  [▶ Play 3s sample]  Who is this? [ Sarah             ]      │
│                ☑️ Save voiceprint for future meetings                        │
├─────────────────────────────────────────────────────────────────────────────┤
│ 📌 AI Summary & Action Items:                                               │
│   • Sarah: Benchmark CoreML encoder on M4 Max                               │
│   • Abdullah: Finalize ONNX diarization pipeline                            │
├─────────────────────────────────────────────────────────────────────────────┤
│                   [ Discard ]      [ Confirm & Save to Catalog ]            │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Pre-defined Category Taxonomy
1. `🛠️ Engineering / Sprint` (Code reviews, architecture discussions, sprint planning)
2. `👥 1-on-1 Meeting` (Direct reports, manager check-ins, mentoring)
3. `💼 Client & Sales Call` (Customer demos, pitch meetings, discovery calls)
4. `🎨 Product & Design Review` (UI/UX specs, roadmap reviews, user research)
5. `💡 Brainstorm / Personal Notes` (Solo thinking out loud, ideation sessions)
6. `🎙️ Interview / Podcast` (Candidate hiring screens, external recordings)

### Structured LLM Output Format
```json
{
  "title": "Sprint Planning: CoreML ANE Integration",
  "category": "Engineering / Sprint",
  "tags": ["backend", "coreml", "sprint-42"],
  "summary": "Reviewed 85x real-time speedup on Apple Neural Engine. Assigned CoreML benchmarking on M4 Max to Sarah and ONNX diarization pipeline to Abdullah.",
  "decisions": [
    "Retain Parakeet for live dictation; add Qwen3-ASR for meeting transcription."
  ],
  "action_items": [
    { "owner": "Sarah", "task": "Benchmark CoreML encoder on M4 Max", "deadline": "Friday" },
    { "owner": "Abdullah", "task": "Finalize ONNX diarization pipeline", "deadline": "Monday" }
  ]
}
```

---

## [RANK 4] Searchable Meeting Catalog & Action Item Hub
* **Difficulty:** ⭐⭐⭐ (Moderate)
* **Estimated Effort:** ~1–2 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Overview
A dedicated **Meetings** view inside the main window that organizes all past recordings, transcripts, summaries, and action items with powerful local search and filtering.

### Key Capabilities
- **Category Filter Tabs**: One click to isolate `Engineering`, `1-on-1s`, or `Client Calls`.
- **Speaker Filtering**: Select a participant (e.g. `Sarah`) to view every meeting she attended, her total speaking time, and all action items assigned to her across history.
- **Full-Text & Audio-Linked Search**: Search across spoken words, summaries, or decisions. Clicking any search result jumps playback to the exact timestamp.
- **Export Formats**:
  - Markdown (`.md`) with timestamps, speaker names, and GitHub-style checklist action items.
  - Notion-compatible block structure.
  - JSON for developer automation.

---

## [RANK 5] Qwen3-ASR Engine Integration (Open ASR Leaderboard SOTA)
* **Difficulty:** ⭐⭐⭐⭐ (Medium–Hard)
* **Estimated Effort:** ~1–2 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Qwen3-ASR?
On the **Hugging Face Open ASR Leaderboard**, Alibaba’s **Qwen3-ASR (0.6B and 1.7B)** model family consistently ranks #1, outperforming Whisper Large-v3 and commercial speech APIs on:
- Spontaneous conversational speech and interruptions.
- Heavy accents, dialects, and technical terminology.
- Code-switching (seamlessly transitioning between English and other languages mid-sentence).

### Coexistence Architecture: Parakeet + Qwen3-ASR
Rather than replacing Parakeet, Taurscribe uses each model where it excels:

```
┌───────────────────────────────┬───────────────────────────────┐
│     PARAKEET NEMOTRON TDT     │           QWEN3-ASR           │
├───────────────────────────────┼───────────────────────────────┤
│ • Sub-80ms streaming latency  │ • Audio-Language Transformer  │
│ • Zero hallucination risk     │ • Understands full context    │
│ • Ultra-low CPU/GPU usage     │ • SOTA benchmark accuracy     │
│                               │                               │
│ 🎯 BEST FOR:                  │ 🎯 BEST FOR:                  │
│ Live system-wide voice typing │ Meeting recording & files     │
│ (Push-to-talk in any app)     │ (Multi-speaker conversations) │
└───────────────────────────────┴───────────────────────────────┘
```

### Technical Integration
- Weights available in ONNX and GGUF format.
- Executes via Taurscribe’s existing `ort` (ONNX Runtime) with CoreML (macOS), DirectML (Windows), and CUDA (NVIDIA), or `llama-cpp-2` for GGUF execution.
- Replaces or elevates the current Granite / Cohere engine slot.

---

## [RANK 6] Speaker Diarization with Isolated Voiceprint Enrollment & Memory
* **Difficulty:** ⭐⭐⭐⭐ (Medium–Hard)
* **Estimated Effort:** ~2–3 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Overview
Automatically detects "who spoke when" in meetings and audio recordings, assigns transcripts to individual speakers, extracts isolated audio snippets for unknown speakers, and remembers voiceprints so people are recognized automatically in future meetings.

### End-to-End Workflow

```
[ Recorded Audio ]
        │
        ├──► 1. Diarization Segmentation (pyannote-segmentation-3.0.onnx)
        │      └─ Finds speech turns & timestamps: [00:00 - 00:14], [00:15 - 00:32]
        │
        ├──► 2. Speaker Embedding Extraction (CAM++.onnx / 3D-Speaker)
        │      └─ Generates 192-dimensional vector per speech turn
        │
        ├──► 3. Cosine-Similarity Clustering
        │      └─ Partitions into Cluster A, Cluster B, Cluster C
        │
        ├──► 4. Voiceprint Vault Lookup (voiceprints.json)
        │      ├─ Cluster A matches "Abdullah" (similarity 0.89 >= 0.75) ──► AUTO-LABEL: "Abdullah"
        │      └─ Cluster B has no match (similarity < 0.75) ─────────────► FLAG: Unrecognized Speaker
        │
        ▼
[ Clean Non-Overlapping Snippet Extraction ]
  Finds a clean 3-5 second window of isolated speech for Cluster B with no cross-talk.
  Generates in-memory base64 WAV data URI.
        ▼
[ Post-Meeting Review Modal ]
  User clicks: [ ▶ Play 3s Sample ] -> Types: "Sarah" -> Checks: [x] Remember voiceprint
  Saves 192-dim embedding to voiceprints.json.
```

### Technical Specification
- **Models**:
  - `pyannote-segmentation-3.0.onnx` (~6 MB): Detects voice activity change points and speech boundaries.
  - `CAM++.onnx` or `3D-Speaker.onnx` (~25 MB): ResNet-based speaker embedding model generating normalized 192-dim float vectors.
- **Voiceprint Vault Schema** (`voiceprints.json` in local AppData):
  ```json
  {
    "version": 1,
    "voiceprints": [
      {
        "id": "vp_01j8m4x",
        "name": "Sarah Chen",
        "embedding": [0.042, -0.198, 0.841, "... 192 floats"],
        "enrolled_at": "2026-09-18T23:25:00Z",
        "sample_count": 3,
        "color": "#a78bfa"
      }
    ]
  }
  ```
- **Matching Math**: Cosine similarity $\cos(\theta) = \frac{\mathbf{u} \cdot \mathbf{v}}{\|\mathbf{u}\|_2 \|\mathbf{v}\|_2}$. Threshold $\tau = 0.75$ for high-confidence automatic naming; $0.65 \le \tau < 0.75$ prompts as *"Is this Sarah? (Likely match)"*.

---

## [RANK 7] Live Floating Capsule with Real-Time Audio Waveform
* **Difficulty:** ⭐⭐⭐⭐ (Hard)
* **Estimated Effort:** ~2–3 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Problem
The current overlay is functional but static. Modern users expect a sleek, unobtrusive floating pill (similar to macOS Dynamic Island or Raycast) that docks near the active text caret.

### Design & Behavior
- **Visuals**: Frosted glass / blurred backdrop capsule with glowing border matching the active engine color (Purple for Whisper, Blue for Parakeet, Teal for Qwen).
- **Audio Visualizer**: 60 FPS mini waveform / frequency bars driven by live RMS audio levels during recording.
- **Engine Badge**: Micro pill showing `[ANE]`, `[MLX]`, or `[CUDA]`.
- **Caret Tracking**: Docks 20px below the active cursor in the focused application.
- **Zero-Focus Steal**: Uses non-activating window flags (`canBecomeKeyWindow = NO` on macOS, `WS_EX_NOACTIVATE` on Windows) so keyboard focus is never lost.

---

## [RANK 8] Dual-Channel System Loopback & Mic Meeting Recorder
* **Difficulty:** ⭐⭐⭐⭐⭐ (Very Hard)
* **Estimated Effort:** ~3–4 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Problem
To record online meetings (Zoom, Google Meet, Microsoft Teams) without inviting an external bot, the app must capture both your voice (mic) and the other participants' voices (computer speakers).

### Native OS Capture Architecture
- **macOS**: Apple `ScreenCaptureKit` (`SCStream`) audio-only tap (macOS 13+, requires Screen Recording permission, zero kernel extensions).
- **Windows**: Windows Audio Session API (`WASAPI` with `AUDCLNT_STREAMFLAGS_LOOPBACK`) on the default render device.
- **Linux**: PipeWire monitor source (`pw_stream`) tapping the default audio sink.

### Audio Merging Pipeline
1. Capture Channel 1 (Mic, 16 kHz Mono).
2. Capture Channel 2 (System Audio, 48 kHz Stereo downsampled to 16 kHz Mono).
3. Software Acoustic Echo Cancellation (AEC) to prevent speaker audio from bleeding into the mic track.
4. Feeds the combined, synchronized audio stream into the Diarization and Transcription pipeline.

---

## [RANK 9] Voice Transformation Commands (Specialized Model Fine-Tuning)
* **Difficulty:** ⭐⭐⭐⭐⭐ (Hardest — High ML Overhead)
* **Status:** **DEFERRED** (Leave for later phase)
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Rationale for Deferral
Unlike standard zero-shot prompting on an off-the-shelf LLM, robust voice transformation commands (*"format as bullet points"*, *"summarize as executive email"*, *"remove hesitation and make formal"*) require:
1. **Dataset Curation & Pairing**: Compiling thousands of raw transcribed voice audio/text pairs paired with ideal formatted targets across diverse domains.
2. **Supervised Fine-Tuning (SFT / LoRA)**: Training a dedicated small language model (e.g. Qwen 2.5 0.5B / 1.5B) to reliably separate the *command prefix* from the *content to transform* with near-zero latency and high fidelity.
3. **Quantization & Edge Conversion**: Quantizing the fine-tuned checkpoint into 4-bit / 8-bit GGUF or CoreML formats for smooth local inference without ballooning user download size.

Because of the specialized training pipeline, data curation, and evaluation cycles required, this feature is placed at **Rank 9** and deferred until the core meeting intelligence and diarization infrastructure is fully deployed.

---

## Updated Implementation Phasing Roadmap

```
   COMPLEXITY
       ▲
   5   │                                      [#8 System Loopback]   [#9 Model Fine-Tuning (Deferred)]
       │
   4   │                         [#5 Qwen3-ASR]   [#6 Diarization & Vault]
       │                         [#7 Floating Capsule]
   3   │                         [#3 Auto-Categorization]
       │                         [#4 Meeting Catalog]
   2   │   [#2 Custom Dictionary]
       │
   1   │   [#1 ANE Downloader]
       └────────────────────────────────────────────────────────────────────────►
           PHASE 1 (Quick Wins)      PHASE 2 (Catalog & Speech)   PHASE 3 (Deep Systems & ML)
                                     DEVELOPMENT TIMELINE
```

### Phase 1: High-Polish Quick Wins (Days 1–2)
1. **#1 Whisper CoreML ANE Auto-Downloader**: 1-click companion bundle download; immediate 85x real-time inference.
2. **#2 Custom Vocabulary & Jargon Injection**: Biasing Whisper `initial_prompt` with user dictionary.

### Phase 2: Meeting Catalog & Next-Gen Speech (Days 3–7)
3. **#3 Automated Meeting Categorization**: Structured JSON classification with user confirmation modal.
4. **#4 Searchable Meeting Catalog**: Multi-filter catalog tab (by Category, Speaker, Date, and Action Item).
5. **#5 Qwen3-ASR Engine Integration**: Add the #1 Open ASR Leaderboard model alongside Parakeet.
6. **#6 Speaker Diarization with Isolated Voiceprint Enrollment**: ONNX CAM++ pipeline with 3s audio snippet player and `voiceprints.json`.

### Phase 3: Deep Systems & Advanced ML (Later Milestones)
7. **#7 Live Floating Capsule with Waveform**: Native non-activating dynamic HUD.
8. **#8 Dual-Channel System Loopback Recorder**: Full local bot-free meeting recording.
9. **#9 Voice Transformation Commands**: Fine-tuning specialized instruction model.
