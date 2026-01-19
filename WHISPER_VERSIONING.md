# Whisper.cpp Versioning in Taurscribe

## How whisper-rs Uses whisper.cpp

### Architecture
```
Taurscribe (Your App)
    └── whisper-rs (Rust bindings)
        └── whisper.cpp (C++ library, as git submodule)
            └── Pinned to specific commit
```

## Current Status

**Your whisper-rs version:** `v0.15.1` (commit `d38738df`)
**whisper.cpp version:** Likely v1.7.x - v1.8.x range

The whisper-rs maintainer periodically updates the whisper.cpp submodule to tested, stable versions.

## Why This Matters

### ✅ Advantages
- **Stability**: You get a tested, stable combo of whisper-rs + whisper.cpp
- **Compatibility**: No breaking changes from bleeding-edge whisper.cpp
- **Safety**: The maintainer ensures features still work after updates

### ⚠️ Trade-offs
- **Lag**: New whisper.cpp features take time to land in whisper-rs
- **Bug fixes**: Latest whisper.cpp bug fixes may not be immediately available

## How to Update

### Check for Updates
```bash
cd src-tauri
cargo update -p whisper-rs
```

This pulls the latest whisper-rs commit, which may include a newer whisper.cpp submodule.

### Check Current Version
```bash
cargo tree -p whisper-rs
```

Look at the commit hash, then check:
- https://codeberg.org/tazz4843/whisper-rs/commits/branch/master

### Manual Update (Advanced)
If you need bleeding-edge whisper.cpp:

1. **Fork whisper-rs**
2. **Update the submodule:**
   ```bash
   cd whisper.cpp  # in the whisper-rs repo
   git checkout master
   git pull
   ```
3. **Test it compiles**
4. **Point Cargo.toml to your fork:**
   ```toml
   whisper-rs = { git = "https://github.com/YourUsername/whisper-rs", features = ["cuda", "vulkan"] }
   ```

## Latest whisper.cpp Features

Features from whisper.cpp v1.8.3 (Jan 2026):
- ✅ **Silero VAD v6.2.0** - Voice activity detection
- ✅ **12x iGPU performance boost** - Vulkan optimizations
- ✅ **VAD API improvements** - Separate from ASR

**Availability in whisper-rs:** Check the repo's recent commits. The maintainer usually updates within 2-4 weeks of major whisper.cpp releases.

## Recommendation

**For production:** Stick with the pinned whisper-rs version
- Tested and stable
- Less likely to break

**For testing/development:** Manually update to get latest features
- More experimental
- May have bugs

## Checking whisper.cpp Version

Currently no direct API to query whisper.cpp version from whisper-rs.

**Workaround:**
Check the git submodule commit in whisper-rs, then compare to:
https://github.com/ggerganov/whisper.cpp/releases
