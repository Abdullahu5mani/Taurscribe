# Taurscribe - Recent Updates & Current Status

> **Last Updated**: January 20, 2026

---

## 🎉 **Latest Changes**

### **✅ AppData Storage Implementation** (January 20, 2026)

**What Changed:**
- Recordings now save to proper AppData directory instead of build folder
- Cross-platform path handling via `dirs` crate
- Automatic directory creation on first run

**File Locations:**

| Platform | Recording Path |
|----------|----------------|
| **Windows** | `C:\Users\{you}\AppData\Local\Taurscribe\temp\` |
| **macOS** | `/Users/{you}/Library/Application Support/Taurscribe/temp/` |
| **Linux** | `/home/{you}/.local/share/Taurscribe/temp/` |

**Benefits:**
- ✅ No more files in build directory (`C:\bld\debug\`)
- ✅ Proper cross-platform behavior
- ✅ Data persists across app updates
- ✅ Clean separation of user data and code

### **🔧 Performance Optimizations** (January 19-20, 2026)

**Added:**
1. **Detailed Timing Logs** - Track exactly where time is spent:
   ```
   [TIMING] Step 1 (File I/O): 131ms
   [TIMING] Step 2 (Stereo→Mono): 24ms
   [TIMING] Step 3 (Resampling 48kHz→16kHz): 61ms
   [TIMING] Step 4 (State Setup): 63ms
   [TIMING] Step 5 (Whisper AI): 1581ms | 11.1x realtime
   [TIMING] Step 6 (Extract Text): 0ms
   [BREAKDOWN] I/O:131ms + Stereo:24ms + Resample:61ms + Setup:63ms + AI:1581ms + Extract:0ms
   ```

2. **Optimized Whisper Parameters:**
   - Increased threads: 4 → 8
   - Disabled token timestamps (speed boost)
   - Added `max_len` limit

3. **File I/O Optimization:**
   - Pre-allocated vectors with known capacity
   - Reduced memory allocations

4. **Fixed Audio Duration Calculation:**
   - Now correctly handles stereo channels
   - Accurate realtime speed reporting

**Results:**
- base.en-q5_0: 40-45× realtime on CUDA
- large-v3-turbo: 9-12× realtime on CUDA/Vulkan
- Model stays loaded in GPU memory (no reloading)

---

## 📊 **Current Performance** (January 2026)

### **Hardware:** NVIDIA RTX 4070 + AMD 780M (Ryzen 9 7940HS)

| Model | Backend | Speed | Use Case |
|-------|---------|-------|----------|
| tiny.en | CUDA | 74× realtime | Prototyping |
| base.en-q5_0 | CUDA | 41× realtime | **Recommended** |
| large-v3 | CUDA | 12× realtime | Maximum accuracy |
| large-v3-turbo-q5_0 | Vulkan | 9× realtime | Current test model |

### **Performance Breakdown** (13s audio, large-v3-turbo, Vulkan):
```
Total Time: 1860ms (9.4× realtime)
├─ File I/O:     131ms (7%)
├─ Stereo→Mono:   24ms (1%)
├─ Resampling:    61ms (3%)
├─ State Setup:   63ms (3%)
├─ Whisper AI:  1581ms (85%)  ← The actual AI
└─ Extract Text:   0ms (0%)
```

**Key Insight**: 85% of time is in AI inference - overhead is minimal!

---

## 🏗️ **Architecture Status**

### **✅ Working Features:**

1. **Audio Recording**
   - Cross-platform microphone access (cpal)
   - Real-time dual-pipeline processing
   - Automatic AppData storage

2. **Whisper Transcription**
   - GPU auto-detection (CUDA → Vulkan → CPU)
   - Model persistence (loaded once, stays in memory)
   - Live chunk processing (6-second buffer)
   - Final high-quality pass

3. **Performance**
   - GPU warm-up to eliminate cold start
   - Optimized threading (8 threads)
   - Detailed performance metrics

### **🚧 In Development:**

1. **SQLite Database** (Planned)
   - Schema designed (see `DATABASE_PLAN.md` concept)
   - Will store: transcripts, timestamps, metadata
   - Features: Full-text search (FTS5), date grouping

2. **History UI** (Planned)
   - Scrollable transcript list (like Wispr Flow)
   - Search functionality
   - Export options

3. **Audio Auto-Delete** (Next Step)
   - Save transcript to database
   - Delete audio file immediately after
   - Privacy + disk space savings

---

## 🔬 **Technical Details**

### **Dependencies Added:**

```toml
# New in January 2026:
dirs = "6.0"              # Cross-platform AppData paths
tauri-plugin-fs = "2"     # File system operations

# Planned:
rusqlite = "0.31"         # SQLite database (not yet added)
```

### **Code Changes:**

**`lib.rs`:**
- Added `get_recordings_dir()` helper function
- Updated recording path to use AppData
- Added logging for file save location

**`whisper.rs`:**
- Added detailed timing for each processing step
- Optimized thread count (8)
- Disabled token timestamps
- Pre-allocated I/O buffers

**`Cargo.toml`:**
- Added `dirs` dependency
- Added `tauri-plugin-fs`

---

## 🌍 **Cross-Platform Status**

| Platform | Status | GPU Support | Notes |
|----------|--------|-------------|-------|
| **Windows** | ✅ Fully Tested | CUDA, Vulkan | Primary development platform |
| **macOS** | ⚠️ Untested | CPU, Metal* | Paths work, GPU needs Metal feature |
| **Linux** | ⚠️ Untested | Vulkan, CUDA | Paths work, should work fine |

**To enable macOS GPU:**
```toml
# Add to Cargo.toml:
whisper-rs = { 
    git = "...", 
    features = ["cuda", "vulkan", "metal", "coreml"] 
}
```

---

## 🐛 **Known Issues**

### **Resolved:**
- ✅ Files saving to build directory → Fixed (AppData)
- ✅ Model reloading every time → Fixed (stays in memory)
- ✅ Thermal throttling → Fixed (plug in laptop!)
- ✅ Incorrect audio duration → Fixed (stereo calculation)

### **Active:**
- ⚠️ Transcripts only in console (no UI display yet)
- ⚠️ Audio files not auto-deleted (upcoming)
- ⚠️ No persistent history (database planned)
- ⚠️ Model switching requires code edit

---

## 📚 **Documentation Guide**

| File | Purpose | Audience |
|------|---------|----------|
| **README.md** | Project overview, setup | Everyone |
| **ARCHITECTURE.md** | Code flow, Rust concepts | Beginners learning Rust/Tauri |
| **THREADING_VISUAL_GUIDE.md** | Visual guide to threading | Visual learners |
| **PERFORMANCE_OPTIMIZATION.md** | Speed optimization tips | Power users |
| **LATEST_UPDATES.md** | This file - recent changes | Developers continuing work |
| **DOCS_GUIDE.md** | Nav

igation guide | New contributors |

---

## 🎯 **Next Steps**

### **Immediate (This Week):**
1. ✅ AppData storage - **DONE**
2. Add auto-delete for audio files after transcription
3. Set up SQLite database schema

### **Short Term (Next 2 Weeks):**
4. Implement database save/retrieve functions
5. Build history UI component
6. Add full-text search

### **Medium Term (Next Month):**
7. Live transcript display in UI (replace console)
8. Export functionality (TXT, SRT)
9. LLM formatting integration
10. Model selector dropdown

---

## 💡 **Performance Tips for Users**

### **For Maximum Speed:**
1. Use `base.en-q5_0` model (41× realtime)
2. Keep laptop plugged in (avoids power throttling)
3. Ensure CUDA is detected (check logs for "CUDA" not "Vulkan")
4. Close Chrome/browsers (GPU contention)

### **For Maximum Accuracy:**
1. Use `large-v3` model (12× realtime, still faster than realtime!)
2. Speak clearly into microphone
3. Minimize background noise
4. Use good quality microphone

### **Troubleshooting Variable Performance:**
- Check GPU temperature (thermal throttling if >75°C)
- Close background apps using GPU
- Set Windows Power Plan to "High Performance"
- Increase fan speed if using laptop

See `PERFORMANCE_OPTIMIZATION.md` for detailed guide!

---

## 🔗 **Quick Links**

- **Whisper Models**: [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp)
- **Tauri Docs**: [tauri.app/v2](https://tauri.app/v2/)
- **whisper-rs**: [Codegerg](https://codeberg.org/tazz4843/whisper-rs)
- **Issue Reports**: [GitHub Issues](https://github.com/Abdullahu5mani/Taurscribe/issues)

---

## 📝 **Session Notes**

### **Debugging Session (Jan 19-20)**
- Identified thermal throttling as cause of variable performance
- Plugging in laptop eliminated performance variance
- Model stays loaded - 85% of time is pure inference (optimal!)

### **Database Planning Session (Jan 20)**
- Designed SQLite schema for transcript storage
- Planned Wispr Flow-like history UI
- FTS5 for instant full-text search
- Auto-delete audio option for privacy

---

**Status**: Production-ready for personal use, database feature in development

**Contributors**: Working solo, contributions welcome!

**Last Build**: January 20, 2026 - All tests passing ✅
