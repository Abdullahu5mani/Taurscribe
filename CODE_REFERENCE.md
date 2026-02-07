# Taurscribe Complete Code Reference

> **Learning Order**: This guide is organized bottom-up. Start from Level 1 (simplest files with no dependencies) and work your way up. By the time you reach Level 5, you'll understand how everything connects!

---

## Table of Contents

- [Level 1: Foundation](#level-1-foundation) - Core building blocks
- [Level 2: AI Engines](#level-2-ai-engines) - The brains
- [Level 3: Commands](#level-3-commands) - Tauri API
- [Level 4: Features](#level-4-features) - Tray & Hotkeys
- [Level 5: Entry Point](#level-5-entry-point) - Where it all begins

---

# Level 1: Foundation

These files have **zero dependencies** on other project files. They're the building blocks everything else uses.

---

## 📄 `main.rs` - The True Entry Point

**Location**: `src-tauri/src/main.rs`  
**Lines**: 7  
**Purpose**: The very first code that runs when you start the app.

### 🏠 Analogy: The Front Door

`main.rs` is like the **front door** of a house. It doesn't contain any furniture or rooms - it just lets you in and points you to the living room (`lib.rs`).

### 📝 Complete Code

```rust
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    taurscribe_lib::run()
}
```

### 🧠 Rust Concepts Explained

#### 1. `#![cfg_attr(...)]` - Conditional Compilation Attribute

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
```

**What it does**: This line tells Windows to hide the console window, but ONLY in release builds.

**Breaking it down**:
- `#![...]` - An "inner attribute" that applies to the whole file
- `cfg_attr(condition, attribute)` - "If condition is true, apply this attribute"
- `not(debug_assertions)` - True when building in release mode (`cargo build --release`)
- `windows_subsystem = "windows"` - Tells Windows "this is a GUI app, no console needed"

**Why?**: During development (`cargo run`), you WANT the console to see `println!` debug messages. In production, users don't need to see that.

#### 2. `taurscribe_lib::run()`

```rust
fn main() {
    taurscribe_lib::run()
}
```

**What's happening**:
- `taurscribe_lib` is the crate name (defined in `Cargo.toml` as `name = "taurscribe-lib"`)
- `::run()` calls the `run` function from `lib.rs`

**Why separate main.rs and lib.rs?**: 
- `main.rs` = Binary entry point (what gets executed)
- `lib.rs` = Library code (can be tested, reused, shared)

### ⚠️ Gotcha

**Never delete the `#![cfg_attr...]` line!** Without it, your release build will show an ugly console window behind your app.

---

## 📄 `types.rs` - Shared Data Structures

**Location**: `src-tauri/src/types.rs`  
**Lines**: 31  
**Purpose**: Defines data types used across the entire application.

### 🎯 Analogy: The Dictionary

`types.rs` is like a **dictionary** that defines what words mean. When code elsewhere says "AppState", everyone agrees on what that means because it's defined here.

### 📝 Complete Code with Explanations

```rust
/// Defines the possible states of our application
/// This helps us decide which icon to show in the tray
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    Ready,      // Green: Waiting for user input
    Recording,  // Red: Mic is active, recording audio
    Processing, // Yellow: Computing/Transcribing
}
```

### 🧠 Rust Concepts Explained

#### 1. `enum` - Enumerations

```rust
pub enum AppState {
    Ready,
    Recording,
    Processing,
}
```

**What it is**: An enum is a type that can be ONE of several variants. Think of it like a dropdown menu - you can only pick one option.

**Why use enum instead of strings?**:
```rust
// ❌ Bad: Typos possible, no validation
let state = "recordingg";  // Oops, typo!

// ✅ Good: Compiler catches mistakes
let state = AppState::Recordingg;  // Error! Variant doesn't exist
```

#### 2. `#[derive(...)]` - Auto-Implementing Traits

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
```

**What it does**: Automatically implements common functionality:

| Trait | What it gives you | Example |
|-------|-------------------|---------|
| `Debug` | Can print with `{:?}` | `println!("{:?}", state);` |
| `Clone` | Can make copies with `.clone()` | `let copy = state.clone();` |
| `Copy` | Auto-copies (no `.clone()` needed) | `let copy = state;` |
| `PartialEq` | Can compare with `==` | `if state == AppState::Ready` |

#### 3. `serde::Serialize` - JSON Conversion

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct TranscriptionChunk {
    pub text: String,
    pub processing_time_ms: u32,
    pub method: String,
}
```

**Why?**: Tauri sends data between Rust and JavaScript as JSON. `Serialize` automatically converts your struct to JSON:

```rust
TranscriptionChunk {
    text: "Hello".to_string(),
    processing_time_ms: 150,
    method: "Whisper".to_string(),
}
// Becomes: {"text":"Hello","processing_time_ms":150,"method":"Whisper"}
```

### 📊 All Types Defined

| Type | Purpose | Used By |
|------|---------|---------|
| `AppState` | Track app status (Ready/Recording/Processing) | Tray icons, UI state |
| `ASREngine` | Which AI engine (Whisper/Parakeet) | Engine selection |
| `TranscriptionChunk` | Live transcription data | Real-time UI updates |
| `SampleFile` | Test audio file info | Benchmark feature |

---

## 📄 `audio.rs` - Audio Stream Wrappers

**Location**: `src-tauri/src/audio.rs`  
**Lines**: 17  
**Purpose**: Makes audio streams safe to use across threads.

### 🚂 Analogy: The Train Track Adapter

Audio streams from `cpal` (the audio library) are like trains that can only run on one track. `audio.rs` provides an **adapter** that lets the train safely switch tracks (threads).

### 📝 Complete Code

```rust
use crossbeam_channel::Sender;

// Wrapper struct to make the Audio Stream "moveable" between threads.
#[allow(dead_code)]
pub struct SendStream(pub cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

/// Keeps track of the tools needed while recording.
pub struct RecordingHandle {
    pub stream: SendStream,
    pub file_tx: Sender<Vec<f32>>,
    pub whisper_tx: Sender<Vec<f32>>,
}
```

### 🧠 Rust Concepts Explained

#### 1. `unsafe impl Send` and `Sync` - Thread Safety Markers

```rust
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}
```

**The problem**: Rust prevents data races by default. `cpal::Stream` isn't marked as thread-safe, so you can't move it between threads.

**The solution**: We're telling Rust "trust me, I know what I'm doing":
- `Send` = "This can be MOVED to another thread"
- `Sync` = "This can be ACCESSED from multiple threads"
- `unsafe` = "I'm bypassing the compiler's safety checks"

**Why is this safe?**: We only access the stream from one place at a time (start recording, then stop recording). We never share it simultaneously.

#### 2. Newtype Pattern

```rust
pub struct SendStream(pub cpal::Stream);
```

**What it is**: A struct with a single unnamed field. Written as `StructName(Type)`.

**Why?**: We can't add traits to types we don't own (`cpal::Stream` is from another crate). Wrapping it in our own type lets us add `Send` and `Sync`.

#### 3. `Sender<Vec<f32>>` - Channel Endpoints

```rust
pub file_tx: Sender<Vec<f32>>,
pub whisper_tx: Sender<Vec<f32>>,
```

**What they are**: These are "pipes" to send audio data to other threads:
- `file_tx` sends audio to the file-writing thread
- `whisper_tx` sends audio to the transcription thread

**How channels work**:
```
Audio Callback Thread                Other Threads
        │                                  │
        │ tx.send(audio_data)              │
        │ ─────────────────────────────►   │ rx.recv()
        │                                  │
```

### ⚠️ Gotcha

**Never remove `unsafe impl Send/Sync`!** Without them, you'll get compiler errors like:
```
error: `cpal::Stream` cannot be sent between threads safely
```

---

## 📄 `utils.rs` - Helper Functions

**Location**: `src-tauri/src/utils.rs`  
**Lines**: 64  
**Purpose**: Standalone utility functions used throughout the app.

### 🧰 Analogy: The Toolbox

`utils.rs` is like a **toolbox** - it contains helpful tools that any part of the app can grab and use.

### 📝 Function: `clean_transcript`

```rust
/// Simple Post-Processing to clean up raw ASR artifacts
pub fn clean_transcript(text: &str) -> String {
    let mut cleaned = text.trim().to_string();

    // Fix floating punctuation
    cleaned = cleaned.replace(" ,", ",");
    cleaned = cleaned.replace(" .", ".");
    cleaned = cleaned.replace(" ?", "?");
    cleaned = cleaned.replace(" !", "!");
    cleaned = cleaned.replace(" %", "%");

    // Fix double spaces
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    // Capitalize first letter
    if let Some(first) = cleaned.chars().next() {
        if first.is_lowercase() {
            let mut c = cleaned.chars();
            cleaned = match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            };
        }
    }

    cleaned
}
```

**What it does**: Cleans up messy AI output:
- `" ,"` → `","` (fix floating commas)
- `"  "` → `" "` (fix double spaces)
- `"hello"` → `"Hello"` (capitalize first letter)

### 📝 Function: `get_recordings_dir`

```rust
pub fn get_recordings_dir() -> Result<std::path::PathBuf, String> {
    let app_data = dirs::data_local_dir()
        .ok_or("Could not find AppData directory")?;
    
    let recordings_dir = app_data.join("Taurscribe").join("temp");
    
    std::fs::create_dir_all(&recordings_dir)
        .map_err(|e| format!("Failed to create directory: {}", e))?;

    Ok(recordings_dir)
}
```

### 🧠 Rust Concepts Explained

#### 1. `Result<T, E>` - Error Handling

```rust
pub fn get_recordings_dir() -> Result<std::path::PathBuf, String>
```

**What it is**: A Result is either `Ok(value)` or `Err(error)`. It forces you to handle errors.

```rust
match get_recordings_dir() {
    Ok(path) => println!("Directory: {}", path.display()),
    Err(e) => println!("Error: {}", e),
}
```

#### 2. `?` Operator - Error Propagation

```rust
let app_data = dirs::data_local_dir().ok_or("Error message")?;
```

**What it does**: If the value is `Err`, immediately return that error. Otherwise, unwrap the `Ok` value.

**Equivalent to**:
```rust
let app_data = match dirs::data_local_dir() {
    Some(path) => path,
    None => return Err("Error message".to_string()),
};
```

#### 3. `PathBuf` vs `Path`

| Type | Ownership | Analogy |
|------|-----------|---------|
| `PathBuf` | Owned (like `String`) | Your own notebook |
| `&Path` | Borrowed (like `&str`) | Looking at someone's notebook |

```rust
let owned: PathBuf = PathBuf::from("/home/user");  // You own it
let borrowed: &Path = owned.as_path();              // Just a reference
```

---

## 📄 `state.rs` - Global Application State

**Location**: `src-tauri/src/state.rs`  
**Lines**: 60  
**Purpose**: Holds all data that lives for the entire app lifetime.

### 🧠 Analogy: The Control Room

`state.rs` is like the **control room** of a factory. It has monitors for every department (Whisper, Parakeet, VAD, etc.) and switches to control them.

### 📝 Complete Code

```rust
pub struct AudioState {
    pub recording_handle: Mutex<Option<RecordingHandle>>,
    pub whisper: Arc<Mutex<WhisperManager>>,
    pub parakeet: Arc<Mutex<ParakeetManager>>,
    pub vad: Arc<Mutex<VADManager>>,
    pub last_recording_path: Mutex<Option<String>>,
    pub current_app_state: Mutex<AppState>,
    pub active_engine: Mutex<ASREngine>,
    pub session_transcript: Arc<Mutex<String>>,
    pub llm: Arc<Mutex<Option<crate::llm::LLMEngine>>>,
    pub spellcheck: Arc<Mutex<Option<SpellChecker>>>,
}
```

### 🧠 Rust Concepts Explained

#### 1. `Mutex<T>` - Mutual Exclusion Lock

```rust
pub recording_handle: Mutex<Option<RecordingHandle>>
```

**The problem**: Multiple threads might try to read/write the same data simultaneously = data corruption!

**The solution**: A Mutex is like a **bathroom lock**. Only one thread can "lock" it at a time:

```rust
// Thread 1
let mut handle = state.recording_handle.lock().unwrap();
*handle = Some(new_recording);  // Safe! We have the lock
// Lock automatically released when `handle` goes out of scope

// Thread 2 (waits until Thread 1 releases)
let handle = state.recording_handle.lock().unwrap();
```

#### 2. `Arc<T>` - Atomic Reference Counting

```rust
pub whisper: Arc<Mutex<WhisperManager>>
```

**The problem**: We need to share WhisperManager across multiple threads, but Rust ownership says only ONE owner.

**The solution**: `Arc` (Atomic Reference Counter) lets multiple owners share the same data:

```rust
let shared = Arc::new(Mutex::new(WhisperManager::new()));

// Clone creates another owner (not a copy of data!)
let clone1 = Arc::clone(&shared);  // Now 2 owners
let clone2 = Arc::clone(&shared);  // Now 3 owners
// All point to the SAME WhisperManager

// When all owners are dropped, the data is freed
```

#### 3. `Arc<Mutex<T>>` Pattern

**Why both?**:
- `Arc` = Share ownership across threads
- `Mutex` = Prevent simultaneous access

```
Thread 1                    Thread 2
    │                           │
    │  Arc::clone()             │  Arc::clone()
    │      │                    │      │
    ▼      ▼                    ▼      ▼
┌──────────────────────────────────────────┐
│  Arc<Mutex<WhisperManager>>              │
│  ┌────────────────────────────────────┐  │
│  │  Mutex (only 1 thread at a time)  │  │
│  │  ┌──────────────────────────────┐  │  │
│  │  │     WhisperManager           │  │  │
│  │  └──────────────────────────────┘  │  │
│  └────────────────────────────────────┘  │
└──────────────────────────────────────────┘
```

#### 4. `Option<T>` - Nullable Values

```rust
pub llm: Arc<Mutex<Option<crate::llm::LLMEngine>>>
```

**Why Option?**: The LLM isn't loaded immediately - it's loaded on-demand. `Option` represents "might exist, might not":
- `Some(engine)` = LLM is loaded
- `None` = LLM not loaded yet

### ⚠️ Gotcha: Deadlocks

**Never hold two locks at once if possible!**

```rust
// ❌ Bad: Can deadlock if another thread locks in opposite order
let whisper = state.whisper.lock().unwrap();
let parakeet = state.parakeet.lock().unwrap();

// ✅ Good: Release first lock before getting second
let result = {
    let whisper = state.whisper.lock().unwrap();
    whisper.transcribe(...)
};  // Lock released here
let parakeet = state.parakeet.lock().unwrap();
```

---

# Level 1 Complete! ✅

You now understand the **foundation** of Taurscribe:
- `main.rs` - Entry point
- `types.rs` - Shared data types
- `audio.rs` - Thread-safe audio wrappers
- `utils.rs` - Helper functions
- `state.rs` - Global state management

**Next**: Level 2 covers the AI engines (VAD, Whisper, Parakeet, LLM, Spellcheck).

---

# Level 2: AI Engines

These are the "brains" of the application. They depend on Level 1 types but do the actual heavy lifting.

```
┌─────────────────────────────────────────────────────────────┐
│                    AI Engines Overview                       │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Audio Input                                                │
│      │                                                      │
│      ▼                                                      │
│  ┌───────────┐                                              │
│  │    VAD    │ ◄── "Is anyone speaking?"                    │
│  └─────┬─────┘                                              │
│        │ Yes                                                │
│        ▼                                                    │
│  ┌───────────┐     ┌────────────┐                           │
│  │  Whisper  │ OR  │  Parakeet  │ ◄── "What did they say?"  │
│  └─────┬─────┘     └──────┬─────┘                           │
│        │                  │                                 │
│        └────────┬─────────┘                                 │
│                 ▼                                           │
│           Raw Transcript                                    │
│                 │                                           │
│                 ▼                                           │
│  ┌───────────┐     ┌────────────┐                           │
│  │    LLM    │ AND │ SpellCheck │ ◄── "Fix the grammar"     │
│  └─────┬─────┘     └──────┬─────┘                           │
│        │                  │                                 │
│        └────────┬─────────┘                                 │
│                 ▼                                           │
│          Clean Transcript                                   │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

---

## 📄 `vad.rs` - Voice Activity Detection

**Location**: `src-tauri/src/vad.rs`  
**Lines**: 215  
**Purpose**: Detect when someone is speaking vs. silence.

### 🚦 Analogy: The Traffic Light Sensor

VAD is like those **sensors at traffic lights** that detect if a car is waiting. Instead of detecting cars, it detects voices:
- 🔴 Silence = "Don't transcribe, save CPU"
- 🟢 Speech = "Someone's talking! Wake up Whisper!"

### 📊 Structure Overview

```rust
pub struct VADManager {
    threshold: f32,  // Volume level that counts as "speech" (0.005 default)
}
```

### 📝 Function: `new()` - Constructor

```rust
pub fn new() -> Result<Self, String> {
    // Try to find Silero model (future AI-based VAD)
    if let Ok(models_dir) = Self::get_models_dir() {
        let vad_model_path = models_dir.join("silero_vad.onnx");
        if vad_model_path.exists() {
            println!("[VAD] Found Silero VAD model");
        }
    }
    
    // For now, use simple energy-based detection
    Ok(Self { threshold: 0.005 })
}
```

**Why 0.005?**: This threshold was tuned through testing. Too low = background noise triggers it. Too high = quiet speech is missed.

### 📝 Function: `is_speech()` - Core Detection

```rust
pub fn is_speech(&mut self, audio: &[f32]) -> Result<f32, String> {
    // Calculate RMS (Root Mean Square) - a measure of "loudness"
    let sum_squares: f32 = audio.iter().map(|&x| x * x).sum();
    let rms = (sum_squares / audio.len() as f32).sqrt();

    // Convert to probability (0.0 = silence, 1.0 = speech)
    let prob = if rms < self.threshold {
        0.0  // Too quiet
    } else if rms > self.threshold * 5.0 {
        1.0  // Very loud
    } else {
        // In between - calculate proportionally
        ((rms - self.threshold) / (self.threshold * 4.0)).min(1.0)
    };

    Ok(prob)
}
```

### 🧠 Rust Concepts Explained

#### 1. Iterator Chain: `map` and `sum`

```rust
let sum_squares: f32 = audio.iter().map(|&x| x * x).sum();
```

**Breaking it down**:
```rust
audio.iter()           // Create iterator over the slice
     .map(|&x| x * x)  // Transform each value: square it
     .sum()            // Add all squared values together
```

**The `|&x|` syntax**: This is a closure (anonymous function). The `&x` means "dereference the reference":
```rust
// audio is &[f32], so iter() gives &f32 references
// |&x| destructures the reference to get f32 value
|&x| x * x  // x is f32, not &f32
```

#### 2. `&[f32]` - Slice Reference

```rust
pub fn is_speech(&mut self, audio: &[f32]) -> Result<f32, String>
```

**What is `&[f32]`?**: A "slice" - a reference to a contiguous sequence of f32 values. It's like borrowing a portion of an array:

```rust
let full_array: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0, 5.0];
let slice: &[f32] = &full_array[1..4];  // [2.0, 3.0, 4.0]
```

**Why use slices?**: You can pass any contiguous data without copying:
- A `Vec<f32>`
- A portion of a `Vec<f32>`
- A fixed-size array `[f32; 100]`

### 📝 Function: `get_speech_timestamps()` - Advanced Segmentation

```rust
pub fn get_speech_timestamps(
    &mut self,
    audio: &[f32],
    padding_ms: usize,
) -> Result<Vec<(f32, f32)>, String>
```

**What it does**: Scans through audio and returns a list of (start_time, end_time) pairs where speech was detected.

**Key constants**:
```rust
const SAMPLE_RATE: f32 = 16000.0;      // 16kHz (standard for AI)
const FRAME_SIZE: usize = 512;         // Check every 32ms
const MIN_SPEECH_FRAMES: usize = 5;    // Need 150ms minimum to count
```

### 🧠 Rust Concept: State Machine with `match`

```rust
match (is_speech, speech_start) {
    (true, None) => {
        // NEW: Started speaking
        speech_start = Some(i);
    }
    (true, Some(_)) => {
        // CONTINUE: Still speaking
        consecutive_speech_frames += 1;
    }
    (false, Some(_)) => {
        // PAUSE: Stopped (maybe temporarily)
        silence_frames += 1;
        if silence_frames > padding_frames {
            // Long pause - end the segment
            segments.push((start_time, end_time));
            speech_start = None;
        }
    }
    (false, None) => {
        // IDLE: Still silent
    }
}
```

**Why tuple matching?**: It elegantly handles all 4 combinations of (currently speaking?, were we speaking?).

### ⚠️ Gotcha

**The current VAD is simple (energy-based), not AI-based**. It can be fooled by:
- Loud background noise (fan, typing) = false positives
- Quiet whispers = false negatives

Future improvement: Use the Silero ONNX model for neural VAD.

---

## 📄 `llm.rs` - Grammar Correction LLM

**Location**: `src-tauri/src/llm.rs`  
**Lines**: 199  
**Purpose**: Use a local AI model to fix grammar in transcripts.

### 🎓 Analogy: The Proofreader

The LLM is like a **proofreader** who reads your rough draft and fixes:
- Grammar mistakes ("he go" → "he goes")
- Punctuation ("hello how are you" → "Hello, how are you?")
- Capitalization

### 📊 Structure Overview

```rust
pub struct LLMEngine {
    model: model::ModelWeights,   // The AI brain (neural network weights)
    tokenizer: Tokenizer,         // Converts text ↔ numbers
    device: Device,               // CPU or GPU
    eos_token_id: u32,           // "End of sentence" token ID
    newline_token_id: u32,       // Newline character token ID
}
```

### 📝 Function: `new()` - Load the Model

```rust
pub fn new() -> Result<Self> {
    let base_path = PathBuf::from(
        r"c:\Users\abdul\OneDrive\Desktop\Taurscribe\taurscribe-runtime\models\GRMR-V3-G1B-GGUF"
    );
    let model_path = base_path.join("GRMR-V3-G1B-Q4_K_M.gguf");
    let tokenizer_path = base_path.join("tokenizer.json");
    
    // Force CPU for now
    let device = Device::Cpu;
    
    // Load tokenizer (text → numbers)
    let tokenizer = Tokenizer::from_file(&tokenizer_path)?;
    
    // Load model weights (GGUF format)
    let mut file = std::fs::File::open(&model_path)?;
    let content = gguf_file::Content::read(&mut file)?;
    let model = model::ModelWeights::from_gguf(content, &mut file, &device)?;
    
    Ok(Self { model, tokenizer, device, eos_token_id, newline_token_id })
}
```

### 🧠 Rust Concepts Explained

#### 1. `anyhow::Result` - Simplified Error Handling

```rust
use anyhow::{Error, Result};

pub fn new() -> Result<Self>  // Same as Result<Self, anyhow::Error>
```

**What is anyhow?**: A crate that makes error handling easier:
- Any error type can be converted to `anyhow::Error`
- You don't need to define your own error types
- Great for applications (not libraries)

```rust
// Without anyhow - must handle each error type
fn load() -> Result<Model, Box<dyn std::error::Error>>

// With anyhow - simpler
fn load() -> anyhow::Result<Model>
```

#### 2. Raw String Literals

```rust
let base_path = PathBuf::from(
    r"c:\Users\abdul\OneDrive\Desktop\Taurscribe\..."
);
```

**The `r"..."` prefix**: Raw string - backslashes are NOT escape characters.

```rust
// Without raw string - must escape backslashes
let path = "c:\\Users\\abdul\\OneDrive";

// With raw string - cleaner
let path = r"c:\Users\abdul\OneDrive";
```

### 📝 Function: `run()` - Generate Corrected Text

```rust
pub fn run(&mut self, prompt: &str) -> Result<String> {
    // Format prompt for GRMR model
    let formatted_prompt = format!("text\n{}\ncorrected\n", prompt.trim());
    
    // Tokenize (text → numbers)
    let tokens = self.tokenizer.encode(formatted_prompt, true)?;
    
    // Run through neural network
    let input = Tensor::new(tokens.as_slice(), &self.device)?;
    let logits = self.model.forward(&input, 0)?;
    
    // Sample next token
    let next_token = logits_processor.sample(&logits)?;
    
    // Generation loop
    for i in 0..max_gen_tokens {
        if next_token == self.eos_token_id { break; }
        // ... generate more tokens ...
    }
    
    // Decode back to text
    let decoded = self.tokenizer.decode(&generated_tokens, true)?;
    Ok(decoded)
}
```

### 🧠 LLM Concepts Explained

#### Tokenization

```
"Hello world" → [15496, 995] → AI processes → [15496, 11, 995] → "Hello, world"
     Text         Tokens                         Tokens           Corrected
```

#### The Generation Loop

```rust
for i in 0..max_gen_tokens {
    // Stop if EOS (end of sentence) token
    if next_token == self.eos_token_id { break; }
    
    // Feed current token → get next token prediction
    let input = Tensor::new(&[next_token], &self.device)?;
    let logits = self.model.forward(&input, pos)?;
    next_token = logits_processor.sample(&logits)?;
    
    generated_tokens.push(next_token);
    pos += 1;
}
```

**What's happening**: The model predicts one token at a time. Each new token becomes input for predicting the next.

### ⚠️ Gotcha

**LLM uses CPU only** currently. For speed, consider:
1. Smaller quantized models (Q4 instead of Q8)
2. GPU acceleration (CUDA support in candle)

---

## 📄 `spellcheck.rs` - Spell Checking

**Location**: `src-tauri/src/spellcheck.rs`  
**Lines**: 140  
**Purpose**: Fast spell correction using SymSpell algorithm.

### 📖 Analogy: The Dictionary Lookup

SpellCheck is like having a **pocket dictionary** with every English word. For each word you write, it quickly checks:
1. Is this word in the dictionary?
2. If not, what's the closest match?

### 📊 Structure Overview

```rust
pub struct SpellChecker {
    symspell: SymSpell<UnicodeStringStrategy>,  // The spell-check engine
}
```

### 📝 Function: `new()` - Load Dictionary

```rust
pub fn new() -> Result<Self> {
    let dict_path = PathBuf::from(
        r"...\frequency_dictionary_en_82_765.txt"
    );
    
    let mut symspell: SymSpell<UnicodeStringStrategy> = SymSpell::default();
    
    symspell.load_dictionary(
        dict_path.to_str().unwrap(),
        0,   // term_index (which column is the word)
        1,   // count_index (which column is the frequency)
        " "  // separator
    );
    
    Ok(Self { symspell })
}
```

**The dictionary file format**:
```
the 23135851162
of 13151942776
and 12997637966
to 12136980858
...
```

### 📝 Function: `correct()` - Fix Spelling

```rust
pub fn correct(&self, text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut corrected_words = Vec::with_capacity(words.len());
    
    for word in &words {
        // Skip short words, numbers, punctuation
        if word.len() <= 1 { 
            corrected_words.push(word.to_string());
            continue;
        }
        
        // Strip punctuation for lookup
        let (prefix, clean_word, suffix) = strip_punctuation(word);
        
        // Look up in dictionary
        let suggestions = self.symspell.lookup(
            &clean_word.to_lowercase(),
            Verbosity::Closest,
            2  // max edit distance
        );
        
        if let Some(suggestion) = suggestions.first() {
            let corrected = match_case(&suggestion.term, &clean_word);
            corrected_words.push(format!("{}{}{}", prefix, corrected, suffix));
        }
    }
    
    corrected_words.join(" ")
}
```

### 🧠 Rust Concepts Explained

#### 1. `Vec::with_capacity()` - Pre-allocation

```rust
let mut corrected_words = Vec::with_capacity(words.len());
```

**Why?**: Vecs grow by reallocating memory. If we know the size ahead of time, pre-allocate to avoid extra copies:

```rust
// ❌ Slow: Reallocates as it grows
let mut v = Vec::new();
for i in 0..1000 { v.push(i); }

// ✅ Fast: One allocation upfront
let mut v = Vec::with_capacity(1000);
for i in 0..1000 { v.push(i); }
```

#### 2. Generic Type Parameters

```rust
symspell: SymSpell<UnicodeStringStrategy>
```

**What's `<UnicodeStringStrategy>`?**: A type parameter that tells SymSpell how to handle strings. It's like choosing a "mode":
- `UnicodeStringStrategy` - Full Unicode support (emojis, accents)
- `AsciiStringStrategy` - ASCII only (faster but limited)

### 📝 Helper: `match_case()` - Preserve Capitalization

```rust
fn match_case(suggestion: &str, original: &str) -> String {
    if original.chars().all(|c| c.is_uppercase()) {
        // ALL CAPS → ALL CAPS
        suggestion.to_uppercase()
    } else if original.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        // Title Case → Title Case
        let mut chars: Vec<char> = suggestion.chars().collect();
        if let Some(first) = chars.first_mut() {
            *first = first.to_uppercase().next().unwrap_or(*first);
        }
        chars.into_iter().collect()
    } else {
        suggestion.to_lowercase()
    }
}
```

**Examples**:
- `"TSET"` → `"TEST"` (preserve ALL CAPS)
- `"Tset"` → `"Test"` (preserve Title Case)
- `"tset"` → `"test"` (preserve lowercase)

### ⚠️ Gotcha

**SymSpell doesn't understand context**. It might "correct" valid words:
- `"US"` → `"us"` (proper noun vs pronoun)
- `"IT"` → `"it"` (industry term vs pronoun)

---

## 📄 `whisper.rs` - Whisper Transcription Engine

**Location**: `src-tauri/src/whisper.rs`  
**Lines**: 781  
**Purpose**: OpenAI's Whisper model for speech-to-text.

### 🎧 Analogy: The Court Reporter

Whisper is like a **court reporter** who listens to everything said and types it out perfectly. It:
- Understands different accents
- Handles background noise
- Adds punctuation automatically

### 📊 Structure Overview

```rust
pub struct WhisperManager {
    context: Option<WhisperContext>,  // The loaded AI brain
    last_transcript: String,          // Memory of previous text (for context)
    backend: GpuBackend,              // CPU/CUDA/Vulkan
    current_model: Option<String>,    // Which model is loaded
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>,  // Audio converter
}
```

### 📝 Key Functions

#### `new()` - Constructor

```rust
pub fn new() -> Self {
    Self {
        context: None,           // No model loaded yet
        last_transcript: String::new(),
        backend: GpuBackend::Cpu,
        current_model: None,
        resampler: None,
    }
}
```

#### `initialize()` - Load a Model

```rust
pub fn initialize(&mut self, model_id: Option<&str>) -> Result<String, String> {
    // Disable C++ library logs
    unsafe { set_log_callback(Some(null_log_callback), std::ptr::null_mut()); }
    
    // Pick model (default: tiny.en-q5_1)
    let target_model = model_id.unwrap_or("tiny.en-q5_1");
    let file_name = format!("ggml-{}.bin", target_model);
    
    // Try GPU first, fallback to CPU
    let (ctx, backend) = self.try_gpu(&absolute_path)
        .or_else(|_| self.try_cpu(&absolute_path))?;
    
    // Save state
    self.context = Some(ctx);
    self.backend = backend;
    
    // Warm up GPU (first run is always slow)
    let warmup_audio = vec![0.0_f32; 16000];  // 1 second silence
    self.transcribe_chunk(&warmup_audio, 16000)?;
    
    Ok(format!("Backend: {}", backend))
}
```

### 🧠 Rust Concepts Explained

#### 1. `or_else()` - Fallback Pattern

```rust
let (ctx, backend) = self.try_gpu(&path)
    .or_else(|_| self.try_cpu(&path))?;
```

**What it does**: If `try_gpu` returns `Err`, try `try_cpu` instead.

```rust
// Pseudocode:
if try_gpu() succeeds:
    use GPU result
else:
    try CPU instead
```

#### 2. `unsafe` Blocks - FFI with C

```rust
unsafe {
    set_log_callback(Some(null_log_callback), std::ptr::null_mut());
}
```

**Why unsafe?**: Whisper is a C++ library. Rust can't verify C code is correct, so we mark it `unsafe`.

**What we're doing**: Telling the C library to use our custom log function (which does nothing, silencing verbose logs).

### 📝 Function: `transcribe_chunk()` - Real-Time Transcription

```rust
pub fn transcribe_chunk(
    &mut self,
    samples: &[f32],        // Audio data
    input_sample_rate: u32, // e.g., 48000
) -> Result<String, String> {
    // 1. Resample to 16kHz if needed
    let audio_data = if input_sample_rate != 16000 {
        // Use resampler...
    } else {
        samples.to_vec()
    };
    
    // 2. Create state for this task
    let mut state = ctx.create_state()?;
    
    // 3. Configure parameters
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(4);
    params.set_language(Some("en"));
    
    // 4. Feed previous text as context
    if !self.last_transcript.is_empty() {
        params.set_initial_prompt(&self.last_transcript);
    }
    
    // 5. Run AI
    state.full(params, &audio_data)?;
    
    // 6. Extract text
    let num_segments = state.full_n_segments();
    let mut transcript = String::new();
    for i in 0..num_segments {
        if let Some(segment) = state.get_segment(i) {
            transcript.push_str(&segment.to_string());
        }
    }
    
    // 7. Update context memory
    self.last_transcript.push_str(&transcript);
    
    Ok(transcript)
}
```

### 🧠 Why Context Matters

```
Chunk 1: "The"     → Whisper hears "the"
Chunk 2: "cat sat" → With context "The", Whisper knows to continue the sentence

Without context: "The" + "cat sat" = two separate sentences
With context:    "The cat sat" = one flowing transcript
```

### 📝 Function: `transcribe_file()` - Batch Processing

Optimized for processing a complete audio file (not real-time):

```rust
pub fn transcribe_file(&mut self, file_path: &str) -> Result<String, String> {
    // 1. Read WAV file
    let mut reader = hound::WavReader::open(file_path)?;
    
    // 2. Convert stereo to mono
    let mono_samples = if spec.channels == 2 {
        samples.chunks(2)
            .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
            .collect()
    } else {
        samples
    };
    
    // 3. Resample to 16kHz
    // 4. Run with 8 threads (more than real-time)
    params.set_n_threads(8);
    
    // 5. Return full transcript
}
```

### ⚠️ Gotchas

1. **GPU Warm-up**: First transcription is slow (GPU initialization). That's why `initialize()` runs a warmup.

2. **Context Accumulation**: `last_transcript` grows during recording. Call `clear_context()` when starting fresh!

3. **16kHz Requirement**: Whisper ONLY accepts 16kHz audio. Always resample first.

---

## 📄 `parakeet.rs` - Parakeet Transcription Engine

**Location**: `src-tauri/src/parakeet.rs`  
**Lines**: 615  
**Purpose**: NVIDIA's Parakeet models - faster alternative to Whisper.

### 🏎️ Analogy: The Sports Car

If Whisper is a reliable sedan, Parakeet is a **sports car**:
- Much faster (optimized for NVIDIA GPUs)
- Multiple model types for different needs
- Streaming-capable (Nemotron can transcribe as you speak)

### 📊 Model Types

```rust
enum LoadedModel {
    Nemotron(Nemotron),   // Streaming (real-time)
    Ctc(Parakeet),        // CTC (batch)
    Eou(ParakeetEOU),     // End-of-Utterance detection
    Tdt(ParakeetTDT),     // Token-and-Duration Transducer
}
```

| Model | Best For | Speed | Accuracy |
|-------|----------|-------|----------|
| Nemotron | Live streaming | ⚡⚡⚡ | ⭐⭐⭐ |
| CTC | Batch processing | ⚡⚡ | ⭐⭐⭐⭐ |
| TDT | Timestamps | ⚡⚡ | ⭐⭐⭐⭐ |
| EOU | Sentence detection | ⚡⚡ | ⭐⭐⭐ |

### 📊 Structure Overview

```rust
pub struct ParakeetManager {
    model: Option<LoadedModel>,
    model_name: Option<String>,
    backend: GpuBackend,  // Cuda, DirectML, or Cpu
    resampler: Option<(u32, usize, Box<SincFixedIn<f32>>)>,
}
```

### 📝 Function: `initialize()` - Load Model

```rust
pub fn initialize(&mut self, model_id: Option<&str>) -> Result<String, String> {
    // Parse model ID: "nemotron:parakeet-nemotron" or "ctc:parakeet-ctc"
    let (model, backend) = match info.model_type.as_str() {
        "Nemotron" => {
            let (m, b) = Self::init_nemotron(&model_path)?;
            (LoadedModel::Nemotron(m), b)
        }
        "CTC" => {
            let (m, b) = Self::init_ctc(&model_path)?;
            (LoadedModel::Ctc(m), b)
        }
        // ... other types
    };
    
    self.model = Some(model);
    self.backend = backend;
    Ok(format!("Loaded {} ({})", info.display_name, backend))
}
```

### 🧠 Rust Concepts Explained

#### 1. `#[cfg(...)]` - Conditional Compilation

```rust
fn init_nemotron(path: &PathBuf) -> Result<(Nemotron, GpuBackend), String> {
    #[cfg(target_os = "macos")]
    {
        // macOS: Always use CPU
        let m = Self::try_cpu_nemotron(path)?;
        return Ok((m, GpuBackend::Cpu));
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Windows/Linux: Try CUDA, then DirectML, then CPU
        if let Ok(m) = Self::try_gpu_nemotron(path) {
            return Ok((m, GpuBackend::Cuda));
        }
        // ... fallbacks
    }
}
```

**What it does**: Different code compiles for different platforms.

**Why?**: macOS doesn't have CUDA. DirectML is Windows-only. We handle each OS appropriately.

#### 2. `split_once()` - String Parsing

```rust
let subpath = target_id.split_once(':')
    .map(|(_, p)| p)
    .unwrap_or(target_id);
```

**What it does**: Split "nemotron:parakeet-model" into ("nemotron", "parakeet-model"), then take the second part.

### 📝 Function: `transcribe_chunk()` - Transcription

```rust
pub fn transcribe_chunk(
    &mut self,
    samples: &[f32],
    sample_rate: u32,
) -> Result<String, String> {
    // 1. Resample to 16kHz
    let audio = self.resample_if_needed(samples, sample_rate)?;
    
    // 2. Route to correct model type
    match &mut self.model {
        Some(LoadedModel::Nemotron(m)) => {
            // Nemotron: Process in 560ms chunks
            const CHUNK_SIZE: usize = 8960;  // 560ms at 16kHz
            let mut transcript = String::new();
            for chunk in audio.chunks(CHUNK_SIZE) {
                transcript.push_str(&m.transcribe_chunk(&chunk)?);
            }
            Ok(transcript)
        }
        Some(LoadedModel::Ctc(m)) => {
            // CTC: Process entire audio at once
            let result = m.transcribe_samples(audio, 16000, 1, Some(TimestampMode::Words))?;
            Ok(result.text)
        }
        // ... other model types
    }
}
```

### 🧠 Nemotron Streaming Explained

```
Audio Stream: ─────────────────────────────────►
              |  560ms  |  560ms  |  560ms  |

Nemotron:     ├─────────┤
              └──► "Hel"
                        ├─────────┤
                        └──► "lo wor"
                                  ├─────────┤
                                  └──► "ld how"

Result:       "Hello world how..."
```

**Why 560ms?**: Nemotron is optimized for 8960 samples (560ms). Shorter or longer chunks work but are less efficient.

### ⚠️ Gotchas

1. **Model Detection**: Parakeet detects model type by checking for specific files:
   - `encoder.onnx` + `decoder_joint.onnx` = Nemotron or EOU
   - `model.onnx` + `tokenizer.json` = CTC

2. **DirectML Only on Windows**: Don't try to use DirectML on Linux/macOS.

3. **Nemotron State**: Call `clear_context()` between recordings to reset Nemotron's internal state.

---

# Level 2 Complete! ✅

You now understand all 5 AI engines:

| Engine | Purpose | Key Function |
|--------|---------|--------------|
| `vad.rs` | Detect speech | `is_speech()` |
| `whisper.rs` | Transcription (accurate) | `transcribe_chunk()` |
| `parakeet.rs` | Transcription (fast) | `transcribe_chunk()` |
| `llm.rs` | Grammar fixing | `run()` |
| `spellcheck.rs` | Spell correction | `correct()` |

**Rust concepts covered**:
- Iterator methods (`map`, `sum`, `chunks`)
- Slice references (`&[f32]`)
- Pattern matching with tuples
- `anyhow` error handling
- Raw strings (`r"..."`)
- Conditional compilation (`#[cfg]`)
- FFI and `unsafe` blocks

---

# Level 3: Commands

Commands are **Tauri's API layer** - they're functions that JavaScript can call from the frontend. Think of them as the "remote control buttons" for your Rust backend.

```
┌─────────────────────────────────────────────────────────────────┐
│                        COMMANDS OVERVIEW                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  Frontend (React/TypeScript)                                    │
│      │                                                          │
│      │ invoke("start_recording")                                │
│      │ invoke("switch_model", { model_id: "large-v3" })         │
│      │                                                          │
│      ▼                                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   TAURI COMMANDS                        │    │
│  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐     │    │
│  │  │  recording   │ │    models    │ │   settings   │     │    │
│  │  │  .rs         │ │    .rs       │ │   .rs        │     │    │
│  │  └──────────────┘ └──────────────┘ └──────────────┘     │    │
│  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐     │    │
│  │  │     llm      │ │  spellcheck  │ │  downloader  │     │    │
│  │  │     .rs      │ │     .rs      │ │     .rs      │     │    │
│  │  └──────────────┘ └──────────────┘ └──────────────┘     │    │
│  └─────────────────────────────────────────────────────────┘    │
│      │                                                          │
│      ▼                                                          │
│  Level 2: AI Engines (Whisper, Parakeet, LLM, etc.)             │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 📄 `commands/mod.rs` - Module Hub

**Lines**: 21  
**Purpose**: Exports all commands from one place.

```rust
mod llm;
mod misc;
mod models;
mod recording;
mod settings;
mod spellcheck;
mod transcription;

// Re-export everything publicly
pub use llm::*;
pub use misc::*;
pub use models::*;
pub use recording::*;
pub use settings::*;
pub use spellcheck::*;
pub use transcription::*;

// Downloader needs special handling (pub mod instead of mod)
pub mod downloader;
pub use downloader::*;
```

### 🧠 Rust Concepts Explained

#### `mod` vs `pub mod` vs `pub use`

```rust
mod llm;           // Declare llm.rs exists (private to this crate)
pub mod downloader; // Declare AND make the module itself public
pub use llm::*;    // Re-export all public items from llm
```

**Why `pub use`?**: It lets us write `commands::start_recording` instead of `commands::recording::start_recording`.

---

## 📄 `commands/misc.rs` - Test Command

**Lines**: 6  
**Purpose**: Simple test to verify Rust is working.

```rust
#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}
```

### 🧠 Rust Concepts Explained

#### `#[tauri::command]` - The Magic Attribute

This attribute does A LOT of work behind the scenes:

```rust
#[tauri::command]
pub fn greet(name: &str) -> String
```

**What it does automatically**:
1. Makes the function callable from JavaScript
2. Deserializes JSON arguments → Rust types
3. Serializes Rust return value → JSON
4. Handles async/sync automatically

**JavaScript usage**:
```typescript
const result = await invoke("greet", { name: "World" });
// result = "Hello, World! You've been greeted from Rust!"
```

---

## 📄 `commands/settings.rs` - Configuration Commands

**Lines**: 56  
**Purpose**: Engine selection, backend info, tray updates.

### 📝 Commands

| Command | Parameters | Returns | Purpose |
|---------|------------|---------|---------|
| `get_backend_info` | - | `String` | "CUDA", "Vulkan", or "CPU" |
| `set_active_engine` | `engine: String` | `String` | Switch Whisper ↔ Parakeet |
| `get_active_engine` | - | `ASREngine` | Current engine |
| `set_tray_state` | `new_state: String` | `()` | Update tray icon |

### 📝 Function: `set_active_engine()`

```rust
#[tauri::command]
pub fn set_active_engine(
    state: State<AudioState>,  // Tauri injects this
    engine: String,            // From JavaScript
) -> Result<String, String> {
    // Convert string to enum
    let new_engine = match engine.to_lowercase().as_str() {
        "whisper" => ASREngine::Whisper,
        "parakeet" => ASREngine::Parakeet,
        _ => return Err(format!("Unknown engine: {}", engine)),
    };

    // Update state
    *state.active_engine.lock().unwrap() = new_engine;
    
    Ok(format!("Engine switched to {:?}", new_engine))
}
```

### 🧠 Rust Concepts Explained

#### `State<T>` - Tauri's Dependency Injection

```rust
state: State<AudioState>
```

**What it is**: Tauri automatically injects global state into commands.

**How it works**:
1. In `lib.rs`: `.manage(AudioState::new(...))`
2. In commands: `State<AudioState>` parameter
3. Tauri connects them automatically

**Why?**: You don't need to pass state manually - Tauri handles it.

---

## 📄 `commands/models.rs` - Model Management

**Lines**: 61  
**Purpose**: List, switch, and manage AI models.

### 📝 Commands

| Command | Returns | Purpose |
|---------|---------|---------|
| `list_models` | `Vec<ModelInfo>` | All available Whisper models |
| `get_current_model` | `Option<String>` | Currently loaded model |
| `switch_model` | `String` | Load different model |
| `list_parakeet_models` | `Vec<ParakeetModelInfo>` | Parakeet models |
| `init_parakeet` | `String` | Initialize Parakeet |
| `get_parakeet_status` | `ParakeetStatus` | Parakeet status |

### 📝 Function: `switch_model()`

```rust
#[tauri::command]
pub fn switch_model(
    state: State<AudioState>,
    model_id: String,
) -> Result<String, String> {
    // 1. Safety: Can't switch while recording!
    let handle = state.recording_handle.lock().unwrap();
    if handle.is_some() {
        return Err("Cannot switch models while recording".to_string());
    }
    drop(handle);  // Release lock before getting another

    // 2. Load the new model
    let mut whisper = state.whisper.lock().unwrap();
    whisper.initialize(Some(&model_id))
}
```

### 🧠 Rust Concept: `drop()` - Explicit Resource Release

```rust
let handle = state.recording_handle.lock().unwrap();
// ... check something ...
drop(handle);  // Release lock NOW, not at end of function
```

**Why?**: Locks are released when variables go out of scope. But if we need the lock released BEFORE getting another lock (to avoid deadlocks), we `drop()` it explicitly.

---

## 📄 `commands/recording.rs` - THE HEART ❤️

**Lines**: 366  
**Purpose**: Start/stop recording, real-time transcription.

### 🏭 Analogy: The Factory Floor

Recording is like a **factory with three assembly lines**:

```
              ┌─────────────────────────────────────────────────┐
              │              MICROPHONE                         │
              │         (Raw Audio Input)                       │
              └───────────────────┬─────────────────────────────┘
                                  │
                      ┌───────────┴───────────┐
                      │                       │
                      ▼                       ▼
         ┌────────────────────┐   ┌────────────────────┐
         │  ASSEMBLY LINE 1   │   │  ASSEMBLY LINE 2   │
         │  "File Writer"     │   │  "Transcriber"     │
         │                    │   │                    │
         │  Saves audio to    │   │  Sends to AI →     │
         │  .wav file         │   │  Emits text        │
         └────────────────────┘   └────────────────────┘
```

### 📝 Function: `start_recording()` - The Big One

```rust
#[tauri::command]
pub fn start_recording(
    app_handle: AppHandle,
    state: State<AudioState>,
) -> Result<String, String> {
    // 1. Setup Microphone
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No input device")?;
    let config = device.default_input_config()?.into();

    // 2. Prepare Output File
    let filename = format!("recording_{}.wav", chrono::Utc::now().timestamp());
    let path = recordings_dir.join(&filename);

    // 3. Reset AI Context
    state.whisper.lock().unwrap().clear_context();

    // 4. Create Communication Pipes
    let (file_tx, file_rx) = unbounded::<Vec<f32>>();
    let (whisper_tx, whisper_rx) = unbounded::<Vec<f32>>();

    // 5. Spawn Thread 1: File Writer
    std::thread::spawn(move || {
        while let Ok(samples) = file_rx.recv() {
            for sample in samples {
                writer.write_sample(sample).ok();
            }
        }
        writer.finalize().ok();
    });

    // 6. Spawn Thread 2: Real-Time Transcriber
    std::thread::spawn(move || {
        while let Ok(samples) = whisper_rx.recv() {
            // Process in 6-second chunks
            // Check VAD → if speech → transcribe → emit to frontend
        }
    });

    // 7. Build Audio Stream
    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _| {
            file_tx_clone.send(data.to_vec()).ok();      // → File Thread
            whisper_tx_clone.send(mono_data).ok();       // → AI Thread
        },
        |err| eprintln!("Error: {}", err),
        None,
    )?;

    // 8. Start Recording
    stream.play()?;

    Ok("Recording started")
}
```

### 🧠 Rust Concepts Explained

#### 1. Channels for Thread Communication

```rust
let (file_tx, file_rx) = unbounded::<Vec<f32>>();
```

**What it creates**: A sender (`tx`) and receiver (`rx`) pair.

```
Main Thread          File Thread
    │                    │
    │ file_tx.send(data) │
    │ ──────────────────►│ file_rx.recv()
    │                    │
```

**`unbounded`**: No limit on queue size (contrast with `bounded(100)` which blocks after 100 items).

#### 2. `move` Closures

```rust
std::thread::spawn(move || {
    while let Ok(samples) = file_rx.recv() {
        // ...
    }
});
```

**What `move` does**: Transfers ownership of captured variables INTO the closure.

```rust
// Without move:
let x = 5;
std::thread::spawn(|| println!("{}", x));  // ❌ Error: x might be dropped

// With move:
let x = 5;
std::thread::spawn(move || println!("{}", x));  // ✅ x is owned by thread
```

#### 3. The Audio Callback Pattern

```rust
device.build_input_stream(
    &config,
    move |data: &[f32], _info: &_| {
        // This closure is called every few milliseconds with new audio
        file_tx_clone.send(data.to_vec()).ok();
    },
    move |err| { eprintln!("Error: {}", err); },
    None,
)?
```

**Real-time constraint**: This callback runs on a high-priority audio thread. You CANNOT:
- Lock mutexes (might block)
- allocate memory (might block)
- do slow operations

**Solution**: Send data to another thread via channel (non-blocking).

### 📝 Function: `stop_recording()` - Cleanup

```rust
#[tauri::command]
pub fn stop_recording(state: State<AudioState>) -> Result<String, String> {
    let mut handle = state.recording_handle.lock().unwrap();
    
    if let Some(recording) = handle.take() {
        // 1. Drop stream (stops microphone)
        drop(recording.stream);
        
        // 2. Drop senders (signals threads to exit)
        drop(recording.file_tx);
        drop(recording.whisper_tx);
        
        // 3. Wait for threads to finish
        std::thread::sleep(Duration::from_millis(500));
        
        // 4. For Whisper: Run final high-quality transcription
        if active_engine == ASREngine::Whisper {
            let audio_data = whisper.load_audio(&path)?;
            let timestamps = vad.get_speech_timestamps(&audio_data, 500)?;
            // ... transcribe only speech segments ...
        }
        
        // 5. Type out result
        type_out_text(&final_text);
        
        Ok(final_text)
    } else {
        Err("Not recording".to_string())
    }
}
```

### 🧠 Rust Concept: `take()` - Option Extraction

```rust
if let Some(recording) = handle.take() {
```

**What `take()` does**: Removes the value from `Option`, leaving `None`.

```rust
let mut opt = Some(5);
let value = opt.take();  // value = Some(5), opt = None
```

**Why?**: We want to "consume" the recording handle - stop it and release resources.

### ⚠️ Gotchas

1. **Don't lock in audio callback**: Use channels instead.

2. **Clear context on start**: Call `clear_context()` or old text affects new transcription.

3. **Drop order matters**: Drop senders BEFORE waiting for threads.

---

## 📄 `commands/llm.rs` - Grammar Correction Commands

**Lines**: 102  
**Purpose**: Initialize and run the LLM.

### 📝 Commands

| Command | Async? | Purpose |
|---------|--------|---------|
| `init_llm` | ✅ | Load LLM model |
| `run_llm_inference` | ✅ | Raw inference |
| `check_llm_status` | ❌ | Is LLM loaded? |
| `correct_text` | ✅ | Fix grammar |

### 📝 Function: `init_llm()` - Async Command

```rust
#[tauri::command]
pub async fn init_llm(state: State<'_, AudioState>) -> Result<String, String> {
    // Check if already loaded
    {
        let llm_guard = state.llm.lock().unwrap();
        if llm_guard.is_some() {
            return Ok("LLM already initialized".to_string());
        }
    }  // Lock released here

    // Load in blocking task (LLM loading is slow)
    let result = tauri::async_runtime::spawn_blocking(move || {
        LLMEngine::new()
    })
    .await
    .map_err(|e| format!("JoinError: {}", e))?;

    // Store in state
    match result {
        Ok(engine) => {
            *state.llm.lock().unwrap() = Some(engine);
            Ok("LLM initialized")
        }
        Err(e) => Err(format!("Failed: {}", e))
    }
}
```

### 🧠 Rust Concepts Explained

#### 1. `async fn` and `await`

```rust
pub async fn init_llm(...) -> Result<String, String>
```

**What it means**: This function can pause and resume (non-blocking).

**When to use async?**:
- I/O operations (network, file)
- Long-running operations that shouldn't freeze the UI

#### 2. `spawn_blocking` - Running Sync Code in Async Context

```rust
tauri::async_runtime::spawn_blocking(move || {
    LLMEngine::new()  // Slow synchronous code
})
.await
```

**The problem**: LLM loading is slow (~5 seconds) and synchronous. If we run it on the async executor, it blocks OTHER async tasks.

**The solution**: `spawn_blocking` runs it on a dedicated thread pool for blocking operations.

#### 3. Lifetime `'_` in State

```rust
state: State<'_, AudioState>
```

**What `'_` means**: "Let the compiler figure out the lifetime."

In async functions, we need this because the state reference might live across `.await` points.

---

## 📄 `commands/downloader.rs` - Model Downloads

**Lines**: 538  
**Purpose**: Download AI models from Hugging Face.

### 📊 Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     DOWNLOAD SYSTEM                          │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  1. User clicks "Download"                                  │
│      │                                                      │
│      ▼                                                      │
│  2. download_model("whisper-large-v3")                      │
│      │                                                      │
│      ▼                                                      │
│  3. get_model_config() → URL + SHA1 hash                    │
│      │                                                      │
│      ▼                                                      │
│  4. HTTP Stream from Hugging Face                           │
│      │   │   │                                              │
│      │   │   └── emit("download-progress", {50%})           │
│      │   └────── emit("download-progress", {75%})           │
│      └────────── emit("download-progress", {100%})          │
│                                                             │
│  5. verify_model_hash() → Check SHA1                        │
│      │                                                      │
│      ▼                                                      │
│  6. Create .verified marker file                            │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 📝 Key Functions

```rust
// Get download URL and hash for a model ID
fn get_model_config(model_id: &str) -> Option<ModelConfig>

// Download model files from Hugging Face
pub async fn download_model(app: AppHandle, model_id: String)

// Verify SHA1 hash of downloaded files
pub async fn verify_model_hash(app: AppHandle, model_id: String)

// Check which models are downloaded
pub async fn get_download_status(app: AppHandle, model_ids: Vec<String>)

// Delete a model
pub async fn delete_model(app: AppHandle, model_id: String)
```

### 🧠 Rust Concepts: Async HTTP Streaming

```rust
let res = client.get(&url).send().await?;
let mut stream = res.bytes_stream();

while let Some(item) = stream.next().await {
    let chunk = item?;
    file.write_all(&chunk)?;
    downloaded += chunk.len() as u64;
    
    // Emit progress to frontend
    app.emit("download-progress", progress_payload);
}
```

**What's happening**:
1. `bytes_stream()` returns an async iterator of chunks
2. `stream.next().await` gets the next chunk
3. We write each chunk and emit progress

---

# Level 3 Summary ✅

| File | Commands | Key Concept |
|------|----------|-------------|
| `mod.rs` | - | Module re-exports |
| `misc.rs` | `greet` | Basic command |
| `settings.rs` | 4 | State injection |
| `models.rs` | 6 | Lock management |
| `recording.rs` | 2 | Threads + channels |
| `llm.rs` | 4 | Async + spawn_blocking |
| `spellcheck.rs` | 3 | Same pattern as LLM |
| `transcription.rs` | 2 | Benchmarking |
| `downloader.rs` | 4 | HTTP streaming |

**Rust concepts covered**:
- `#[tauri::command]` attribute
- `State<T>` dependency injection
- `drop()` explicit resource release
- Channels (`tx`/`rx`) for thread communication
- `move` closures
- `async`/`await` and `spawn_blocking`
- Lifetime elision (`'_`)

---

# Level 4: Features

These modules integrate with the operating system to provide native functionality.

---

## 📄 `tray/icons.rs` - System Tray

**Location**: `src-tauri/src/tray/icons.rs`  
**Lines**: 93  
**Purpose**: System tray icon and menu.

### 🖥️ Analogy: The Notification Area Widget

The tray icon is like a **quick-access widget** in your notification area:
- 🟢 Green = Ready to record
- 🔴 Red = Currently recording
- 🟡 Yellow = Processing audio

### 📊 Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     SYSTEM TRAY                          │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  ┌─────────┐                                             │
│  │  Icon   │ ─── Left Click ──► Show Window             │
│  │  🟢/🔴/🟡│                                             │
│  └────┬────┘                                             │
│       │                                                  │
│       └── Right Click ──► Menu                           │
│                          ├── Show Taurscribe             │
│                          ├── ────────────                │
│                          └── Exit                        │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### 🧠 Rust Concepts Explained

#### 1. `macro_rules!` - Compile-Time Code Generation

```rust
macro_rules! tray_icon_green {
    () => {
        tauri::include_image!("icons/emoji-green_circle.ico")
    };
}
```

**What it does**: Creates a macro that returns an embedded image.

**Macro vs Function**:
```rust
// Function: Runs at runtime
fn get_icon() -> Image { ... }

// Macro: Expands at compile time
macro_rules! get_icon { () => { ... } }
```

**Why macro here?**: `include_image!` MUST be called at compile time to embed the file.

#### 2. `include_image!` - Embed Files at Compile Time

```rust
tauri::include_image!("icons/emoji-green_circle.ico")
```

**What it does**: Reads the file and embeds its bytes directly into the executable.

**Benefits**:
- No external files needed at runtime
- Faster (no disk I/O)
- Can't "lose" the icon file

### 📝 Function: `update_tray_icon()`

```rust
pub fn update_tray_icon(app: &AppHandle, state: AppState) -> Result<(), String> {
    // 1. Pick icon based on state
    let icon = match state {
        AppState::Ready => tray_icon_green!(),
        AppState::Recording => tray_icon_red!(),
        AppState::Processing => tray_icon_yellow!(),
    };

    // 2. Pick tooltip
    let tooltip = match state {
        AppState::Ready => "Taurscribe - Ready",
        AppState::Recording => "Taurscribe - Recording...",
        AppState::Processing => "Taurscribe - Processing...",
    };

    // 3. Find tray by ID and update
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_icon(Some(icon))?;
        tray.set_tooltip(Some(tooltip))?;
    }

    Ok(())
}
```

### 📝 Function: `setup_tray()` - Builder Pattern

```rust
pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Create menu items
    let show_item = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

    // Build the tray
    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(tray_icon_green!())
        .tooltip("Taurscribe - Ready")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Left click: Show window
            if let TrayIconEvent::Click { .. } = event {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}
```

### 🧠 The Builder Pattern

```rust
TrayIconBuilder::with_id("main-tray")
    .icon(icon)
    .tooltip("Ready")
    .menu(&menu)
    .build(app)?
```

**What it is**: Configure an object step-by-step, then create it.

**Advantages**:
- Clear, readable configuration
- Optional settings (just don't call that method)
- Enforces valid construction (can't build without required fields)

---

## 📄 `hotkeys/listener.rs` - Global Hotkeys

**Location**: `src-tauri/src/hotkeys/listener.rs`  
**Lines**: 75  
**Purpose**: Listen for Ctrl+Win system-wide.

### ⌨️ Analogy: The Spy Microphone

The hotkey listener is like a **spy listening to everything**:
- It hears EVERY key press on the system
- It ignores most, but waits for the magic combo: **Ctrl+Win**

### 📊 State Machine

```
                    ┌─────────────┐
                    │    IDLE     │
                    │ Waiting...  │
                    └──────┬──────┘
                           │
            Ctrl pressed   │   Win pressed
                           ▼
                    ┌─────────────┐
                    │   PRIMED    │
                    │ One key held│
                    └──────┬──────┘
                           │
              Other key pressed
                           ▼
                    ┌─────────────┐
                    │  RECORDING  │◄─── emit("hotkey-start-recording")
                    │  Both held  │
                    └──────┬──────┘
                           │
              Either key released
                           ▼
                    ┌─────────────┐
                    │   STOPPED   │◄─── emit("hotkey-stop-recording")
                    └─────────────┘
```

### 📝 Function: `start_hotkey_listener()`

```rust
pub fn start_hotkey_listener(app_handle: AppHandle) {
    // Shared flags (key states)
    let ctrl_held = Arc::new(AtomicBool::new(false));
    let meta_held = Arc::new(AtomicBool::new(false));
    let recording_active = Arc::new(AtomicBool::new(false));

    // Clone for closure
    let ctrl_held_clone = ctrl_held.clone();
    let meta_held_clone = meta_held.clone();
    let recording_active_clone = recording_active.clone();
    let app_handle_clone = app_handle.clone();

    // Callback for EVERY key event
    let callback = move |event: Event| {
        match event.event_type {
            EventType::KeyPress(key) => {
                match key {
                    Key::ControlLeft | Key::ControlRight => {
                        ctrl_held_clone.store(true, Ordering::SeqCst);
                    }
                    Key::MetaLeft | Key::MetaRight => {
                        meta_held_clone.store(true, Ordering::SeqCst);
                    }
                    _ => {}
                }

                // Both pressed? Start recording!
                if ctrl_held_clone.load(Ordering::SeqCst)
                    && meta_held_clone.load(Ordering::SeqCst)
                    && !recording_active_clone.load(Ordering::SeqCst)
                {
                    recording_active_clone.store(true, Ordering::SeqCst);
                    let _ = app_handle_clone.emit("hotkey-start-recording", ());
                }
            }
            EventType::KeyRelease(key) => {
                // Update flags
                // If recording and either key released, stop
                if recording_active_clone.load(Ordering::SeqCst)
                    && (!ctrl_held_clone.load(Ordering::SeqCst)
                        || !meta_held_clone.load(Ordering::SeqCst))
                {
                    recording_active_clone.store(false, Ordering::SeqCst);
                    let _ = app_handle_clone.emit("hotkey-stop-recording", ());
                }
            }
            _ => {}
        }
    };

    // Start listening (BLOCKS FOREVER)
    if let Err(error) = listen(callback) {
        eprintln!("[ERROR] Hotkey listener error: {:?}", error);
    }
}
```

### 🧠 Rust Concepts Explained

#### 1. `AtomicBool` - Lock-Free Thread Safety

```rust
let ctrl_held = Arc::new(AtomicBool::new(false));

// Write
ctrl_held.store(true, Ordering::SeqCst);

// Read
if ctrl_held.load(Ordering::SeqCst) { ... }
```

**What it is**: A boolean that can be safely read/written from multiple threads WITHOUT a mutex.

**Why not `Mutex<bool>`?**:
- `AtomicBool` is faster (no locking)
- Simpler for single values
- Perfect for flags like "is key pressed?"

#### 2. `Ordering::SeqCst` - Memory Ordering

```rust
ctrl_held.store(true, Ordering::SeqCst);
```

**What it means**: "Sequentially Consistent" - strongest guarantee. All threads see operations in the same order.

**Other options** (for advanced use):
- `Relaxed` - Fastest, weakest guarantees
- `Acquire`/`Release` - For producer/consumer patterns
- `SeqCst` - Slowest, safest (use when in doubt)

#### 3. `Arc` with Atomics

```rust
let ctrl_held = Arc::new(AtomicBool::new(false));
let ctrl_held_clone = ctrl_held.clone();
```

**Why Arc?**: We need the same `AtomicBool` accessible from:
1. The main thread (lib.rs)
2. The callback closure

`Arc` gives us a shared pointer. The `AtomicBool` inside handles the actual thread-safe access.

### ⚠️ Gotcha

**`listen()` blocks forever!** That's why we spawn it in a separate thread:

```rust
// In lib.rs
std::thread::spawn(move || {
    hotkeys::start_hotkey_listener(app_handle);
});
```

---

# Level 5: Entry Point

Everything connects in `lib.rs` - the application bootstrap.

---

## 📄 `lib.rs` - The Grand Assembly

**Location**: `src-tauri/src/lib.rs`  
**Lines**: 127  
**Purpose**: Initialize everything and start the app.

### 🏗️ Analogy: The Factory Manager

`lib.rs` is like the **factory manager on opening day**:
1. Hire the AI workers (Whisper, Parakeet, VAD)
2. Set up the control room (State)
3. Install the intercom system (Commands)
4. Open the doors (Run)

### 📊 Initialization Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                        lib.rs run()                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. Initialize Whisper                                          │
│     └── WhisperManager::new()                                   │
│     └── whisper.initialize(None) [on separate thread]           │
│                                                                 │
│  2. Initialize VAD                                              │
│     └── VADManager::new()                                       │
│                                                                 │
│  3. Initialize Parakeet                                         │
│     └── ParakeetManager::new()                                  │
│     └── parakeet.initialize("nemotron:nemotron")                │
│                                                                 │
│  4. Build Tauri App                                             │
│     ├── .plugin(opener)                                         │
│     ├── .plugin(store)                                          │
│     ├── .manage(AudioState)                                     │
│     ├── .setup(|app| { ... })                                   │
│     │   ├── setup_tray(app)                                     │
│     │   └── spawn hotkey listener                               │
│     ├── .on_window_event(...)                                   │
│     ├── .invoke_handler([...commands...])                       │
│     └── .run()                                                  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 📝 The `run()` Function

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 1. Initialize Whisper (on large stack thread)
    println!("[INFO] Initializing Whisper...");
    let whisper = WhisperManager::new();

    let (whisper, init_result) = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)  // 8 MiB stack
        .spawn(move || {
            let mut whisper = whisper;
            let res = whisper.initialize(None);
            (whisper, res)
        })
        .expect("Failed to spawn thread")
        .join()
        .expect("Thread panicked");

    // 2. Initialize VAD
    let vad = VADManager::new().unwrap_or_else(|e| {
        panic!("VAD init failed: {}", e);
    });

    // 3. Initialize Parakeet
    let mut parakeet = ParakeetManager::new();
    match parakeet.initialize(Some("nemotron:nemotron")) {
        Ok(msg) => println!("[SUCCESS] {}", msg),
        Err(_) => {
            // Fallback to any available model
            match parakeet.initialize(None) {
                Ok(msg) => println!("[SUCCESS] Fallback: {}", msg),
                Err(e) => eprintln!("[WARN] No Parakeet: {}", e),
            }
        }
    }

    // 4. Build and Run Tauri
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(AudioState::new(whisper, parakeet, vad))
        .setup(|app| {
            tray::setup_tray(app)?;

            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                hotkeys::start_hotkey_listener(app_handle);
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Minimize to tray instead of closing
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::start_recording,
            commands::stop_recording,
            // ... 20+ more commands ...
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}
```

### 🧠 Rust Concepts Explained

#### 1. `#[cfg_attr(...)]` - Conditional Attributes

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run()
```

**What it does**: Apply an attribute ONLY on certain platforms.

**Breakdown**:
- `mobile` - The condition (true on iOS/Android)
- `tauri::mobile_entry_point` - The attribute to apply

**When expanded**:
```rust
// On mobile:
#[tauri::mobile_entry_point]
pub fn run()

// On desktop:
pub fn run()  // No extra attribute
```

#### 2. Thread Builder - Custom Stack Size

```rust
std::thread::Builder::new()
    .stack_size(8 * 1024 * 1024)  // 8 MiB
    .spawn(...)
```

**Why?**: Default stack (1-2 MiB) is too small for Whisper model loading (needs ~6 MiB).

**Thread Builder vs `thread::spawn`**:
```rust
// Simple (default settings)
std::thread::spawn(|| { ... });

// Customizable
std::thread::Builder::new()
    .name("whisper-init".into())
    .stack_size(8 * 1024 * 1024)
    .spawn(|| { ... })
```

#### 3. Tauri Builder Pattern

```rust
tauri::Builder::default()
    .plugin(...)
    .manage(...)
    .setup(...)
    .on_window_event(...)
    .invoke_handler(...)
    .run(...)
```

**Method chaining**: Each method returns `self`, allowing fluent configuration.

**Key methods**:
| Method | Purpose |
|--------|---------|
| `.plugin()` | Add Tauri plugin |
| `.manage()` | Register global state |
| `.setup()` | Run code after app starts |
| `.on_window_event()` | Handle window events |
| `.invoke_handler()` | Register commands |
| `.run()` | Start the app |

#### 4. `generate_handler!` - Command Registration

```rust
.invoke_handler(tauri::generate_handler![
    commands::greet,
    commands::start_recording,
    // ...
])
```

**What it does**: Generates code to route JS `invoke()` calls to Rust functions.

**Behind the scenes**: Creates a function map:
```rust
// Pseudocode of what the macro generates:
match command_name {
    "greet" => call greet(args),
    "start_recording" => call start_recording(args),
    // ...
}
```

#### 5. Close-to-Tray Pattern

```rust
.on_window_event(|window, event| {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        let _ = window.hide();      // Hide instead of close
        api.prevent_close();        // Don't actually close
        println!("Minimized to tray");
    }
})
```

**What it does**: When user clicks ❌, hide the window instead of quitting.

**UX Pattern**: Common for apps that run in background (Discord, Slack, etc.).

### ⚠️ Gotchas

1. **Plugin order matters**: Some plugins depend on others.

2. **`.manage()` before `.invoke_handler()`**: State must be registered before commands use it.

3. **`.setup()` runs BEFORE window shows**: Good for initialization, but don't block too long.

---

# 🎉 CODE REFERENCE COMPLETE!

## Summary

You've now learned the entire Taurscribe codebase:

### Level 1: Foundation
| File | Key Concept |
|------|-------------|
| `main.rs` | Entry point, `cfg_attr` |
| `types.rs` | Enums, derive macros |
| `audio.rs` | `unsafe Send/Sync` |
| `utils.rs` | `Result`, `?` operator |
| `state.rs` | `Arc<Mutex<T>>` |

### Level 2: AI Engines
| File | Key Concept |
|------|-------------|
| `vad.rs` | Iterators, slices |
| `whisper.rs` | FFI, `or_else` |
| `parakeet.rs` | `#[cfg]` conditional compilation |
| `llm.rs` | `anyhow`, tokenization |
| `spellcheck.rs` | Generics |

### Level 3: Commands
| File | Key Concept |
|------|-------------|
| `recording.rs` | Threads, channels, `move` |
| `llm.rs` | `async`, `spawn_blocking` |
| `downloader.rs` | HTTP streaming |
| Others | `#[tauri::command]`, `State<T>` |

### Level 4: Features
| File | Key Concept |
|------|-------------|
| `tray/icons.rs` | Macros, builder pattern |
| `hotkeys/listener.rs` | `AtomicBool`, `Arc` |

### Level 5: Entry Point
| File | Key Concept |
|------|-------------|
| `lib.rs` | Tauri Builder, `generate_handler!` |

---

## 🔑 Key Rust Patterns Used

1. **Thread Safety**: `Arc<Mutex<T>>`, `AtomicBool`
2. **Error Handling**: `Result<T, E>`, `?` operator, `anyhow`
3. **Memory Safety**: `unsafe` blocks, `Send`/`Sync` traits
4. **Concurrency**: Channels (`tx`/`rx`), `move` closures
5. **Async**: `async`/`await`, `spawn_blocking`
6. **Metaprogramming**: Macros, derive macros, `#[cfg]`
7. **Builder Pattern**: Method chaining for configuration

---

## 📚 Next Steps

1. **Explore the frontend**: `src/` contains the React/TypeScript UI
2. **Run the app**: `npm run tauri dev`
3. **Modify a command**: Try adding a parameter to `greet`
4. **Read the Tauri docs**: [tauri.app/v2/guides](https://tauri.app/v2/guides)

Happy coding! 🦀
