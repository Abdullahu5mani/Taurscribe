# macOS Deployment Target Fix - Deep Dive

## The Problem
The GitHub Actions build was failing on macOS (both Intel and Apple Silicon) with C++ filesystem API errors from whisper.cpp, even after setting `MACOSX_DEPLOYMENT_TARGET`.

## Root Cause Analysis

### What Went Wrong
1. **whisper.cpp uses C++17 filesystem APIs** that were introduced in macOS 10.15
2. **CMake was hardcoding** the deployment target to 10.13 with the flag `-mmacosx-version-min=10.13`
3. **Setting MACOSX_DEPLOYMENT_TARGET alone wasn't enough** because CMake has its own environment variable

### The Build Chain
```
Cargo build → whisper-rs-sys → CMake → whisper.cpp C++ code
```

The `whisper-rs-sys` crate uses a `build.rs` script that invokes CMake to compile whisper.cpp. CMake needs to know the correct deployment target to pass the right flags to the C++ compiler.

## The Solution

### Environment Variables Required
CMake looks for **TWO** environment variables when determining the macOS deployment target:

1. **`MACOSX_DEPLOYMENT_TARGET`** - Standard macOS environment variable
2. **`CMAKE_OSX_DEPLOYMENT_TARGET`** - CMake-specific override

Both must be set to `10.15` for the build to succeed.

### Implementation
In `.github/workflows/build.yml`:

```yaml
- name: Build Tauri application
  uses: tauri-apps/tauri-action@v0
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
    # Set macOS deployment target to 10.15 for C++ filesystem API support
    MACOSX_DEPLOYMENT_TARGET: ${{ matrix.platform == 'macos-latest' && '10.15' || '' }}
    CMAKE_OSX_DEPLOYMENT_TARGET: ${{ matrix.platform == 'macos-latest' && '10.15' || '' }}
  with:
    args: ${{ matrix.args }}
```

### Why This Works
- **`CMAKE_OSX_DEPLOYMENT_TARGET`** is specifically checked by CMake's platform detection
- CMake will then configure the build with `-mmacosx-version-min=10.15` instead of 10.13
- The C++ compiler can then use the C++17 filesystem APIs from `<filesystem>` header
- Only set for macOS builds using a conditional expression

## Verification

After this fix, the CMake configuration should show:
```
-- CMAKE_OSX_DEPLOYMENT_TARGET = 10.15
```

And the compiler flags should include:
```
-mmacosx-version-min=10.15
```

Instead of the previous:
```
-mmacosx-version-min=10.13
```

## What macOS 10.15 Gives Us
- **C++17 `<filesystem>` library** - Full support for std::filesystem::path and related APIs
- **Modern C++ features** - Better standard library support
- **Compatibility** - Still supports most modern Macs (macOS Catalina from 2019 onwards)

## Lessons Learned
1. **CMake has its own environment variables** - Don't assume standard env vars are enough
2. **Check the actual compiler flags** - The logs show `-mmacosx-version-min` which is the smoking gun
3. **Multiple variables for safety** - Setting both MACOSX_DEPLOYMENT_TARGET and CMAKE_OSX_DEPLOYMENT_TARGET ensures compatibility
4. **Build scripts can override** - The whisper-rs-sys build.rs uses the cmake crate which respects CMAKE_* variables

## References
- [CMake CMAKE_OSX_DEPLOYMENT_TARGET](https://cmake.org/cmake/help/latest/variable/CMAKE_OSX_DEPLOYMENT_TARGET.html)
- [C++17 filesystem availability on macOS](https://developer.apple.com/documentation/xcode-release-notes/xcode-11-release-notes)
- [whisper.cpp repository](https://github.com/ggerganov/whisper.cpp)
