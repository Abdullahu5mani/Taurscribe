# Taurscribe Runtime Files

This folder contains **only** the files needed to run Whisper models.

## 📁 Folder Structure

```
taurscribe-runtime/
├── bin/                          # Executables & DLLs
│   ├── whisper-cuda.exe          # NVIDIA GPU (RTX 4070, etc.)
│   ├── whisper-vulkan.exe        # AMD/Intel/NVIDIA GPU (Universal)
│   ├── whisper-cpu.exe           # CPU fallback
│   ├── ggml.dll                  # Core GGML library
│   ├── ggml-cpu.dll              # CPU backend
│   ├── ggml-cuda.dll             # CUDA backend
│   └── ggml-vulkan.dll           # Vulkan backend
│
├── models/                       # AI Models
│   ├── ggml-tiny.en.bin          # 75 MB - Fastest
│   ├── ggml-base.en.bin          # 142 MB - Recommended
│   ├── ggml-base.en-q5_0.bin     # 52 MB - Best for distribution
│   ├── ggml-small.en.bin         # 487 MB - High accuracy
│   └── ggml-silero-v6.2.0.bin    # 864 KB - Voice Activity Detection
│
└── samples/                      # Test audio
    └── jfk.wav                   # 11s test file
```

## 🚀 Usage

### Test CUDA (NVIDIA GPU)
```bash
bin/whisper-cuda.exe -m models/ggml-base.en.bin -f samples/jfk.wav
```

### Test Vulkan (Any GPU)
```bash
bin/whisper-vulkan.exe -m models/ggml-base.en.bin -f samples/jfk.wav
```

### Test CPU
```bash
bin/whisper-cpu.exe -m models/ggml-base.en.bin -f samples/jfk.wav
```

## 📦 What to Ship in Taurscribe

### Minimal (Recommended)
- `bin/whisper-vulkan.exe` + DLLs
- `models/ggml-base.en-q5_0.bin` (52 MB)
- `models/ggml-silero-v6.2.0.bin` (VAD)

**Total Size:** ~90 MB

### Full Package
- All 3 executables
- All 4 models
- All DLLs

**Total Size:** ~850 MB

## 🎯 Recommended Model

**`ggml-base.en-q5_0.bin`** (52 MB)
- Fast: 0.37s on RTX 4070, 0.55s on Radeon 780M
- Small: 3× smaller than base.en
- Accurate: Near-identical quality to full base.en
