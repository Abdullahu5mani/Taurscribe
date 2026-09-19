# Taurscribe: Strategic Implementation Roadmap
### Optimal Build Sequence (Architectural Dependencies & Cumulative User Value)

> **Vision**: Transform Taurscribe from a local speech-to-text dictation utility into the premier **100% offline, cross-platform AI Meeting Intelligence & Dictation Suite**—combining sub-80ms streaming voice typing with speaker-diarized meeting cataloging and local LLM intelligence, with zero cloud dependency and zero subscription fees.

---

## Why This Sequence? (Architectural Dependency Graph)

Rather than building in order of raw difficulty, this roadmap is engineered so that **each step creates the foundation for the next**:

```
[ STEP 1: ANE Downloader ] ──► Unlocks 85x RT speed on Mac so all future transcription is instant
            │
[ STEP 2: Custom Jargon  ] ──► Ensures domain terms & personal names never fail in transcripts
            │
[ STEP 3: Meeting Catalog] ──► Builds the database & UI home where meetings live
            │
[ STEP 4: Auto-Categorize] ──► Hooks post-meeting LLM classification into that catalog
            │
[ STEP 5: Diarization    ] ──► Layers speaker separation, 3s audio snippets, & voiceprints on top
            │
[ STEP 6: Qwen3-ASR SOTA ] ──► Powers meetings with the #1 conversational model on Open ASR
            │
[ STEP 7: System Loopback] ──► Unlocks direct bot-free Zoom/Teams call recording into the pipeline
            │
[ STEP 8: Floating HUD   ] ──► Polishes daily voice typing with a Dynamic Island floating capsule
            │
[ STEP 9: Model Tuning   ] ──► (Deferred) Fine-tunes custom model for voice transform commands
```

---

## Master Implementation Sequence

| Step | Milestone | Strategic Rationale & Architectural Dependency | Effort |
|:---:|:---|:---|:---:|
| **1** | [1-Click Whisper CoreML ANE Auto-Downloader](#step-1-1-click-whisper-coreml-ane-auto-downloader) | **Immediate Performance Multiplier**: Runs Whisper at 85x real-time on Apple Silicon with 0 fan noise. Makes all future testing and meeting processing 2.2x faster. | ~1–2 hrs |
| **2** | [Custom Vocabulary & Jargon Injection](#step-2-custom-vocabulary--context-jargon-injection) | **Accuracy Multiplier**: Biases decoder logits toward technical terms, acronyms, and names. Eliminates misspellings before building meeting transcripts. | ~2–3 hrs |
| **3** | [Searchable Meeting Catalog Hub](#step-3-searchable-meeting-catalog-hub) | **Data & UI Foundation**: You cannot categorize or diarize meetings until there is a database store and a dedicated "Meetings" view in the UI to hold them. | ~1–2 days |
| **4** | [Automated Meeting Categorization + Confirmation](#step-4-automated-meeting-categorization--confirmation) | **First Meeting Intelligence Layer**: Hooks into the end of recordings to classify meetings (Engineering, 1-on-1, etc.), generate titles, and save into the Catalog. | ~1 day |
| **5** | [Speaker Diarization + Voiceprint Vault & Audio Snippets](#step-5-speaker-diarization-voiceprint-vault--audio-snippets) | **Speaker Intelligence**: Layers on top of meeting recording: separates speakers, extracts 3s isolated audio clips for user labeling, and remembers voiceprints. | ~2–3 days |
| **6** | [Qwen3-ASR Engine Integration (Open ASR SOTA)](#step-6-qwen3-asr-engine-integration-open-asr-leaderboard-sota) | **Conversational Speech Champion**: Integrates the #1 model from the Open ASR Leaderboard to handle overlapping, accented, and jargon-dense meetings. | ~1–2 days |
| **7** | [Dual-Channel System Loopback & Mic Recorder](#step-7-dual-channel-system-loopback--mic-recorder) | **Bot-Free Call Capture**: Feeds computer speaker audio (Zoom, Teams, Meet) directly into the now-complete diarization, categorization, and cataloging pipeline. | ~3–4 days |
| **8** | [Live Floating Capsule with Audio Waveform](#step-8-live-floating-capsule-with-audio-waveform) | **Daily UX Polish**: Replaces the static overlay with a sleek Dynamic Island-style floating pill tracking the active caret with a 60 FPS visualizer. | ~2–3 days |
| **9** | [Voice Transformation Commands (Fine-Tuning)](#step-9-voice-transformation-commands-specialized-model-fine-tuning) | **Advanced Post-Processing (Deferred)**: Curates dataset and fine-tunes a specialized instruction model for voice-directed editing (*"bullet this"*, *"make formal"*). | Deferred |

---

## STEP 1: 1-Click Whisper CoreML ANE Auto-Downloader
* **Strategic Role:** Performance Multiplier
* **Estimated Effort:** ~1–2 hours
* **Target Platforms:** macOS (Apple Silicon M-Series)

### Why Build This First?
Whisper is the foundation for meeting transcription in Taurscribe. Running Whisper at **38x real-time** on Metal GPU is fast, but running it at **85x real-time** (2.2x faster) on the **Apple Neural Engine (ANE)** means a 30-minute meeting transcribes in ~21 seconds with zero battery drain. Building this first accelerates every subsequent test and feature in development.

### Implementation Architecture
1. In [`model_registry.rs`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src-tauri/src/commands/model_registry.rs), link base Whisper models to their companion HuggingFace CoreML `.mlmodelc.zip` bundles.
2. When downloading a Whisper model on Apple Silicon, [`downloader.rs`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src-tauri/src/commands/downloader.rs) automatically pulls both binary and encoder archive.
3. Show an **"⚡ ANE Accelerated"** badge on the model row in [`ModelsTab.tsx`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src/components/settings/ModelsTab.tsx) and provide an instant `[⚡ Enable 85x ANE Speedup]` button if the companion is missing.

---

## STEP 2: Custom Vocabulary & Context Jargon Injection
* **Strategic Role:** Accuracy & Baseline Quality
* **Estimated Effort:** ~2–3 hours
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Second?
Before storing meetings or categorizing conversations, the transcript itself must be accurate. Generic speech models constantly misspell developer terms (`useCallback`, `tauri-plugin-store`, `x86_64`), company names, and personal names.

### Implementation Architecture
1. Users maintain their wordlist in the existing [`DictionaryTab.tsx`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src/components/settings/AboutTab.tsx).
2. Feed the custom word list into Whisper's `params.set_initial_prompt(...)` during initialization/transcription.
3. Automatically bias beam-search token logits toward those spellings, driving Word Error Rate (WER) on proprietary terms to near-zero.

---

## STEP 3: Searchable Meeting Catalog Hub
* **Strategic Role:** Storage & UI Backbone for Meetings
* **Estimated Effort:** ~1–2 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Third?
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

## STEP 4: Automated Meeting Categorization & Confirmation
* **Strategic Role:** First AI Intelligence Layer
* **Estimated Effort:** ~1 day
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Fourth?
Now that the Meeting Catalog view exists, we connect the end of a recording to our local LLM (Qwen 2.5 / FlowScribe). The user immediately experiences the magic of automated meeting summaries and categorization.

### Implementation Architecture
1. **Structured LLM Inference** in [`llm.rs`](file:///Volumes/ExternalSSD/Projects/Code%20Projects/Taurscribe/src-tauri/src/llm.rs):
   - Takes transcript, returns structured JSON: `title`, `category`, `tags`, `summary`, and `action_items`.
2. **Post-Meeting Confirmation Modal**:
   - Slides up when meeting recording finishes.
   - Editable Title input.
   - Category dropdown (pre-selected with the AI's choice).
   - `[Confirm & Save to Catalog]` button writes directly into the Step 3 Catalog store.

---

## STEP 5: Speaker Diarization, Voiceprint Vault & Audio Snippets
* **Strategic Role:** Flagship Differentiator (Who Spoke When)
* **Estimated Effort:** ~2–3 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Fifth?
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

## STEP 6: Qwen3-ASR Engine Integration (Open ASR SOTA)
* **Strategic Role:** Maximum Conversational Accuracy
* **Estimated Effort:** ~1–2 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Sixth?
Now that multi-speaker meeting intelligence is fully functional, we give users access to the **#1 model on the Hugging Face Open ASR Leaderboard**. While Parakeet remains the undisputed speed demon for live push-to-talk typing, Qwen3-ASR is unmatched for complex conversational meetings, dialectal speech, and technical jargon.

### Implementation Architecture
1. Load Qwen3-ASR ONNX weights via existing `ort` pipeline (or GGUF via `llama-cpp-2`).
2. Expose in Engine Picker:
   - **Parakeet**: Real-time typing (sub-80ms keyboard replacement).
   - **Whisper**: Multilingual standard with CoreML ANE offload.
   - **Qwen3-ASR**: SOTA conversational accuracy for long-form meetings.

---

## STEP 7: Dual-Channel System Loopback & Mic Recorder
* **Strategic Role:** The Complete Bot-Free Meeting Machine
* **Estimated Effort:** ~3–4 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Seventh?
Up to this point, meetings could be recorded via room mic or file import. By adding direct internal system audio capture, users can record **Zoom, Google Meet, Microsoft Teams, and podcasts directly from their headphones**:
- **Channel 1 (Mic)**: Captures your voice cleanly.
- **Channel 2 (System Audio)**: Captures remote participants via OS loopback with zero external bots.

### Implementation Architecture
- **macOS**: `ScreenCaptureKit` (`SCStream`) audio tap (macOS 13+).
- **Windows**: `WASAPI` Loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`).
- **Linux**: PipeWire monitor source (`pw_stream`).
- Streams feed directly into the Step 5 Diarization & Step 4 Categorization pipeline.

---

## STEP 8: Live Floating Capsule with Real-Time Audio Waveform
* **Strategic Role:** Daily Dictation UI/UX Polish
* **Estimated Effort:** ~2–3 days
* **Target Platforms:** All Platforms (macOS, Windows, Linux)

### Why Build This Eighth?
With the meeting intelligence pipeline complete, this step circles back to polish the **day-to-day push-to-talk dictation experience**:
- Upgrades the static overlay into an ultra-sleek, frosted-glass Dynamic Island pill.
- Tracks active text cursor/caret in whatever app you are typing into.
- Displays a 60 FPS real-time audio waveform visualizer without stealing keyboard focus.

---

## STEP 9: Voice Transformation Commands (Specialized Model Fine-Tuning)
* **Strategic Role:** Advanced Future Milestone (Deferred)
* **Estimated Effort:** Multi-Week Research & Training Cycle
* **Status:** **DEFERRED**

### Why Build This Last?
Unlike standard prompting, zero-latency voice transformation commands (*"bullet this"*, *"executive email format"*, *"clean up hesitation"*) require curated paired audio/text datasets and supervised fine-tuning (LoRA) of a custom local model checkpoint. This is saved for a later version milestone after the core product features are fully shipped.

---

## Summary Timeline by Milestone

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ PHASE 1: Immediate Engine Polish (Days 1–2)                                │
│   ✓ Step 1: 1-Click Whisper CoreML ANE Downloader (85x Real-Time)           │
│   ✓ Step 2: Custom Vocabulary & Context Jargon Injection                    │
├─────────────────────────────────────────────────────────────────────────────┤
│ PHASE 2: Meeting Catalog & Intelligence Architecture (Days 3–6)             │
│   ✓ Step 3: Searchable Meeting Catalog Hub & Storage                        │
│   ✓ Step 4: Automated Meeting Categorization & Confirmation Modal           │
├─────────────────────────────────────────────────────────────────────────────┤
│ PHASE 3: Flagship Speech & Diarization (Week 2)                             │
│   ✓ Step 5: Speaker Diarization + Voiceprint Vault & Audio Snippets         │
│   ✓ Step 6: Qwen3-ASR Engine Integration (Open ASR #1 SOTA)                 │
├─────────────────────────────────────────────────────────────────────────────┤
│ PHASE 4: Deep OS Audio & UI Polish (Week 3)                                 │
│   ✓ Step 7: Dual-Channel System Loopback & Mic Meeting Recorder             │
│   ✓ Step 8: Live Floating Capsule with Waveform Visualizer                  │
├─────────────────────────────────────────────────────────────────────────────┤
│ FUTURE MILESTONE: Specialized Model Training (Deferred)                     │
│   • Step 9: Voice Transformation Commands (Fine-Tuned Instruction Model)    │
└─────────────────────────────────────────────────────────────────────────────┘
```
