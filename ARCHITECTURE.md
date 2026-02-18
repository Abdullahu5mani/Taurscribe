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
12. [File & Function Reference](#file--function-reference)
13. [Common Beginner Questions](#common-beginner-questions)

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
  - ✨ **LLM** - Grammar correction with SmolLM2
  - 🔤 **Spell Check** - Catch any spelling mistakes

**Key Features**:
- ✅ Real-time transcription while you speak (see words appear as you talk!)
- ✅ High-quality final transcript when you stop
- ✅ GPU acceleration for blazing speed (uses your graphics card!)
- ✅ Two AI engines to choose from (Whisper or Parakeet)
- ✅ Multiple models for each engine (pick small & fast or large & accurate)
- ✅ Voice Activity Detection (automatically skips silence)
- ✅ Grammar correction with local LLM
- ✅ Spell checking for final polish
- ✅ Model download manager (download models from within the app)

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
      │                    │    App.tsx - 340 lines  │           Display
      │                    │    =====================│              ▲
      │                    │    • Recording buttons  │              │
      │                    │    • Model selection    │              │
      │                    │    • Settings modal     │              │
      │                    │    • Transcription view │              │
      │                    │    (logic in hooks/)    │              │
      │                    └────────────┬────────────┘              │
     │                                 │                           │
     │                    Tauri IPC Bridge (JavaScript ↔ Rust)     │
     │                                 │                           │
     ▼                    ┌────────────▼────────────┐              │
  ════════════            │     BACKEND (Rust)      │              │
  │ Microphone │─────────►│     lib.rs - 131 lines  │──────────────┘
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
     │  ~600 lines     │    │  ~270 lines     │    │  ~280 lines     │
     └─────────────────┘    └─────────────────┘    └─────────────────┘
               │                        │
               │              ┌─────────────────┐
               │              │parakeet_loaders │
               │              │(GPU/CPU loaders)│
               │              │  ~220 lines     │
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
| **Context** | Manual (we provide previous text) | Automatic (built-in state) |
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
│ Remove every 3rd sample (simplified)                                 │
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
│ Save transcript for next chunk                                       │
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
│  │          SmolLM2 (135M parameters)           │               │
│  │                                               │               │
│  │  System: "Fix grammar errors. Output only    │               │
│  │           the corrected text."               │               │
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
taurscribe-runtime/models/llm/
├── config.json         ← Model architecture (576 hidden, 30 layers)
├── tokenizer.json      ← Vocabulary (49,152 tokens)
└── model.safetensors   ← Weights (~270 MB, 135M parameters)
```

### 🔄 LLM Processing Flow

```
Text Input │
           ▼
┌─────────────────────┐
│ Build ChatML Prompt │  ← "<|im_start|>system\nFix errors...<|im_end|>"
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Tokenize            │  ← "Hello wrold" → [15339, 9923, 820]
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Neural Network      │  ← 30 transformer layers
│ Forward Pass        │     Attention → MLP → Repeat
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Sample Next Token   │  ← Pick from 49,152 possibilities
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Repeat Until        │  ← Stop when <|im_end|> generated
│ <|im_end|> Token    │
└──────────┬──────────┘
           ▼
┌─────────────────────┐
│ Decode to Text      │  ← [15496, 995] → "Hello world"
└──────────┬──────────┘
           ▼
Corrected Text Output
```

### ⚠️ Gotcha: LLM Memory Usage

**Common Mistake**: "The app is using too much RAM!"

**Answer**: SmolLM2 uses ~500MB-1GB RAM when loaded. If this is too much:
1. The LLM is loaded on-demand (only when you click "Correct Grammar")
2. GPU acceleration reduces CPU memory usage
3. Consider closing other memory-heavy applications

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

The spell checker:
1. Loads a dictionary of known words
2. Splits input text into words
3. For each unknown word, finds closest matches
4. Suggests or auto-corrects based on edit distance

### ⚠️ Gotcha: Technical Terms

**Common Mistake**: "It keeps marking my technical terms as misspelled!"

**Solution**: Technical terms (like "Taurscribe", "ONNX", "CUDA") may not be in the dictionary. The system is designed to be conservative - it won't auto-correct words it's unsure about.

---

## 📥 Model Downloads

### 📦 Package Delivery Analogy

```
┌─────────────────────────────────────────────────────────────────┐
│                    📦 MODEL DOWNLOAD SYSTEM                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1️⃣ USER REQUEST                                               │
│     Click "Download" button for ggml-base.en-q5_0.bin           │
│                │                                                │
│                ▼                                                │
│  2️⃣ DOWNLOAD MANAGER (commands/downloader.rs)                  │
│     • Get model URL (Hugging Face CDN)                          │
│     • Calculate expected SHA-1 hash                             │
│     • Start async download                                       │
│                │                                                │
│                ▼                                                │
│  3️⃣ PROGRESS TRACKING                                          │
│     • Track bytes downloaded                                    │
│     • Emit progress events to frontend                          │
│     • Handle network errors/retries                             │
│                │                                                │
│                ▼                                                │
│  4️⃣ VERIFICATION                                               │
│     • Calculate SHA-1 of downloaded file                        │
│     • Compare with expected hash                                 │
│     • Delete if mismatch (corrupted download)                   │
│                │                                                │
│                ▼                                                │
│  5️⃣ COMPLETION                                                 │
│     • Move to models directory                                  │
│     • Notify frontend                                            │
│     • Model ready to use!                                        │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 📁 Available Models

| Model | Size | SHA-1 Hash (first 7 chars) | Speed/Accuracy |
|-------|------|---------------------------|----------------|
| **tiny.en-q5_1** | ~30 MB | ... | ⚡⚡⚡⚡⚡ / ⭐⭐ |
| **base.en-q5_0** | ~53 MB | ... | ⚡⚡⚡⚡ / ⭐⭐⭐ |
| **small.en** | ~465 MB | ... | ⚡⚡⚡ / ⭐⭐⭐⭐ |
| **large-v3-turbo** | ~547 MB | ... | ⚡⚡ / ⭐⭐⭐⭐⭐ |
| **large-v3** | ~2.9 GB | ... | ⚡ / ⭐⭐⭐⭐⭐ |

### ⚠️ Gotcha: Download Verification

**Common Mistake**: "The model downloaded but won't load!"

**Answer**: The download might be corrupted. The downloader:
1. Checks SHA-1 hash after download
2. Deletes corrupted files automatically
3. You'll see an error message if verification fails

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
│                                                                 │
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
│  FRONTEND (App.tsx)                                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  <button onClick={() => invoke("start_recording")}>             │
│     Start Recording                                              │
│  </button>                                                       │
│                                                                  │
│      │                                                           │
│      │  Tauri invoke() call                                     │
│      ▼                                                           │
│                                                                  │
│  BACKEND (commands/recording.rs)                                 │
│                                                                  │
│  #[tauri::command]                                               │
│  pub fn start_recording(state: State<AudioState>) {              │
│      1. Get microphone                                           │
│      2. Create WAV file writer                                   │
│      3. Create audio channels                                    │
│      4. Spawn file writer thread                                 │
│      5. Spawn transcription thread                               │
│      6. Start audio stream                                       │
│  }                                                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 🎤 Phase 2: Audio Capture

```
┌─────────────────────────────────────────────────────────────────┐
│  AUDIO CALLBACK (Every ~10ms)                                    │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  device.build_input_stream(                                      │
│      &config,                                                    │
│      move |data: &[f32], _| {                                   │
│          // Data callback - runs every 10ms!                    │
│                                                                  │
│          // 1. Send to file writer (original quality)           │
│          file_tx.send(data.to_vec()).ok();                      │
│                                                                  │
│          // 2. Convert stereo to mono                            │
│          let mono_data = convert_to_mono(data);                 │
│                                                                  │
│          // 3. Send to transcription thread                      │
│          whisper_tx.send(mono_data).ok();                       │
│      },                                                          │
│      ...                                                         │
│  );                                                              │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 🧠 Phase 3: Transcription

```
┌─────────────────────────────────────────────────────────────────┐
│  TRANSCRIPTION THREAD                                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  loop {                                                          │
│      // 1. Receive audio from channel                            │
│      let samples = rx.recv()?;                                  │
│                                                                  │
│      // 2. Add to buffer                                         │
│      buffer.extend(samples);                                     │
│                                                                  │
│      // 3. Check if enough for a chunk                           │
│      if buffer.len() >= chunk_size {                            │
│                                                                  │
│          // 4. VAD check (Whisper only)                          │
│          if vad.is_speech(&chunk) {                             │
│                                                                  │
│              // 5. Transcribe                                    │
│              let text = engine.transcribe(&chunk)?;             │
│                                                                  │
│              // 6. Send to frontend                              │
│              emit("transcription-chunk", text);                 │
│          }                                                       │
│      }                                                           │
│  }                                                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 🛑 Phase 4: Stop Recording

```
┌─────────────────────────────────────────────────────────────────┐
│  STOP RECORDING                                                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Drop audio stream (stops microphone)                        │
│  2. Drop channels (signals threads to finish)                   │
│  3. Wait for file to finalize                                    │
│  4. Run final transcription on complete file                     │
│  5. (Optional) Run LLM grammar correction                        │
│  6. (Optional) Run spell check                                   │
│  7. Return final transcript to frontend                          │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### ⚠️ Gotcha: Channel Closing

**Common Mistake**: "The app hangs when I stop recording!"

**How it works**: 
1. `drop(file_tx)` closes the sending end of the channel
2. The file writer thread's `rx.recv()` returns `Err`
3. Thread exits its loop and finalizes the file
4. Without `drop()`, the thread would wait forever!

---

## 📐 Module Architecture

### 🗂️ Current File Structure (Updated February 2026)

```
Taurscribe/
├── 🎨 Frontend
│   ├── src/
│   │   ├── App.tsx               # UI assembly only (~340 lines)
│   │   ├── App.css               # Styling
│   │   ├── main.tsx              # React entry point
│   │   │
│   │   ├── hooks/                # Custom React hooks (all logic lives here)
│   │   │   ├── useHeaderStatus.ts    # Temporary status messages (~25 lines)
│   │   │   ├── useModels.ts          # Whisper + Parakeet model lists (~55 lines)
│   │   │   ├── usePostProcessing.ts  # LLM + SymSpell toggle logic (~75 lines)
│   │   │   ├── useEngineSwitch.ts    # Engine switching + model loading (~210 lines)
│   │   │   └── useRecording.ts       # Recording state + post-processing (~185 lines)
│   │   │
│   │   └── components/
│   │       └── settings/         # Settings modal sub-components
│   │           ├── SettingsModal.tsx  # Modal shell + tab routing (~220 lines)
│   │           ├── GeneralTab.tsx     # LLM + Spell Check toggles (~90 lines)
│   │           ├── DownloadsTab.tsx   # Model library list (~120 lines)
│   │           ├── ModelRow.tsx       # Single model row with actions (~130 lines)
│   │           └── types.ts           # Shared types + MODELS constant (~125 lines)
│   │
│   └── index.html                # HTML shell
│
├── 🦀 Backend (Rust)
│   └── src-tauri/
│       ├── src/
│       │   ├── 🎯 Core
│       │   │   ├── lib.rs              # App entry + module declarations (131 lines)
│       │   │   ├── main.rs             # Binary entry (6 lines)
│       │   │   ├── types.rs            # Shared types (~30 lines)
│       │   │   ├── state.rs            # AudioState (~68 lines)
│       │   │   ├── utils.rs            # Helpers (models dir, etc.) (~62 lines)
│       │   │   └── audio.rs            # Audio primitives (~24 lines)
│       │   │
│       │   ├── 🎤 Audio Processing
│       │   │   ├── whisper.rs          # Whisper AI manager (~600 lines)
│       │   │   ├── parakeet.rs         # Parakeet manager + transcription (~270 lines)
│       │   │   ├── parakeet_loaders.rs # GPU/CPU loader helpers (~220 lines)
│       │   │   └── vad.rs              # Voice Activity Detection (~280 lines)
│       │   │
│       │   ├── ✨ Post-Processing
│       │   │   ├── llm.rs              # Grammar correction LLM (~200 lines)
│       │   │   └── spellcheck.rs       # SymSpell spell checker (~150 lines)
│       │   │
│       │   ├── 📡 Commands (Tauri API)
│       │   │   └── commands/
│       │   │       ├── mod.rs              # Module exports
│       │   │       ├── model_registry.rs   # Model configs + get_model_config() (~260 lines)
│       │   │       ├── downloader.rs       # HTTP download/verify/delete (~230 lines)
│       │   │       ├── models.rs           # Whisper model management
│       │   │       ├── settings.rs         # Engine config commands
│       │   │       ├── recording.rs        # Start/stop recording (~380 lines)
│       │   │       ├── llm.rs              # LLM commands
│       │   │       ├── spellcheck.rs       # Spell check commands
│       │   │       └── misc.rs             # Utility commands
│       │   │
│       │   ├── 🖼️ System Tray
│       │   │   └── tray.rs / tray/         # Tray setup + icons
│       │   │
│       │   ├── ⌨️ Global Hotkeys
│       │   │   └── hotkeys.rs / hotkeys/   # Ctrl+Win listener
│       │   │
│       │   └── 👁️ File Watcher
│       │       └── watcher.rs              # Models directory watcher
│       │
│       ├── build.rs              # Build script
│       └── Cargo.toml            # Rust dependencies
│
├── 📦 Runtime Assets
│   └── taurscribe-runtime/
│       ├── models/               # AI models (.bin, .onnx, .gguf)
│       │   ├── llm/              # Grammar LLM files
│       │   ├── spellcheck/       # SymSpell dictionary
│       │   └── parakeet-*/       # Parakeet ONNX model folders
│       └── samples/              # Test audio (.wav)
│
└── 📚 Documentation
    ├── ARCHITECTURE.md           # This file!
    └── README.md
```

### 🏗️ Module Dependency Diagram

```
┌──────────────────────────────────────────────────────┐
│                   lib.rs (top)                       │  ← Entry point, declares all modules
├──────────────────────────────────────────────────────┤
│  commands/   tray   hotkeys   watcher                │  ← Feature modules
├──────────────────────────────────────────────────────┤
│  whisper   parakeet   vad   llm   spellcheck         │  ← AI engines
│                 │                                    │
│          parakeet_loaders                            │  ← Loader helpers (used by parakeet)
├──────────────────────────────────────────────────────┤
│  commands/model_registry   commands/downloader       │  ← Download subsystem
│  (registry has no deps)    (uses registry + utils)   │
├──────────────────────────────────────────────────────┤
│  types   state   utils   audio                       │  ← Core (no dependencies)
└──────────────────────────────────────────────────────┘

Rule: Lower modules NEVER depend on higher modules!

Frontend hook dependency order:
  useHeaderStatus  ←  (no deps)
  useModels        ←  useHeaderStatus
  usePostProcessing←  useHeaderStatus
  useEngineSwitch  ←  useModels, useHeaderStatus
  useRecording     ←  useEngineSwitch, usePostProcessing, useHeaderStatus
  App.tsx          ←  all hooks
```

### ⚠️ Gotcha: Circular Dependencies

**Common Mistake**: "I added `use crate::commands` to `whisper.rs` and it won't compile!"

**Solution**: Lower-level modules (like `whisper.rs`) should NEVER import from higher-level modules (like `commands/`). Instead:
- Put shared types in `types.rs`
- Put shared utilities in `utils.rs`
- Let the higher-level module import from lower-level ones

---

## File & Function Reference

### 🔍 Quick Lookup Table

| I want to... | Go to | Function/Section |
|-------------|-------|------------------|
| Add a new Tauri command | `commands/*.rs` | Create function with `#[tauri::command]` |
| Change recording behavior | `commands/recording.rs` | `start_recording()`, `stop_recording()` |
| Modify Whisper logic | `whisper.rs` | `transcribe_chunk()`, `transcribe_file()` |
| Modify Parakeet transcription | `parakeet.rs` | `transcribe_chunk()`, `initialize()` |
| Change how Parakeet loads GPU/CPU | `parakeet_loaders.rs` | `init_*()`, `try_gpu_*()`, `try_cpu_*()` |
| Add a new downloadable model | `commands/model_registry.rs` | Add entry to `get_model_config()` |
| Change download/verify logic | `commands/downloader.rs` | `download_model()`, `verify_model_hash()` |
| Change LLM behavior | `llm.rs` | `generate_correction()` |
| Change spell check | `spellcheck.rs` | Correction logic |
| Modify tray icon | `tray.rs` | `setup_tray()` |
| Change hotkey | `hotkeys.rs` | Key detection logic |
| Add shared type | `types.rs` | Define struct/enum |
| Add utility function | `utils.rs` | Create public function |
| Change UI recording logic | `src/hooks/useRecording.ts` | `handleStartRecording()`, `handleStopRecording()` |
| Change engine switching UI | `src/hooks/useEngineSwitch.ts` | `handleSwitchToWhisper()`, `handleSwitchToParakeet()` |
| Add a new model to the download UI | `src/components/settings/types.ts` | Add entry to `MODELS` array |
| Change settings tab layout | `src/components/settings/` | `GeneralTab.tsx`, `DownloadsTab.tsx` |

### 📋 All Tauri Commands (as of February 2026)

```rust
// From lib.rs invoke_handler (matches tauri::generate_handler! exactly):
commands::greet,                   // Test/greeting
commands::start_recording,         // Start mic + transcription
commands::stop_recording,          // Stop + get final transcript
commands::get_backend_info,        // Get GPU backend info
commands::list_models,             // List Whisper models
commands::get_current_model,       // Get active Whisper model
commands::switch_model,            // Switch Whisper model
commands::list_parakeet_models,    // List Parakeet models
commands::init_parakeet,           // Initialize Parakeet model
commands::get_parakeet_status,     // Check Parakeet status
commands::set_active_engine,       // Switch Whisper/Parakeet
commands::get_active_engine,       // Get active engine
commands::set_tray_state,          // Update tray icon
commands::init_llm,                // Initialize LLM
commands::unload_llm,              // Unload LLM to free memory
commands::run_llm_inference,       // Run raw LLM inference
commands::check_llm_status,        // Check if LLM loaded
commands::correct_text,            // Grammar correction
commands::type_text,               // Type text via Enigo (keyboard injection)
commands::init_spellcheck,         // Initialize spell checker
commands::unload_spellcheck,       // Unload spell checker
commands::check_spellcheck_status, // Check spell checker status
commands::correct_spelling,        // Fix spelling errors
commands::download_model,          // Download model file (from model_registry)
commands::get_download_status,     // Get per-model download status
commands::delete_model,            // Delete model file(s)
commands::verify_model_hash,       // Verify model SHA-1 integrity
```

> **Note**: `benchmark_test` and `list_sample_files` were removed in the January 2026 cleanup.
> `unload_llm`, `unload_spellcheck`, and `type_text` were added in the same pass.

---

## Common Beginner Questions

### Q1: Why are there two transcription engines?

**Answer**: Different use cases need different trade-offs:
- **Whisper** - Higher accuracy, 6-second latency → Best for recordings
- **Parakeet** - Lower latency, slightly less accurate → Best for real-time

### Q2: Can I use this for other languages?

**Yes!** Change the language in settings. Whisper supports 99 languages. Parakeet currently focuses on English.

### Q3: How much RAM does this use?

| Component | RAM Usage |
|-----------|-----------|
| Whisper (tiny) | ~100 MB |
| Whisper (base) | ~200 MB |
| Whisper (large) | ~3 GB |
| Parakeet | ~500 MB |
| LLM (SmolLM2) | ~500 MB |
| Audio buffer | ~10 MB |

### Q4: Why does the first transcription take longer?

**Answer**: GPU "warm-up"! The first run compiles CUDA/Vulkan kernels. Taurscribe does a warm-up pass during initialization to avoid this delay during actual use.

### Q5: What if my recording crashes?

**Safety features**:
1. WAV file is saved continuously (won't lose audio)
2. File location: `AppData/Local/Taurscribe/temp/`
3. Console shows real-time transcription (check logs)

---

## Conclusion

Taurscribe demonstrates modern Rust practices:

✅ **Ownership** - Threads take ownership of data they need  
✅ **Borrowing** - Functions borrow without taking ownership  
✅ **Concurrency** - Multiple threads work safely in parallel  
✅ **Error Handling** - `Result` and `?` operator for safety  
✅ **Modularity** - Clean separation into focused modules  

**Architecture Benefits**:

| Feature | Benefit |
|---------|---------|
| Separate threads | UI never freezes |
| Channels | Safe thread communication |
| Arc<Mutex<T>> | Shared state protection |
| Two AI engines | Speed OR accuracy |
| GPU acceleration | 12-60x faster processing |
| Modular commands | Easy to extend |

**Key Takeaway**: Rust's strict compiler prevents entire categories of bugs. Once your code compiles, it usually works correctly!

---

## Next Steps

**To learn more Rust**:
1. [The Rust Book](https://doc.rust-lang.org/book/) - Official, comprehensive
2. [Rust By Example](https://doc.rust-lang.org/rust-by-example/) - Learn by doing
3. [Rustlings](https://github.com/rust-lang/rustlings) - Interactive exercises

**To extend Taurscribe**:
1. Add a new AI model variant
2. Implement speaker diarization (who's speaking)
3. Add keyboard shortcuts (already has Ctrl+Win)
4. Implement real-time subtitle overlay
5. Add export formats (SRT, VTT, TXT)

**Questions?** Review this guide, check code comments, or explore the Rust documentation!
