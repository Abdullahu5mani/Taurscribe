# Taurscribe

**High-Performance Real-Time Transcription Engine**

Taurscribe is a **local-first, privacy-focused** speech-to-text application built with Rust and Tauri. It delivers real-time transcription using OpenAI's Whisper model with GPU acceleration (CUDA/Vulkan), achieving latency competitive with cloud services—all without sending your audio data to external servers.

## 🎯 Project Vision

> *"Fast, Practical, Local, Private: Beat commercial cloud latency with bare-metal Rust + Whisper"*

Taurscribe aims to rival commercial cloud services in transcription speed and accuracy while keeping all processing entirely on your machine. No internet required, no data leaks, no API costs.

---

## 🏗️ Architecture Overview

Taurscribe uses a **dual-transcription pipeline** for optimal user experience:

```
┌─────────────────────────────────────────────────────────────┐
│                      FRONTEND (React + Vite)                 │
│  ┌──────────────┐                        ┌──────────────┐   │
│  │ Start Button │                        │ Stop Button  │   │
│  └──────┬───────┘                        └──────┬───────┘   │
└─────────┼──────────────────────────────────────┼───────────┘
          │                                       │
          │ invoke("start_recording")             │ invoke("stop_recording")
          │                                       │
┌─────────▼───────────────────────────────────────▼───────────┐
│                    BACKEND (Rust + Tauri)                    │
│                                                              │
│  ┌────────────────────────────────────────────────────┐     │
│  │            Audio Input Stream (cpal)               │     │
│  │         [Microphone → f32 samples @ 48kHz]        │     │
│  └─────┬───────────────────────────────────┬─────────┘     │
│        │                                    │               │
│        │ Stereo                             │ Mono          │
│        ▼                                    ▼               │
│  ┌──────────┐                      ┌────────────────┐      │
│  │  Thread 1│                      │    Thread 2    │      │
│  │   WAV    │                      │    WHISPER     │      │
│  │  Writer  │                      │  (Real-Time)   │      │
│  │          │                      │                │      │
│  │ Saves    │                      │ • Buffers 6s   │      │
│  │ Full     │                      │ • Converts     │      │
│  │ Quality  │                      │   48→16kHz     │      │
│  │ Audio    │                      │ • Transcribes  │      │
│  └──────────┘                      │ • Prints live  │      │
│                                    │   to console   │      │
│                                    └────────────────┘      │
│                                                              │
│  ┌────────────────────────────────────────────────────┐     │
│  │            ON STOP: Final Transcription            │     │
│  │  1. Close channels → flush threads                 │     │
│  │  2. Load saved WAV file                            │     │
│  │  3. Run high-quality transcription on full file    │     │
│  │  4. Return final transcript to frontend            │     │
│  └────────────────────────────────────────────────────┘     │
│                                                              │
│  ┌────────────────────────────────────────────────────┐     │
│  │          Whisper Manager (whisper.rs)              │     │
│  │  • GPU Auto-Detection (CUDA → Vulkan → CPU)       │     │
│  │  • Model: ggml-base.en-q5_0.bin                    │     │
│  │  • Context history for better accuracy             │     │
│  │  • Automatic resampling (any rate → 16kHz)         │     │
│  └────────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────────┘
```

---

## 🧩 How It Works

### 1. **Audio Capture** (`lib.rs`)

When you click **Start Recording**:

- Uses `cpal` to access your system microphone
- Creates a real-time audio stream (typically 48kHz stereo)
- Spawns **two parallel processing threads** via `crossbeam-channel`

### 2. **Dual Processing Pipeline**

#### **Thread 1: File Writer**
- Receives **stereo** audio samples (L/R channels intact)
- Writes to WAV file using `hound` crate
- Preserves full quality for final transcription
- Finishes when `stop_recording` is called

#### **Thread 2: Live Transcription**
- Receives **mono** audio (stereo mixed to single channel)
  - *Why?* Whisper interprets stereo as 2× speed, causing hallucinations
- Buffers 6-second chunks (balancing speed vs. accuracy)
  - *Why 6s?* 3s clips cause sentence cuts → "Our evidence is a key" errors
- Resamples 48kHz → 16kHz using `rubato`
- Feeds to Whisper for transcription
- Prints live results to console

### 3. **Whisper Transcription** (`whisper.rs`)

The `WhisperManager` handles all AI processing:

```rust
┌─────────────────────────────────────────┐
│      WhisperManager::initialize()       │
├─────────────────────────────────────────┤
│ 1. Load model from disk                 │
│    (taurscribe-runtime/models/)         │
│                                         │
│ 2. Try GPU acceleration:                │
│    ✓ CUDA (NVIDIA RTX 4070)            │
│    ✓ Vulkan (AMD 780M/any GPU)         │
│    ✓ CPU (fallback)                    │
│                                         │
│ 3. Suppress C++ logs                   │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│   WhisperManager::transcribe_chunk()    │
├─────────────────────────────────────────┤
│ INPUT: f32 samples, sample_rate         │
│                                         │
│ 1. Resample to 16kHz (if needed)       │
│ 2. Create Whisper state                │
│ 3. Set params:                          │
│    • Language: English                  │
│    • Threads: 4                         │
│    • Context: Previous transcript       │
│ 4. Run inference                        │
│ 5. Extract segments                     │
│ 6. Return text + performance metrics    │
│                                         │
│ OUTPUT: String (transcribed text)       │
└─────────────────────────────────────────┘

┌─────────────────────────────────────────┐
│   WhisperManager::transcribe_file()     │
├─────────────────────────────────────────┤
│ INPUT: WAV file path                    │
│                                         │
│ 1. Load WAV with hound                  │
│ 2. Convert stereo → mono               │
│ 3. Resample to 16kHz in chunks          │
│ 4. Run full transcription               │
│ 5. Return complete transcript           │
│                                         │
│ OUTPUT: High-quality final text         │
└─────────────────────────────────────────┘
```

### 4. **Memory Safety**

Rust's ownership system ensures:
- No data races between threads
- Automatic cleanup when recording stops
- Type-safe communication via channels

The `crossbeam-channel` crate provides:
```rust
let (tx, rx) = unbounded::<Vec<f32>>();
tx.send(samples)?;  // Thread-safe send
rx.recv()?;         // Thread-safe receive
```

### 5. **GPU Acceleration**

Built with `whisper-rs` featuring:
- **CUDA**: NVIDIA RTX cards (fastest)
- **Vulkan**: Universal GPU support (AMD, Intel, NVIDIA)
- **CPU**: Automatic fallback

The backend auto-selects the best available option at runtime.

---

## 📊 Available Models

Taurscribe supports multiple Whisper models located in `taurscribe-runtime/models/`:

| Model Name | Size | Speed (RTX 4070) | Use Case | Currently Used |
|------------|------|------------------|----------|----------------|
| **ggml-tiny.en.bin** | 75 MB | ~0.15s | Ultra-fast, lower accuracy | ❌ |
| **ggml-base.en-q5_0.bin** | 52 MB | ~0.37s | **Recommended** - Best balance | ✅ **Active** |
| **ggml-base.en.bin** | 142 MB | ~0.45s | Good balance of speed/quality | ❌ |
| **ggml-small.en.bin** | 487 MB | ~1.2s | High accuracy, slower | ❌ |
| **ggml-large-v3.bin** | 3.0 GB | ~3.5s | Maximum accuracy | ❌ |
| **ggml-silero-v6.2.0.bin** | 864 KB | N/A | Voice Activity Detection (VAD) | ❌ |

### Model Details

#### **Currently Active: `ggml-base.en-q5_0.bin`**
- **Accuracy**: 97% of base.en quality
- **Speed**: 0.37s for 11s audio (~30× realtime)
- **Size**: 3× smaller than base.en
- **Best for**: Production deployment - fast, accurate, small

#### **Maximum Quality: `ggml-large-v3.bin`**
- **Accuracy**: Best available (multilingual, 1550M parameters)
- **Speed**: 3.5s for 11s audio on RTX 4070 (~3.1× realtime)
- **RAM**: ~6-8 GB during inference
- **Best for**: Maximum transcription quality

#### **Speed Demon: `ggml-tiny.en.bin`**
- **Accuracy**: Basic transcription
- **Speed**: 0.15s for 11s audio (~73× realtime)
- **Best for**: Testing, prototyping

#### **Voice Activity Detection: `ggml-silero-v6.2.0.bin`**
- **Purpose**: Detect speech vs. silence
- **Use**: Can optimize by skipping silent chunks

### Switching Models

To change the active model, edit `src-tauri/src/whisper.rs`:

```rust
// Line 60
let model_path = "taurscribe-runtime/models/ggml-base.en-q5_0.bin";  // Change this
```

Or for dynamic selection (future feature), implement model switching via Tauri commands.

---

## 🚀 Getting Started

### Prerequisites

- **Rust** (1.70+): [Install](https://rustup.rs/)
- **Node.js** (18+): [Install](https://nodejs.org/)
- **GPU Drivers** (optional but recommended):
  - NVIDIA: CUDA Toolkit 11.8+
  - AMD/Intel: Latest Vulkan drivers

### Installation

```bash
# Clone repository
git clone https://github.com/Abdullahu5mani/Taurscribe.git
cd Taurscribe

# Install frontend dependencies
npm install

# Download Whisper models (if not included)
# Models go in: taurscribe-runtime/models/
# Download from: https://huggingface.co/ggerganov/whisper.cpp
```

### Development

```bash
# Run in development mode (hot reload for frontend, auto-recompile for Rust)
npm run tauri dev
# or with bun:
bun run tauri dev
```

**Performance Tips:**
- ⚡ **First run is slow** (~2-5 min) - compiling whisper-rs with CUDA/Vulkan
- 🔥 **Keep it running!** - Frontend changes hot-reload instantly
- 🦀 **Rust changes** - Only recompile what changed (~10-30s)
- 💡 **Don't restart** unless you change Cargo.toml dependencies

### Production Build

```bash
# Build optimized executable
npm run tauri build
```

Output: `src-tauri/target/release/taurscribe.exe`

---

## 🧪 Testing

### 1. **Basic Recording Test**
1. Launch the app
2. Click **Start Recording**
3. Speak clearly: *"This is a test of the Taurscribe transcription system"*
4. Click **Stop Recording**
5. Check console for live transcription + final output

### 2. **Console Output Example**

```
[INFO] Initializing Whisper transcription engine...
[INFO] Loading Whisper model from disk: 'C:\...\ggml-base.en-q5_0.bin'
[GPU] Attempting GPU acceleration...
[SUCCESS] ✓ GPU acceleration enabled (CUDA)
[INFO] Backend: CUDA
[INFO] Warming up GPU...
[INFO] GPU warm-up complete

[INFO] Whisper thread started
[PROCESSING] Transcribing 6.00s chunk (288000 samples)...
[PERF] Processed 6.00s audio in 370ms | Speed: 16.2x
[TRANSCRIPT] "This is a test"

[INFO] Recording stopped, processing remaining audio...
[PROCESSING] Running final high-quality transcription on: recording_1737280323.wav
[FINAL_TRANSCRIPT]
This is a test of the Taurscribe transcription system.
```

### 3. **Performance Benchmarks**

On **NVIDIA RTX 4070** + **AMD Ryzen 9 7940HS**:

| Model | Cold Start | Warm Encode | Total (11s audio) | Realtime Factor |
|-------|-----------|-------------|-------------------|-----------------|
| base.en-q5_0 (CUDA) | 0.22s | 0.15s | 0.37s | 30× |
| large-v3 (CUDA) | 1.85s | 1.65s | 3.50s | 3.1× |
| tiny.en (CUDA) | 0.08s | 0.07s | 0.15s | 73× |

See `BENCHMARK_RESULTS.md` for detailed metrics.

---

## 📁 Project Structure

```
Taurscribe/
├── src/                         # Frontend (React + TypeScript)
│   ├── App.tsx                  # Main UI component
│   ├── App.css                  # Styles
│   └── main.tsx                 # Entry point
│
├── src-tauri/                   # Backend (Rust)
│   ├── src/
│   │   ├── lib.rs               # Audio recording + Tauri commands
│   │   └── whisper.rs           # Whisper transcription engine
│   ├── Cargo.toml               # Rust dependencies
│   └── tauri.conf.json          # Tauri configuration
│
├── taurscribe-runtime/          # Whisper binaries & models
│   ├── bin/                     # whisper.exe executables + DLLs
│   ├── models/                  # GGML model files (.bin)
│   └── samples/                 # Test audio (jfk.wav)
│
├── package.json                 # Node.js dependencies
├── vite.config.ts               # Vite bundler config
└── README.md                    # This file
```

---

## 🔧 Key Technologies

| Component | Technology | Purpose |
|-----------|-----------|---------|
| **Framework** | [Tauri 2.0](https://tauri.app/) | Lightweight Rust + Web UI |
| **Frontend** | React 19 + Vite | Modern reactive UI |
| **Audio** | [cpal](https://github.com/RustAudio/cpal) | Cross-platform audio I/O |
| **AI Model** | [whisper-rs](https://github.com/tazz4843/whisper-rs) | Rust bindings for Whisper.cpp |
| **GPU** | CUDA + Vulkan | Hardware acceleration |
| **Resampling** | [rubato](https://github.com/HEnquist/rubato) | High-quality audio resampling |
| **WAV I/O** | [hound](https://github.com/ruuda/hound) | WAV file reading/writing |
| **Threading** | [crossbeam](https://github.com/crossbeam-rs/crossbeam) | Lock-free concurrent channels |

---

## 🎛️ Configuration

### Audio Settings

Edit `src-tauri/src/lib.rs`:

```rust
// Chunk size (line 99)
let chunk_size = (sample_rate * 6) as usize;  // 6 seconds

// Max buffer (line 100)
let max_buffer_size = chunk_size * 2;  // 12 seconds total

// Thread count for Whisper (whisper.rs line 149)
params.set_n_threads(4);
```

### Whisper Parameters

Edit `src-tauri/src/whisper.rs`:

```rust
// Language (line 151)
params.set_language(Some("en"));  // "en", "es", "fr", etc.

// Model path (line 60)
let model_path = "taurscribe-runtime/models/ggml-base.en-q5_0.bin";

// GPU toggle (line 66)
params.use_gpu(true);  // false to force CPU
```

---

## 🐛 Troubleshooting

### **"No input device"**
- **Cause**: Microphone not connected or permissions denied
- **Fix**: Check Windows Privacy Settings → Microphone → Allow desktop apps

### **"Failed to initialize Whisper"**
- **Cause**: Model file missing or corrupted
- **Fix**: Re-download model from [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp)
- **Path**: Must be in `taurscribe-runtime/models/`

### **"GPU failed" / Fallback to CPU**
- **Cause**: Missing GPU drivers or unsupported GPU
- **Fix**: 
  - NVIDIA: Install [CUDA Toolkit](https://developer.nvidia.com/cuda-downloads)
  - AMD/Intel: Update graphics drivers for Vulkan support
- **Workaround**: CPU mode still works, just slower

### **Hallucinations** ("Thank you for watching!", random text)
- **Cause**: Silent audio or background noise
- **Fix**:
  1. Ensure microphone is working (`recording_*.wav` should have actual audio)
  2. Reduce chunk size (line 99 in `lib.rs`)
  3. Use Voice Activity Detection (VAD) to skip silence

### **Slow Transcription**
- **Cause**: Using large model on CPU
- **Solutions**:
  1. Switch to smaller model (`base.en-q5_0`)
  2. Enable GPU acceleration
  3. Reduce thread count if thermal throttling

---

## 🚧 Known Limitations

1. **Model switching requires code edit** (no GUI selector yet)
2. **No persistent storage** (transcripts only shown in console)
3. **English-only optimization** (multilingual support exists but untested)
4. **Windows-only tested** (Linux/macOS should work but unverified)

---

## 🗺️ Roadmap

- [x] Real-time audio capture
- [x] GPU-accelerated transcription
- [x] Dual-pipeline live + final transcription
- [x] GPU backend detection (CUDA/Vulkan/CPU)
- [ ] **GUI model selector**
- [ ] **Save transcripts to file**
- [ ] **Export to TXT/SRT/VTT**
- [ ] **Voice Activity Detection integration**
- [ ] **WebSocket frontend updates** (replace console logs)
- [ ] **macOS/Linux support**
- [ ] **Installer packaging**

---

## 📄 License

This project is open-source under the MIT License.

**Third-Party Components:**
- Whisper.cpp: MIT License
- Tauri: MIT/Apache 2.0
- All Rust crates: See individual licenses in `Cargo.toml`

---

## 🙏 Acknowledgments

- **OpenAI** for Whisper
- **Georgi Gerganov** for [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
- **tazz4843** for [whisper-rs](https://codeberg.org/tazz4843/whisper-rs)
- **Tauri Team** for the amazing framework

---

## 📞 Support

- **Issues**: [GitHub Issues](https://github.com/Abdullahu5mani/Taurscribe/issues)
- **Discussions**: [GitHub Discussions](https://github.com/Abdullahu5mani/Taurscribe/discussions)

---

**Built with ❤️ using Rust and Tauri**
