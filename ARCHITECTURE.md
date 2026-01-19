# Taurscribe Architecture Guide for Beginners

> **Audience**: This document is written for developers new to Rust, Tauri, or real-time audio processing.

---

## Table of Contents
1. [Complete Code Flow](#complete-code-flow)
2. [Function-by-Function Breakdown](#function-by-function-breakdown)
3. [Ownership & Memory Management](#ownership--memory-management)
4. [Dependencies Explained (Cargo.toml)](#dependencies-explained-cargotoml)
5. [Model Embedding vs. Separate Files](#model-embedding-vs-separate-files)
6. [Common Beginner Questions](#common-beginner-questions)

---

## Complete Code Flow

### 🎬 **The Big Picture**

Think of Taurscribe like a **restaurant kitchen**:

```
┌─────────────────────────────────────────────────────────────────┐
│                         🍽️ RESTAURANT ANALOGY                    │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  👤 Customer (User)                                             │
│      │                                                          │
│      │ "I want food!" (Clicks Start Recording)                 │
│      ▼                                                          │
│  📋 Waiter (Frontend - App.tsx)                                │
│      │                                                          │
│      │ Takes order via Tauri Invoke                            │
│      ▼                                                          │
│  👨‍🍳 Head Chef (Backend - lib.rs)                              │
│      │                                                          │
│      ├──► 🎤 Audio Stream (Microphone = Farm)                  │
│      │                                                          │
│      ├──► 👨‍🍳 Cook 1: File Writer Thread                       │
│      │         "I'll save the raw ingredients"                 │
│      │                                                          │
│      ├──► 👨‍🍳 Cook 2: Whisper Thread                           │
│      │         "I'll taste-test as we go"                      │
│      │                                                          │
│      └──► 🧑‍🔬 Whisper Manager (whisper.rs)                     │
│            "I'm the expert chef who analyzes everything"       │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

### 📊 **Detailed Flow Diagram**

```
USER CLICKS "START RECORDING"
    │
    ├───► [1] Frontend (App.tsx)
    │         invoke("start_recording")
    │         
    ├───► [2] Backend (lib.rs) - start_recording()
    │         │
    │         ├─ [2.1] Get microphone access (cpal)
    │         │        "Hey Windows, give me the mic!"
    │         │
    │         ├─ [2.2] Create WAV file
    │         │        recording_1737280000.wav
    │         │
    │         ├─ [2.3] Create two "conveyor belts" (channels)
    │         │        ┌──────────────────┐
    │         │        │ Belt 1: file_tx  │ → File Writer Thread
    │         │        └──────────────────┘
    │         │        ┌──────────────────┐
    │         │        │ Belt 2: whisper_tx│ → Whisper Thread
    │         │        └──────────────────┘
    │         │
    │         ├─ [2.4] ⚡ SPAWN THREAD 1: File Writer
    │         │        while let Ok(samples) = file_rx.recv() {
    │         │            writer.write_sample(sample)  // Save to disk
    │         │        }
    │         │
    │         ├─ [2.5] ⚡ SPAWN THREAD 2: Whisper Processor
    │         │        let mut buffer = Vec::new();
    │         │        while let Ok(samples) = whisper_rx.recv() {
    │         │            buffer.extend(samples);
    │         │            if buffer.len() >= 6_seconds {
    │         │                transcribe_chunk(&buffer) // 🎤→📝
    │         │            }
    │         │        }
    │         │
    │         └─ [2.6] 🎙️ START AUDIO STREAM
    │                  device.build_input_stream(
    │                      |audio_data| {
    │                          file_tx.send(audio_data);     // To disk
    │                          whisper_tx.send(mono_data);   // To AI
    │                      }
    │                  )
    │
    ├───► [3] RECORDING IN PROGRESS...
    │         🎤 Mic → Stream → Channels → Threads
    │         Every 10ms: New audio chunk arrives
    │         Thread 1: Writes to file
    │         Thread 2: Buffers, then transcribes every 6s
    │
USER CLICKS "STOP RECORDING"
    │
    ├───► [4] Backend (lib.rs) - stop_recording()
    │         │
    │         ├─ [4.1] Drop stream (stop mic)
    │         │
    │         ├─ [4.2] Close channels
    │         │        drop(file_tx);     → Thread 1 finishes & saves WAV
    │         │        drop(whisper_tx);  → Thread 2 processes remaining audio
    │         │
    │         ├─ [4.3] 💤 Sleep 500ms (let file finish writing)
    │         │
    │         └─ [4.4] 🎯 FINAL TRANSCRIPTION
    │                  whisper.transcribe_file("recording_XXX.wav")
    │                  Returns full, high-quality transcript
    │
    └───► [5] Frontend receives result
              Display transcript to user
```

---

## Function-by-Function Breakdown

### 🎯 **Frontend: App.tsx**

#### **Component State**
```typescript
const [greetMsg, setGreetMsg] = useState("");      // Display message
const [isRecording, setIsRecording] = useState(false);  // Button state
```

**Think of this as**: A note on the refrigerator that tracks whether someone is cooking.

---

#### **Function: Start Recording Button**

```typescript
onClick={async () => {
    const res = await invoke("start_recording");  // 📞 Call Rust
    setGreetMsg(res as string);  // 📝 Show result
    setIsRecording(true);        // 🔴 Update UI
}}
```

**Step-by-Step**:
1. User clicks button
2. JavaScript calls `invoke()` - this is like dialing a phone number to the Rust backend
3. Rust backend answers the call and starts recording
4. Rust sends back a message: "Recording started: recording_1737280000.wav"
5. Frontend updates the UI

**Analogy**: You press the doorbell (button), someone inside (Rust) hears it and opens the door.

---

#### **Function: Stop Recording Button**

```typescript
onClick={async () => {
    const res = await invoke("stop_recording");  // 📞 Call Rust
    setGreetMsg(res as string);  // 📝 Show final transcript
    setIsRecording(false);       // ⚫ Update UI
}}
```

Same process, but now Rust returns the **final transcript** instead of just a status message.

---

### 🦀 **Backend: lib.rs**

This is the **heart of the application**. Let's break down every function.

---

#### **1. AudioState Struct**

```rust
struct AudioState {
    recording_handle: Mutex<Option<RecordingHandle>>,
    whisper: Arc<Mutex<WhisperManager>>,
    last_recording_path: Mutex<Option<String>>,
}
```

**What is this?**

This is like a **shared bulletin board** that all parts of the app can read/write to.

| Field | Type | Purpose | Analogy |
|-------|------|---------|---------|
| `recording_handle` | `Mutex<Option<RecordingHandle>>` | Current recording session | "Is someone currently using the microphone?" |
| `whisper` | `Arc<Mutex<WhisperManager>>` | The AI transcription engine | "Our expert translator is shared by everyone" |
| `last_recording_path` | `Mutex<Option<String>>` | Where we saved the audio | "Remember where we put the audio file!" |

**Why `Mutex`?** = Lock on a bathroom door. Only one person can use it at a time.
**Why `Arc`?** = Shared ownership. Multiple threads can hold a "ticket" to access the same data.
**Why `Option`?** = Maybe there's nothing there yet. `Some(value)` or `None`.

---

#### **2. RecordingHandle Struct**

```rust
struct RecordingHandle {
    stream: SendStream,              // The microphone connection
    file_tx: Sender<Vec<f32>>,      // Conveyor belt to file writer
    whisper_tx: Sender<Vec<f32>>,   // Conveyor belt to Whisper
}
```

**What is this?**

This is the "control panel" for an active recording. It holds:
1. The microphone stream (so we can stop it later)
2. Two conveyor belts (channels) to send audio to different places

---

#### **3. start_recording() - The Big Function**

Let's break this down into **10 digestible steps**:

---

##### **Step 1: Get the Microphone**

```rust
let host = cpal::default_host();  // "What audio system is available?"
let device = host.default_input_device()  // "Get default mic"
    .ok_or("No input device")?;
```

**Analogy**: "Hey computer, give me the microphone!" If none exists, return an error.

---

##### **Step 2: Get Mic Settings**

```rust
let config: cpal::StreamConfig = device
    .default_input_config()  // "What format does the mic use?"
    .map_err(|e| e.to_string())?
    .into();
```

This tells us:
- **Sample rate**: 48000 Hz (48,000 measurements per second)
- **Channels**: 2 (Left + Right, or Mono)
- **Format**: f32 (floating-point numbers -1.0 to 1.0)

**Analogy**: "What language does the mic speak?" Answer: "48kHz stereo f32"

---

##### **Step 3: Create Filename**

```rust
let filename = format!("recording_{}.wav", chrono::Utc::now().timestamp());
```

Creates: `recording_1737280000.wav`  
(Uses current Unix timestamp)

---

##### **Step 4: Create WAV Writer**

```rust
let spec = hound::WavSpec {
    channels: config.channels,
    sample_rate: config.sample_rate.0,
    bits_per_sample: 32,
    sample_format: hound::SampleFormat::Float,
};
let writer = hound::WavWriter::create(&path, spec)?;
```

**What is this?**

We're creating a "recipe card" for the WAV file. This tells the file:
- How many channels (1 or 2)
- Sample rate (48000)
- Bit depth (32-bit float)

Then `WavWriter::create()` creates the actual file.

---

##### **Step 5: Create Two Channels (Conveyor Belts)**

```rust
let (file_tx, file_rx) = unbounded::<Vec<f32>>();      // Belt 1
let (whisper_tx, whisper_rx) = unbounded::<Vec<f32>>(); // Belt 2
```

**Analogy**: 

```
             🎤 MICROPHONE
                   │
                   ▼
        ┌──────────┴──────────┐
        │                     │
        ▼                     ▼
   🔵 Belt 1              🟢 Belt 2
   file_tx               whisper_tx
        │                     │
        ▼                     ▼
   Thread 1              Thread 2
   (File Writer)         (Whisper AI)
```

**Why `unbounded`?**: The conveyor belt has infinite capacity. In a production app, you'd use `bounded()` to prevent memory overflow.

---

##### **Step 6: SPAWN THREAD 1 - File Writer**

```rust
std::thread::spawn(move || {
    let mut writer = writer;
    while let Ok(samples) = file_rx.recv() {  // Wait for audio chunks
        for sample in samples {
            writer.write_sample(sample).ok();  // Write to WAV
        }
    }
    writer.finalize().ok();  // Close file
    println!("WAV file saved.");
});
```

**Flow**:
1. Thread sits and waits for audio on `file_rx`
2. When audio arrives, write each sample to the WAV file
3. When the channel closes (we call `drop(file_tx)`), exit the loop
4. Finalize the file (writes WAV header)

**Key Point**: `move` means this thread **takes ownership** of `writer` and `file_rx`. They're no longer accessible from the main code.

---

##### **Step 7: SPAWN THREAD 2 - Whisper Processor**

```rust
std::thread::spawn(move || {
    let mut buffer = Vec::new();
    let chunk_size = (sample_rate * 6) as usize;  // 6 seconds of audio

    while let Ok(samples) = whisper_rx.recv() {
        buffer.extend(samples);  // Add to buffer

        while buffer.len() >= chunk_size {
            let chunk: Vec<f32> = buffer.drain(..chunk_size).collect();
            
            // 🎤→📝 TRANSCRIBE!
            match whisper.lock().unwrap().transcribe_chunk(&chunk, sample_rate) {
                Ok(text) => println!("[TRANSCRIPT] {}", text),
                Err(e) => eprintln!("[ERROR] {}", e),
            }
        }
    }
});
```

**Flow**:

```
Audio arrives → Buffer
    │
    ├─ Buffer < 6 seconds? → Keep collecting
    │
    └─ Buffer >= 6 seconds? 
           │
           ├─ Extract 6 seconds of audio
           ├─ Send to Whisper AI
           ├─ Get transcript
           └─ Print to console
```

**Why 6 seconds?**  
- Too short (1s): Cuts sentences → hallucinations  
- Too long (30s): High latency → feels slow  
- 6s: Sweet spot for real-time feel + accuracy

---

##### **Step 8: Mix Stereo to Mono**

```rust
let mono_data: Vec<f32> = if channels > 1 {
    data.chunks(2)  // Split into pairs [L, R]
        .map(|chunk| chunk.iter().sum::<f32>() / 2.0)  // Average
        .collect()
} else {
    data.to_vec()
};
```

**Why?**

Whisper expects **mono** audio (single channel). If we send stereo `[L, R, L, R]`, Whisper thinks it's mono audio at **2× speed** → Chipmunk voices → Hallucinations!

**Stereo → Mono**:
```
Before: [L1, R1, L2, R2, L3, R3]
After:  [(L1+R1)/2, (L2+R2)/2, (L3+R3)/2]
```

---

##### **Step 9: Build and Start Audio Stream**

```rust
let stream = device.build_input_stream(
    &config,
    move |data: &[f32], _: &_| {
        file_tx_clone.send(data.to_vec()).ok();      // Send stereo to file
        whisper_tx_clone.send(mono_data).ok();       // Send mono to Whisper
    },
    move |err| {
        eprintln!("Audio input error: {}", err);
    },
    None,
)?;

stream.play()?;  // ▶️ START!
```

**What is this?**

This creates a **callback function** that runs every ~10ms when new audio arrives.

**Think of it like a timer**:
```
Every 10ms:
    1. Microphone captures new samples
    2. Send copy to file writer (stereo, full quality)
    3. Send copy to Whisper (mono, for AI)
```

---

##### **Step 10: Save Recording Handle**

```rust
*state.recording_handle.lock().unwrap() = Some(RecordingHandle {
    stream: SendStream(stream),
    file_tx,
    whisper_tx,
});
```

**Why?** So `stop_recording()` can later access the stream to stop it!

**Rust Magic**: `lock().unwrap()` gets exclusive access to the `Mutex`, then we overwrite the value with `Some(...)`.

---

#### **4. stop_recording() - Cleanup & Final Transcription**

```rust
fn stop_recording(state: State<AudioState>) -> Result<String, String> {
    // [1] Get the recording handle
    let mut handle = state.recording_handle.lock().unwrap();
    
    if let Some(recording) = handle.take() {
        // [2] Stop everything
        drop(recording.stream);      // Stop mic
        drop(recording.file_tx);     // Close file channel → Thread 1 exits
        drop(recording.whisper_tx);  // Close Whisper channel → Thread 2 finishes
        
        // [3] Wait for file to finish writing
        std::thread::sleep(std::time::Duration::from_millis(500));
        
        // [4] Get the saved file path
        let path = state.last_recording_path.lock().unwrap().clone().unwrap();
        
        // [5] 🎯 HIGH-QUALITY FINAL TRANSCRIPTION
        match state.whisper.lock().unwrap().transcribe_file(&path) {
            Ok(text) => Ok(text),  // Return transcript to frontend!
            Err(e) => Ok(format!("Recording saved, but transcription failed: {}", e))
        }
    } else {
        Err("Not recording".to_string())
    }
}
```

**Step-by-Step**:

1. **Lock the `recording_handle`** (like opening a vault)
2. **`handle.take()`** removes the `RecordingHandle` from the `Option`
   - Before: `Some(RecordingHandle { ... })`
   - After: `None`
3. **`drop(stream)`** stops the microphone
4. **`drop(file_tx)`** closes the channel → Thread 1 sees "no more data" → exits loop → saves file
5. **`drop(whisper_tx)`** closes channel → Thread 2 processes remaining audio → exits
6. **Sleep 500ms** to ensure file write completes
7. **`transcribe_file()`** runs Whisper on the full WAV file (slow but accurate)
8. **Return transcript** to frontend

---

### 🧠 **Whisper Manager: whisper.rs**

This file manages the AI model. Let's break down each function.

---

#### **1. WhisperManager::new()**

```rust
pub fn new() -> Self {
    Self {
        context: None,          // No model loaded yet
        last_transcript: String::new(),  // No previous text
    }
}
```

**Simple constructor**. Creates an empty manager.

---

#### **2. WhisperManager::initialize() - Load the Model**

```rust
pub fn initialize(&mut self) -> Result<(), String> {
    // [1] Suppress C++ logs
    unsafe {
        set_log_callback(Some(null_log_callback), std::ptr::null_mut());
    }
    
    // [2] Find the model file
    let model_path = "taurscribe-runtime/models/ggml-large-v3.bin";
    let absolute_path = std::fs::canonicalize(model_path)?;
    
    // [3] Try GPU, fallback to CPU
    let ctx = self.try_gpu(&absolute_path)
        .or_else(|_| self.try_cpu(&absolute_path))?;
    
    // [4] Save the loaded context
    self.context = Some(ctx);
    Ok(())
}
```

**Flow**:
```
1. Silence C++ logs (whisper.cpp is very verbose)
2. Find model file (try current dir, parent dirs, etc.)
3. Try loading with GPU acceleration
4. If GPU fails, try CPU
5. If both fail, return error
6. If success, save the context
```

---

#### **3. try_gpu() & try_cpu()**

```rust
fn try_gpu(&self, model_path: &Path) -> Result<WhisperContext, String> {
    let mut params = WhisperContextParameters::default();
    params.use_gpu(true);  // ✅ Enable GPU
    
    match WhisperContext::new_with_params(path, params) {
        Ok(ctx) => {
            println!("✓ GPU acceleration enabled");
            Ok(ctx)
        }
        Err(e) => Err(format!("GPU failed: {:?}", e))
    }
}
```

**What happens?**

1. Create parameters with `use_gpu(true)`
2. Try to load the model with CUDA/Vulkan support
3. If success: Print success message, return context
4. If fail: Try CPU fallback

---

#### **4. transcribe_chunk() - Real-Time Transcription**

This is called every 6 seconds during recording.

```rust
pub fn transcribe_chunk(
    &mut self,
    samples: &[f32],      // Audio data
    input_sample_rate: u32,  // e.g., 48000
) -> Result<String, String> {
    
    // [1] Resample to 16kHz if needed
    let audio_data = if input_sample_rate != 16000 {
        // Use rubato to convert 48kHz → 16kHz
        resample(samples, input_sample_rate)
    } else {
        samples.to_vec()
    };
    
    // [2] Create Whisper state
    let mut state = self.context.create_state()?;
    
    // [3] Set parameters
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_n_threads(4);
    
    // [4] Use previous transcript as context (improves accuracy!)
    if !self.last_transcript.is_empty() {
        params.set_initial_prompt(&self.last_transcript);
    }
    
    // [5] Run transcription
    state.full(params, &audio_data)?;
    
    // [6] Extract text from segments
    let mut transcript = String::new();
    for i in 0..state.full_n_segments() {
        transcript.push_str(&state.get_segment(i).to_string());
    }
    
    // [7] Save for next chunk
    self.last_transcript = transcript.clone();
    
    Ok(transcript)
}
```

**Key Insight**: `set_initial_prompt()` tells Whisper "here's what was said before." This dramatically reduces hallucinations!

**Example**:
- Chunk 1: "Hello my name"
- Chunk 2: Uses "Hello my name" as context → knows we're mid-sentence → transcribes "is John" instead of random words

---

#### **5. transcribe_file() - Final High-Quality Transcription**

Called when recording stops.

```rust
pub fn transcribe_file(&mut self, file_path: &str) -> Result<String, String> {
    // [1] Load WAV file
    let mut reader = hound::WavReader::open(file_path)?;
    
    // [2] Read all samples
    let samples: Vec<f32> = reader.samples::<f32>().collect();
    
    // [3] Convert stereo → mono if needed
    let mono_samples = if spec.channels == 2 {
        samples.chunks(2).map(|chunk| (chunk[0] + chunk[1]) / 2.0).collect()
    } else {
        samples
    };
    
    // [4] Resample to 16kHz
    let audio_data = resample(mono_samples, spec.sample_rate);
    
    // [5] Transcribe entire file at once
    let mut state = self.context.create_state()?;
    let mut params = FullParams::new(...);
    state.full(params, &audio_data)?;
    
    // [6] Extract all segments
    let mut transcript = String::new();
    for i in 0..state.full_n_segments() {
        transcript.push_str(&state.get_segment(i).to_string());
    }
    
    Ok(transcript)
}
```

**Difference from `transcribe_chunk()`**:
- Processes **entire file** at once (not 6s chunks)
- No need for context prompts (has full audio)
- Slower but more accurate

---

## Ownership & Memory Management

This is where Rust shines! Let's understand how ownership prevents bugs.

---

### 🔐 **Rust Ownership Rules**

1. **Each value has ONE owner**
2. **When the owner goes out of scope, the value is dropped**
3. **You can borrow values** (`&T` or `&mut T`) without taking ownership

---

### **Example 1: Moving Ownership to Threads**

```rust
let (file_tx, file_rx) = unbounded();

std::thread::spawn(move || {
    while let Ok(samples) = file_rx.recv() {  // file_rx is now owned by this thread
        // ...
    }
});

// ❌ ERROR: Can't use file_rx here anymore!
// file_rx.recv();  // Compile error: "value used after move"
```

**What happened?**

- `move` keyword **transfers ownership** of `file_rx` to the thread
- Main function can no longer access `file_rx`
- This prevents data races! (Two threads can't access the same data)

---

### **Example 2: Shared Ownership with Arc**

```rust
let whisper = Arc::new(Mutex::new(WhisperManager::new()));

// Clone the Arc (creates new reference, doesn't copy the data)
let whisper_clone = whisper.clone();

std::thread::spawn(move || {
    let result = whisper_clone.lock().unwrap().transcribe_chunk(...);
});

// ✅ Original `whisper` is still usable!
whisper.lock().unwrap().transcribe_file(...);
```

**What's happening?**

- `Arc` = "Atomic Reference Counter"
- Each `clone()` increments a counter
- When all clones are dropped, the data is freed
- `Mutex` ensures only one thread accesses the data at a time

**Think of `Arc<Mutex<T>>` as**:
- `Arc`: Shared treasure map (multiple people can have copies)
- `Mutex`: Lock on the treasure chest (only one person can open it at a time)

---

### **Example 3: Channels for Thread Communication**

```rust
let (tx, rx) = unbounded::<Vec<f32>>();

// Thread 1: Producer
std::thread::spawn(move || {
    tx.send(vec![1.0, 2.0, 3.0]).unwrap();
});

// Thread 2: Consumer
std::thread::spawn(move || {
    let data = rx.recv().unwrap();
    println!("{:?}", data);
});
```

**Ownership transfer**:
```
Thread 1 (tx)                Thread 2 (rx)
    │                             │
    │  tx.send(vec![...])         │
    │  ─────────────────────────► │
    │  (ownership transferred)    │
    │                             │
    │                          rx.recv()
    │                          (now owns the vec)
```

**Safety**: The `Vec` can only be accessed by Thread 2. No data races possible!

---

### **Example 4: Mutex for Shared State**

```rust
struct AudioState {
    recording_handle: Mutex<Option<RecordingHandle>>,
}

// Thread 1: Start recording
{
    let mut handle = state.recording_handle.lock().unwrap();
    *handle = Some(RecordingHandle { ... });
}  // Lock released here

// Thread 2: Stop recording
{
    let mut handle = state.recording_handle.lock().unwrap();
    if let Some(rec) = handle.take() {
        drop(rec);
    }
}  // Lock released here
```

**What's happening?**

1. `lock()` blocks until it can acquire the mutex
2. Returns a `MutexGuard` that allows accessing the data
3. When the guard goes out of scope (`}`), the lock is released
4. This ensures no two threads modify `recording_handle` simultaneously

---

### **Ownership Summary in Taurscribe**

| Type | Ownership Strategy | Why? |
|------|-------------------|------|
| **AudioState** | `State<AudioState>` (managed by Tauri) | Shared across all Tauri commands |
| **WhisperManager** | `Arc<Mutex<WhisperManager>>` | Shared between threads, protected by lock |
| **Audio Samples** | Sent via channels (`tx.send()`) | Ownership transferred to receiving thread |
| **Channels** | `move` into threads | Each thread owns its sender/receiver |
| **WAV Writer** | `move` into file thread | Exclusively owned by file writer |

---

## Dependencies Explained (Cargo.toml)

Let's break down EVERY dependency and why it's needed.

### **src-tauri/Cargo.toml**

```toml
[package]
name = "taurscribe"
version = "0.1.0"
description = "A Tauri App"
authors = ["you"]
edition = "2021"  # Rust 2021 edition (latest stable features)
```

**Metadata**: Basic project info.

---

```toml
[lib]
name = "taurscribe_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

**What is this?**

Tauri compiles your Rust code in three formats:
- **`staticlib`**: Static library for bundling
- **`cdylib`**: C-compatible dynamic library (for Windows DLL)
- **`rlib`**: Rust library for linking

**Why?** Tauri needs different formats for different platforms.

---

```toml
[build-dependencies]
tauri-build = { version = "2", features = [] }
```

**What is this?**

A **build script** that runs before compilation. It generates Rust code from `tauri.conf.json`.

---

### **[dependencies] - The Main Libraries**

#### **1. Tauri Core**

```toml
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
```

| Crate | Purpose | What it does |
|-------|---------|--------------|
| `tauri` | Framework | Bridges Rust ↔ Frontend, manages windows, state |
| `tauri-plugin-opener` | File/URL opener | Opens files/URLs in default apps |

---

#### **2. Serialization**

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

| Crate | Purpose | What it does |
|-------|---------|--------------|
| `serde` | Serialization framework | Converts Rust structs ↔ JSON |
| `serde_json` | JSON support | Parses/stringifies JSON |

**Why needed?** Tauri commands send data as JSON between Rust and JavaScript.

**Example**:
```rust
#[derive(Serialize)]
struct Response {
    message: String,
}

// Rust → JSON → JavaScript
return Response { message: "Hello".to_string() };
// Becomes: {"message": "Hello"}
```

---

#### **3. Audio Processing**

```toml
cpal = "0.15"
hound = "3.5"
```

| Crate | Purpose | What it does |
|-------|---------|--------------|
| `cpal` | Cross-platform audio | Access microphone, play audio |
| `hound` | WAV file I/O | Read/write WAV files |

**Why `cpal`?** Works on Windows, macOS, Linux.

**Example**:
```rust
// cpal: Get microphone
let device = cpal::default_host().default_input_device()?;

// hound: Create WAV file
let writer = hound::WavWriter::create("output.wav", spec)?;
```

---

#### **4. Threading & Concurrency**

```toml
crossbeam-channel = "0.5"
```

| Crate | Purpose | What it does |
|-------|---------|--------------|
| `crossbeam-channel` | Lock-free channels | Send data between threads |

**Why not `std::sync::mpsc`?** `crossbeam` is faster and more ergonomic.

---

#### **5. AI & Transcription**

```toml
whisper-rs = { 
    git = "https://codeberg.org/tazz4843/whisper-rs.git", 
    features = ["cuda", "vulkan"] 
}
rubato = "0.14"
```

| Crate | Purpose | Features |
|-------|---------|----------|
| `whisper-rs` | Whisper.cpp bindings | `cuda`: NVIDIA GPU<br>`vulkan`: Universal GPU |
| `rubato` | Audio resampling | Convert 48kHz → 16kHz |

**Why from Git?** The published crate doesn't have CUDA/Vulkan support yet.

**Why resampling?** Whisper requires 16kHz mono audio.

---

#### **6. Utilities**

```toml
chrono = "0.4"
```

| Crate | Purpose | What it does |
|-------|---------|--------------|
| `chrono` | Date/time | Generate timestamps for filenames |

**Example**:
```rust
let timestamp = chrono::Utc::now().timestamp();  // 1737280000
let filename = format!("recording_{}.wav", timestamp);
```

---

### **src-tauri/.cargo/config.toml**

This file configures **build settings** for Rust. Here's what's in yours:

```toml
[target.x86_64-pc-windows-msvc]
rustflags = [
    "-L", "C:/Users/abdul/OneDrive/Desktop/Taurscribe/taurscribe-runtime/bin",
]

[env]
WHISPER_LIB_DIR = "C:/Users/abdul/OneDrive/Desktop/Taurscribe/taurscribe-runtime/bin"
VULKAN_SDK = "C:/VulkanSDK/1.4.335.0"
LIBCLANG_PATH = "C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/Llvm/x64/bin"

[target.x86_64-pc-windows-gnu]
linker = "C:\\msys64\\ucrt64\\bin\\gcc.exe"
ar = "C:\\msys64\\ucrt64\\bin\\ar.exe"

[build]
target-dir = "C:/bld"
```

#### **Line-by-Line Explanation**:

| Section | What it does | Why? |
|---------|--------------|------|
| `[target.x86_64-pc-windows-msvc]` | Settings for Windows + MSVC compiler | Your primary build target |
| `rustflags = ["-L", "..."]` | Add library search path | Tells linker where to find `ggml.dll`, `ggml-cuda.dll`, etc. |
| `WHISPER_LIB_DIR` | Where pre-built Whisper libraries are | Skips building from source (saves hours!) |
| `VULKAN_SDK` | Vulkan SDK installation path | Needed for Vulkan GPU acceleration |
| `LIBCLANG_PATH` | Clang compiler for bindings | `whisper-rs` uses `bindgen` which needs Clang |
| `[target.x86_64-pc-windows-gnu]` | Settings for MinGW/GCC builds | Alternative compiler (not used by default) |
| `linker = "gcc.exe"` | Use GCC as linker | For MinGW builds |
| `target-dir = "C:/bld"` | Build output directory | Shortens paths to avoid Windows 260-char limit |

**Critical**: Without this file, `cargo build` would **fail** because it couldn't find the Whisper DLLs or Vulkan SDK!

---

## Model Embedding vs. Separate Files

This is a **critical architectural decision**. Let's analyze both approaches.

---

### **Option 1: Embed Models in Binary** ❌ Not Recommended

```rust
const MODEL_BYTES: &[u8] = include_bytes!("../models/ggml-base.en.bin");

pub fn initialize(&mut self) -> Result<(), String> {
    let ctx = WhisperContext::new_from_buffer(MODEL_BYTES)?;
    self.context = Some(ctx);
    Ok(())
}
```

#### **Pros**:
✅ Single executable (no external files)  
✅ Can't "lose" the model file  
✅ Simpler deployment

#### **Cons**:
❌ **Massive binary size** (3 GB for large-v3!)  
❌ **Long compile times** (includes model in every build)  
❌ **No user choice** (can't switch models without recompiling)  
❌ **Memory usage** (entire model loaded into RAM always)  
❌ **Not practical for large models**

---

### **Option 2: Separate Files in `taurscribe-runtime/`** ✅ **RECOMMENDED**

```rust
let model_path = "taurscribe-runtime/models/ggml-base.en-q5_0.bin";
let ctx = WhisperContext::new(&model_path)?;
```

#### **Pros**:
✅ **Small binary** (~10 MB vs. 3 GB)  
✅ **User can choose models** (swap files without recompiling)  
✅ **Fast compilation** (model not included in build)  
✅ **Multiple models** (user can download tiny, base, large as needed)  
✅ **Lazy loading** (only load model when needed)  
✅ **Easy updates** (replace model file without app update)

#### **Cons**:
❌ Must ship model files separately  
❌ User could delete/corrupt model files  
❌ Need to handle missing file errors

---

### **Hybrid Approach: Best of Both Worlds** ⭐ **IDEAL**

```rust
pub fn initialize(&mut self) -> Result<(), String> {
    // 1. Try user-selected model from config
    if let Some(custom_path) = load_user_config() {
        if let Ok(ctx) = WhisperContext::new(&custom_path) {
            self.context = Some(ctx);
            return Ok(());
        }
    }
    
    // 2. Try default model directory
    let default_path = "taurscribe-runtime/models/ggml-base.en-q5_0.bin";
    if let Ok(ctx) = WhisperContext::new(default_path) {
        self.context = Some(ctx);
        return Ok(());
    }
    
    // 3. Fallback: Download model on first run
    println!("No model found, downloading ggml-base.en-q5_0.bin...");
    download_model("base.en-q5_0")?;
    
    let ctx = WhisperContext::new(default_path)?;
    self.context = Some(ctx);
    Ok(())
}
```

---

### **Recommendation for Taurscribe**

**Current Approach**: ✅ **Separate files in `taurscribe-runtime/`**

**Future Enhancements**:

1. **Add model selector in UI**:
   ```typescript
   <select onChange={(e) => setModel(e.target.value)}>
     <option value="tiny.en">Tiny (75 MB, fastest)</option>
     <option value="base.en-q5_0">Base Q5 (52 MB, recommended)</option>
     <option value="large-v3">Large V3 (3 GB, best quality)</option>
   </select>
   ```

2. **Add Tauri command**:
   ```rust
   #[tauri::command]
   fn set_model(path: String, state: State<AudioState>) -> Result<(), String> {
       state.whisper.lock().unwrap().load_model(&path)
   }
   ```

3. **Add model downloader**:
   ```rust
   async fn download_model(model_name: &str) -> Result<(), String> {
       let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}.bin", model_name);
       // Download and save to taurscribe-runtime/models/
   }
   ```

---

### **Distribution Strategy**

**Option A**: Ship with one model (recommended)
```
taurscribe-installer.exe (150 MB)
└── Contains:
    ├── taurscribe.exe (10 MB)
    └── taurscribe-runtime/models/ggml-base.en-q5_0.bin (52 MB)
```

**Option B**: Ship with model downloader
```
taurscribe-installer.exe (10 MB)
└── On first run:
    User chooses model → Downloads to taurscribe-runtime/models/
```

**Option C**: Ship with multiple models (power users)
```
taurscribe-installer.exe (850 MB)
└── Contains all 6 models
    User chooses in settings
```

---

## Common Beginner Questions

### **Q1: Why do we need threads?**

**A**: Audio recording must be **non-blocking**. Imagine:

```
❌ WITHOUT THREADS:
1. Record audio (blocks for 30 seconds)
2. Save to file (blocks for 2 seconds)
3. Transcribe (blocks for 5 seconds)
Total: 37 seconds of frozen UI!

✅ WITH THREADS:
1. Main thread: Handles UI (always responsive)
2. Audio thread: Records in background
3. File thread: Saves in background
4. Whisper thread: Transcribes in background
Result: UI never freezes!
```

---

### **Q2: What's the difference between `Arc`, `Mutex`, and `Rc`?**

| Type | Usage | Thread-Safe? | Example |
|------|-------|--------------|---------|
| `Rc<T>` | Single thread | ❌ No | Shared data in single-threaded app |
| `Arc<T>` | Multiple threads | ✅ Yes | Shared read-only data |
| `Mutex<T>` | Exclusive access | ✅ Yes | Shared **mutable** data |
| `Arc<Mutex<T>>` | Shared mutable data across threads | ✅ Yes | Our `WhisperManager` |

---

### **Q3: Why `unwrap()` everywhere? Isn't that bad?**

**A**: Yes! In production, replace with proper error handling:

```rust
// ❌ Current code
let handle = state.recording_handle.lock().unwrap();

// ✅ Better
let handle = state.recording_handle.lock()
    .map_err(|e| format!("Failed to acquire lock: {}", e))?;
```

`unwrap()` panics (crashes) if the `Result` is `Err`. Use `?` or `match` for graceful errors.

---

### **Q4: What's the `?` operator?**

**A**: Shorthand for error propagation:

```rust
// ❌ Verbose way
let file = match File::open("data.txt") {
    Ok(f) => f,
    Err(e) => return Err(e.to_string()),
};

// ✅ With `?`
let file = File::open("data.txt").map_err(|e| e.to_string())?;
```

If `Result` is `Ok(value)`, extract `value`. If `Err(e)`, return early with the error.

---

### **Q5: Why is audio at 16kHz, not 48kHz?**

**A**: Whisper's AI model was trained on 16kHz audio. Higher rates:
- Waste processing power (more samples to analyze)
- Don't improve accuracy (speech is mostly < 8kHz frequencies)
- Need resampling anyway

**Analogy**: Whisper is like a chef trained on Italian recipes. Giving it French ingredients (48kHz) requires translation (resampling) to Italian (16kHz).

---

### **Q6: What happens if I click "Stop" while transcription is running?**

**A**: Graceful shutdown!

```
1. drop(whisper_tx) → Closes channel
2. Whisper thread sees "channel closed"
3. Processes remaining buffered audio
4. Exits cleanly
5. Main thread runs final transcription on saved file
```

No data loss, no crashes!

---

### **Q7: Can I use this for non-English?**

**A**: Yes! Edit `whisper.rs`:

```rust
params.set_language(Some("es"));  // Spanish
params.set_language(Some("fr"));  // French
params.set_language(Some("ja"));  // Japanese
params.set_language(None);        // Auto-detect
```

---

## Conclusion

This architecture provides:

✅ **Real-time feedback** (6-second chunks)  
✅ **High-quality final transcript** (full file processing)  
✅ **Thread safety** (Rust's ownership prevents bugs)  
✅ **GPU acceleration** (CUDA/Vulkan auto-detection)  
✅ **Flexible model management** (separate files, user choice)  
✅ **Responsive UI** (non-blocking background processing)

**Next Steps**:
1. Add model selector in UI
2. Save transcripts to files
3. Add progress indicators
4. Implement Voice Activity Detection
5. Package as installer

---

**Questions?** Review this document or check the code comments in `lib.rs` and `whisper.rs`!
