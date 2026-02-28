# Taurscribe Architecture Guide for Beginners

> **Perfect for**: Complete beginners to programming, Rust newcomers, or anyone curious about how speech recognition works!  
> **Goal**: Understand how Taurscribe works through simple explanations, fun analogies, and visual diagrams.

---

## Table of Contents

1. [What is Taurscribe?](#what-is-taurscribe)
2. [The Big Picture](#the-big-picture)
3. [🖥️ Platform Support & Hardware Acceleration](#️-platform-support--hardware-acceleration)
4. [🎙️ Audio Processing: Whisper vs Parakeet](#-audio-processing-whisper-vs-parakeet)
5. [🔇 Voice Activity Detection (VAD)](#-voice-activity-detection-vad)
6. [🧠 LLM Integration: Grammar Correction](#-llm-integration-grammar-correction)
7. [📝 Spell Checking](#-spell-checking)
8. [📥 Model Downloads](#-model-downloads)
9. [Rust Basics You Need to Know](#rust-basics-you-need-to-know)
10. [Complete Flow: Start to Finish](#complete-flow-start-to-finish)
11. [📐 Module Architecture](#-module-architecture)
12. [🔬 Deep Dives: How the Tricky Code Actually Works](#-deep-dives-how-the-tricky-code-actually-works)
13. [File & Function Reference](#file--function-reference)
14. [Common Beginner Questions](#common-beginner-questions)
15. [⌨️ Text Insertion: How Transcribed Text Gets Into Your App](#️-text-insertion-how-transcribed-text-gets-into-your-app)
16. [🚀 First Launch & Setup Wizard](#-first-launch--setup-wizard)
17. [🏪 App State & Settings Persistence](#-app-state--settings-persistence)
18. [🪝 Frontend Hook Architecture](#-frontend-hook-architecture)
19. [🍎 CoreML Acceleration (Apple Silicon)](#-coreml-acceleration-apple-silicon)
20. [⌨️ Customizable Global Hotkey](#️-customizable-global-hotkey)

---

## What is Taurscribe?

### 🎬 Movie Theater Analogy

Imagine Taurscribe is like a **movie theater with live subtitles**:

```
┌─────────────────────────────────────────────────────────────────┐
│                    🎬 TAURSCRIBE THEATER                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  🎤 ACTOR (You speaking)                                        │
│      │                                                          │
│      │ Your voice travels through the air                      │
│      ▼                                                          │
│  🎧 SOUND ENGINEER (Microphone + Audio Processing)              │
│      │                                                          │
│      │ Captures and prepares the sound                         │
│      ▼                                                          │
│  ⚡ TRANSCRIBER #1 (Parakeet - Speed)                           │
│      │   "I write instantly but might miss details"            │
│      │                                                          │
│  🎯 TRANSCRIBER #2 (Whisper - Accuracy)                         │
│      │   "I wait 6 seconds but write perfectly"                │
│      ▼                                                          │
│  📝 EDITOR (LLM Grammar Correction)                             │
│      │   "I fix any grammar mistakes"                          │
│      ▼                                                          │
│  🔤 PROOFREADER (Spell Checker)                                 │
│      │   "I catch any spelling errors"                         │
│      ▼                                                          │
│  📺 SUBTITLE SCREEN (Frontend UI)                               │
│         "The audience sees the final text!"                     │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

Taurscribe is a **desktop application** that listens to your voice and magically turns it into text using artificial intelligence!

**Technology Stack** (in plain English):
- **Frontend**: React + TypeScript (the pretty buttons and screens you see)
- **Backend**: Rust + Tauri (the super-fast engine that does all the hard work)
- **AI Engines**: Two powerful brains to choose from:
  - 🧠 **Whisper AI** - Very accurate, great for all situations
  - ⚡ **Parakeet Nemotron** - Lightning fast, optimized for real-time streaming
- **Post-Processing**:
  - ✨ **LLM** - Grammar & style correction with fine-tuned Qwen 2.5 0.5B (GGUF)
  - 🔤 **Spell Check** - Catch any spelling mistakes with SymSpell

**Key Features**:
- ✅ Real-time transcription while you speak (see words appear as you talk!)
- ✅ High-quality final transcript when you stop
- ✅ GPU acceleration for blazing speed (uses your graphics card!)
- ✅ Two AI engines to choose from (Whisper or Parakeet)
- ✅ Multiple models for each engine (pick small & fast or large & accurate)
- ✅ Voice Activity Detection (automatically skips silence)
- ✅ Grammar & style correction with local fine-tuned LLM (CPU or GPU)
- ✅ Spell checking for final polish
- ✅ Model download manager (download models from within the app)
- ✅ Global hotkey (Ctrl+Win) works from any application

---

## The Big Picture

### 🏭 Factory Analogy

Think of Taurscribe as a **speech-to-text factory**:

```
═════════════════════════════════════════════════════════════════════
                    🏭 TAURSCRIBE FACTORY OVERVIEW
═════════════════════════════════════════════════════════════════════

  RAW MATERIAL                    PROCESSING                     OUTPUT
  ════════════                    ══════════                     ══════

  🎤 Your Voice           ┌─────────────────────────┐
      │                    │    FRONTEND (React)     │           📺 UI
      │                    │    App.tsx + hooks/     │           Display
      │                    │    =====================│              ▲
      │                    │    • Recording buttons  │              │
      │                    │    • Model selection    │              │
      │                    │    • Settings modal     │              │
      │                    │    • Transcription view │              │
      │                    │    (logic split into 5  │              │
      │                    │     custom hooks)       │              │
      │                    └────────────┬────────────┘              │
     │                                 │                           │
     │                    Tauri IPC Bridge (JavaScript ↔ Rust)     │
     │                                 │                           │
     ▼                    ┌────────────▼────────────┐              │
  ════════════            │     BACKEND (Rust)      │              │
  │ Microphone │─────────►│     lib.rs - 132 lines  │──────────────┘
  ════════════            │     =====================│
                          │     Entry point, setup   │
                          └────────────┬────────────┘
                                       │
              ┌────────────────────────┼────────────────────────┐
              │                        │                        │
              ▼                        ▼                        ▼
     ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
     │  whisper.rs     │    │  parakeet.rs    │    │  vad.rs         │
     │  (Whisper AI)   │    │  (Parakeet AI)  │    │  (Silence Det.) │
     │  ~630 lines     │    │  ~339 lines     │    │  ~162 lines     │
     └─────────────────┘    └─────────────────┘    └─────────────────┘
               │                        │
               │              ┌─────────────────┐
               │              │parakeet_loaders │
               │              │(GPU/CPU loaders)│
               │              │  ~300 lines     │
               │              └─────────────────┘
              │                        │                        │
              └────────────────────────┼────────────────────────┘
                                       │
                                       ▼
                          ┌─────────────────────────┐
                          │    POST-PROCESSING      │
                          │    llm.rs (Grammar)     │
                          │    spellcheck.rs        │
                          └─────────────────────────┘

═════════════════════════════════════════════════════════════════════
```

### 🔄 Simple Data Flow

```
🎤 Your Voice
    │
    ├──► Microphone captures sound waves (48kHz stereo)
    │
    ├──► Converts to numbers (audio samples: -1.0 to 1.0)
    │
    ├──► Resamples to 16kHz mono (AI requirement)
    │
    ├──► Split into two streams:
    │
    ├──► Stream 1 → 💾 Save to disk (WAV file)
    │
    └──► Stream 2 → 🤖 AI transcription → 📝 Text
                            │
                            ▼
                    ✨ Grammar Correction (LLM)
                            │
                            ▼
                    🔤 Spell Check
                            │
                            ▼
                    📺 Display to User
```

### ⚠️ Gotcha: Why Two Audio Streams?

**Common Mistake**: Beginners often ask "Why not just use one stream?"

**Answer**: The WAV file is saved in **original quality** (48kHz stereo) while the AI needs **processed audio** (16kHz mono). If we only kept the processed version, we'd lose quality. By saving the original, you can:
- Re-transcribe with different settings later
- Share the original recording
- Use it for other purposes

---

## 🖥️ Platform Support & Hardware Acceleration

### 🚗 Car Engine Analogy

Think of hardware acceleration like **different car engines**:

```
┌─────────────────────────────────────────────────────────────────────┐
│                    🚗 ACCELERATION COMPARISON                        │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ⚡ CUDA (NVIDIA GPU)     = Tesla Electric (0-60 in 2s)             │
│     Fastest when available, requires NVIDIA                         │
│                                                                      │
│  🌋 Vulkan (Any GPU)      = Sports Car (0-60 in 4s)                 │
│     Good speed, works with AMD/Intel too                            │
│                                                                      │
│  🪟 DirectML (Windows)    = Modern Sedan (0-60 in 5s)               │
│     Windows universal, works with NPUs                              │
│                                                                      │
│  🍎 CoreML (Apple)        = BMW Electric (0-60 in 3s)               │
│     Mac-optimized, uses Neural Engine                               │
│                                                                      │
│  💨 XNNPACK (CPU)         = Economy Car (0-60 in 8s)                │
│     Works everywhere, uses SIMD                                      │
│                                                                      │
│  🐢 Pure CPU              = Bicycle (0-60 in... eventually)         │
│     Always available as fallback                                     │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### 📊 Platform Matrix

| Platform | Whisper Acceleration | Parakeet Acceleration | Best Use Case |
|----------|---------------------|----------------------|---------------|
| **Windows x64 + NVIDIA** | CUDA + Vulkan | CUDA + TensorRT | ⭐⭐⭐⭐⭐ Gaming PCs |
| **Windows x64 + AMD** | Vulkan | DirectML | ⭐⭐⭐⭐ AMD systems |
| **Windows ARM64** | CPU | DirectML (NPU) | ⭐⭐⭐⭐ Snapdragon laptops |
| **macOS Apple Silicon** | Metal | CoreML | ⭐⭐⭐⭐⭐ MacBook M1/M2/M3 |
| **macOS Intel** | CPU | XNNPACK | ⭐⭐⭐ Older MacBooks |
| **Linux x64 + NVIDIA** | CUDA + Vulkan | CUDA + TensorRT | ⭐⭐⭐⭐⭐ Linux workstations |
| **Linux ARM64** | CPU | XNNPACK | ⭐⭐⭐ Raspberry Pi |

### 🔍 How GPU Detection Works

```
┌───────────────────────────────────────────────────────────────┐
│                 GPU DETECTION FLOW (whisper.rs)               │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │  Try GPU Mode   │
                    │  use_gpu(true)  │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │  Load Model     │
                    │  with GPU       │
                    └────────┬────────┘
                             │
              ┌──────────────┴──────────────┐
              │                             │
         Success?                      Failure?
              │                             │
              ▼                             ▼
    ┌─────────────────┐           ┌─────────────────┐
    │ Run nvidia-smi  │           │ Try CPU Mode    │
    │ command         │           │ use_gpu(false)  │
    └────────┬────────┘           └────────┬────────┘
             │                             │
    ┌────────▼────────┐                    ▼
    │ Command exists? │           Return CPU Backend
    └────────┬────────┘
             │
     ┌───────┴───────┐
     │               │
    Yes             No
     │               │
     ▼               ▼
   CUDA          Vulkan
```

### ⚠️ Gotcha: CUDA Requires nvidia-smi

**Common Mistake**: "I have an NVIDIA GPU but it's using Vulkan!"

**Solution**: Make sure NVIDIA drivers are properly installed. The detection runs:
```rust
std::process::Command::new("nvidia-smi").output()
```
If this fails, Taurscribe assumes Vulkan is available instead.

---

## 🎙️ Audio Processing: Whisper vs Parakeet

### 🍕 Pizza Delivery Analogy

```
┌─────────────────────────────────────────────────────────────────────┐
│                    🍕 AUDIO PROCESSING COMPARISON                    │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  🧠 WHISPER AI = Traditional Pizza Delivery                         │
│                                                                      │
│     • Waits for full order (6 seconds of audio)                     │
│     • Checks if pizza is worth delivering (VAD check)               │
│     • Delivers high-quality pizza (accurate transcription)          │
│     • Latency: 6+ seconds                                           │
│                                                                      │
│  ⚡ PARAKEET = Speed Delivery Service                                │
│                                                                      │
│     • Delivers slices as they're ready (0.56s chunks)               │
│     • No quality check (skips VAD for speed)                        │
│     • Words appear almost instantly                                  │
│     • Latency: ~0.6 seconds                                          │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### 📊 Technical Comparison

| Feature | Whisper AI | Parakeet Nemotron |
|---------|-----------|------------------|
| **Chunk Size** | 6.0 seconds (96,000 samples) | 0.56 seconds (8,960 samples) |
| **Latency** | ~6.15 seconds | ~0.635 seconds |
| **VAD** | ✅ Yes (energy-based) | ❌ No (speed priority) |
| **Context** | Manual (we provide previous text) | Automatic (built-in state via `m.reset()`) |
| **GPU Support** | CUDA, Vulkan, CPU | CUDA, DirectML, CPU |
| **Model Format** | GGML (.bin files) | ONNX (.onnx files) |
| **Accuracy** | Excellent (95-98%) | Very Good (92-96%) |
| **Best For** | Meetings, lectures | Live streaming, gaming |

### 🔄 Whisper Processing Pipeline

```
═══════════════════════════════════════════════════════════════════════
                        🎤 WHISPER PIPELINE
═══════════════════════════════════════════════════════════════════════

STEP 1: 🎤 MICROPHONE CAPTURE
┌──────────────────────────────────────────────────────────────────────┐
│ Raw Audio: 48,000 samples/second, Stereo, Float32                    │
│ Example: [0.01, -0.02, 0.03, -0.01, 0.04, ...]                      │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
STEP 2: 🎛️ CONVERT TO MONO
┌──────────────────────────────────────────────────────────────────────┐
│ Stereo [L1, R1, L2, R2] → Mono [(L1+R1)/2, (L2+R2)/2]              │
│ Why? AI models expect single-channel audio                          │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
STEP 3: 🔄 RESAMPLE (48kHz → 16kHz)
┌──────────────────────────────────────────────────────────────────────┐
│ Uses `rubato` SincFixedIn resampler (high quality)                   │
│ Why? Whisper was trained on 16kHz audio                             │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
STEP 4: 📦 BUFFER INTO 6-SECOND CHUNKS
┌──────────────────────────────────────────────────────────────────────┐
│ Accumulate until: buffer.len() >= 96,000 samples                    │
│ Then: Extract chunk, continue buffering                              │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
STEP 5: 🔇 VAD CHECK (Voice Activity Detection)
┌──────────────────────────────────────────────────────────────────────┐
│ Calculate RMS (Root Mean Square) "loudness"                         │
│ If RMS < 0.005 → Skip (silence)                                     │
│ If RMS > 0.005 → Process (speech detected)                          │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
STEP 6: 🧠 WHISPER AI TRANSCRIPTION
┌──────────────────────────────────────────────────────────────────────┐
│ model.forward(audio_chunk) → "Hello world"                          │
│ Processing time: ~150ms on GPU (40x realtime!)                      │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
STEP 7: 💾 CUMULATIVE CONTEXT
┌──────────────────────────────────────────────────────────────────────┐
│ Save transcript for next chunk (last_transcript field)               │
│ Helps AI understand: "He said" → who is "he"?                       │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
STEP 8: 📤 SEND TO UI
┌──────────────────────────────────────────────────────────────────────┐
│ emit("transcription-chunk", { text, method: "Whisper" })            │
└──────────────────────────────────────────────────────────────────────┘

═══════════════════════════════════════════════════════════════════════
```

### ⚠️ Gotcha: Why 6-Second Chunks?

**Common Mistake**: "Why not 1-second chunks for faster updates?"

**Answer**: 
- Too short (1-2s) → Cuts words mid-sentence → AI "hallucinates" (makes up text)
- Too long (30s+) → High latency → Feels slow
- **6 seconds** → Sweet spot: complete sentences + reasonable latency

---

## 🔇 Voice Activity Detection (VAD)

### 🚥 Traffic Light Analogy

```
┌─────────────────────────────────────────────────────────────────┐
│                    🚥 VAD = TRAFFIC LIGHT                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  AUDIO CHUNK ARRIVES                                            │
│         │                                                       │
│         ▼                                                       │
│    ┌─────────┐                                                  │
│    │   VAD   │                                                  │
│    │  Check  │                                                  │
│    └────┬────┘                                                  │
│         │                                                       │
│    ┌────┴────┐                                                  │
│    │         │                                                  │
│    ▼         ▼                                                  │
│  🟢 GREEN  🔴 RED                                               │
│  Speech!   Silence.                                             │
│    │         │                                                  │
│    ▼         ▼                                                  │
│  PROCESS   SKIP                                                 │
│  with AI   (save CPU)                                           │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 🧮 How VAD Works (Energy-Based)

```rust
// Simplified VAD logic from vad.rs
fn is_speech(audio: &[f32]) -> bool {
    // Calculate RMS (Root Mean Square) - a measure of "loudness"
    let sum_squares: f32 = audio.iter().map(|s| s * s).sum();
    let rms = (sum_squares / audio.len() as f32).sqrt();
    
    // Compare to threshold
    rms > 0.005  // Returns true if louder than threshold
}
```

### 📊 VAD Benefits

| Feature | Without VAD | With VAD | Benefit |
|---------|-------------|----------|---------| 
| **CPU Load** | Constant | Low during pauses | Cooler system |
| **Final Speed** | ~1000ms | ~550ms | **45% Faster** |
| **Accuracy** | May hallucinate | Clean silence | No phantom text |

### ⚠️ Gotcha: VAD Threshold

**Common Mistake**: "VAD keeps marking my speech as silence!"

**Solution**: The threshold (0.005) might be too high for quiet speakers. You can:
1. Increase microphone volume in system settings
2. Speak closer to the microphone
3. (Advanced) Adjust the threshold in `vad.rs`

---

## 🧠 LLM Integration: Grammar Correction

### 📝 Editor Analogy

```
┌─────────────────────────────────────────────────────────────────┐
│                    📝 LLM = PERSONAL EDITOR                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  INPUT (from transcription):                                    │
│  "the quick brown fox jump over the lazy dog"                   │
│                    │                                            │
│                    ▼                                            │
│  ┌──────────────────────────────────────────────┐               │
│  │     Qwen 2.5 0.5B Instruct (fine-tuned)      │               │
│  │         Q4_K_M GGUF quantized                │               │
│  │                                               │               │
│  │  System: "You are Wispr Flow, an AI that      │               │
│  │           transcribes and polishes speech.    │               │
│  │           Style: Professional"               │               │
│  │                                               │               │
│  │  User: "the quick brown fox jump over..."    │               │
│  │                                               │               │
│  │  Assistant: "The quick brown fox jumps       │               │
│  │              over the lazy dog."             │               │
│  └──────────────────────────────────────────────┘               │
│                    │                                            │
│                    ▼                                            │
│  OUTPUT:                                                        │
│  "The quick brown fox jumps over the lazy dog."                 │
│                                                                 │
│  ✅ Fixed: Capitalization, subject-verb agreement               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 📁 LLM Files Required

```
taurscribe-runtime/models/qwen_finetuned_gguf/
└── model_q4_k_m.gguf    ← Fine-tuned Qwen 2.5 0.5B weights (~400 MB)
```

> **Note**: The LLM path is resolved in `llm.rs → get_grammar_llm_dir()`:
> 1. **Hardcoded absolute path** — `GRAMMAR_LLM_PATH` const at the top of `llm.rs` (points to the developer's local machine path; update when deploying)
> 2. Falls back to `GRAMMAR_LLM_DIR` environment variable
> 3. Final fallback: `%LOCALAPPDATA%\Taurscribe\models\qwen_finetuned_gguf\`

### 🔄 LLM Processing Flow

```
Text Input │
           ▼
┌─────────────────────┐
│ Build ChatML Prompt │  ← "<|im_start|>system\nYou are Wispr Flow...<|im_end|>"
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Clear KV Cache      │  ← CRITICAL: Prevents "inconsistent sequence" errors
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Tokenize            │  ← "Hello wrold" → [token_ids...]
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Prefill Batch       │  ← Process all prompt tokens at once (fast)
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Decode Loop         │  ← Generate one token at a time
│ (Temperature 0.3)   │     Temp=0.3 means more deterministic output
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Stop When EOS token │  ← Stops when <|im_end|> or EOS token found
│ or max_tokens hit   │     max_tokens = (text.len() / 2) + 128 (dynamic)
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Decode to Text      │  ← token_ids → "The quick brown fox..."
└──────────┬──────────┘
           ▼
Corrected Text Output
```

### 🎨 Transcription Styles

The LLM supports 6 styles selectable from the **LLM & Grammar** settings tab:

| Style | What it does |
|-------|-------------|
| **Auto** | Default — clean and natural |
| **Casual** | Relaxed tone, contractions kept |
| **Verbatim** | Minimal changes, preserves original phrasing |
| **Enthusiastic** | Energetic tone, exclamation marks |
| **Software Dev** | Preserves technical terms, camelCase, CLI flags |
| **Professional** | Formal grammar, business-ready |

### ⚠️ Gotcha: LLM KV Cache Must Be Cleared

**Common Mistake**: "The LLM crashes after the second transcription!"

**Answer**: Each new request **must** call `ctx.clear_kv_cache_seq(None)` before filling the batch. Without this, llama.cpp throws a sequence inconsistency error and panics.

### ⚠️ Gotcha: LLM Backend Selection

The **Auto / GPU** option sets `n_gpu_layers = 99` (offloads all layers to GPU). If GPU loading fails, it automatically retries with `n_gpu_layers = 0` (CPU only). On macOS, GPU is always forced off regardless of selection.

---

## 📝 Spell Checking

### 🔤 Dictionary Analogy

```
┌─────────────────────────────────────────────────────────────────┐
│                    🔤 SPELL CHECK FLOW                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Input: "The quck brown fox"                                    │
│                │                                                │
│                ▼                                                │
│  ┌───────────────────────────────────────┐                      │
│  │  For each word:                       │                      │
│  │    "The"   → Found in dictionary ✓   │                      │
│  │    "quck"  → NOT FOUND! ❌            │                      │
│  │    "brown" → Found in dictionary ✓   │                      │
│  │    "fox"   → Found in dictionary ✓   │                      │
│  └───────────────────────────────────────┘                      │
│                │                                                │
│                ▼                                                │
│  ┌───────────────────────────────────────┐                      │
│  │  Find similar words to "quck":        │                      │
│  │    "quick" (edit distance: 1) ← BEST  │                      │
│  │    "duck"  (edit distance: 1)         │                      │
│  │    "quack" (edit distance: 2)         │                      │
│  └───────────────────────────────────────┘                      │
│                │                                                │
│                ▼                                                │
│  Output: "The quick brown fox"                                  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 📁 Spell Check Implementation

**File**: `src-tauri/src/spellcheck.rs`  
**Commands**: `src-tauri/src/commands/spellcheck.rs`

The spell checker uses **SymSpell** (frequency-based edit distance):
1. Loads a frequency dictionary (`frequency_dictionary_en_82_765.txt`)
2. Splits input text into words
3. For each unknown word, finds closest matches by edit distance
4. Auto-corrects based on word frequency ranking

**Dictionary location**: `%LOCALAPPDATA%\Taurscribe\models\symspell\`

### ⚠️ Gotcha: Technical Terms

**Common Mistake**: "It keeps marking my technical terms as misspelled!"

**Solution**: Technical terms (like "ONNX", "CUDA", "API") may not be in the dictionary. SymSpell is conservative — it won't auto-correct a word if no close match exists.

---

## 📥 Model Downloads

### 📦 Package Delivery Analogy

```
┌─────────────────────────────────────────────────────────────────┐
│                    📦 MODEL DOWNLOAD SYSTEM                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1️⃣ USER REQUEST                                               │
│     Click "Download" in the Downloads tab                       │
│                │                                                │
│                ▼                                                │
│  2️⃣ FRONTEND → invoke("download_model", { modelId })           │
│     Looks up config in commands/model_registry.rs               │
│                │                                                │
│                ▼                                                │
│  3️⃣ DOWNLOAD MANAGER (commands/downloader.rs)                  │
│     • Fetches file(s) from Hugging Face CDN                     │
│     • Streams bytes to disk with progress                       │
│     • Emits "download-progress" events to UI                    │
│                │                                                │
│                ▼                                                │
│  4️⃣ VERIFICATION                                               │
│     • SHA-1 hash checked against model_registry.rs             │
│     • File deleted if hash mismatch (corrupted)                 │
│                │                                                │
│                ▼                                                │
│  5️⃣ COMPLETION                                                 │
│     • emit("download-progress", { status: "done" })             │
│     • Frontend refreshes model list                             │
│     • Model instantly available for use!                        │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 📁 Available Models (from `commands/model_registry.rs`)

| Model ID | Type | Files | Size |
|----------|------|-------|------|
| `whisper-tiny` | Whisper GGML | 1 `.bin` | ~75 MB |
| `whisper-tiny-q5_1` | Whisper GGML (quantized) | 1 `.bin` | ~30 MB |
| `whisper-base` | Whisper GGML | 1 `.bin` | ~142 MB |
| `whisper-base-en` | Whisper GGML | 1 `.bin` | ~142 MB |
| `whisper-small` | Whisper GGML | 1 `.bin` | ~466 MB |
| `whisper-small-en` | Whisper GGML | 1 `.bin` | ~466 MB |
| `whisper-medium` | Whisper GGML | 1 `.bin` | ~1.5 GB |
| `whisper-large-v3` | Whisper GGML | 1 `.bin` | ~2.9 GB |
| `whisper-large-v3-turbo` | Whisper GGML | 1 `.bin` | ~1.6 GB |
| `parakeet-nemotron` | Parakeet ONNX | 4 files | ~700 MB |
| `qwen2.5-0.5b-instruct` | GGUF | 1 `.gguf` | ~400 MB |
| `qwen2.5-0.5b-instruct-tokenizer` | Tokenizer JSON files | 4 files | ~2 MB |
| `qwen2.5-0.5b-safetensors` | SafeTensors (GPU) | multi-file | ~1 GB |
| `symspell-en-82k` | Dictionary | 1 `.txt` | ~6 MB |

### ⚠️ Gotcha: Download Verification

**Common Mistake**: "The model downloaded but won't load!"

**Answer**: The download might be corrupted. The downloader:
1. Checks SHA-1 hash after download (hash stored in `model_registry.rs`)
2. Deletes the file if hash doesn't match
3. You'll see an error toast if verification fails

Try re-downloading or check your internet connection.

---

## Rust Basics You Need to Know

### 🧩 Ownership Puzzle Analogy

```
┌─────────────────────────────────────────────────────────────────┐
│                    🧩 RUST OWNERSHIP RULES                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Rule 1: Each value has ONE owner                               │
│  ─────────────────────────────────                               │
│  let s1 = String::from("hello");                                │
│  let s2 = s1;  // s1 is MOVED to s2                             │
│  // println!("{}", s1);  ← ERROR! s1 no longer valid           │
│                                                                 │
│  Rule 2: When owner goes out of scope, value is dropped         │
│  ──────────────────────────────────────────────────────          │
│  {                                                               │
│      let s = String::from("hello");                             │
│      // s is valid here                                          │
│  }  // s is dropped here (memory freed)                         │
│                                                                 │
│  Rule 3: You can BORROW with references                         │
│  ──────────────────────────────────────                          │
│  fn print_length(s: &String) {  // Borrows, doesn't own        │
│      println!("{}", s.len());                                    │
│  }                                                               │
│  let s = String::from("hello");                                 │
│  print_length(&s);  // Borrow                                    │
│  println!("{}", s);  // Still valid! ✓                          │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 📝 Quick Reference Table

| Concept | Syntax | Example |
|---------|--------|---------| 
| Variable | `let x = 5;` | `let name = "Rust";` |
| Mutable | `let mut x = 5;` | `let mut counter = 0;` |
| Reference | `&x` | `let ref = &value;` |
| Mutable ref | `&mut x` | `let mut_ref = &mut value;` |
| Option | `Option<T>` | `Some(5)` or `None` |
| Result | `Result<T, E>` | `Ok(5)` or `Err("error")` |
| Match | `match x { ... }` | Pattern matching |
| If let | `if let Some(x) = opt { }` | Pattern matching shortcut |
| Unwrap | `x.unwrap()` | Get value or panic |
| Question mark | `x?` | Propagate error |

### 🔒 How Shared State Works (`Arc<Mutex<T>>`)

Taurscribe shares engines (Whisper, Parakeet, LLM) across threads safely:

```rust
// In state.rs — wrapping the WhisperManager for thread-safe sharing
pub whisper: Arc<Mutex<WhisperManager>>,
//           ^^^  ^^^^^
//           │     └── Mutual Exclusion: only one thread at a time
//           └── Atomic Reference Count: multiple owners across threads

// In commands/recording.rs — using it from a background thread
let whisper = Arc::clone(&state.whisper);
std::thread::spawn(move || {
    let mut w = whisper.lock().unwrap(); // Lock, then use
    w.transcribe_chunk(&audio)?;
});
```

### ⚠️ Gotcha: `unwrap()` is Dangerous!

**Common Mistake**: Using `unwrap()` everywhere

**Problem**: `unwrap()` panics if the value is `None` or `Err`, crashing your app!

**Solution**: Use safer alternatives:
```rust
// ❌ Bad
let value = maybe.unwrap();  // Crashes if None!

// ✅ Good - provide default
let value = maybe.unwrap_or(0);

// ✅ Good - handle both cases
if let Some(v) = maybe {
    println!("Got: {}", v);
}

// ✅ Good - propagate error
let value = maybe.ok_or("No value")?;
```

---

## Complete Flow: Start to Finish

### 📱 Phase 1: User Clicks "Start Recording"

```
┌─────────────────────────────────────────────────────────────────┐
│  FRONTEND (useRecording.ts + App.tsx)                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  handleStartRecording() {                                        │
│      1. Check engine is loaded (Whisper or Parakeet)            │
│      2. invoke("start_recording")  →  Backend                   │
│      3. Set UI state to "Recording"                              │
│      4. Update tray icon via invoke("set_tray_state")           │
│  }                                                               │
│                                                                  │
│  BACKEND (commands/recording.rs)                                 │
│                                                                  │
│  pub fn start_recording(state: State<AudioState>) {              │
│      1. Clear engine context (last_transcript = "")             │
│      2. Open default microphone (cpal)                          │
│      3. Create WAV file writer (hound)                          │
│      4. Create channels: file_tx, transcriber_tx               │
│      5. Spawn writer_thread   → saves audio to disk             │
│      6. Spawn transcriber_thread → real-time AI inference       │
│      7. Start audio stream (calls callback every ~10ms)         │
│  }                                                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 🎤 Phase 2: Audio Capture (Every ~10ms)

```
┌─────────────────────────────────────────────────────────────────┐
│  AUDIO CALLBACK (runs on CPAL audio thread)                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  move |data: &[f32], _| {                                        │
│                                                                  │
│      // 1. Send raw stereo to file writer                        │
│      file_tx.send(data.to_vec()).ok();                           │
│                                                                  │
│      // 2. Convert stereo → mono                                 │
│      let mono = data.chunks(2)                                   │
│                     .map(|c| (c[0] + c[1]) / 2.0)              │
│                     .collect();                                  │
│                                                                  │
│      // 3. Send mono to transcription thread                     │
│      transcriber_tx.send(mono).ok();                             │
│  }                                                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 🧠 Phase 3: Transcription Thread Loop

```
┌─────────────────────────────────────────────────────────────────┐
│  TRANSCRIPTION THREAD (background thread)                        │
├─────────────────────────────────────────────────────────────────┤
│  loop {                                                          │
│      // 1. Receive mono audio samples from channel               │
│      let samples = rx.recv()?;  // Blocks until data arrives    │
│                                                                  │
│      // 2. Add to ring buffer                                    │
│      buffer.extend(samples);                                     │
│                                                                  │
│      // 3. Check if buffer is large enough                       │
│      if buffer.len() >= chunk_size {   // 96k for Whisper        │
│          let chunk = buffer.drain(..chunk_size).collect();       │
│                                                                  │
│          // 4. [Whisper only] Skip silence with VAD              │
│          if engine == Whisper && !vad.is_speech(&chunk) {        │
│              continue; // skip this chunk                        │
│          }                                                       │
│                                                                  │
│          // 5. Resample to 16kHz                                  │
│          let resampled = resample_to_16k(&chunk);               │
│                                                                  │
│          // 6. Transcribe with AI engine                         │
│          let text = engine.transcribe_chunk(&resampled)?;        │
│                                                                  │
│          // 7. Emit live result to frontend                       │
│          app.emit("transcription-chunk", &text);                 │
│      }                                                           │
│  }  // Loop ends when channel is dropped (recording stopped)     │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 🛑 Phase 4: Stop Recording + Post-Processing

```
┌─────────────────────────────────────────────────────────────────┐
│  STOP RECORDING (Frontend → Backend → Frontend)                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  FRONTEND (useRecording.ts):                                     │
│  1. invoke("stop_recording") → gets raw transcript back         │
│                                                                  │
│  BACKEND (commands/recording.rs):                                │
│  2. drop(file_tx)        → signals writer thread to finish      │
│  3. writer_thread.join() → waits for WAV file to finalize       │
│  4. [Whisper] Final pass on full WAV file (higher accuracy)     │
│  5. [Parakeet] Returns accumulated session transcript           │
│  6. clean_transcript()   → fixes spacing, punctuation          │
│  7. Returns final text to frontend                               │
│                                                                  │
│  FRONTEND post-processing pipeline:                              │
│  8.  [if spell check ON] → invoke("correct_spelling")           │
│  9.  [if grammar LLM ON] → invoke("correct_text", { style })    │
│  10. invoke("type_text") → Enigo types text into active window  │
│  11. Update UI transcript display                                │
│  12. Update tray icon back to "Ready"                            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### ⚠️ Gotcha: Channel Closing

**Common Mistake**: "The app hangs when I stop recording!"

**How channels work**:
1. `drop(file_tx)` closes the **sending end** of the channel
2. The writer thread's `rx.recv()` returns `Err` when the sender is gone
3. Thread exits its loop and finalizes the WAV file
4. **Without `drop()`**, the thread would block forever waiting for data!

---

## 📐 Module Architecture

### 🗂️ Current File Structure (Updated February 2026)

```
Taurscribe/
├── 🎨 Frontend
│   ├── src/
│   │   ├── App.tsx                   # UI assembly + event wiring (~440 lines)
│   │   ├── App.css                   # App-level styling
│   │   ├── main.tsx                  # React entry point
│   │   │
│   │   ├── hooks/                    # All logic lives here — App.tsx just assembles
│   │   │   ├── useHeaderStatus.ts    # Transient status ticker messages (~25 lines)
│   │   │   ├── useModels.ts          # Whisper + Parakeet model lists (~55 lines)
│   │   │   ├── usePostProcessing.ts  # LLM + SymSpell toggle/auto-load (~94 lines)
│   │   │   ├── useEngineSwitch.ts    # Engine switching + model loading (~193 lines)
│   │   │   └── useRecording.ts       # Record start/stop + post-processing (~197 lines)
│   │   │
│   │   └── components/
│   │       ├── SettingsModal.tsx      # Modal shell + tab router (~357 lines)
│   │       ├── SettingsModal.css      # Modal styling
│   │       └── settings/             # Settings tab sub-components
│   │           ├── GeneralTab.tsx     # Spell check toggle tab (~90 lines)
│   │           ├── DownloadsTab.tsx   # Model download list tab (~120 lines)
│   │           ├── ModelRow.tsx       # Single downloadable model row (~130 lines)
│   │           └── types.ts           # Shared types + MODELS constant (~125 lines)
│   │
│   └── index.html                    # HTML shell
│
├── 🦀 Backend (Rust)
│   └── src-tauri/
│       ├── src/
│       │   ├── 🎯 Core
│       │   │   ├── lib.rs              # App entry + module declarations (~132 lines)
│       │   │   ├── main.rs             # Binary entry point (6 lines)
│       │   │   ├── types.rs            # Shared enums: AppState, ASREngine (~30 lines)
│       │   │   ├── state.rs            # AudioState struct + new() (~68 lines)
│       │   │   ├── utils.rs            # get_models_dir(), get_recordings_dir(),
│       │   │   │                       # clean_transcript() (~64 lines)
│       │   │   └── audio.rs            # RecordingHandle struct (~24 lines)
│       │   │
│       │   ├── 🎤 Audio & ASR Engines
│       │   │   ├── whisper.rs          # WhisperManager: load, transcribe, resample
│       │   │   │                       # GPU detection (CUDA→Vulkan→CPU) (~630 lines)
│       │   │   ├── parakeet.rs         # ParakeetManager: Nemotron/CTC/EOU/TDT
│       │   │   │                       # transcription + model status (~339 lines)
│       │   │   ├── parakeet_loaders.rs # GPU/CPU loader helpers for each
│       │   │   │                       # Parakeet model type (~300 lines)
│       │   │   └── vad.rs              # Energy-based VAD: is_speech(),
│       │   │                           # get_speech_timestamps() (~162 lines)
│       │   │
│       │   ├── ✨ Post-Processing
│       │   │   ├── llm.rs              # LLMEngine: Qwen 2.5 0.5B GGUF via
│       │   │   │                       # llama-cpp-2, format_transcript() (~343 lines)
│       │   │   └── spellcheck.rs       # SymSpell spell checker (~150 lines)
│       │   │
│       │   ├── 📡 Commands (Tauri IPC)
│       │   │   └── commands/
│       │   │       ├── mod.rs              # Re-exports all pub commands
│       │   │       ├── recording.rs        # start_recording, stop_recording, type_text
│       │   │       ├── models.rs           # list_models, switch_model, init_parakeet,
│       │   │       │                       # set_active_engine, get_backend_info, etc.
│       │   │       ├── llm.rs              # init_llm, unload_llm, correct_text,
│       │   │       │                       # check_llm_status
│       │   │       ├── spellcheck.rs       # init_spellcheck, unload_spellcheck,
│       │   │       │                       # correct_spelling, check_spellcheck_status
│       │   │       ├── downloader.rs       # download_model, get_download_status,
│       │   │       │                       # delete_model, verify_model_hash
│       │   │       ├── model_registry.rs   # get_model_config(): all model URLs + SHA1s
│       │   │       ├── settings.rs         # set_tray_state
│       │   │       └── misc.rs             # greet (placeholder)
│       │   │
│       │   ├── 🖼️ System Tray
│       │   │   └── tray/
│       │   │       ├── mod.rs              # setup_tray() + icon switching
│       │   │       └── (icon assets)
│       │   │
│       │   ├── ⌨️ Global Hotkeys
│       │   │   └── hotkeys/
│       │   │       ├── mod.rs              # Re-exports start_hotkey_listener
│       │   │       └── listener.rs         # rdev Ctrl+Win listener (~75 lines)
│       │   │
│       │   └── 👁️ File Watcher
│       │       └── watcher.rs              # notify watcher on models dir,
│       │                                   # emits "models-changed" event (~60 lines)
│       │
│       ├── build.rs              # macOS deployment target, CUDA linker paths
│       └── Cargo.toml            # All Rust dependencies + feature flags
│
├── 📦 Runtime Assets
│   └── taurscribe-runtime/
│       └── models/
│           ├── qwen_finetuned_gguf/  # model_q4_k_m.gguf
│           └── parakeet-*/           # ONNX model folders (dev only)
│
├── assets/                       # App icons, tray icons (.png / .icns / .ico)
│
└── 📚 Documentation
    ├── ARCHITECTURE.md           # This file!
    └── README.md
```

### 🏗️ Module Dependency Diagram

```
┌─────────────────────────────────────────────────────────┐
│                lib.rs  (top level)                      │  ← Declares all modules
├─────────────────────────────────────────────────────────┤
│  commands/   tray/   hotkeys/   watcher                 │  ← Feature modules
├─────────────────────────────────────────────────────────┤
│  whisper   parakeet   vad   llm   spellcheck            │  ← AI engines
│                │                                        │
│         parakeet_loaders                                │  ← Loader helpers
├─────────────────────────────────────────────────────────┤
│  commands/model_registry   commands/downloader          │  ← Download subsystem
│  (registry has no deps)    (uses registry + utils)      │
├─────────────────────────────────────────────────────────┤
│  types   state   utils   audio                          │  ← Core (no dependencies)
└─────────────────────────────────────────────────────────┘

Rule: Lower modules NEVER depend on higher modules!

Frontend hook dependency order:
  useHeaderStatus   ←  (no deps)
  useModels         ←  useHeaderStatus
  usePostProcessing ←  useHeaderStatus
  useEngineSwitch   ←  useModels, useHeaderStatus
  useRecording      ←  useEngineSwitch, usePostProcessing, useHeaderStatus
  App.tsx           ←  all hooks
```

### ⚠️ Gotcha: Circular Dependencies

**Common Mistake**: "I added `use crate::commands` to `whisper.rs` and it won't compile!"

**Solution**: Lower-level modules (`whisper.rs`, `llm.rs`) must NEVER import from higher-level modules (`commands/`). Instead:
- Put shared types in `types.rs`
- Put utility functions in `utils.rs`
- Let the higher-level module (commands) import from the lower-level ones

---

## 🔬 Deep Dives: How the Tricky Code Actually Works

> These sections break down the most confusing or "magic-looking" parts of the codebase  
> into the simplest possible explanations. Each example is taken directly from the real code.

---

### 1️⃣ Channels — Threads Talking to Each Other

In `commands/recording.rs`, the code uses **channels** to send audio from the microphone thread to the transcription thread. Think of a channel exactly like a **walkie-talkie**:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    📻 HOW CHANNELS WORK                                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│   SENDER end                               RECEIVER end                     │
│   (tx = "transmitter")                     (rx = "receiver")               │
│                                                                              │
│   🎤 Audio Thread            channel pipe          🧠 Transcription Thread  │
│   ┌──────────────┐          ═══════════════         ┌──────────────────┐    │
│   │              │──────── [data] [data] ──────────►│                  │    │
│   │  tx.send()   │          (queue of data)         │    rx.recv()     │    │
│   │  audio data  │                                  │  waits here      │    │
│   └──────────────┘                                  └──────────────────┘    │
│                                                                              │
│   Key rules:                                                                 │
│   • tx.send(data) → puts data into the pipe (never blocks)                  │
│   • rx.recv()     → takes data OUT (BLOCKS until data arrives)              │
│   • drop(tx)      → closes pipe → rx.recv() returns Err → thread exits     │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Annotated real code from `commands/recording.rs`:**

```rust
// Step 1: Create TWO channels — one for file writing, one for transcription
let (file_tx, file_rx) = crossbeam_channel::unbounded::<Vec<f32>>();
//    ^^^^^^  ^^^^^^^                                   ^^^^^^^^^^
//    Sender  Receiver                                  Any size queue (no limit)

let (transcriber_tx, transcriber_rx) = crossbeam_channel::unbounded::<Vec<f32>>();

// Step 2: Spawn a background thread that owns the RECEIVER end
std::thread::spawn(move || {
//                 ^^^^
//                 "move" = this thread now OWNS transcriber_rx
    loop {
        match transcriber_rx.recv() {   // ← BLOCKS here, waiting for audio data
            Ok(samples) => { /* transcribe */ }
            Err(_) => break,             // ← tx was dropped = recording stopped
        }
    }
});

// Step 3: Audio callback runs on CPAL's thread (every ~10ms)
let callback = move |data: &[f32], _: &_| {
    file_tx.send(data.to_vec()).ok();         // → file writer thread
    transcriber_tx.send(data.to_vec()).ok();  // → transcription thread
    //                                  ^^^^
    //                   .ok() = ignore send error if receiver is gone
};

// Step 4: When recording stops, drop the sender → threads finish naturally
drop(file_tx);            // ← File writer thread sees Err and exits
drop(transcriber_tx);     // ← Transcription thread sees Err and exits
writer_thread.join().unwrap();  // ← Wait for both to finish cleanly
```

> **⚠️ Gotcha — Why `move` before the closure?**  
> Without `move`, the closure would borrow `transcriber_rx` by reference. But references can't cross thread boundaries in Rust (the original thread might die first). The `move` keyword transfers **ownership** into the new thread, making it safe.

---

### 2️⃣ `Arc<Mutex<T>>` — Sharing a Resource Between Threads

Taurscribe's AI engines (Whisper, Parakeet, LLM) live in `state.rs` and need to be accessed from *multiple* threads. Here's how that works:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│               🏦 Arc<Mutex<T>> = Thread-Safe Safe Deposit Box                │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│   Arc (Atomic Reference Count)                                                │
│   ───────────────────────────                                                 │
│   Imagine a "shared photocopy" of a key.                                      │
│   You can make as many copies as you need.                                    │
│   The box is destroyed only when ALL copies are gone.                         │
│                                                                               │
│   ┌─────────────────────────────────────────────────────────────────┐         │
│   │ Original Arc  ──── copy 1 (Thread A: recording command)         │         │
│   │               └─── copy 2 (Thread B: transcription thread)      │         │
│   │               └─── copy 3 (Thread C: stop command)              │         │
│   │                                                                  │         │
│   │  ref count: 3  →  box still alive                               │         │
│   └─────────────────────────────────────────────────────────────────┘         │
│                                                                               │
│   Mutex (Mutual Exclusion)                                                    │
│   ─────────────────────────                                                   │
│   Only ONE thread can look inside the box at a time.                          │
│   Others must wait outside until the door opens.                              │
│                                                                               │
│   Thread A ──► lock() ──► USES WhisperManager ──► DROP (auto-unlock)         │
│   Thread B ──► lock() ──► [WAITING...] ──────────────────────────────────►   │
│                           (waits until Thread A is done)                      │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Annotated real code from `state.rs` and `commands/recording.rs`:**

```rust
// In state.rs — declaring the shared state
pub struct AudioState {
    pub whisper: Arc<Mutex<WhisperManager>>,
    //           ^^^  ^^^^^
    //           │     └── "One thread at a time" lock
    //           └── "Multiple owners" reference-counted pointer

    pub parakeet: Arc<Mutex<ParakeetManager>>,
    pub llm:      Arc<Mutex<LLMEngine>>,
}

// In commands/recording.rs — using WhisperManager from a background thread
pub fn start_recording(state: tauri::State<AudioState>) {

    // Clone the Arc (cheap! just increments the reference count)
    let whisper_clone = Arc::clone(&state.whisper);
    //                             ^^^^^^^^^^^^^^^
    //                             borrow to clone — doesn't move the original

    std::thread::spawn(move || {   // Move the clone INTO the thread

        // Lock the mutex — we now have exclusive access
        let mut w = whisper_clone.lock().unwrap();
        //          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
        //          Returns MutexGuard<WhisperManager>
        //          Auto-unlocks when `w` goes out of scope (RAII)

        w.transcribe_chunk(&audio)?;  // Use WhisperManager safely

    }); // `w` drops here = mutex UNLOCKED = other threads can now use it
}
```

> **⚠️ Gotcha — Deadlock!**  
> What if Thread A locks `whisper`, then tries to lock `llm`, while Thread B has `llm` locked and waits for `whisper`? Both threads wait forever — **deadlock**!  
> **Rule**: Always lock mutexes in the same order everywhere in the code.

---

### 3️⃣ The Tauri IPC Bridge — JavaScript Calling Rust

This is the "magic" that lets the React UI call Rust functions:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                🌉 THE TAURI IPC BRIDGE                                        │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  FRONTEND (TypeScript / React)           BACKEND (Rust)                       │
│  ─────────────────────────────           ───────────────                      │
│                                                                               │
│  // 1. Call a Rust function             // 3. Rust function receives it       │
│  const text = await invoke(             #[tauri::command]                     │
│    "correct_text",          ═══════════►pub fn correct_text(                  │
│    { text: "hello wrold",                   text: String,                     │
│      style: "Professional" }                style: String,                    │
│  );                                         state: State<AudioState>          │
│  // 4. Rust return value flows back     ) -> Result<String, String> {         │
│  console.log(text);         ◄═══════════    // ... LLM correction ...         │
│  // "Hello world."                          Ok(corrected_text)                │
│                                         }                                     │
│                                                                               │
│  // 2. Event: Rust PUSHES to frontend   // 5. Rust emits an event             │
│  listen("transcription-chunk",          app_handle.emit(                      │
│    (event) => {             ◄═══════════    "transcription-chunk",            │
│      showText(event.payload)                &chunk_text                       │
│    }                                    );                                    │
│  );                                                                           │
│                                                                               │
│  invoke = SYNCHRONOUS request/response (you await the answer)                 │
│  listen = ASYNCHRONOUS subscription   (Rust fires whenever it wants)         │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

**How a command gets registered — `lib.rs`:**

```rust
// lib.rs — this is like a phone directory: "here are all the functions
//           the frontend is allowed to call"
tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
        //                   ^^^^^^^^^^^^^^^^^
        //                   Macro that wires up the IPC handler
        commands::start_recording,  // JS can call invoke("start_recording")
        commands::stop_recording,   // JS can call invoke("stop_recording")
        commands::correct_text,     // JS can call invoke("correct_text", {text, style})
        // ... etc for every command
    ])
```

> **⚠️ Gotcha — Naming matters!**  
> The string you pass to `invoke("start_recording")` in JavaScript must **exactly** match the Rust function name. A typo gives a silent runtime error, not a compile error.

---

### 4️⃣ `Result<T, E>` and the `?` Operator — Rust Error Handling

Rust has no exceptions. Instead, functions return `Result<Ok, Err>` — a box that contains *either* a success value or an error:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│               📦 Result<T, E> = A Box With Two Compartments                  │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│    Result<String, String>                                                     │
│    ┌─────────────────────────────────────────────┐                            │
│    │                                             │                            │
│    │   Compartment A: Ok(String)                 │                            │
│    │    ✅ "The corrected text result"            │                            │
│    │                                             │                            │
│    │   Compartment B: Err(String)                │                            │
│    │    ❌ "Model not loaded: file not found"    │                            │
│    │                                             │                            │
│    └─────────────────────────────────────────────┘                            │
│                                                                               │
│    The caller MUST check which compartment has data before using it.          │
│    Rust forces this — you literally cannot use the value without checking.    │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Three ways to handle a `Result`:**

```rust
// ─────────────────────────────────────────────────────────────
// WAY 1: match — explicit, handle both cases
// ─────────────────────────────────────────────────────────────
match load_model(path) {
    Ok(model)  => { /* use model */ }
    Err(e)     => { eprintln!("Failed: {}", e); }
}

// ─────────────────────────────────────────────────────────────
// WAY 2: ? operator — short-circuit on error (used EVERYWHERE)
// ─────────────────────────────────────────────────────────────
fn start_recording(state: State<AudioState>) -> Result<(), String> {
    let model = load_model(path)?;
    //                          ^
    //   If load_model returns Err(e), this function IMMEDIATELY
    //   returns Err(e) — no need to write the match manually.
    //   If load_model returns Ok(m), execution continues with m.

    let text = model.transcribe(&audio)?;  // Same pattern — bail on error
    Ok(())  // If we got here, everything worked!
}

// ─────────────────────────────────────────────────────────────
// WAY 3: unwrap_or_else — provide a default value
// ─────────────────────────────────────────────────────────────
let dir = get_models_dir().unwrap_or_else(|_| PathBuf::from("/tmp"));
//                                        ^
//                              If it fails, use /tmp as fallback
```

> **⚠️ Gotcha — `?` only works inside functions that return `Result`**  
> If you try `let x = something()?;` inside `main()` or a closure that returns `()`, the compiler will complain. Wrap the code in a helper function that returns `Result<_, _>` first.

---

### 5️⃣ Thread Lifetimes — What "Spawning" a Thread Actually Means

Here is a visual timeline of how threads start and stop during a recording session:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    🕐 THREAD LIFETIME DURING A RECORDING                     │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  Time ──────────────────────────────────────────────────────────────────►    │
│                                                                               │
│  Main Thread ══╦═══════════════════════════════════════════╦══════════════   │
│  (Tauri cmds)  ║ start_recording()                         ║ stop_recording() │
│                ║                                           ║                  │
│                ║ spawn ──────────────────────────── ►      ║                  │
│  Writer Thread ╠════════ writing WAV samples ═══════════►──╣ join() → done   │
│                ║  (owns file_rx)                           ║ → finalize WAV   │
│                ║                                           ║                  │
│                ║ spawn ──────────────────────────── ►      ║                  │
│  Transcriber   ╠════════ recv → AI → emit ═════════════►──╣ join() → done   │
│  Thread        ║  (owns transcriber_rx + AI engine)        ║                  │
│                ║                                           ║                  │
│  CPAL Thread   ╠══ audio callback (every ~10ms) ═════════►─╣ stream.stop()   │
│  (audio driver)║  [sends data via tx channels]             ║                  │
│                ║                                           ║                  │
│         ────────── user speaks ───────────────────────────────────────────   │
│                                                                               │
│  Legend:  ═══════ Thread alive and running                                    │
│           ──►     Thread spawned at this point                                │
│           ──╣     Thread receives stop signal (tx dropped)                   │
│           join()  = "wait here until that thread finishes"                    │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Why `join()` matters:**

```rust
// Without join(): WAV file might be half-written when we return!
writing_thread.join().unwrap();
//             ^^^^^^
//             Blocks (waits) until the writing thread finishes
//             finalizing the WAV file header. THEN we return.

// The WAV format requires the file SIZE in the header.
// The writer thread fixes the header LAST, right before it exits.
// Without join(), we'd return a corrupt WAV file.
```

---

### 6️⃣ The Audio Resampling Math

Whisper requires 16,000 samples per second. Your microphone records 48,000 samples per second. Here's what resampling actually does:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│               🎵 AUDIO RESAMPLING: 48kHz → 16kHz                             │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  Original (48kHz) — 48,000 numbers per second:                               │
│  [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.09, ...]               │
│    ^─────────────────^─────────────────^                                     │
│    position 0        position 2 (→ kept)  position 4 (→ kept)               │
│                                                                               │
│  After resampling (16kHz) — 16,000 numbers per second:                       │
│  [0.02,              0.05,             0.08, ...]                            │
│                                                                               │
│  The `rubato` library uses a sinc filter (math magic) to:                   │
│  • Keep every 3rd sample approximately                                        │
│  • Blend neighboring samples to avoid aliasing (audio distortion)            │
│  • Result: same audio, just 3× fewer numbers                                  │
│                                                                               │
│  Ratio: 16000 / 48000 = 1/3  →  output has 1/3 as many samples              │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

> **Why not just take every 3rd sample directly?**  
> That's called "downsampling without anti-aliasing" — it causes high-frequency audio artifacts (ugly distortion). The `rubato` sinc resampler applies a low-pass filter first to prevent this. It's like reducing a photo's resolution properly vs. just deleting every 3rd pixel.

---

### 7️⃣ The ChatML Prompt Format — How the LLM "Understands" Instructions

The LLM (Qwen 2.5) uses a special text format called **ChatML** to understand your instructions:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│           📝 CHATML FORMAT — The "Protocol" the LLM Speaks                   │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  <|im_start|>system                                                           │
│  You are Wispr Flow, an AI that transcribes and polishes speech.              │
│  Style: Professional. Fix grammar. Output ONLY the corrected text.            │
│  <|im_end|>                                                                   │
│  ─────────────────────────────────────────────────────────────────────────   │
│  ▲                                                                            │
│  │ "system" message — sets the AI's personality and rules.                   │
│    Think of it like a job description given before work starts.               │
│                                                                               │
│  <|im_start|>user                                                             │
│  the quick brown fox jump over the lazy dog                                   │
│  <|im_end|>                                                                   │
│  ─────────────────────────────────────────────────────────────────────────   │
│  ▲                                                                            │
│  │ "user" turn — this is the raw transcription text we want corrected.        │
│                                                                               │
│  <|im_start|>assistant                                                        │
│  ─────────────────────────────────────────────────────────────────────────   │
│  ▲                                                                            │
│  │ We leave this EMPTY — the model fills in the corrected text here.          │
│  │ It generates: "The quick brown fox jumps over the lazy dog."               │
│  │ Then stops when it produces <|im_end|>                                     │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Annotated code from `llm.rs`:**

```rust
fn build_chatml_prompt(text: &str, style: &str) -> String {
    format!(
        // The system message — tells the AI what personality to have
        "<|im_start|>system\n\
         You are Wispr Flow, an AI assistant that transcribes speech.\n\
         Style: {style}.\n\
         Output ONLY the corrected text. No explanations.\n\
         <|im_end|>\n\
         \
         <|im_start|>user\n\
         {text}\n\
         <|im_end|>\n\
         \
         <|im_start|>assistant\n",
         //                    ^
         // No closing <|im_end|> here — the model writes everything AFTER this
        style = style,
        text  = text,
    )
}

// During inference, stop generating when we see the end token:
if token == eos_token || decoded.contains("<|im_end|>") {
    break;   // LLM is done! Collect what we have.
}
```

---

### 8️⃣ Closures — Anonymous Functions ("functions without a name")

Closures appear **everywhere** in Taurscribe. They look confusing at first:

```rust
// A normal named function:
fn add_one(x: i32) -> i32 {
    x + 1
}

// A closure (same thing, but inline and anonymous):
let add_one = |x: i32| x + 1;
//            ^^      ^
//   Parameters      Body (no curly braces needed for one expression)

// Multi-line closure:
let process = |data: Vec<f32>| {
    let mono = convert_to_mono(&data);
    resample_to_16k(&mono)
};
```

**The audio callback is a closure capturing variables from the outer scope:**

```rust
// These variables are declared OUTSIDE the closure:
let file_tx        = /* channel sender */;
let transcriber_tx = /* channel sender */;

// The closure is passed to CPAL as the audio callback.
// It "captures" file_tx and transcriber_tx from the surrounding scope.
let callback = move |data: &[f32], _info: &cpal::InputCallbackInfo| {
    //         ^^^^
    //         Moves captured variables INTO the closure
    //         (transfers ownership — outer scope can no longer use them)

    file_tx.send(data.to_vec()).ok();
    //  ^^^^^^^
    //  file_tx was "moved in" above — the closure now owns it

    transcriber_tx.send(data.to_vec()).ok();
};
// CPAL calls this closure every ~10ms on its internal audio thread
let stream = device.build_input_stream(&config, callback, err_fn, None);
```

---

### 9️⃣ Iterators and Chaining — Reading the "Fluent" Style

Rust loves chaining iterator methods. Here's how to read them:

```rust
// Converting stereo [L, R, L, R, ...] to mono [(L+R)/2, ...]
let mono: Vec<f32> = stereo_samples
    .chunks(2)                      // Step 1: Group into pairs [L,R], [L,R], ...
    .map(|chunk| {                  // Step 2: For each pair...
        let left  = chunk[0];       //         get left channel sample
        let right = chunk.get(1).copied().unwrap_or(left); // right (or left if missing)
        (left + right) / 2.0        //         average them → one mono sample
    })
    .collect();                     // Step 3: Gather all results into Vec<f32>
//  ^^^^^^^^^
//  Iterators are LAZY — nothing runs until collect() is called!
```

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    🔗 ITERATOR CHAIN VISUALIZATION                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Input:  [0.1, 0.3, 0.2, 0.4, 0.5, 0.7]                                    │
│  (stereo: left=0.1, right=0.3, left=0.2, right=0.4, ...)                   │
│                                                                              │
│  .chunks(2)  ──►  [0.1, 0.3]   [0.2, 0.4]   [0.5, 0.7]                    │
│                        │            │             │                          │
│  .map(avg)   ──►      0.2          0.3           0.6                        │
│                        │            │             │                          │
│  .collect()  ──►  [0.2, 0.3, 0.6]   ← mono output!                         │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

### 🔟 `Option<T>` — When Something Might Not Exist

Many things in Taurscribe might not exist yet: the loaded model, an active recording, a found word. `Option<T>` represents "maybe a value, maybe nothing":

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    📦 Option<T> = A Box That Might Be Empty                  │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│   Some(value)    ← The box HAS something inside                              │
│   None           ← The box is EMPTY                                          │
│                                                                               │
│   Real example in state.rs:                                                   │
│   pub model: Option<WhisperModel>                                             │
│   //         ^^^^^^                                                           │
│   //         model might not be loaded yet!                                  │
│                                                                               │
│   WRONG — crashes if None:                                                    │
│   let m = state.model.unwrap();    // ❌ panics if no model loaded            │
│                                                                               │
│   RIGHT — check first:                                                        │
│   if let Some(m) = &state.model {  // ✅ safe                                 │
│       m.transcribe(&audio)?;                                                 │
│   } else {                                                                   │
│       return Err("No model loaded".into());                                  │
│   }                                                                          │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

### 1️⃣1️⃣ The VAD Math Explained Simply (RMS)

The VAD (Voice Activity Detection) uses a formula called **RMS (Root Mean Square)**. Here's what it means in plain English:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    📊 RMS FORMULA — STEP BY STEP                             │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  Input audio chunk: [0.01, -0.02, 0.03, -0.01]                              │
│                                                                               │
│  Step 1: SQUARE every sample (makes all numbers positive)                    │
│  [0.01²,   0.02²,   0.03²,   0.01²  ]                                       │
│  [0.0001,  0.0004,  0.0009,  0.0001 ]                                        │
│                                                                               │
│  Step 2: AVERAGE the squares  (sum / count)                                  │
│  (0.0001 + 0.0004 + 0.0009 + 0.0001) / 4  =  0.000375                      │
│                                                                               │
│  Step 3: SQUARE ROOT (undo the squaring from step 1)                         │
│  √0.000375  ≈  0.019                                                         │
│                                                                               │
│  Result:    RMS = 0.019                                                      │
│  Threshold: 0.005                                                            │
│  0.019 > 0.005  →  ✅ SPEECH DETECTED                                       │
│                                                                               │
│  Intuition: RMS = "average loudness" of the audio chunk                     │
│  • Loud speech  →  high RMS  (e.g., 0.05–0.2)                               │
│  • Quiet room   →  low RMS   (e.g., 0.001–0.003)                            │
│  • Threshold 0.005 = the dividing line between speech and silence            │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

### 1️⃣2️⃣ `unsafe impl Send` — Breaking (Safely) Through Rust's Thread Rules

**File**: `audio.rs` lines 8–9

```rust
pub struct SendStream(pub cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}
```

This is the most dangerous-looking code in the codebase. Here is what it actually means:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│            🚧 Send AND Sync — Rust's Thread-Safety Markers                   │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  Send  = "It is safe to MOVE this value to another thread"                   │
│  Sync  = "It is safe to SHARE a reference to this value across threads"      │
│                                                                               │
│  Most types get these automatically (e.g. String, Vec<f32>, i32).            │
│  Some types do NOT — because they contain raw OS handles or raw pointers     │
│  that are tied to one specific thread.                                        │
│                                                                               │
│  cpal::Stream is NOT Send by default:                                         │
│   • It wraps a raw Windows/macOS audio handle                                 │
│   • The OS audio API was created on the main thread                           │
│   • Rust refuses to let you move it to another thread — it might crash       │
│                                                                               │
│  SendStream wraps cpal::Stream and says "trust me, I know it's safe here":   │
│   • We never actually USE the stream from another thread                      │
│   • We just STORE it in RecordingHandle (which crosses the IPC boundary)     │
│   • We only play/stop it from the same thread it was created on               │
│                                                                               │
│  ANALOGY: Rust says "don't carry scissors while running".                    │
│  unsafe impl Send says "I am a trained safety professional — I've got this." │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

> **⚠️ Rule of Thumb**: If you see `unsafe impl Send`, ask:  
> "Is this safe because the type is never *actually* used from multiple threads simultaneously?"  
> If yes → it's a careful engineering decision, not a hack.  
> If no → it's a bug waiting to happen.

---

### 1️⃣3️⃣ `OnceLock` — A Global Variable That Can Only Be Written Once

**File**: `llm.rs` lines 21 & 76–78

```rust
static BACKEND: OnceLock<Arc<LlamaBackend>> = OnceLock::new();

// Later, when the LLM loads:
let backend = BACKEND.get_or_init(|| {
    Arc::new(LlamaBackend::init().expect("Failed to initialize llama backend"))
});
```

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    🔒 OnceLock — Write-Once Global Storage                   │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  static = Lives for the ENTIRE program lifetime (not inside any function)    │
│           Created before main() runs, destroyed after main() exits           │
│                                                                               │
│  Problem with regular statics in Rust:                                        │
│  static mut BACKEND: LlamaBackend = ...;   // ❌ Rust REFUSES this           │
│  Because: any thread could write at the same time as another reads → crash   │
│                                                                               │
│  OnceLock solves this:                                                        │
│  • Starts EMPTY                                                               │
│  • First call to get_or_init() → runs the closure, stores the value          │
│  • All later calls → just returns the already-stored value (no re-init)      │
│  • Thread-safe: if two threads race to initialize, only one wins             │
│                                                                               │
│  Timeline:                                                                    │
│  App starts      → BACKEND = (empty)                                          │
│  LLM loads (1st) → BACKEND = Arc<LlamaBackend>  ← closure runs ONCE         │
│  LLM loads (2nd) → BACKEND = Arc<LlamaBackend>  ← closure SKIPPED           │
│  LLM loads (3rd) → BACKEND = Arc<LlamaBackend>  ← closure SKIPPED           │
│                                                                               │
│  WHY? llama.cpp initializes GPU/CPU backends globally.                        │
│  Creating two backends simultaneously → crash. OnceLock prevents this.       │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

### 1️⃣4️⃣ `extern "C"` + `#[cfg(...)]` — Calling C Code & Conditional Compilation

**File**: `whisper.rs` lines 64–77

```rust
#[cfg(target_os = "macos")]
unsafe extern "C" fn null_log_callback(_level: u32, _text: *const c_char, _user_data: *mut c_void) {
    // suppress all whisper.cpp logs
}

#[cfg(target_os = "windows")]
unsafe extern "C" fn null_log_callback(_level: i32, _text: *const c_char, _user_data: *mut c_void) {
    // suppress all whisper.cpp logs
}
```

**Two completely different things are happening here:**

```
┌──────────────────────────────────────────────────────────────────────────────┐
│  THING 1: extern "C" — Bridging Rust ↔ C Code (FFI)                         │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  whisper.cpp is written in C++. Rust can call C/C++ code, but the two        │
│  languages must agree on HOW function calls work (the "calling convention").  │
│                                                                               │
│  extern "C" = "use the C calling convention for this function"               │
│                                                                               │
│  Calling Convention = a contract about:                                       │
│  • Which registers hold arguments?                                            │
│  • Who cleans up the stack after the call?                                    │
│  • How is the return value passed back?                                       │
│                                                                               │
│  Rust's default calling convention is different from C's.                    │
│  extern "C" makes the Rust function look exactly like a C function           │
│  so whisper.cpp can call it as a callback ("call this when you want to log") │
│                                                                               │
│  *const c_char = a C-style string pointer (NOT a Rust &str)                  │
│  *mut c_void  = a raw "anything" pointer (like void* in C)                   │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────────┐
│  THING 2: #[cfg(target_os = "...")] — Code That Doesn't Exist on Other OSes  │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  #[cfg(...)] is a COMPILE-TIME if statement.                                 │
│  The annotated code is physically removed from the binary on other platforms.│
│                                                                               │
│  On Windows binary:  only the i32 version exists in the compiled .exe        │
│  On macOS binary:    only the u32 version exists in the compiled app          │
│  On Linux binary:    only the u32 version exists in the compiled binary       │
│                                                                               │
│  WHY different types? The C header for ggml_log_callback uses:               │
│  • int    on Windows (MSVC uses signed int for log levels)                   │
│  • unsigned int on macOS/Linux (Apple/GCC headers use unsigned)              │
│  If you mismatch, the linker fails with a "type mismatch" error.             │
│                                                                               │
│  Common #[cfg] targets:                                                       │
│  #[cfg(target_os = "windows")]     → Windows only                            │
│  #[cfg(target_os = "macos")]       → macOS only                              │
│  #[cfg(target_arch = "x86_64")]    → 64-bit Intel/AMD only                   │
│  #[cfg(feature = "cuda")]          → Only when "cuda" feature flag is on     │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

### 1️⃣5️⃣ `handle.take()` + `drop()` Order — Controlled Teardown

**File**: `commands/recording.rs` lines 294–298

```rust
let mut handle = state.recording_handle.lock().unwrap();
if let Some(recording) = handle.take() {   // ← .take() is the key move
    drop(recording.stream);     // 1st: Stop the microphone
    drop(recording.file_tx);    // 2nd: Signal file writer thread to stop
    drop(recording.whisper_tx); // 3rd: Signal transcriber thread to stop
```

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    🎬 WHY .take() INSTEAD OF .as_ref()                       │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  state.recording_handle  =  Mutex<Option<RecordingHandle>>                   │
│                                                                               │
│  What .take() does on an Option<T>:                                          │
│  BEFORE: recording_handle = Some(RecordingHandle { ... })                    │
│  AFTER:  recording_handle = None       ← replaced with None atomically       │
│          returned value   = RecordingHandle { ... }  ← you get ownership    │
│                                                                               │
│  Why this matters:                                                            │
│  • If stop_recording() is called TWICE (e.g. hotkey + button), the second   │
│    call sees None → takes the else branch → returns "Not recording" safely  │
│  • Without .take(), both calls would try to stop the same stream → crash    │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────────┐
│                    ⚡ WHY DROP ORDER MATTERS                                  │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  drop(recording.stream)      ← Stop the microphone FIRST                     │
│       │                                                                       │
│       │  The microphone callback (which calls tx.send()) must stop           │
│       │  BEFORE we drop the senders, otherwise:                              │
│       │  • Callback tries to send on a closed channel                        │
│       │  • This returns an error (.ok() swallows it), but it's messy        │
│       │                                                                       │
│  drop(recording.file_tx)     ← Now close the file channel sender            │
│       │                                                                       │
│       │  file_rx.recv() in the writer thread now returns Err                 │
│       │  Writer thread exits its loop & calls writer.finalize()              │
│       │                                                                       │
│  drop(recording.whisper_tx)  ← Close the transcription channel sender       │
│                                                                               │
│       whisper_rx.recv() in transcriber thread now returns Err                │
│       Transcriber thread exits, processes remaining buffer, then stops       │
│                                                                               │
│  recording.writer_thread.join()  ← Wait for WAV file to be finalized        │
│  recording.transcriber_thread.join() ← Wait for final transcript            │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

> **⚠️ Gotcha**: Rust drops struct fields in **declaration order** at the end of a scope. If you rely on drop ordering, call `drop()` explicitly rather than waiting for scope to end — it makes the intent clear and prevents bugs when struct fields are reordered.

---

### 1️⃣6️⃣ `unsafe { std::mem::transmute(context) }` — Lifetime Erasure

**File**: `llm.rs` line 159  *(⚠️ Advanced — don't copy this pattern without understanding it)*

```rust
// The context has a lifetime tied to the model: LlamaContext<'model>
let context = unsafe { std::mem::transmute(context) };
// Now it's: LlamaContext<'static>  (pretends to live forever)
```

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    ⚠️ WHAT transmute DOES                                    │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  std::mem::transmute(x) = "reinterpret the raw bytes of x as a different type"│
│                                                                               │
│  It does NOT change any memory. It just tells the compiler to treat the      │
│  same bits as if they were a different type. Essentially lying to the         │
│  borrow checker.                                                              │
│                                                                               │
│  WHY is this needed here?                                                     │
│                                                                               │
│  LlamaContext<'a> has a lifetime 'a that says:                               │
│  "I borrow from LlamaModel — I cannot outlive my model"                      │
│                                                                               │
│  We want to store BOTH model and context in the same struct:                 │
│  struct ModelContext {                                                        │
│      model:   LlamaModel,                                                    │
│      context: LlamaContext<'???>,  ← what lifetime goes here?                │
│  }                                                                            │
│                                                                               │
│  Rust cannot express "I borrow from a field of the same struct" — this       │
│  is called a "self-referential struct" and Rust's borrow checker rejects it. │
│                                                                               │
│  SOLUTION (careful workaround):                                               │
│  • Transmute the lifetime to 'static ("lives forever")                       │
│  • This is only safe because model and context live in THE SAME STRUCT       │
│  • model will always outlive context — they are both dropped together        │
│  • We never move context out of the struct separately                         │
│                                                                               │
│  SAFER ALTERNATIVES (for new code):                                           │
│  • Use Pin<Box<T>> for self-referential structs                               │
│  • Use the ouroboros or self_cell crates                                      │
│  • Redesign to avoid self-referential structs entirely                        │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

> **Bottom line**: `transmute` here is a deliberate workaround for a Rust language limitation (self-referential structs). It is safe *only* because the author guarantees that `model` always outlives `context` by keeping them in the same struct that is dropped together. This is not a beginner pattern.

---

### 1️⃣7️⃣ `#[derive(...)]` — Auto-Generating Code with Macros

**File**: `types.rs` line 11

```rust
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum ASREngine { Whisper, Parakeet }
```

Every `#[derive(...)]` item tells the Rust compiler to **automatically write code** for you, as if you had typed it by hand:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    🪄 WHAT EACH DERIVE GENERATES                              │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  Debug                                                                        │
│  ──────                                                                       │
│  Generates: impl fmt::Debug for ASREngine { ... }                            │
│  Enables:   println!("{:?}", engine)  →  prints  "Whisper"                   │
│  Use when:  Debugging, logging, error messages                                │
│                                                                               │
│  Clone                                                                        │
│  ─────                                                                        │
│  Generates: impl Clone for ASREngine { fn clone(&self) -> Self { *self } }   │
│  Enables:   let copy = engine.clone()                                         │
│  Use when:  You need to duplicate a value explicitly                          │
│                                                                               │
│  Copy                                                                         │
│  ────                                                                         │
│  Generates: impl Copy for ASREngine {}  (marker — no code body needed)       │
│  Enables:   let a = engine;  let b = engine;  // BOTH valid — no move!       │
│  Use when:  Small, stack-allocated values (enums, integers, coords)           │
│  Cannot use: Types containing String, Vec, Box (heap-allocated)              │
│                                                                               │
│  PartialEq                                                                    │
│  ─────────                                                                    │
│  Generates: impl PartialEq for ASREngine { fn eq(&self, other: &Self) -> bool}│
│  Enables:   if engine == ASREngine::Whisper { ... }                          │
│  Use when:  You need == and != comparisons                                    │
│                                                                               │
│  serde::Serialize                                                             │
│  ─────────────────                                                            │
│  Generates: impl Serialize for ASREngine { ... }  (JSON conversion)          │
│  Enables:   Tauri can send this enum to JavaScript as  "Whisper"  or         │
│             "Parakeet"  (a plain JSON string)                                 │
│                                                                               │
│  serde::Deserialize                                                           │
│  ───────────────────                                                          │
│  Generates: impl Deserialize for ASREngine { ... }                           │
│  Enables:   JavaScript can send the string "Parakeet" → Rust gets the enum   │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Why not just write these by hand?** For a simple 2-variant enum it would be 40+ lines of boilerplate. `derive` generates it in one line and keeps it in sync automatically when you add new variants.

---

### 1️⃣8️⃣ `.or_else()` — Chaining Fallback Operations

**File**: `whisper.rs` line 286–288

```rust
let (ctx, backend) = self
    .try_gpu(&absolute_path)
    .or_else(|_| self.try_cpu(&absolute_path))?;
```

This reads like English: *"Try GPU. If that fails for any reason, try CPU instead."*

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                    🔄 or_else() — The Functional Fallback Pattern            │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  try_gpu()  →  Result<(ctx, GpuBackend), String>                             │
│                                                                               │
│  CASE A: GPU succeeded                                                        │
│  try_gpu() = Ok((ctx, Cuda))                                                  │
│                │                                                              │
│  .or_else()  → GPU was Ok, so or_else SKIPS its closure entirely             │
│                │                                                              │
│  ?           → unwraps Ok((ctx, Cuda)) — assigned to (ctx, backend)          │
│                                                                               │
│  CASE B: GPU failed                                                           │
│  try_gpu() = Err("GPU failed: ...")                                           │
│                │                                                              │
│  .or_else(|_| self.try_cpu(...))                                              │
│     │     ^^^                                                                 │
│     │     |_ ignores the GPU error message (we don't need it)                │
│     │                                                                         │
│     └── Runs try_cpu() → returns Ok((ctx, Cpu)) or Err("CPU also failed")   │
│                │                                                              │
│  ?           → If Ok: assigned to (ctx, backend)                             │
│                If Err: the whole function returns Err (both GPU & CPU failed) │
│                                                                               │
│  EQUIVALENT with if/else (much more verbose):                                 │
│  let result = match self.try_gpu(&path) {                                    │
│      ok @ Ok(_) => ok,                                                       │
│      Err(_) => self.try_cpu(&path),                                          │
│  };                                                                           │
│  let (ctx, backend) = result?;                                               │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

> **`or_else` vs `unwrap_or_else`**:  
> - `Result::or_else(|e| ...)` → fallback for a **Result** (the closure returns another `Result`)  
> - `Option::unwrap_or_else(|| ...)` → fallback for an **Option** (the closure returns a plain value)

---

### 1️⃣9️⃣ `while let Ok(samples) = rx.recv()` — The Self-Terminating Loop

**File**: `commands/recording.rs` lines 63–69

```rust
let writer_thread = std::thread::spawn(move || {
    let mut writer = writer;
    while let Ok(samples) = file_rx.recv() {  // ← loop until channel closes
        for sample in samples {
            writer.write_sample(sample).ok();
        }
    }
    writer.finalize().ok();  // ← runs AFTER loop exits!
});
```

```
┌──────────────────────────────────────────────────────────────────────────────┐
│              🔄 while let — Loop Until the Pattern Fails                     │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  while let Ok(samples) = file_rx.recv()                                      │
│  ─────────────────────────────────────                                        │
│  • recv() returns Ok(data) when data arrives → loop body runs                │
│  • recv() returns Err(_)   when ALL senders are dropped → loop EXITS         │
│                                                                               │
│  This is the ONLY way the loop can exit. There is no break statement.        │
│  The loop is entirely "driven" by whether the channel is open.               │
│                                                                               │
│  Timeline:                                                                    │
│                                                                               │
│  [Recording active]                                                           │
│  recv() → Ok([0.01, 0.02, ...]) → write samples → loop again                │
│  recv() → Ok([0.03, -0.01, ...]) → write samples → loop again               │
│  recv() → [BLOCKING — waiting for more audio]                                │
│  ...                                                                          │
│  [stop_recording() calls drop(file_tx)]                                      │
│  recv() → Err(RecvError) → while let condition FAILS → LOOP EXITS            │
│                                                                               │
│  writer.finalize()   ← This line runs NOW (after loop)                       │
│  Writes the WAV file header with the correct total byte count.               │
│                                                                               │
│  ⚠️ COMMON MISTAKE: Putting finalize() INSIDE the loop:                      │
│  while let Ok(samples) = file_rx.recv() {                                    │
│      write samples...                                                         │
│      writer.finalize().ok();  // ❌ WRONG — called after every chunk!        │
│  }                            // The WAV will be corrupt and the header      │
│                               // will only contain the FIRST chunk's size   │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

### 2️⃣0️⃣ Unicode-Safe String Capitalization — Why `str[0]` Doesn't Work in Rust

**File**: `utils.rs` lines 22–29

```rust
// Capitalize the first letter of a string
if let Some(first) = cleaned.chars().next() {
    if first.is_lowercase() {
        let mut c = cleaned.chars();
        cleaned = match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        };
    }
}
```

This looks overly complex just to capitalize one letter. Here's *why*:

```
┌──────────────────────────────────────────────────────────────────────────────┐
│               🌍 WHY YOU CAN'T DO cleaned[0].to_uppercase()                 │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  In many languages:  str[0]  = first character. Simple!                      │
│                                                                               │
│  In Rust: str[0] is ILLEGAL. The compiler refuses it.                        │
│                                                                               │
│  Why? Rust strings are UTF-8 encoded. One "character" can be 1–4 BYTES.     │
│                                                                               │
│  Example:                                                                     │
│  "hello"    →  [h, e, l, l, o]       5 bytes,  5 chars  ✓ simple            │
│  "héllo"    →  [h, é, l, l, o]       6 bytes,  5 chars  ← é is 2 bytes      │
│  "こんにちは"  →  15 bytes,  5 chars            ← each char is 3 bytes        │
│                                                                               │
│  If Rust let you do str[0], you'd get one BYTE, which might be the          │
│  middle of a multi-byte character → invalid Unicode → undefined behavior.    │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────────┐
│               🔍 WHAT THE CODE ACTUALLY DOES — STEP BY STEP                 │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                               │
│  Input: "hello world"                                                         │
│                                                                               │
│  let mut c = cleaned.chars();                                                 │
│  //          ^^^^^^^^^^^^^^                                                   │
│  //          Iterator over Unicode scalar values (real characters)           │
│  //          c = ['h', 'e', 'l', 'l', 'o', ' ', 'w', 'o', 'r', 'l', 'd']  │
│                                                                               │
│  Some(f) => f.to_uppercase().collect::<String>() + c.as_str()               │
│             ^                ^^^^^^^^^^^^^^^^^^     ^^^^^^^^^                │
│             │                         │                    │                 │
│             │         Converts 'h' → 'H' (returns a        │                 │
│             │         ToUppercase iterator, not a char,     │                 │
│             │         because some chars uppercase to        │                 │
│             │         multiple chars: ß → SS)               │                 │
│             │                                               │                 │
│             │         .collect::<String>()                  │                 │
│             │          converts the iterator → "H"          │                 │
│             │                                               │                 │
│             │         c.as_str() returns the REMAINING      │                 │
│             │         string after consuming 'h':           │                 │
│             │         "ello world"                          │                 │
│             │                                               │                 │
│  "H" + "ello world" = "Hello world"  ✅                                      │
│                                                                               │
│  The turbofish ::<String> tells collect() which type to produce              │
│  (it can't figure it out from context alone here)                            │
│                                                                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

> **Key takeaway**: Rust's string model is Unicode-correct by design. Operations that seem trivial in ASCII-only languages (indexing, slicing, uppercasing) require explicit Unicode handling. `.chars()` is the safe way to iterate over real characters.

---

## File & Function Reference

### 🔍 Quick Lookup Table

| I want to... | Go to | Function/Item |
|-------------|-------|--------------|
| Add a new Tauri command | `commands/*.rs` | Add `#[tauri::command]` fn + register in `lib.rs` |
| Change recording behavior | `commands/recording.rs` | `start_recording()`, `stop_recording()` |
| Modify Whisper logic | `whisper.rs` | `transcribe_chunk()`, `WhisperManager::new()` |
| Modify Parakeet transcription | `parakeet.rs` | `transcribe_chunk()`, `initialize()` |
| Change how Parakeet loads GPU/CPU | `parakeet_loaders.rs` | `init_*_gpu()`, `init_*_cpu()` |
| Add a new downloadable model | `commands/model_registry.rs` | Add entry to `get_model_config()` |
| Add model to the downloads UI | `src/components/settings/types.ts` | Add entry to `MODELS` array |
| Change download/verify logic | `commands/downloader.rs` | `download_model()`, `verify_model_hash()` |
| Change LLM prompt or style | `llm.rs` | `format_transcript()` |
| Change LLM inference params | `llm.rs` | `run_with_options()` |
| Change spell check | `spellcheck.rs` | Correction logic |
| Modify tray icon/behavior | `tray/mod.rs` | `setup_tray()` |
| Change global hotkey | `hotkeys/listener.rs` | Modify key match arms |
| Add shared enum/struct | `types.rs` | Define struct/enum |
| Add utility function | `utils.rs` | Create `pub fn` |
| Change UI recording logic | `src/hooks/useRecording.ts` | `handleStartRecording()`, `handleStopRecording()` |
| Change engine switching UI | `src/hooks/useEngineSwitch.ts` | `handleSwitchToWhisper()`, `handleSwitchToParakeet()` |
| Change LLM/spell UI toggles | `src/hooks/usePostProcessing.ts` | Toggle + load/unload logic |
| Change settings tabs | `src/components/SettingsModal.tsx` | `renderContent()`, tab list |
| Modify General settings tab | `src/components/settings/GeneralTab.tsx` | Spell check toggle UI |
| Modify Downloads tab | `src/components/settings/DownloadsTab.tsx` | Model list + ModelRow |

### 📋 All Tauri Commands (as of February 2026)

```rust
// From lib.rs invoke_handler — matches tauri::generate_handler! exactly:

// 🔧 Misc
commands::greet,                   // Test/greeting placeholder

// 🎤 Recording
commands::start_recording,         // Start mic + real-time transcription
commands::stop_recording,          // Stop + final transcript + post-process
commands::type_text,               // Type text via Enigo keyboard injection

// 🧠 Whisper model management
commands::list_models,             // List downloaded Whisper .bin files
commands::get_current_model,       // Get active Whisper model name
commands::switch_model,            // Load a different Whisper model

// ⚡ Parakeet model management
commands::list_parakeet_models,    // List Parakeet models + their status
commands::init_parakeet,           // Initialize a Parakeet model (GPU/CPU)
commands::get_parakeet_status,     // Check if Parakeet is loaded + which model

// 🔀 Engine switching
commands::set_active_engine,       // Switch between Whisper / Parakeet
commands::get_active_engine,       // Get the currently active engine
commands::get_backend_info,        // Get GPU backend info string

// 🖼️ System tray
commands::set_tray_state,          // Update tray icon (Ready/Recording/Processing)

// ✨ LLM grammar correction
commands::init_llm,                // Load Qwen GGUF model (GPU or CPU)
commands::unload_llm,              // Unload LLM to free VRAM
commands::run_llm_inference,       // Raw LLM text generation
commands::check_llm_status,        // Returns bool: true = loaded, false = not loaded
commands::correct_text,            // Format transcript with style via LLM

// 🔤 Spell checking
commands::init_spellcheck,         // Load SymSpell dictionary
commands::unload_spellcheck,       // Unload spell checker
commands::check_spellcheck_status, // Check if spell checker is loaded
commands::correct_spelling,        // Run SymSpell correction on text

// 📥 Download manager
commands::download_model,          // Stream download from Hugging Face
commands::get_download_status,     // Check downloaded/verified status per model
commands::delete_model,            // Delete model file(s) from disk
commands::verify_model_hash,       // Verify SHA-1 integrity of model file
```

---

## Common Beginner Questions

### Q1: Why are there two transcription engines?

**Answer**: Different use cases need different trade-offs:
- **Whisper** — Higher accuracy, 6-second latency → Best for dictation, meetings
- **Parakeet** — Lower latency (~0.6s), slightly less accurate → Best for real-time streaming

### Q2: Can I use this for other languages?

Whisper supports 99 languages — just speak and it auto-detects. Parakeet is English-only (NVIDIA Nemotron model).

### Q3: How much RAM does this use?

| Component | RAM Usage |
|-----------|-----------|
| Whisper tiny | ~100 MB |
| Whisper base | ~200 MB |
| Whisper large-v3 | ~3 GB |
| Parakeet Nemotron | ~500 MB |
| Qwen LLM (Q4_K_M) | ~400 MB |
| Audio buffer | ~10 MB |

> LLM and Spell Checker are **not loaded at startup** — only when you enable them.

### Q4: Why does the first transcription take longer?

**Answer**: GPU "warm-up"! The first run compiles CUDA/Vulkan shader kernels. Taurscribe optionally runs a warm-up pass during model initialization to hide this delay from the user.

### Q5: What if my recording crashes mid-session?

**Safety features**:
1. WAV file is written continuously stream → disk (you don't lose audio)
2. File saved to: `%LOCALAPPDATA%\Taurscribe\temp\`
3. You can manually re-transcribe the WAV with any tool

### Q6: Where do downloaded models go?

All models land in `%LOCALAPPDATA%\Taurscribe\models\`:
```
models/
├── ggml-tiny.bin                ← Whisper models
├── ggml-base.en.bin
├── parakeet-nemotron/           ← Parakeet ONNX folders
│   ├── encoder.onnx
│   └── decoder.onnx
├── qwen_finetuned_gguf/         ← Grammar LLM
│   └── model_q4_k_m.gguf
└── symspell/                    ← Spell check dictionary
    └── frequency_dictionary_en_82_765.txt
```

### Q7: How does the global hotkey work?

`hotkeys/listener.rs` spawns a background thread that uses `rdev::listen()` to capture **every** key event system-wide. When both `Ctrl` + `Win (Meta)` are held:
- Sends `hotkey-start-recording` event → Frontend starts recording
- On key release → Sends `hotkey-stop-recording` → Frontend stops recording

---

## Conclusion

Taurscribe demonstrates modern Rust practices in a real production app:

✅ **Ownership** — Threads take ownership of data they need  
✅ **Borrowing** — Functions borrow without taking ownership  
✅ **Concurrency** — Multiple threads work safely in parallel  
✅ **Error Handling** — `Result`, `?` operator, `anyhow` for safety  
✅ **Modularity** — Clean separation into focused modules after refactoring  

**Architecture Benefits**:

| Feature | Benefit |
|---------|---------|
| Separate threads | UI never freezes during AI inference |
| Crossbeam channels | Safe, backpressure-aware thread communication |
| `Arc<Mutex<T>>` | Shared engine state protection |
| Two AI engines | User picks speed OR accuracy |
| GPU acceleration | 12–60× faster than CPU-only |
| `commands/` split | Each command file has one clear responsibility |
| `model_registry.rs` | Single source of truth for all model configs |
| On-demand loading | Parakeet + LLM don't use memory until needed |

**Key Takeaway**: Rust's strict compiler prevents entire categories of bugs (data races, null pointer crashes, use-after-free). Once your code compiles, it usually works correctly!

---

## ⌨️ Text Insertion: How Transcribed Text Gets Into Your App

This is one of the trickiest parts of any dictation tool. Once Taurscribe has your final transcript, it needs to **put that text wherever your cursor is** — inside VS Code, Notepad, a browser input, Slack, anything. This section explains exactly how that works on each platform.

---

### The Problem: Why "Just Type It" Doesn't Work Well

The naive approach is to simulate pressing every key on the keyboard, one character at a time:

```
"Hello" → press H → press e → press l → press l → press o
```

This is what the original Enigo `enigo.text()` call did. It breaks badly in practice:

- **Slow** — 500 characters takes hundreds of milliseconds of simulated keystrokes
- **Breaks on special characters** — accented letters, emoji, symbols get garbled
- **Breaks in apps with input handlers** — autocomplete, shortcuts, and input validators intercept individual keystrokes and produce wrong results
- **Language-dependent** — keyboard layout matters; `"` on a US keyboard is different from a French AZERTY layout

Every professional dictation tool (Superwhisper, Wispr Flow, Dragon NaturallySpeaking) avoids character-by-character typing. Here's what Taurscribe does instead.

---

### Platform Strategy at a Glance

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    TEXT INSERTION STRATEGY BY PLATFORM                      │
├──────────────┬──────────────────────────────────────────────────────────────┤
│  macOS       │  1st try: AXUIElement Accessibility API (native insertion)   │
│              │  Fallback: Clipboard + Cmd+V                                 │
├──────────────┼──────────────────────────────────────────────────────────────┤
│  Windows     │  Clipboard + Ctrl+V  (save → set → paste → restore)         │
├──────────────┼──────────────────────────────────────────────────────────────┤
│  Linux       │  Clipboard + Ctrl+V  (save → set → paste → restore)         │
└──────────────┴──────────────────────────────────────────────────────────────┘
```

---

### macOS: The Accessibility API (AXUIElement)

#### What it is

macOS has a built-in **Accessibility API** that lets assistive technology (screen readers, dictation tools) directly communicate with UI elements. It's the same mechanism the OS itself uses when you use Voice Control.

Every text field in every app exposes itself through this API with named **attributes**. The one we care about is:

```
kAXSelectedTextAttribute
```

Setting this attribute on a focused element is equivalent to "type this text at the cursor position, replacing any current selection." It is **not** a keyboard event — it is a direct write into the text field.

#### Why it's better than clipboard paste on macOS

| Property | AXUIElement | Clipboard + Cmd+V |
|---|---|---|
| Clipboard touched? | No | Yes (briefly) |
| Works in secure fields? | Mostly yes | Yes |
| Inserts at cursor? | Yes | Yes |
| Works in every app? | ~90% of apps | ~99% of apps |
| Speed | Instant | ~350ms (50ms + 300ms wait) |

The 10% where AX fails (games, some Electron apps, terminal emulators) gets caught by the fallback.

#### macOS Code Flow

```
type_text("Hello world")
    │
    ▼
insert_text()   ← dispatched on a background thread
    │
    ├─── [macOS only] ──► ax_insert("Hello world")
    │                         │
    │                         ▼
    │                   AXUIElementCreateSystemWide()
    │                   ┌─────────────────────────────────┐
    │                   │  Creates a handle to the entire  │
    │                   │  macOS accessibility tree        │
    │                   └────────────────┬────────────────┘
    │                                    │
    │                                    ▼
    │                   AXUIElementCopyAttributeValue(
    │                       system,
    │                       kAXFocusedUIElementAttribute,  ← "what has focus right now?"
    │                       &mut focused
    │                   )
    │                   ┌─────────────────────────────────┐
    │                   │  focused = the text field the    │
    │                   │  user's cursor is inside         │
    │                   └────────────────┬────────────────┘
    │                                    │
    │                                    ▼
    │                   AXUIElementSetAttributeValue(
    │                       focused,
    │                       kAXSelectedTextAttribute,      ← "set selected text to..."
    │                       CFString("Hello world")        ← our transcript
    │                   )
    │                   ┌─────────────────────────────────┐
    │                   │  Text appears at cursor.         │
    │                   │  No clipboard. No key events.    │
    │                   └────────────────┬────────────────┘
    │                                    │
    │                   CFRelease(system) + CFRelease(focused)
    │                   (clean up memory — AX objects are ref-counted)
    │                                    │
    │                   returns true ◄───┘
    │
    └─── returns early, done ✓

    If ax_insert() returns false (no Accessibility permission,
    or the focused app doesn't expose AX attributes):
    │
    ▼
clipboard_paste()   ← fallback path (same as Windows/Linux)
```

#### Accessibility Permission Requirement

The AX path requires the user to grant **Accessibility access** to Taurscribe in:

```
System Settings → Privacy & Security → Accessibility
```

If the permission isn't granted, `AXUIElementCopyAttributeValue` returns `kAXErrorAPIDisabled (-25211)` and `ax_insert()` returns `false`, silently falling through to clipboard paste.

---

### Windows & Linux: Clipboard + Paste Keystroke

#### Why not UI Automation on Windows?

Windows has an equivalent API called **UI Automation** (`IUIAutomation`). However, the primary write method — `IUIAutomationValuePattern::SetValue` — **replaces the entire content** of the field. If a user is dictating into the middle of a document, this would wipe everything they've written. That's destructive and wrong.

The clipboard approach is actually the correct behavior for Windows:
- Inserts at the cursor position (Ctrl+V always pastes where the caret is)
- Works in every app: Win32, WPF, Electron, browsers, terminals
- Used by Wispr Flow, Dragon NaturallySpeaking, and Windows Voice Access for cross-app insertion

#### The Clipboard Save/Restore Trick

Simply writing to the clipboard and pressing Ctrl+V would clobber whatever the user had copied previously. The implementation saves and restores:

```
Before paste:  clipboard = "user's previous copy"   (saved)
During paste:  clipboard = "Hello world"             (our text)
After paste:   clipboard = "user's previous copy"   (restored)
```

The user never sees their clipboard change.

#### Windows/Linux Code Flow

```
type_text("Hello world")
    │
    ▼
insert_text()   ← dispatched on a background thread
    │
    │   [not macOS — goes straight to clipboard path]
    │
    ▼
clipboard_paste("Hello world")
    │
    ├──► Clipboard::new()
    │    ┌──────────────────────────────────────────┐
    │    │  Opens a handle to the OS clipboard.      │
    │    │  arboard crate handles Win32/X11/Wayland  │
    │    └─────────────────┬────────────────────────┘
    │                      │
    ├──► previous = clipboard.get_text().ok()
    │    ┌──────────────────────────────────────────┐
    │    │  Saves whatever was in the clipboard.     │
    │    │  Returns None if clipboard had non-text   │
    │    │  content (image, file, etc.)              │
    │    └─────────────────┬────────────────────────┘
    │                      │
    ├──► clipboard.set_text("Hello world")
    │    ┌──────────────────────────────────────────┐
    │    │  Writes our transcript into the clipboard │
    │    └─────────────────┬────────────────────────┘
    │                      │
    ├──► sleep(50ms)
    │    ┌──────────────────────────────────────────┐
    │    │  Gives the OS time to propagate the new  │
    │    │  clipboard content before we paste.       │
    │    │  Without this, some apps paste the OLD    │
    │    │  clipboard content.                       │
    │    └─────────────────┬────────────────────────┘
    │                      │
    ├──► Enigo::key(Ctrl/Cmd, Press)
    │    Enigo::key('v', Click)              ← simulates Ctrl+V / Cmd+V
    │    Enigo::key(Ctrl/Cmd, Release)
    │    ┌──────────────────────────────────────────┐
    │    │  The focused app receives the paste       │
    │    │  shortcut and pulls "Hello world" from    │
    │    │  the clipboard, inserting at cursor.      │
    │    └─────────────────┬────────────────────────┘
    │                      │
    ├──► sleep(300ms)
    │    ┌──────────────────────────────────────────┐
    │    │  Wait for the paste to fully land before  │
    │    │  we overwrite the clipboard again.        │
    │    └─────────────────┬────────────────────────┘
    │                      │
    └──► clipboard.set_text(previous)   ← restore original content
         ┌──────────────────────────────────────────┐
         │  User's clipboard is back to normal.      │
         │  If previous was None (non-text), we      │
         │  leave our text — clearing entirely       │
         │  would be more surprising behavior.        │
         └──────────────────────────────────────────┘
         Done ✓
```

---

### Full Decision Tree (All Platforms)

```
transcript ready
      │
      ▼
invoke("type_text", { text })   [frontend → Rust IPC]
      │
      ▼
  type_text()  [Rust, main thread]
      │
      └──► spawn background thread
                │
                ▼
           insert_text()
                │
         ┌──────┴──────────────┐
         │ cfg(target_os =     │ cfg(not macOS)
         │ "macos")            │
         ▼                     ▼
     ax_insert()         clipboard_paste()
         │                     │
    success? ──Yes──► done ✓   └──► done ✓
         │
        No (no permission,
            app blocks AX,
            error from OS)
         │
         ▼
    clipboard_paste()   [macOS fallback]
         │
         └──► done ✓
```

---

### Crates Used

| Crate | Purpose | Platforms |
|---|---|---|
| `accessibility-sys` | Raw FFI bindings to macOS Accessibility framework (`AXUIElement*` functions and constants) | macOS only |
| `core-foundation` | Rust wrappers for Core Foundation types (`CFString`, `CFTypeRef`, ref-counting with `CFRelease`) | macOS only |
| `arboard` | Cross-platform clipboard read/write. Handles Win32, X11, and Wayland backends transparently | All |
| `enigo` | Simulates the paste keystroke (`Ctrl+V` / `Cmd+V`). Used only in the clipboard fallback path | All |

---

### Relevant Source Files

| File | What it contains |
|---|---|
| `src-tauri/src/commands/recording.rs` | `type_text` command, `insert_text`, `clipboard_paste`, `ax_insert` |
| `src-tauri/Cargo.toml` | `arboard` in `[dependencies]`; `accessibility-sys` + `core-foundation` in `[target.'cfg(target_os = "macos")'.dependencies]` |
| `src/hooks/useRecording.ts` | Line 168: `invoke("type_text", { text: finalTrans })` — the call site after all post-processing is done |

---

### Why This Matches What Superwhisper / Wispr Flow Do

- **Superwhisper (macOS)** — uses `kAXSelectedTextAttribute` as the primary path, clipboard as fallback. This is exactly our macOS implementation.
- **Wispr Flow (Windows)** — their changelog mentions "delayed clipboard rendering" and their manual fallback is a paste shortcut. This confirms they use clipboard + paste on Windows, same as us.
- **Dragon NaturallySpeaking (Windows)** — uses `EM_REPLACESEL` (Win32 edit controls only) with clipboard paste as the universal fallback.

The clipboard + paste approach on Windows is not a compromise — it is the industry standard for cross-app text insertion because it is the only mechanism that inserts at the cursor position reliably across all application frameworks (Win32, WPF, Qt, Electron, web browsers).

---

## 🚀 First Launch & Setup Wizard

### Overview

When Taurscribe opens for the first time, instead of the main UI it shows a **5-step animated setup wizard**. On every subsequent launch it skips straight to the app. The gate is a single boolean flag — `setup_complete` — stored in Tauri's persistent key-value store (`settings.json`).

```
┌─────────────────────────────────────────────────────────────────────┐
│                    FIRST LAUNCH DECISION TREE                        │
└─────────────────────────────────────────────────────────────────────┘

   App starts
       │
       ▼
   Load settings.json  ◄─── @tauri-apps/plugin-store
       │
       ├── store.get("setup_complete")
       │
       ├── value === true  ──────────────────────► Show Main App
       │
       ├── value === false / null / missing ──────► Show Setup Wizard
       │
       └── store load fails entirely ─────────────► Show Setup Wizard
                                                      (safe fallback)
```

### The 5-Step Wizard Flow

```
╔═══════════════════════════════════════════════════════════════════╗
║                    SETUP WIZARD — 5 STEPS                         ║
╠═══════════════════════════════════════════════════════════════════╣
║                                                                   ║
║  STEP 0 ─── STEP 1 ─── STEP 2 ─── STEP 3 ─── STEP 4             ║
║  Welcome    Hardware   Engines    Hotkey     Ready                ║
║    ●  ────    ○  ────    ○  ────    ○  ────    ○   (progress dots)║
║                                                                   ║
╠═══════════════════════════════════════════════════════════════════╣
║                                                                   ║
║  STEP 0: Welcome                                                  ║
║  ┌─────────────────────────────────┐                             ║
║  │  Taurscribe                     │                             ║
║  │  ─────────────────────          │                             ║
║  │  Local AI speech recognition    │                             ║
║  │                                 │                             ║
║  │  · 100% offline                 │                             ║
║  │  · GPU-accelerated              │                             ║
║  │  · Types into any app           │                             ║
║  │                   [Begin Setup →]│                             ║
║  └─────────────────────────────────┘                             ║
║                                                                   ║
║  STEP 1: System Analysis (hardware scan)                          ║
║  ┌─────────────────────────────────┐                             ║
║  │  CPU   Intel i9-13900K · 32 t  ● │                            ║
║  │  RAM   32.0 GB                 ● │                            ║
║  │  GPU   NVIDIA RTX 4090         ● │                            ║
║  │  VRAM  24.0 GB                 ● │                            ║
║  │  AI    CUDA                    ● │                            ║
║  │                                 │                             ║
║  │  GPU acceleration ready.        │                             ║
║  └─────────────────────────────────┘                             ║
║                                                                   ║
║  STEP 2: Two Engines                                              ║
║  ┌──────────────┐  ┌──────────────┐                             ║
║  │  Whisper     │  │  Parakeet    │                             ║
║  │  by OpenAI   │  │  by NVIDIA   │                             ║
║  │  · Accurate  │  │  · Streaming │                             ║
║  │  · Multi-    │  │  · <500ms    │                             ║
║  │    lingual   │  │  · NVIDIA GPU│                             ║
║  └──────────────┘  └──────────────┘                             ║
║                                                                   ║
║  STEP 3: One Hotkey                                               ║
║  ┌─────────────────────────────────┐                             ║
║  │  [Ctrl]  +  [Win]               │                             ║
║  │  01 Focus any text field        │                             ║
║  │  02 Press Ctrl + Win → record   │                             ║
║  │  03 Speak naturally             │                             ║
║  │  04 Press again to stop         │                             ║
║  │  05 Text appears at cursor      │                             ║
║  └─────────────────────────────────┘                             ║
║                                                                   ║
║  STEP 4: Ready                                                    ║
║  ┌─────────────────────────────────┐                             ║
║  │  ✓ Hardware detected            │                             ║
║  │  ✓ AI engines ready             │                             ║
║  │  ✓ Hotkey active: Ctrl + Win    │                             ║
║  │  ✓ Pastes into any app          │                             ║
║  │  [Open Settings & Download]     │ ← sets setup_complete=true  ║
║  │  [Launch App]                   │ ← sets setup_complete=true  ║
║  └─────────────────────────────────┘                             ║
║                                                                   ║
╚═══════════════════════════════════════════════════════════════════╝
```

### Slide Animation System

The wizard uses a **dual-buffer enter/exit animation** — both the old and new steps render simultaneously for 400ms so one slides out while the other slides in.

```
┌─────────────────────────────────────────────────────────────────────┐
│                    ANIMATION STATE MACHINE                           │
└─────────────────────────────────────────────────────────────────────┘

  State: { current: StepEntry, exiting: StepEntry | null }

  User presses "Next"  (going from Step 1 → Step 2)
       │
       ▼
  goTo(2) called
       │
       ├── forward = true  (target 2 > current 1)
       │
       ├── setExiting({ idx: 1, exitDir: "left",  key: N   })
       │     └── renders old step with class  "setup-step--exit-left"
       │                                           ↓
       │                           CSS: translate(0) → translateX(-100%)
       │
       ├── setCurrent({ idx: 2, enterDir: "right", key: N+1 })
       │     └── renders new step with class  "setup-step--enter-right"
       │                                           ↓
       │                           CSS: translateX(100%) → translate(0)
       │
       └── setTimeout(400ms) → setExiting(null)  (old step removed)

  Going "Back" reverses the directions:
       exitDir = "right"  (old step slides right)
       enterDir = "left"  (new step slides in from left)

CSS keyframes (SetupWizard.css):

  @keyframes slideInRight  { from { transform: translateX(100%) } to { transform: none } }
  @keyframes slideInLeft   { from { transform: translateX(-100%) } to { transform: none } }
  @keyframes slideOutLeft  { from { transform: none } to { transform: translateX(-100%) } }
  @keyframes slideOutRight { from { transform: none } to { transform: translateX(100%) } }

  .setup-step--enter-right { animation: slideInRight  400ms ease }
  .setup-step--enter-left  { animation: slideInLeft   400ms ease }
  .setup-step--exit-left   { animation: slideOutLeft  400ms ease; position: absolute }
  .setup-step--exit-right  { animation: slideOutRight 400ms ease; position: absolute }
```

### Hardware Detection (Step 1)

The hardware scan calls the `get_system_info` Tauri command which runs OS-level queries:

```
┌─────────────────────────────────────────────────────────────────────┐
│              get_system_info() — commands/misc.rs                    │
└─────────────────────────────────────────────────────────────────────┘

  sysinfo::System::new_all()
       │
       ├── .cpus().first().brand()         → cpu_name   "Intel Core i9-13900K"
       ├── .cpus().len()                   → cpu_cores  32
       └── .total_memory() / 1_073_741_824 → ram_total_gb  32.0

  detect_gpu()
       │
       ├── 1st: try nvidia-smi --query-gpu=name,memory.total
       │         success → (gpu_name, cuda=true, vram_gb)
       │
       ├── 2nd (Windows):  wmic path win32_VideoController get name
       │         success → (gpu_name, cuda=name.contains("nvidia"), vram=None)
       │
       ├── 2nd (macOS):    system_profiler SPDisplaysDataType
       │         success → (gpu_name, cuda=false, vram=None)
       │
       ├── 2nd (Linux):    lspci | grep "VGA\|3D controller"
       │         success → (gpu_name, cuda=name.contains("nvidia"), vram=None)
       │
       └── fallback        ("Unknown", cuda=false, vram=None)

  backend_hint determination:
       cuda_available=true  → "CUDA"
       macOS + no CUDA      → "Metal"
       gpu detected + no CUDA → "Vulkan / DirectML"
       no GPU               → "CPU"

  Result: SystemInfo { cpu_name, cpu_cores, ram_total_gb,
                       gpu_name, cuda_available, vram_gb, backend_hint }
```

Status indicators in the UI map as follows:

| Condition | Indicator |
|---|---|
| CPU always detected | `hw-status--ok` (green dot) |
| RAM ≥ 8 GB | `hw-status--ok` |
| RAM < 8 GB | `hw-status--warn` (amber dot) |
| GPU name found | `hw-status--ok` |
| GPU unknown | `hw-status--warn` |
| CUDA available | ok verdict: "GPU acceleration ready" |
| GPU found, no CUDA | amber verdict: "GPU detected (no CUDA)" |
| No GPU | neutral verdict: "No GPU detected, use small model" |

### Resetting the Wizard (Dev Workflow)

```bash
# Delete settings.json — wizard reappears on next launch
del "%APPDATA%\abdul\settings.json"

# Or edit just the key (keeps other settings intact):
# Open %APPDATA%\abdul\settings.json and delete the "setup_complete" key
```

---

## 🏪 App State & Settings Persistence

### The Plugin-Store

Taurscribe uses `@tauri-apps/plugin-store` to persist settings between launches. On Windows this writes to:

```
%APPDATA%\abdul\settings.json
```

The store is loaded once at startup in `App.tsx`'s `loadInitialData()` and stored in a React ref so all async handlers can access the same instance:

```
App.tsx
  │
  ├── storeRef = useRef<Store | null>(null)
  │
  └── loadInitialData()
        │
        ├── Store.load("settings.json")   ← opens or creates the file
        ├── storeRef.current = loadedStore
        │
        ├── loadedStore.get("setup_complete")
        ├── loadedStore.get("active_engine")
        └── loadedStore.get("parakeet_model")
```

### Persisted Keys Table

| Key | Type | Written when | Read when |
|---|---|---|---|
| `setup_complete` | `boolean` | User completes wizard (Step 4 CTA) | App startup |
| `active_engine` | `"whisper" \| "parakeet"` | Engine switch in main UI | App startup |
| `parakeet_model` | `string` (model ID) | User selects Parakeet model | App startup (auto-load) |

### Write Pattern

Every key is written via a two-step pattern to ensure the file is flushed to disk:

```typescript
storeRef.current?.set("active_engine", activeEngine)  // update in-memory
  .then(() => storeRef.current?.save())                // flush to disk
```

The `?.` optional chaining guards against the ref being null during the brief window before the store is loaded.

### Auto-Loading Engines on Start

The startup sequence tries to restore exactly the state the user left the app in:

```
┌─────────────────────────────────────────────────────────────────────┐
│                    STARTUP AUTO-LOAD SEQUENCE (App.tsx)              │
└─────────────────────────────────────────────────────────────────────┘

  store.get("active_engine") → savedEngine
       │
       ├── savedEngine === "parakeet"  AND  parakeetModels.length > 0
       │     │
       │     ├── try store.get("parakeet_model") for specific model
       │     └── invoke("init_parakeet", { modelId }) → auto-loads on launch
       │
       ├── savedEngine === "whisper"
       │     └── Whisper loads lazily (on first recording, not at startup)
       │
       └── no savedEngine
             └── leave engine at default (whisper), user selects manually
```

---

## 🪝 Frontend Hook Architecture

### The 5-Hook Pattern

`App.tsx` is a **pure assembly component** — it contains no logic of its own beyond wiring the hooks together and rendering JSX. All stateful logic lives in five focused custom hooks:

```
┌─────────────────────────────────────────────────────────────────────┐
│                    HOOK DEPENDENCY DIAGRAM                           │
└─────────────────────────────────────────────────────────────────────┘

                         ┌──────────────┐
                         │   App.tsx    │
                         │  (assembler) │
                         └──────┬───────┘
                                │ wires together
            ┌───────────────────┼───────────────────┐
            │                   │                   │
            ▼                   ▼                   ▼
  ┌──────────────────┐  ┌──────────────┐  ┌──────────────────┐
  │ useHeaderStatus  │  │  useModels   │  │ usePostProcessing│
  │                  │  │              │  │                  │
  │ headerStatus     │  │ models[]     │  │ enableGrammarLM  │
  │ setHeaderStatus()│  │ parakeet[]   │  │ enableSpellCheck │
  │                  │  │ refreshModels│  │ llmStatus        │
  └──────────┬───────┘  └──────┬───────┘  └───────┬──────────┘
             │                 │                   │
             │    setHeaderStatus passed as prop   │
             │                 │                   │
             └────────────┬────┘                   │
                          │                        │
                          ▼                        │
               ┌──────────────────────┐            │
               │   useEngineSwitch    │            │
               │                     │            │
               │ activeEngine        │◄───────────┘
               │ loadedEngine        │  (reads enableGrammarLMRef
               │ handleSwitchTo*()   │   from usePostProcessing
               │ handleModelChange() │   via refs in useRecording)
               └──────────┬──────────┘
                          │
                          ▼
               ┌──────────────────────┐
               │    useRecording      │
               │                     │
               │ isRecording         │
               │ liveTranscript      │
               │ handleStart/Stop    │
               │                     │
               │ reads:              │
               │  activeEngineRef    │  ← from useEngineSwitch
               │  enableGrammarLMRef │  ← from usePostProcessing
               │  enableSpellCheckRef│  ← from usePostProcessing
               └─────────────────────┘
```

### Why Refs Alongside State?

React state updates are **asynchronous** — when an event handler (like a hotkey listener) fires, it captures a stale closure over the state values from when it was created. Refs are updated synchronously and always hold the latest value.

```
┌─────────────────────────────────────────────────────────────────────┐
│                    THE STALE CLOSURE PROBLEM                         │
└─────────────────────────────────────────────────────────────────────┘

  // ❌ WRONG — hotkey handler captures stale "activeEngine"
  useEffect(() => {
    listen("hotkey-start-recording", () => {
      // activeEngine is frozen to its value when this effect ran
      if (activeEngine === "parakeet") { ... }
    });
  }, []);   // empty deps = runs once = stale forever

  // ✅ RIGHT — ref is always current
  const activeEngineRef = useRef(activeEngine);
  useEffect(() => { activeEngineRef.current = activeEngine; });

  useEffect(() => {
    listen("hotkey-start-recording", () => {
      // activeEngineRef.current is always the latest value
      if (activeEngineRef.current === "parakeet") { ... }
    });
  }, []);
```

### Hook Responsibilities at a Glance

| Hook | State it owns | Refs it exposes | Key actions |
|---|---|---|---|
| `useHeaderStatus` | `headerStatusMessage`, `headerStatusIsProcessing` | — | `setHeaderStatus(msg, timeoutMs?)` |
| `useModels` | `models[]`, `parakeetModels[]`, `currentModel`, `currentParakeetModel` | — | `refreshModels()` |
| `usePostProcessing` | `enableGrammarLM`, `enableSpellCheck`, `llmStatus`, `transcriptionStyle`, `llmBackend` | `enableGrammarLMRef`, `enableSpellCheckRef`, `transcriptionStyleRef` | auto-load/unload LLM and spell checker on toggle |
| `useEngineSwitch` | `activeEngine`, `loadedEngine`, `isLoading`, `loadingTargetEngine`, `transferLineFadingOut` | `activeEngineRef`, `isLoadingRef` | `handleSwitchToWhisper()`, `handleSwitchToParakeet()`, `handleModelChange()` |
| `useRecording` | `isRecording`, `liveTranscript`, `latestLatency`, `isProcessingTranscript`, `isCorrecting` | `isRecordingRef` | `handleStartRecording()`, `handleStopRecording()` |

### Post-Processing Pipeline (inside useRecording)

When `handleStopRecording()` is called, the transcript goes through a sequential pipeline before being typed into the active window:

```
┌─────────────────────────────────────────────────────────────────────┐
│              POST-PROCESSING PIPELINE (useRecording.ts)              │
└─────────────────────────────────────────────────────────────────────┘

  invoke("stop_recording")
       │
       ▼
  rawTranscript (string)
       │
       ├── enableSpellCheckRef.current === true?
       │     YES → invoke("correct_spelling", { text: rawTranscript })
       │               → spellCheckedText
       │     NO  → pass through unchanged
       │
       ▼
  spellCheckedText
       │
       ├── enableGrammarLMRef.current === true?
       │     YES → invoke("correct_text", {
       │               text: spellCheckedText,
       │               style: transcriptionStyleRef.current
       │           })
       │           → correctedText
       │     NO  → pass through unchanged
       │
       ▼
  finalTranscript
       │
       ├── invoke("type_text", { text: finalTranscript })
       │     └── Enigo/clipboard pastes into the active window
       │
       └── setLiveTranscript(finalTranscript)  → updates UI display
```

### IPC Event Map

Events flow in both directions across the Tauri IPC bridge:

```
FRONTEND calls Backend (invoke):           BACKEND emits to Frontend (listen):

invoke("start_recording")          ◄──►   emit("hotkey-start-recording")
invoke("stop_recording")           ◄──►   emit("hotkey-stop-recording")
invoke("list_models")              ◄──►   emit("transcription-chunk", text)
invoke("list_parakeet_models")     ◄──►   emit("models-changed")
invoke("init_parakeet", {modelId})
invoke("get_current_model")
invoke("get_parakeet_status")
invoke("get_backend_info")
invoke("get_system_info")          ← used by Setup Wizard
invoke("set_active_engine")
invoke("switch_model", {modelId})
invoke("init_llm")
invoke("unload_llm")
invoke("correct_text", {text, style})
invoke("init_spellcheck")
invoke("unload_spellcheck")
invoke("correct_spelling", {text})
invoke("type_text", {text})
invoke("set_tray_state", {newState})
invoke("download_model", {url, path})
invoke("delete_model", {path})
```

All Tauri commands must be registered in the `invoke_handler!` macro inside `src-tauri/src/lib.rs`. Adding a new command without registering it there will cause a runtime error ("command not found").

---

---

## 🍎 CoreML Acceleration (Apple Silicon)

### What Is CoreML?

**CoreML** is Apple's on-device machine learning framework. Every Mac with an M-series chip (M1, M2, M3, M4) contains a dedicated piece of hardware called the **Apple Neural Engine (ANE)** — a chip designed specifically to run neural network operations at very high speed while using very little power.

```
┌──────────────────────────────────────────────────────────────┐
│                   Apple M-Series Chip                        │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│   ┌──────────────┐   ┌──────────────┐   ┌───────────────┐   │
│   │  CPU Cores   │   │  GPU Cores   │   │ Neural Engine │   │
│   │  (P+E cores) │   │  (Metal)     │   │   (ANE)       │   │
│   │              │   │              │   │               │   │
│   │ General      │   │ Graphics +   │   │ ML inference  │   │
│   │ computation  │   │ Metal ML     │   │ only — very   │   │
│   │              │   │              │   │ fast & cool   │   │
│   └──────────────┘   └──────────────┘   └───────────────┘   │
│                                                              │
│   Whisper WITHOUT CoreML → runs on CPU or GPU (Metal)        │
│   Whisper WITH CoreML    → encoder runs on Neural Engine ⚡   │
└──────────────────────────────────────────────────────────────┘
```

Without CoreML, Whisper runs entirely on the CPU (or GPU via Metal). With CoreML, the **encoder** — the heaviest part of the model — is compiled into a native Apple Neural Engine format and runs on dedicated silicon, often **2–4× faster** with **significantly less power consumption**.

---

### How Whisper Uses CoreML

Whisper (and by extension whisper.cpp) is split into two parts:

| Part | What it does | Size |
|------|-------------|------|
| **Encoder** | Converts raw audio into a rich internal representation | Large — most of the compute |
| **Decoder** | Translates that representation into text tokens | Smaller |

CoreML acceleration targets **only the encoder**. The decoder continues to run on the CPU/GPU as normal. This is why:
- You download a `.mlmodelc` directory **in addition to** the regular `.bin` file — not instead of it.
- The `.bin` file is still required (it contains the decoder weights and model config).
- The `.mlmodelc` directory contains the encoder compiled into Apple's proprietary neural network format.

```
%LOCALAPPDATA%\Taurscribe\models\
├── ggml-small.en.bin               ← GGUF model (decoder + fallback encoder)
└── ggml-small.en-encoder.mlmodelc/ ← CoreML encoder (runs on Neural Engine)
    ├── model.mlmodel
    ├── model.mlmodelc
    └── ... (compiled model assets)
```

whisper.cpp checks for the `.mlmodelc` directory automatically at model load time. If it is present **and** CoreML support was compiled in, the ANE encoder is used. If the directory is missing, Whisper silently falls back to the CPU/GPU encoder inside the `.bin` file. No code change is needed — it is purely file-presence detection.

---

### Enabling CoreML in whisper-rs / whisper.cpp

Taurscribe uses [whisper-rs](https://codeberg.org/tazz4843/whisper-rs), a Rust wrapper around whisper.cpp. whisper-rs exposes a `coreml` Cargo feature that sets the `WHISPER_COREML=1` CMake flag when building the underlying C++ library.

**`src-tauri/Cargo.toml` — macOS target section:**

```toml
[target.'cfg(target_os = "macos")'.dependencies]
whisper-rs = {
    git = "https://codeberg.org/tazz4843/whisper-rs.git",
    features = ["coreml"]   # ← this enables CoreML at compile time
}
```

Without `features = ["coreml"]`, the binary cannot use CoreML even if the `.mlmodelc` directory is present — the C++ code path is simply not compiled in. With it, whisper.cpp links against the `CoreML` and `Foundation` Apple frameworks automatically through the build script.

**What happens at compile time:**

```
cargo build
  └── whisper-rs build script
        └── cmake -DWHISPER_COREML=1 ...
              └── compiles whisper.cpp with CoreML support
                    └── links CoreML.framework + Foundation.framework
                          └── final binary: CoreML support baked in
```

This is a macOS-only dependency section, so Windows and Linux builds are completely unaffected — they never compile the CoreML code path.

---

### The `.mlmodelc.zip` Files on Hugging Face

The CoreML encoders are hosted on the [`ggerganov/whisper.cpp`](https://huggingface.co/ggerganov/whisper.cpp) Hugging Face repository as `.zip` archives:

```
https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en-encoder.mlmodelc.zip
```

Each zip contains a single directory — the `.mlmodelc` bundle. For example:

```
ggml-small.en-encoder.mlmodelc.zip
└── ggml-small.en-encoder.mlmodelc/
    ├── model.espresso.net
    ├── model.espresso.shape
    ├── model.espresso.weights
    └── metadata.json
```

Available CoreML encoder zips and their sizes:

| Model | Zip file | Size |
|-------|----------|------|
| Tiny (multilingual) | `ggml-tiny-encoder.mlmodelc.zip` | 15 MB |
| Tiny (English) | `ggml-tiny.en-encoder.mlmodelc.zip` | 15 MB |
| Base (multilingual) | `ggml-base-encoder.mlmodelc.zip` | 38 MB |
| Base (English) | `ggml-base.en-encoder.mlmodelc.zip` | 38 MB |
| Small (multilingual) | `ggml-small-encoder.mlmodelc.zip` | 163 MB |
| Small (English) | `ggml-small.en-encoder.mlmodelc.zip` | 163 MB |
| Medium (multilingual) | `ggml-medium-encoder.mlmodelc.zip` | 568 MB |
| Medium (English) | `ggml-medium.en-encoder.mlmodelc.zip` | 567 MB |
| Large V3 | `ggml-large-v3-encoder.mlmodelc.zip` | 1.18 GB |
| Large V3 Turbo | `ggml-large-v3-turbo-encoder.mlmodelc.zip` | 1.17 GB |

---

### The Download & Extraction Pipeline

Regular Whisper models are single `.bin` files — download and done. CoreML encoders are `.zip` archives that must be extracted before whisper.cpp can use them. Taurscribe's downloader handles this automatically.

**`src-tauri/src/commands/model_registry.rs` — CoreML entry:**

```rust
"whisper-small-en-coreml" => Some(ModelConfig {
    repo: "ggerganov/whisper.cpp",    // Hugging Face repo
    branch: "main",
    files: vec![ModelFile {
        filename: "ggml-small.en-encoder.mlmodelc",  // extracted directory name
        remote_path: "ggml-small.en-encoder.mlmodelc.zip", // what to download
        sha1: "",
    }],
    subdirectory: None,  // goes straight into the models dir
}),
```

The key design decision: `filename` is the **extracted directory name** (what whisper.cpp looks for), while `remote_path` is the **zip URL path** (what we actually download).

**`src-tauri/src/commands/downloader.rs` — extraction logic:**

```rust
// Detect zip by remote_path extension
let is_zip = file_spec.remote_path.ends_with(".zip");

// Download to a temp path: "ggml-small.en-encoder.mlmodelc.zip"
let download_path = if is_zip {
    base_dir.join(format!("{}.zip", file_spec.filename))
} else {
    base_dir.join(file_spec.filename)
};

// ... streaming download writes to download_path ...

drop(file); // flush & close before reading back

// Extract and clean up
if is_zip {
    let zip_file = File::open(&download_path)?;
    let mut archive = ZipArchive::new(zip_file)?;
    archive.extract(&base_dir)?;        // extracts the .mlmodelc directory
    std::fs::remove_file(&download_path).ok(); // delete the zip
}
```

Full pipeline for a CoreML download:

```
User clicks "Download"
        │
        ▼
download_model("whisper-small-en-coreml")
        │
        ▼
Build URL: huggingface.co/ggerganov/whisper.cpp/.../ggml-small.en-encoder.mlmodelc.zip
        │
        ▼
Stream download → write to models/ggml-small.en-encoder.mlmodelc.zip
        │
        ▼
ZipArchive::extract(models_dir)
  → creates models/ggml-small.en-encoder.mlmodelc/
        │
        ▼
Delete the .zip file
        │
        ▼
Emit "download-progress" { status: "done" }
        │
        ▼
whisper.cpp auto-detects the .mlmodelc dir on next model load ✓
```

---

### Download Status & Deletion for Directories

Because the downloaded artifact is a **directory** (not a single file), the status-check and delete logic needed to be updated to handle both cases.

**Status check (`get_download_status`):**

```rust
for file_spec in &config.files {
    let file_path = base_dir.join(file_spec.filename);
    if file_path.exists() {
        if file_path.is_dir() {
            // .mlmodelc is a directory — mark as present (size = 1 sentinel)
            total_size += 1;
        } else if let Ok(metadata) = std::fs::metadata(&file_path) {
            total_size += metadata.len();
        } else {
            all_exist = false;
        }
    } else {
        all_exist = false;
    }
}
```

**Delete (`delete_model`):**

```rust
for file_spec in &config.files {
    let file_path = base_dir.join(file_spec.filename);
    if file_path.exists() {
        if file_path.is_dir() {
            let _ = std::fs::remove_dir_all(&file_path); // recursive delete
        } else {
            let _ = std::fs::remove_file(&file_path);
        }
    }
}
```

---

### Platform Detection & Frontend Gating

CoreML encoders are meaningless on Windows or Linux — those platforms have no Neural Engine. The Downloads tab is gated to only show the CoreML section when running on macOS.

**New Tauri command — `get_platform` (`src-tauri/src/commands/misc.rs`):**

```rust
#[tauri::command]
pub fn get_platform() -> &'static str {
    #[cfg(target_os = "macos")]  { "macos"   }
    #[cfg(target_os = "windows")] { "windows" }
    #[cfg(target_os = "linux")]   { "linux"   }
}
```

This is compiled at build time using Rust's `cfg` attributes — the correct string is baked into the binary for each platform. There is no runtime OS detection.

**Frontend usage (`src/components/settings/DownloadsTab.tsx`):**

```tsx
const [platform, setPlatform] = useState<string>('');

useEffect(() => {
    invoke<string>('get_platform').then(setPlatform).catch(() => {});
}, []);

const isMac = platform === 'macos';

// ...

{isMac && coremlModels.length > 0 && (
    <div>
        <h4>CoreML Encoders — Apple Silicon</h4>
        <p>Hardware-accelerated encoder via the Neural Engine...</p>
        {coremlModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
    </div>
)}
```

The section renders only when `isMac` is `true`. On Windows and Linux the array exists in the bundle but is never shown.

---

### Model Type in the Frontend

`DownloadableModel` in `src/components/settings/types.ts` now has a `'CoreML'` type and a `macosOnly` flag:

```ts
export interface DownloadableModel {
    id: string;
    name: string;
    type: 'Whisper' | 'Parakeet' | 'LLM' | 'Utility' | 'CoreML';
    size: string;
    description: string;
    downloaded: boolean;
    verified?: boolean;
    macosOnly?: boolean;  // gates visibility to macOS only
}
```

CoreML entries in the `MODELS` array look like:

```ts
{
    id: 'whisper-small-en-coreml',
    name: 'Small (English) CoreML Encoder',
    type: 'CoreML',
    size: '163 MB',
    description: 'Apple Neural Engine encoder for Small (English). Pair with ggml-small.en.bin.',
    downloaded: false,
    macosOnly: true,
}
```

---

### Setup Wizard Note

The Engines step (Step 3 of 5) in the Setup Wizard shows a brief CoreML callout **on all platforms** — it is informational text rather than a functional UI element, so it is not gated by platform. This way users who switch to a Mac later still see the information during their first setup.

The callout (`src/components/SetupWizard.tsx`):

```tsx
<div className="engines-coreml-note">
    <span className="engines-coreml-badge">CoreML</span>
    Apple Silicon · CoreML encoder libraries are available for Whisper — download them
    in Settings → Downloads to offload the encoder to the Neural Engine for faster,
    lower-power transcription on M-series Macs.
</div>
```

---

### End-to-End User Flow (macOS)

```
1. First launch → Setup Wizard
       └── Step 3 shows "CoreML encoders available for Apple Silicon"

2. Open Settings → Downloads
       └── CoreML Encoders section visible (macOS only)
       └── Each row shows: model name, size, Download button

3. User clicks Download on "Small (English) CoreML Encoder"
       └── Rust downloads ggml-small.en-encoder.mlmodelc.zip (~163 MB)
       └── Extracts → models/ggml-small.en-encoder.mlmodelc/
       └── Deletes the zip

4. User also downloads "Small (English)" Whisper model
       └── Rust downloads ggml-small.en.bin (~466 MB)

5. User selects Whisper → Small English in main UI
       └── whisper.cpp loads ggml-small.en.bin
       └── Detects ggml-small.en-encoder.mlmodelc/ alongside it
       └── Loads CoreML encoder onto the Neural Engine
       └── Inference: encoder on ANE ⚡, decoder on CPU

6. User records speech → transcript appears
       └── Encoder: ~2–4× faster, ~50% less power vs CPU
       └── User experience: identical output, noticeably snappier
```

---

### Summary of Files Changed for CoreML

| File | Change |
|------|--------|
| `src-tauri/Cargo.toml` | Added `zip = "0.6"` crate; added `features = ["coreml"]` to macOS whisper-rs |
| `src-tauri/src/commands/misc.rs` | Added `get_platform()` command |
| `src-tauri/src/lib.rs` | Registered `get_platform` in `invoke_handler!` |
| `src-tauri/src/commands/model_registry.rs` | Added 10 CoreML encoder model entries |
| `src-tauri/src/commands/downloader.rs` | Zip extraction after download; directory-aware status check and delete |
| `src/components/settings/types.ts` | Added `'CoreML'` type, `macosOnly` flag, and 10 CoreML model entries |
| `src/components/settings/DownloadsTab.tsx` | Platform detection via `get_platform`; macOS-only CoreML section |
| `src/components/SetupWizard.tsx` | CoreML callout note in Engines step |
| `src/components/SetupWizard.css` | Styles for `.engines-coreml-note` and `.engines-coreml-badge` |

---

---

## ⌨️ Customizable Global Hotkey

### Overview

Taurscribe listens for a global keyboard shortcut to start and stop recording from any application — without the user switching windows. Originally hardcoded to `Ctrl+Win`, the hotkey is now fully user-configurable: up to 2 keys held simultaneously, chosen from modifiers and function keys, persisted across restarts.

---

### The Data Type: `HotkeyBinding`

**`src-tauri/src/types.rs`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HotkeyBinding {
    pub keys: Vec<String>,  // 1 or 2 key codes, e.g. ["ControlLeft", "MetaLeft"]
}

impl Default for HotkeyBinding {
    fn default() -> Self {
        HotkeyBinding { keys: vec!["ControlLeft".to_string(), "MetaLeft".to_string()] }
    }
}
```

Key codes use the same naming convention as the browser's `KeyboardEvent.code` property (`"ControlLeft"`, `"ShiftLeft"`, `"F9"`, etc.). This means the same string that the frontend captures from a `keydown` event is what Rust stores and matches against — no translation layer needed.

---

### Shared State: The Arc<Mutex<>> Bridge

The hotkey binding lives in `AudioState` as a shared reference:

**`src-tauri/src/state.rs`**

```rust
pub struct AudioState {
    // ... other fields ...
    pub hotkey_config: Arc<Mutex<HotkeyBinding>>,
}
```

The key design decision is **sharing the same `Arc`** between two parties:

1. The `set_hotkey` Tauri command (called by the frontend when the user saves a new hotkey)
2. The background hotkey listener thread

```
Frontend saves new hotkey
         │
         ▼
invoke("set_hotkey", { binding: { keys: ["ShiftLeft", "F9"] } })
         │
         ▼
Rust: *state.hotkey_config.lock().unwrap() = new_binding;
         │
         ▼        (same Arc pointer, shared memory)
         ▼
Listener thread: config_c.lock().unwrap().clone()
         │
         ▼
Immediately matches new combo on next keypress ✓
```

No thread restart, no channel message, no polling — the listener reads the current config on every single keystroke via the mutex.

---

### The Listener Thread

**`src-tauri/src/hotkeys/listener.rs`**

The listener is spawned once at app startup and runs for the entire app lifetime:

```rust
// lib.rs — setup closure
let hotkey_config = app.state::<AudioState>().hotkey_config.clone(); // clone the Arc
let app_handle = app.handle().clone();
std::thread::spawn(move || {
    hotkeys::start_hotkey_listener(app_handle, hotkey_config);
});
```

Inside the listener, `rdev::listen()` calls a callback for every OS-level keyboard event. The callback:

1. **Clones the current config** from the mutex at the top of each event (cheap — just a Vec of 1–2 strings)
2. **Maps the rdev `Key` enum to a code string** via `key_to_code()`
3. **Tracks which configured keys are currently held** in a `Vec<String>`
4. **Fires start** when all configured keys are simultaneously held
5. **Fires stop** when any configured key is released while recording is active

```rust
let callback = move |event: Event| {
    let config = config_c.lock().unwrap().clone(); // read current binding

    match event.event_type {
        EventType::KeyPress(key) => {
            if let Some(code) = key_to_code(&key) {
                let mut held = held_keys_c.lock().unwrap();
                if config.keys.contains(&code.to_string()) && !held.contains(&code.to_string()) {
                    held.push(code.to_string());
                }
                // All required keys held? → start recording
                let all_held = config.keys.iter().all(|k| held.contains(k));
                if all_held && !config.keys.is_empty() && !recording_active_c.load(...) {
                    recording_active_c.store(true, ...);
                    let _ = app_c.emit("hotkey-start-recording", ());
                }
            }
        }
        EventType::KeyRelease(key) => {
            if let Some(code) = key_to_code(&key) {
                held_keys_c.lock().unwrap().retain(|k| k != code);
                // A configured key released while recording? → stop
                if recording_active_c.load(...) && config.keys.contains(&code.to_string()) {
                    recording_active_c.store(false, ...);
                    let _ = app_c.emit("hotkey-stop-recording", ());
                }
            }
        }
        _ => {}
    }
};
```

#### Key → code mapping

`rdev`'s `Key` enum uses variants like `Key::ControlLeft`, `Key::F9`, etc. These are mapped to strings by `key_to_code()`:

```rust
fn key_to_code(key: &Key) -> Option<&'static str> {
    match key {
        Key::ControlLeft  => Some("ControlLeft"),
        Key::MetaLeft     => Some("MetaLeft"),    // Windows key / Cmd
        Key::ShiftLeft    => Some("ShiftLeft"),
        Key::Alt          => Some("AltLeft"),
        Key::F9           => Some("F9"),
        // ... F1–F12, CapsLock, Escape, Tab, all modifier variants
        _ => None,  // unmapped keys are silently ignored
    }
}
```

Keys that return `None` (letter keys, number keys, etc.) are completely ignored by the hotkey system — they pass through to the active application untouched.

---

### The `set_hotkey` and `get_hotkey` Commands

**`src-tauri/src/commands/settings.rs`**

```rust
#[tauri::command]
pub fn get_hotkey(state: State<AudioState>) -> HotkeyBinding {
    state.hotkey_config.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_hotkey(state: State<AudioState>, binding: HotkeyBinding) -> Result<(), String> {
    *state.hotkey_config.lock().unwrap() = binding;
    Ok(())
}
```

`set_hotkey` writes through the `Arc` to the same memory the listener thread reads. The change is atomic from the listener's perspective — it either sees the old binding or the new one, never a partial write.

---

### Persistence: The Store

The binding is saved to `settings.json` via `@tauri-apps/plugin-store` so it survives app restarts.

**On save (frontend `GeneralTab.tsx`):**
```ts
await invoke('set_hotkey', { binding });          // update listener immediately
const store = await Store.load('settings.json');
await store.set('hotkey_binding', binding);       // persist to disk
await store.save();
```

**On startup (frontend `App.tsx`):**
```ts
const savedHotkey = await loadedStore.get<{ keys: string[] }>('hotkey_binding');
if (savedHotkey?.keys?.length) {
    invoke('set_hotkey', { binding: savedHotkey }).catch(() => {});
}
```

This runs inside the main startup `useEffect`, right after the store is loaded. The listener starts with the default `Ctrl+Win` binding and is updated to the saved binding within milliseconds of app launch — before the user could realistically trigger a recording.

---

### The Frontend Hotkey Recorder

The UI lives in **`src/components/settings/GeneralTab.tsx`** inside the Settings modal → General tab.

**States:**
- `currentBinding` — the active binding, shown as key chips
- `recording` — whether capture mode is active
- `heldKeys` — keys currently pressed (live feedback)
- `pendingKeys` — the last confirmed combo (persists after release, used for Save)

**Capture flow:**

```
User clicks "Change"
        │
        ▼
recording = true
Window-level keydown/keyup listeners attached (capture phase)
        │
        ▼
User presses e.g. Shift + F9
  keydown "ShiftLeft" → heldKeys = ["ShiftLeft"], pendingKeys = ["ShiftLeft"]
  keydown "F9"        → heldKeys = ["ShiftLeft","F9"], pendingKeys = ["ShiftLeft","F9"]
        │
        ▼
UI shows: [Shift] [F9]   with a Save button (enabled)
        │
        ▼
User releases keys
  keyup → heldKeys clears, but pendingKeys stays ["ShiftLeft","F9"]
        │
        ▼
User clicks Save
  invoke("set_hotkey", { binding: { keys: ["ShiftLeft","F9"] } })
  store.set("hotkey_binding", ...) + store.save()
  currentBinding updated, recording mode exits, "Saved ✓" flashes
```

**Why capture phase (`true` as third argument to `addEventListener`)?**

Using the capture phase intercepts events before they reach the modal's own inputs and buttons. This prevents keys like `Tab`, `Escape`, or `F11` from triggering browser/Tauri default behaviors while the user is recording a hotkey.

```ts
window.addEventListener('keydown', onKeyDown, true);  // capture = true
window.addEventListener('keyup',   onKeyUp,   true);
```

**Key limits:**
- Maximum 2 keys (enforced in `onKeyDown` with `if (heldRef.current.length >= 2) return`)
- Only keys in `ALLOWED_KEYS` are accepted (the same set that `key_to_code()` handles in Rust)
- Regular letter/number keys are silently ignored, preventing accidental bindings that would interfere with typing

---

### Complete Data Flow: From UI to Listener

```
┌─────────────────────────────────────────────────────────────────┐
│                      SETTINGS MODAL                             │
│  GeneralTab: User holds [Ctrl] + [F9], clicks Save             │
└─────────────────────┬───────────────────────────────────────────┘
                      │ invoke("set_hotkey", { keys: [...] })
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│                    RUST COMMAND LAYER                           │
│  set_hotkey() → *state.hotkey_config.lock() = new_binding      │
└─────────────────────┬───────────────────────────────────────────┘
                      │ Arc<Mutex<HotkeyBinding>> (shared pointer)
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│                   LISTENER THREAD (rdev)                        │
│  Every keypress: config = hotkey_config.lock().clone()          │
│  Checks if all config.keys are held                             │
│  Emits "hotkey-start-recording" / "hotkey-stop-recording"       │
└─────────────────────┬───────────────────────────────────────────┘
                      │ Tauri event
                      ▼
┌─────────────────────────────────────────────────────────────────┐
│                    FRONTEND (App.tsx)                           │
│  listen("hotkey-start-recording") → handleStartRecording()      │
│  listen("hotkey-stop-recording")  → handleStopRecording()       │
└─────────────────────────────────────────────────────────────────┘
```

---

### Summary of Files Changed

| File | Change |
|------|--------|
| `src-tauri/src/types.rs` | Added `HotkeyBinding` struct with `Default` impl (Ctrl+Win) |
| `src-tauri/src/state.rs` | Added `hotkey_config: Arc<Mutex<HotkeyBinding>>` to `AudioState` |
| `src-tauri/src/hotkeys/listener.rs` | Full rewrite: accepts shared Arc, dynamic key matching via `key_to_code()`, tracks held keys |
| `src-tauri/src/commands/settings.rs` | Added `get_hotkey` and `set_hotkey` Tauri commands |
| `src-tauri/src/lib.rs` | Clones `hotkey_config` Arc and passes it to the listener; registers new commands |
| `src/App.tsx` | Loads saved binding from store on startup and calls `set_hotkey` |
| `src/components/settings/GeneralTab.tsx` | Hotkey recorder UI with capture-phase event listeners, chip display, Save/Cancel |

---

## Section 21: UI Sound Effects

Taurscribe plays short audio cues to give the user tactile feedback without needing to watch the screen. Three WAV files live in `src/assets/sounds/` and are bundled by Vite at build time.

| File | When it plays |
|---|---|
| `recStart.wav` | Recording starts successfully |
| `paste.wav` | Transcription completes and `type_text` is called |
| `error.wav` | Start failure, recording too short (<1.5 s), or stop/processing error |

---

### `src/hooks/useSounds.ts`

A custom React hook that owns the audio pipeline end-to-end.

**Asset loading**

Vite treats static imports of media files (`.wav`, `.mp3`, …) as URL strings:

```ts
import recStartUrl from '../assets/sounds/recStart.wav';
```

`recStartUrl` is a hashed asset URL like `/assets/recStart-abc123.wav`. Three `HTMLAudioElement` objects are created once in a `useEffect` on mount and stored in refs so they are never recreated on re-render.

**Volume and mute**

Both values live in React state (for the UI) *and* in `useRef` (so async callbacks always read the current value without stale closures):

```ts
const volumeRef = useRef(0.7);
const mutedRef  = useRef(false);
```

When `setVolume` or `setMuted` is called it updates both the ref and the state simultaneously, then persists to `settings.json` via `@tauri-apps/plugin-store`.

**Play function**

```ts
const play = (audio: HTMLAudioElement | null) => {
    if (!audio || mutedRef.current) return;
    audio.currentTime = 0;   // rewind so rapid triggers work
    audio.volume = volumeRef.current;
    audio.play().catch(() => {});   // ignore autoplay policy rejections
};
```

Resetting `currentTime` before play means that if the user starts recording quickly twice in a row the sound still fires each time.

**Persistence**

On mount the hook reads `sound_volume` and `sound_muted` from `settings.json`. On every change it writes back:

```ts
Store.load('settings.json').then(store => {
    store.set('sound_volume', v);
    store.save();
});
```

---

### Integration with `useRecording`

`useSounds` is instantiated in `App.tsx` and three callbacks (`playStart`, `playPaste`, `playError`) are passed into `useRecording` as optional props:

```ts
const { playStart, playPaste, playError, ... } = useSounds();

useRecording({ ..., playStart, playPaste, playError });
```

Inside `useRecording`:

| Trigger point | Sound |
|---|---|
| After `invoke("start_recording")` succeeds | `playStart()` |
| After `invoke("type_text", ...)` succeeds | `playPaste()` |
| Recording start throws | `playError()` |
| Duration < `MIN_RECORDING_MS` (1500 ms) | `playError()` |
| `stop_recording` processing throws | `playError()` |

The props are optional (`playStart?: () => void`) so the hook can be used without sounds if needed.

**Recording session timeline — when each sound fires:**

```
 User presses hotkey / REC button
          │
          ▼
 ┌─────────────────────────────────────────────────────────────────────────────────┐
 │                         handleStartRecording()                                  │
 │                                                                                 │
 │  Engine ready? ──No──▶ show error status ──────────────────────▶ 🔴 error.wav  │
 │       │                                                                         │
 │      Yes                                                                        │
 │       │                                                                         │
 │  invoke("start_recording") ──Err──▶ show error status ─────────▶ 🔴 error.wav  │
 │       │                                                                         │
 │      Ok                                                                         │
 │       │                                                                         │
 │       └──────────────────────────────────────────────────────── 🟢 recStart.wav│
 └─────────────────────────────────────────────────────────────────────────────────┘
          │
          │  (mic is live — audio flowing to threads)
          │
 User releases hotkey / presses STOP
          │
          ▼
 ┌─────────────────────────────────────────────────────────────────────────────────┐
 │                         handleStopRecording()                                   │
 │                                                                                 │
 │  Duration < 1500 ms? ──Yes──▶ "Recording too short" ──────────▶ 🔴 error.wav  │
 │       │                                                                         │
 │      No                                                                         │
 │       │                                                                         │
 │  invoke("stop_recording") + spell check + grammar LLM                          │
 │       │                                                                         │
 │  invoke("type_text", finalTranscript)                                           │
 │       │                    │                                                    │
 │      Ok ───────────────────┘ ──────────────────────────────── 🟡 paste.wav     │
 │      Err ──────────────────────────────────────────────────── 🔴 error.wav     │
 └─────────────────────────────────────────────────────────────────────────────────┘
```

---

### Hook Wiring Diagram

```
                          App.tsx
                             │
              ┌──────────────┴──────────────────────┐
              │                                     │
        useSounds()                         useRecording(...)
              │                                     │
     ┌────────┴────────┐                   receives as props:
     │                 │                   ┌─────────────────┐
  volume            playStart ────────────▶│   playStart?    │
  muted             playPaste ────────────▶│   playPaste?    │
  setVolume         playError ────────────▶│   playError?    │
  setMuted                                 └────────┬────────┘
     │                                              │
     │                                     called at runtime
     ▼                                              │
SettingsModal                              start_recording OK → playStart()
     │                                     type_text OK       → playPaste()
  GeneralTab                               any error          → playError()
     │
  [ Sound Effects card ]
    mute button  → setMuted()
    volume slider → setVolume()
         │
         ▼
   volumeRef / mutedRef (updated immediately)
         │
         ▼
   settings.json  ← persisted on every change
```

---

### Settings UI

The sound controls live in `GeneralTab.tsx` as a new card above the hotkey section.

**Mute toggle button** — a styled `<button>` that calls `setSoundMuted(!soundMuted)`. It renders green "On" or red "Muted" with inline speaker SVG icons.

**Volume slider** — a native `<input type="range" min={0} max={1} step={0.01}>`. It is `disabled` and dimmed (`opacity: 0.4`) when muted. A percentage label (`Math.round(soundVolume * 100)%`) updates live.

```
┌────────────────────────────────────────────────────────────────────┐
│  Sound Effects                                      [ 🔊 On      ] │
│  Plays audio feedback on recording start, paste, and error         │
│                                                                    │
│  🔈  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━●━━━━━━━━  🔊   70%       │
└────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────────┐
│  Sound Effects                                      [ 🔇 Muted   ] │
│  Plays audio feedback on recording start, paste, and error         │
│                                                                    │
│  🔈  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  🔊   70%  (dim) │
└────────────────────────────────────────────────────────────────────┘
```

---

### Complete Data Flow

```
 ┌─────────────────────────────────────────────────────────────────────┐
 │                           App.tsx                                   │
 │                                                                     │
 │  useSounds() ────────────────────────────────────────────────────┐  │
 │    │ returns: playStart, playPaste, playError, volume, muted     │  │
 │    │                                                              │  │
 │    ├──▶ useRecording({                                            │  │
 │    │       ...otherProps,                                         │  │
 │    │       playStart,   ◀─── called after start_recording OK     │  │
 │    │       playPaste,   ◀─── called after type_text OK           │  │
 │    │       playError,   ◀─── called on any failure               │  │
 │    │    })                                                        │  │
 │    │                                                              │  │
 │    └──▶ <SettingsModal                                            │  │
 │              soundVolume={volume}   ─────────────────────────────┘  │
 │              soundMuted={muted}                                      │
 │              setSoundVolume={setVolume}                              │
 │              setSoundMuted={setMuted}                                │
 │          />                                                          │
 └───────────────────────────┬─────────────────────────────────────────┘
                             │
                   ┌─────────▼─────────┐
                   │   SettingsModal   │
                   └─────────┬─────────┘
                             │
                   ┌─────────▼─────────┐
                   │    GeneralTab     │
                   │                   │
                   │  volume slider ───┼──▶ setSoundVolume(v)
                   │  mute button  ───┼──▶ setSoundMuted(!m)
                   └───────────────────┘
                                           │
                             ┌─────────────┴──────────────┐
                             ▼                            ▼
                      volumeRef.current          settings.json
                      mutedRef.current           "sound_volume": v
                       (immediate)               "sound_muted": m
                                                  (persisted)
```

---

### Summary of Files Changed

| File | Change |
|---|---|
| `src/assets/sounds/recStart.wav` | Plays on recording start |
| `src/assets/sounds/paste.wav` | Plays on successful paste |
| `src/assets/sounds/error.wav` | Plays on error or too-short recording |
| `src/hooks/useSounds.ts` | New hook: audio loading, volume/mute state, persistence |
| `src/hooks/useRecording.ts` | Added `playStart?`, `playPaste?`, `playError?` params; calls at trigger points |
| `src/App.tsx` | Instantiates `useSounds`, passes callbacks to `useRecording` and props to `SettingsModal` |
| `src/components/SettingsModal.tsx` | Added sound props; forwards to `GeneralTab` |
| `src/components/settings/GeneralTab.tsx` | New Sound Effects card: mute button + volume slider |

---

## Section 22: Microphone Selection

By default Taurscribe records from whatever the OS considers the system default microphone. This section adds a persistent **Input Device** preference so users can pin a specific mic — a USB headset, a virtual audio cable, or a dedicated audio interface — without changing the OS default.

---

### State (`src-tauri/src/state.rs`)

A single new field is added to `AudioState`:

```rust
pub selected_input_device: Mutex<Option<String>>,
```

- `None` — use the cpal system default (backward-compatible default)
- `Some("Elgato Wave:3")` — open that specific device by name

The value is a plain `String` because cpal identifies devices by their display name (e.g. `"Microphone (USB Audio Device)"`), which is what the OS exposes.

---

### Platform Audio Backend Diagram

`cpal::default_host()` picks the right OS audio API automatically:

```
 ┌──────────────────────────────────────────────────────────────────┐
 │                       cpal::default_host()                       │
 └───────────────────────────┬──────────────────────────────────────┘
                             │
          ┌──────────────────┼──────────────────┐
          │                  │                  │
          ▼                  ▼                  ▼
   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
   │   Windows   │   │   macOS     │   │   Linux     │
   │             │   │             │   │             │
   │   WASAPI    │   │  CoreAudio  │   │    ALSA     │
   └──────┬──────┘   └──────┬──────┘   └──────┬──────┘
          │                  │                  │
          └──────────────────┼──────────────────┘
                             │
                    ┌────────▼────────┐
                    │ host.input_     │
                    │ devices()       │
                    │ (lazy iterator) │
                    └────────┬────────┘
                             │
                    .filter_map(|d| d.name().ok())
                    (silently skips unreadable devices)
                             │
                    ┌────────▼────────┐
                    │  Vec<String>    │
                    │ ["Mic A",       │
                    │  "Mic B", ...]  │
                    └─────────────────┘
```

---

### Rust Commands

#### `list_input_devices` (`commands/misc.rs`)

Enumerates every device the system exposes as a recording source:

```rust
#[tauri::command]
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}
```

`cpal::default_host()` returns the platform's primary audio backend (WASAPI on Windows, CoreAudio on macOS, ALSA on Linux). `input_devices()` is a lazy iterator; `.filter_map(|d| d.name().ok())` silently skips any device whose name can't be read (some virtual devices behave this way).

#### `get_input_device` / `set_input_device` (`commands/settings.rs`)

```rust
#[tauri::command]
pub fn get_input_device(state: State<AudioState>) -> Option<String> {
    state.selected_input_device.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_input_device(state: State<AudioState>, name: Option<String>) {
    *state.selected_input_device.lock().unwrap() = name;
}
```

Passing `None` from the frontend (JavaScript `null`) reverts to the system default. This is safe to call at any time, even while recording — the new value takes effect on the *next* `start_recording` call.

---

### Device Resolution in `start_recording` (`commands/recording.rs`)

The original code always called `host.default_input_device()`. It now checks the preference first:

```rust
let preferred = state.selected_input_device.lock().unwrap().clone();

let device = if let Some(ref name) = preferred {
    // Walk the iterator until we find a device whose name matches exactly.
    host.input_devices()
        .map_err(|e| e.to_string())?
        .find(|d| d.name().ok().as_deref() == Some(name))
        .ok_or_else(|| format!("Input device '{}' not found", name))?
} else {
    host.default_input_device().ok_or("No input device")?
};
```

**Device resolution flowchart:**

```
 start_recording() called
          │
          ▼
 ┌─────────────────────────────────┐
 │  selected_input_device.lock()   │
 │  read preference from state     │
 └────────────────┬────────────────┘
                  │
         ┌────────┴────────┐
         │                 │
      Some(name)          None
         │                 │
         ▼                 ▼
 ┌───────────────┐  ┌──────────────────────┐
 │ host.input_   │  │ host.default_input_  │
 │ devices()     │  │ device()             │
 │ .find(name)   │  └──────────┬───────────┘
 └───────┬───────┘             │
         │                     │
    ┌────┴────┐           ┌────┴────┐
   Found   Missing       Found   None
    │         │            │        │
    ▼         ▼            ▼        ▼
  open     Err(         open     Err(
 stream   "device        stream   "No input
          not found")             device")
          │
          ▼
   shown in header
   status bar — user
   fixes in Settings
```

**Why fail hard if the device is missing?** If we silently fell back to the default, users would record with the wrong mic without knowing. An explicit error is shown in the header status bar ("Error: Input device 'X' not found") and the user can fix it in Settings.

---

### Frontend: `AudioTab` (`src/components/settings/AudioTab.tsx`)

A self-contained component that owns the device-selection UI. It manages its own state rather than lifting it to `App.tsx`, because no other part of the app needs to know which mic is selected at runtime.

**Component state machine:**

```
 ┌─────────────────────────────────────────────────────────────────┐
 │                        AudioTab                                 │
 │                                                                 │
 │  State: devices[], selected, saved, loading                     │
 └──────────────────────┬──────────────────────────────────────────┘
                        │
                        │ useEffect (mount)
                        │
          ┌─────────────┴─────────────┐
          │                           │
          ▼                           ▼
  invoke("list_input_         Store.load("settings.json")
   devices")                  .get("input_device")
          │                           │
          ▼                           ▼
  setDevices([...])          savedDevice found?
  loading = false                     │
                              ┌───────┴───────┐
                              │               │
                             Yes              No
                              │               │
                              ▼               ▼
                       setSelected(name)  setSelected("")
                       invoke("set_       (= system default)
                        input_device",
                        { name })


 ─────────────────────────────────────────────────────

                   User changes <select>
                        │
                        ▼
               handleChange(value)
                        │
           ┌────────────┴────────────┐
           │                         │
    value = ""                value = "Mic B"
    (System Default)                 │
           │                         │
           ▼                         ▼
  invoke("set_input_device",  invoke("set_input_device",
   { name: null })             { name: "Mic B" })
           │                         │
  store.delete(                store.set(
   "input_device")              "input_device", "Mic B")
           │                         │
           └────────────┬────────────┘
                        │
                  store.save()
                        │
                  saved = true
                  (flashes "Saved ✓")
                        │
                setTimeout(2000)
                        │
                  saved = false
```

Passing an empty string from the `<select>` maps to `null` in Rust (`value || null`) which sets `selected_input_device` back to `None`.

---

### App Startup Restore (`src/App.tsx`)

During initial data load, the saved preference is pushed to the backend before the first recording is possible:

```ts
const savedDevice = await loadedStore.get<string>("input_device");
if (savedDevice && !cancelled) {
    invoke("set_input_device", { name: savedDevice }).catch(() => {});
}
```

This ensures the preference is live even before the user opens Settings.

---

### Complete Data Flow

```
 ┌─────────────────────────────────────────────────────────────────────────┐
 │  FRONTEND                          RUST BACKEND                        │
 │                                                                         │
 │  ① Settings opened                                                      │
 │                                                                         │
 │  AudioTab mounts                                                         │
 │    invoke("list_input_devices") ──────────────▶ cpal enumerates OS mics │
 │                                 ◀────────────── ["Mic A","Mic B","Mic C"]│
 │    <select> populated ✓                                                 │
 │                                                                         │
 │    Store.get("input_device")                                             │
 │      → "Mic B" (from last session)                                      │
 │    setSelected("Mic B")                                                  │
 │    invoke("set_input_device",   ──────────────▶ selected_input_device   │
 │      { name: "Mic B" })                          = Some("Mic B")        │
 │                                                                         │
 │ ─────────────────────────────────────────────────────────────────────── │
 │                                                                         │
 │  ② User picks "Mic C"                                                   │
 │                                                                         │
 │    invoke("set_input_device",   ──────────────▶ selected_input_device   │
 │      { name: "Mic C" })                          = Some("Mic C")        │
 │    Store.set("input_device","Mic C")                                    │
 │    "Saved ✓" flashes                                                    │
 │                                                                         │
 │ ─────────────────────────────────────────────────────────────────────── │
 │                                                                         │
 │  ③ App cold-starts next session                                         │
 │                                                                         │
 │  App.tsx loadInitialData()                                               │
 │    Store.get("input_device") → "Mic C"                                  │
 │    invoke("set_input_device",   ──────────────▶ selected_input_device   │
 │      { name: "Mic C" })                          = Some("Mic C")        │
 │                                    (ready before first recording)       │
 │                                                                         │
 │ ─────────────────────────────────────────────────────────────────────── │
 │                                                                         │
 │  ④ User records                                                         │
 │                                                                         │
 │    invoke("start_recording")    ──────────────▶ read selected_input_    │
 │                                                  device → Some("Mic C") │
 │                                                                         │
 │                                                  cpal: search devices   │
 │                                                  "Mic C" found? ─Yes──▶ │
 │                                                    open stream, record  │
 │                                                             └──No──▶   │
 │                                                    Err("device not      │
 │                                 ◀────────────────   found") → show in  │
 │    header: "Error: …not found"                      status bar         │
 └─────────────────────────────────────────────────────────────────────────┘
```

---

### Summary of Files Changed

| File | Change |
|---|---|
| `src-tauri/src/state.rs` | Added `selected_input_device: Mutex<Option<String>>`, initialised to `None` |
| `src-tauri/src/commands/misc.rs` | New `list_input_devices()` — cpal enumeration |
| `src-tauri/src/commands/settings.rs` | New `get_input_device()` / `set_input_device()` |
| `src-tauri/src/commands/recording.rs` | `start_recording()` resolves preferred device before opening stream |
| `src-tauri/src/lib.rs` | Registered three new commands |
| `src/components/settings/AudioTab.tsx` | New component: device list + selector + persistence |
| `src/components/SettingsModal.tsx` | Audio tab now renders `<AudioTab />` instead of placeholder |
| `src/App.tsx` | Restores `input_device` from store on startup |

---

## Next Steps

**To learn more Rust**:
1. [The Rust Book](https://doc.rust-lang.org/book/) — Official, comprehensive
2. [Rust By Example](https://doc.rust-lang.org/rust-by-example/) — Learn by doing
3. [Rustlings](https://github.com/rust-lang/rustlings) — Interactive exercises

**To extend Taurscribe**:
1. Add a new Whisper or Parakeet model variant (edit `model_registry.rs` + `types.ts`)
2. Add a new transcription style to the LLM (edit `format_transcript()` + the style dropdown)
3. Implement speaker diarization (who's speaking)
4. Add export formats (SRT, VTT, plain TXT)
5. Replace energy-based VAD with Silero neural VAD for higher accuracy

**Questions?** Review this guide, check code comments, or explore the Rust documentation!
