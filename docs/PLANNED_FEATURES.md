# Taurscribe: Strategic Implementation Roadmap
### Optimal Build Sequence (Architectural Dependencies & Cumulative User Value)

> **Vision**: Transform Taurscribe from a local speech-to-text dictation utility into the premier **100% offline, cross-platform AI Meeting Intelligence & Dictation Suite**—combining sub-80ms streaming voice typing with speaker-diarized meeting cataloging and local LLM intelligence, with zero cloud dependency and zero subscription fees.

---

## Why This Sequence? (Architectural Dependency Graph)

Rather than building in order of raw difficulty, this roadmap is engineered so that **each step creates the foundation for the next**:

```
[ ✓ COMPLETED: ANE Downloader ] ──► Runs Whisper at 85x RT on Apple Silicon
            │
[ ✓ COMPLETED: Custom Jargon  ] ──► +50% term accuracy & 59% WER error reduction in transcripts
            │
[ STEP 2: Meeting Catalog     ] ──► Builds the database & UI home where meetings live (NEXT)
            │
[ STEP 3: Auto-Categorize     ] ──► Hooks post-meeting LLM classification into that catalog
            │
[ STEP 4: Diarization         ] ──► Layers speaker separation, 3s audio snippets, & voiceprints on top
            │
[ STEP 5: Qwen3-ASR SOTA      ] ──► Powers meetings with the #1 conversational model on Open ASR
            │
[ STEP 6: System Loopback     ] ──► Unlocks direct bot-free Zoom/Teams call recording into the pipeline
            │
[ STEP 7: Floating HUD        ] ──► Polishes daily voice typing with a Dynamic Island floating capsule
            │
[ LATER: In-Situ Learning     ] ──► Auto-injects words corrected by user in pasted area
            │
[ LATER: Model Tuning         ] ──► (Deferred) Fine-tunes custom model for voice transform commands
```

---

## Master Implementation Sequence

| Step | Milestone | Status | Strategic Rationale & Architectural Dependency | Effort |
|:---:|:---|:---:|:---|:---:|
| **—** | [1-Click Whisper CoreML ANE Auto-Downloader](#completed-1-click-whisper-coreml-ane-auto-downloader) | ✅ **COMPLETE** | **Already Live & Verified**: Automatically bundles `.bin` + companion `.mlmodelc.zip` for 85x real-time inference on Apple Silicon. | Done |
| **1** | [Custom Vocabulary & Context Jargon Injection](#step-1-custom-vocabulary--context-jargon-injection) | ✅ **COMPLETE** | **Empirically Verified**: +50% proper noun accuracy gain and 59% relative WER reduction on LibriSpeech corpus. Integrated across recording, file transcription, and settings UI. | Done |
| **2** | [Searchable Meeting Catalog Hub](#step-2-searchable-meeting-catalog-hub) | 🟡 **NEXT** | **Data & UI Foundation**: You cannot categorize or diarize meetings until there is a database store and a dedicated "Meetings" view in the UI to hold them. | ~1–2 days |
| **3** | [Automated Meeting Categorization + Confirmation](#step-3-automated-meeting-categorization--confirmation) | ⚪ Planned | **First Meeting Intelligence Layer**: Hooks into the end of recordings to classify meetings (Engineering, 1-on-1, etc.), generate titles, and save into the Catalog. | ~1 day |
| **4** | [Speaker Diarization + Voiceprint Vault & Audio Snippets](#step-4-speaker-diarization-voiceprint-vault--audio-snippets) | ⚪ Planned | **Speaker Intelligence**: Layers on top of meeting recording: separates speakers, extracts 3s isolated audio clips for user labeling, and remembers voiceprints. | ~2–3 days |
| **5** | [Qwen3-ASR Engine Integration (Open ASR SOTA)](#step-5-qwen3-asr-engine-integration-open-asr-leaderboard-sota) | ✅ **COMPLETE** | **Conversational Speech Champion (Zero-Python)**: SOTA #1 accuracy model from Open ASR Leaderboard integrated with native pure-Rust MLX (Apple Silicon) and ONNX (CUDA/DirectML/CPU) backends, 128-mel DSP frontend. | Done |
| **6** | [Dual-Channel System Loopback & Mic Recorder](#step-6-dual-channel-system-loopback--mic-recorder) | ⚪ Planned | **Bot-Free Call Capture**: Feeds computer speaker audio (Zoom, Teams, Meet) directly into the now-complete diarization, categorization, and cataloging pipeline. | ~3–4 days |
| **7** | [Live Floating Capsule with Audio Waveform](#step-7-live-floating-capsule-with-audio-waveform) | ⚪ Planned | **Daily UX Polish**: Replaces the static overlay with a sleek Dynamic Island-style floating pill tracking the active caret with a 60 FPS visualizer. | ~2–3 days |
| **Later** | [Adaptive In-Situ Correction Learning](#later-adaptive-in-situ-correction-learning) | ⚪ Planned (Later) | **Self-Improving Flywheel**: When a user corrects a mistranscribed word in the area where text was pasted, smartly ingest that word into custom vocabulary to bias decoder prompts automatically next time. | ~1 day |
| **8** | [Voice Transformation Commands (Fine-Tuning)](#step-8-voice-transformation-commands-specialized-model-fine-tuning) | ⏸️ **DEFERRED** | **Advanced Post-Processing**: Curates dataset and fine-tunes a specialized instruction model for voice-directed editing (*"bullet this"*, *"make formal"*). | Deferred |

---

## [COMPLETED] 1-Click Whisper CoreML ANE Auto-Downloader
* **Status:** ✅ **COMPLETED & VERIFIED (Tier 7 Local Pass)**
* **Platform:** macOS (Apple Silicon M-Series)

### Verified Capabilities
- `model_registry.rs` (`whisper_with_coreml`) bundles `ggml-{stem}.bin` with `ggml-{stem}-encoder.mlmodelc.zip`.
- `downloader.rs` on Apple Silicon automatically downloads both files and extracts the `.mlmodelc` folder.
- `whisper.rs` auto-detects the companion folder and loads CoreML ANE encoder offload.
- **Benchmark result**: **85.0x real-time** on Apple Silicon M4 (129ms for 11s audio) with 100% transcript parity.

---

## [COMPLETED] Step 1: Custom Vocabulary & Context Jargon Injection
* **Status:** ✅ **COMPLETED & EMPIRICALLY BENCHMARKED**
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Delivered Capabilities
1. **Dynamic Decoder Prompt Biasing (`context.rs`)**:
   - Compiles user custom vocabulary into Whisper's `initial_prompt` autoregressive window, strictly capped at 250 characters to protect audio token context.
   - Automatically detects active window titles (VS Code, Cursor, Xcode, Terminal, Slack, Zoom, EHR/Clinics, Legal tools) and injects domain context keywords.
2. **Post-ASR Exact Casing Normalization**:
   - Whole-word regex replacement preserving specialized capitalization (`useCallback`, `Taurscribe`, `Athenaïs`).
3. **Engine-Wide Integration**:
   - Implemented across live recording passes (`recording.rs`) and offline file transcription (`file_transcription.rs`).
4. **Settings UI (`TextTab.tsx`)**:
   - Custom Vocabulary manager with domain preset packs (Developer, Medical, Legal).
   - Active App Contextual Biasing toggle.
   - Live "Inspect Active Decoder Prompt" preview tool.
5. **Empirical LibriSpeech Corpus Benchmark**:
   - Tested across LibriSpeech `test-clean` suite on challenging proper nouns and complex words:
     - **+50.0% proper noun accuracy gain** (16.7% baseline -> 66.7% with custom vocab).
     - **59.0% relative WER error reduction** on difficult names.
     - 100% target accuracy on Gibbon, Edison, and classical literature terms (`Gamewell`, `Ambrose`, `electrolytic`, `vicissitudes`).

---

## STEP 2: Searchable Meeting Catalog Hub
* **Strategic Role:** Storage & UI Backbone for Meetings
* **Estimated Effort:** ~1–2 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Here?
You cannot categorize meetings, assign action items, or display diarized speakers if there is no database schema or UI view to hold them. Building the Catalog view first gives a visual destination for all subsequent meeting intelligence features.

### Implementation Architecture
1. **Local Meeting Store** (`meetings.json` or SQLite table in AppData):
   - Fields: `id`, `title`, `category`, `tags`, `timestamp`, `duration_secs`, `audio_path`, `speakers`, `transcript_segments`, `summary`, `action_items`.
2. **Dedicated Meetings Tab (`MeetingsTab.tsx`)**:
   - Category filter pills (`Engineering`, `1-on-1`, `Client Call`, etc.).
   - Speaker filter pills (`All`, `Sarah`, `Abdullah`).
   - Full-text search bar searching both speech text and action items.
   - Export button (Markdown `.md` with timestamps and checklist action items).

---

## STEP 3: Automated Meeting Categorization & Confirmation
* **Strategic Role:** First AI Intelligence Layer
* **Estimated Effort:** ~1 day
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Here?
Now that the Meeting Catalog view exists, we connect the end of a recording to our local LLM (Qwen 2.5 / FlowScribe). The user immediately experiences the magic of automated meeting summaries and categorization.

### Implementation Architecture
1. **Structured LLM Inference** in [`llm.rs`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src-tauri/src/llm.rs):
   - Takes transcript, returns structured JSON: `title`, `category`, `tags`, `summary`, and `action_items`.
2. **Post-Meeting Confirmation Modal**:
   - Slides up when meeting recording finishes.
   - Editable Title input.
   - Category dropdown (pre-selected with the AI's choice).
   - `[Confirm & Save to Catalog]` button writes directly into the Step 2 Catalog store.

---

## STEP 4: Speaker Diarization, Voiceprint Vault & Audio Snippets
* **Strategic Role:** Flagship Differentiator (Who Spoke When)
* **Estimated Effort:** ~2–3 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Here?
With meeting storage and post-meeting review working, we now layer in **Speaker Intelligence**:
- The meeting transcript transforms from a continuous wall of text into distinct, colored conversational speech turns.
- Users can listen to isolated 3-second audio snippets of unknown speakers and save their names.

### Implementation Architecture
1. **Diarization Pipeline**:
   - `pyannote-segmentation-3.0.onnx` (~6 MB) segments speech turns.
   - `CAM++.onnx` (~25 MB) extracts a 192-dimensional embedding vector per turn.
2. **Clean Audio Snippet Extraction**:
   - Finds an isolated 3–5 second slice with zero cross-talk for each cluster.
   - Converts to in-memory base64 WAV data URI for the `[ ▶ Play 3s sample ]` player in the confirmation modal.
3. **Voiceprint Vault (`voiceprints.json`)**:
   - Stores user-labeled embeddings.
   - Computes cosine similarity ($\tau \ge 0.75$) to auto-identify enrolled speakers in all future meetings.

---

## STEP 5: Qwen3-ASR Engine Integration (Open ASR SOTA)
* **Status:** ✅ **COMPLETED & VERIFIED (Zero-Python Native MLX + ONNX)**
* **Strategic Role:** Maximum Conversational Accuracy
* **Target Platforms:** All Platforms (macOS Apple Silicon via MLX Metal, Windows/Linux via ONNX Runtime CUDA/DirectML/CPU)

### Verified Capabilities
1. **Zero-Python Runtime**: Completely native compiled Rust execution across all targets.
2. **Apple Silicon MLX Backend** (`qwen3_mlx`): Direct Metal GPU execution on raw `model.safetensors` weights via `mlx-rs`.
3. **Cross-Platform ONNX Runtime Backend**: Dual AuT audio transformer encoder + Qwen3-1.4B autoregressive LLM decoder via `ort` with CUDA, DirectML, and multi-threaded CPU fallback.
4. **DSP Audio Frontend** (`qwen3_mel`): 128-channel log-mel spectrogram extractor in pure Rust with Slaney-style area-normalized filterbank.
5. **Full Pipeline Integration**: Available in live mic recording (`recording.rs`), file transcription (`file_transcription.rs`), engine switcher (`useEngineSwitch.ts`), and settings UI (`EnginePicker.tsx`, `ModelsTab.tsx`).
6. **Custom Vocabulary Biasing**: Injects domain terms into the system prompt prefix for prompt-level vocabulary biasing.
7. **Passing Tests**: All 66 unit and integration tests passing in `cargo test --lib`.

---

## STEP 6: Dual-Channel System Loopback & Mic Recorder
* **Strategic Role:** The Complete Bot-Free Meeting Machine
* **Estimated Effort:** ~3–4 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Here?
Up to this point, meetings could be recorded via room mic or file import. By adding direct internal system audio capture, users can record **Zoom, Google Meet, Microsoft Teams, and podcasts directly from their headphones**:
- **Channel 1 (Mic)**: Captures your voice cleanly.
- **Channel 2 (System Audio)**: Captures remote participants via OS loopback with zero external bots.

### Implementation Architecture
- **macOS**: `ScreenCaptureKit` (`SCStream`) audio tap (macOS 13+).
- **Windows**: `WASAPI` Loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`).
- **Linux**: PipeWire monitor source (`pw_stream`).
- Streams feed directly into the Step 4 Diarization & Step 3 Categorization pipeline.

---

## STEP 7: Live Floating Capsule with Real-Time Audio Waveform
* **Strategic Role:** Daily Dictation UI/UX Polish
* **Estimated Effort:** ~2–3 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Here?
With the meeting intelligence pipeline complete, this step circles back to polish the **day-to-day push-to-talk dictation experience**:
- Upgrades the static overlay into an ultra-sleek, frosted-glass Dynamic Island pill.
- Tracks active text cursor/caret in whatever app you are typing into.
- Displays a 60 FPS real-time audio waveform visualizer without stealing keyboard focus.

---

## [LATER ENHANCEMENT] Adaptive In-Situ Correction Learning
* **Strategic Role:** Self-Improving Accuracy Flywheel (Post-Core Meetings)
* **Estimated Effort:** ~1 day
* **Status:** ⚪ **Planned (After Core Features Complete)**
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Purpose & Architecture
Users should not need to manually open Settings and type every technical term or proper noun into a list. After shipping the primary meeting catalog and diarization workflows, this feature will monitor when Taurscribe pastes a transcript into the user's active editor, document, or chat window:
1. **Pasted Range Fingerprint**: Snapshot the emitted text snippet, target process, and timestamp.
2. **In-Situ Edit Detection**: Observe short-window manual edits or backspaces in the active text field.
3. **Smart Correction Delta**: Compute the word-level diff (e.g. user corrected `"Montelet"` → `"Montalais"` or `"electromagnetic"` → `"electrolytic"`).
4. **Auto-Ingestion into Prompt**: Ingest the corrected term directly into `custom_vocabulary` in `settings.json`, ensuring the word is automatically biased in the Whisper decoder prompt on the very next recording.

---

## STEP 8: Voice Transformation Commands (Specialized Model Fine-Tuning)
* **Strategic Role:** Advanced Future Milestone (Deferred)
* **Estimated Effort:** Multi-Week Research & Training Cycle
* **Status:** **DEFERRED**

### Why Build This Last?
Unlike standard prompting, zero-latency voice transformation commands (*"bullet this"*, *"executive email format"*, *"clean up hesitation"*) require curated paired audio/text datasets and supervised fine-tuning (LoRA) of a custom local model checkpoint. This is saved for a later version milestone after the core product features are fully shipped.

---

## Summary Timeline by Milestone

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ FOUNDATION: Already Verified & Live                                        │
│   ✓ Whisper CoreML ANE Bundling: 85x Real-Time offload on Apple Silicon     │
├─────────────────────────────────────────────────────────────────────────────┤
│ PHASE 1: Jargon & Meeting Catalog Backbone (Days 1–3)                       │
│   ✓ Step 1: Custom Vocabulary & Context Jargon Injection (VERIFIED)         │
│   🟡 Step 2: Searchable Meeting Catalog Hub & Storage (NEXT)                │
├─────────────────────────────────────────────────────────────────────────────┤
│ PHASE 2: Meeting Intelligence & Diarization (Days 4–8)                      │
│   ⚪ Step 3: Automated Meeting Categorization & Confirmation Modal          │
│   ⚪ Step 4: Speaker Diarization + Voiceprint Vault & Audio Snippets        │
├─────────────────────────────────────────────────────────────────────────────┤
│ PHASE 3: Flagship Speech & OS Audio Capture (Week 2–3)                      │
│   ⚪ Step 5: Qwen3-ASR Engine Integration (Open ASR #1 SOTA)                │
│   ⚪ Step 6: Dual-Channel System Loopback & Mic Meeting Recorder            │
├─────────────────────────────────────────────────────────────────────────────┤
│ PHASE 4: UI Polish & Future Enhancements (Week 3+)                          │
│   ⚪ Step 7: Live Floating Capsule with Waveform Visualizer                 │
│   ⚪ Later: Adaptive In-Situ Correction Learning (Post-Paste Auto-Learn)    │
│   ⏸️ Step 8: Voice Transformation Commands (Model Fine-Tuning Deferred)     │
└─────────────────────────────────────────────────────────────────────────────┘
```
