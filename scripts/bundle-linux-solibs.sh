#!/usr/bin/env bash
# =============================================================================
# Linux shared library bundling for Taurscribe distribution
# =============================================================================
#
# WHY THIS SCRIPT EXISTS:
# llama-cpp-2 uses dynamic-link, producing libllama.so / libggml*.so files.
# ort with download-binaries produces libonnxruntime.so.
# Tauri does not auto-bundle these; without them, users get:
#   error while loading shared libraries: libllama.so: cannot open shared object file
#
# HOW IT WORKS:
# 1. Locate .so files built by Cargo (llama-cpp-sys-2 and ort-sys build outputs)
# 2. Copy them to src-tauri/linux-libs/
# 3. patchelf rewrites the binary's RPATH so it finds the libs at the installed path:
#      $ORIGIN/../share/taurscribe   →  /usr/share/taurscribe/ (.deb / .rpm)
#      $ORIGIN/../share/taurscribe   →  AppDir/usr/share/taurscribe/ (AppImage)
# 4. tauri.linux.conf.json lists linux-libs/* as resources; Tauri copies them
#    into usr/share/taurscribe/ when creating the package
#
# REQUIREMENTS:
#   patchelf — sudo apt install patchelf  OR  sudo dnf install patchelf
#
set -e

# Only run on Linux
if [ "$(uname -s)" != "Linux" ]; then
  exit 0
fi

# ---- Dependency check -------------------------------------------------------
if ! command -v patchelf &>/dev/null; then
  echo "bundle-linux-solibs: ERROR - patchelf not found."
  echo "Install with: sudo apt install patchelf   OR   sudo dnf install patchelf"
  exit 1
fi

# ---- Path setup -------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SRC_TAURI="$PROJECT_ROOT/src-tauri"
TARGET_DIR="${CARGO_TARGET_DIR:-$SRC_TAURI/target}"
TARGET_TRIPLE="${TAURI_BUILD_TARGET:-}"

if [ -z "$TARGET_TRIPLE" ]; then
  TARGET_TRIPLE=$(rustc -vV 2>/dev/null | grep 'host:' | cut -d' ' -f2)
fi

BINARY_CROSS="$TARGET_DIR/$TARGET_TRIPLE/release/taurscribe"
BINARY_HOST="$TARGET_DIR/release/taurscribe"
LIBS_DIR="$SRC_TAURI/linux-libs"

if [ -f "$BINARY_CROSS" ]; then
  BINARY="$BINARY_CROSS"
  BUILD_BASE="$TARGET_DIR/$TARGET_TRIPLE"
elif [ -f "$BINARY_HOST" ]; then
  BINARY="$BINARY_HOST"
  BUILD_BASE="$TARGET_DIR"
else
  echo "bundle-linux-solibs: Binary not found at $BINARY_CROSS or $BINARY_HOST, skipping"
  exit 0
fi

echo "bundle-linux-solibs: Using binary at $BINARY"
mkdir -p "$LIBS_DIR"
# Clean stale libs from a previous build
find "$LIBS_DIR" -name "*.so" -o -name "*.so.*" | xargs rm -f 2>/dev/null || true

# ---- Copy llama-cpp shared libs ---------------------------------------------
# llama-cpp-sys-2 builds libllama.so and libggml*.so into its build output dir.
LLAMA_BUILD_DIR=""
for candidate in \
    "$BUILD_BASE/release/build/llama-cpp-sys-2-"*/out/lib \
    "$TARGET_DIR/release/build/llama-cpp-sys-2-"*/out/lib; do
  if [ -d "$candidate" ]; then
    LLAMA_BUILD_DIR="$candidate"
    break
  fi
done

if [ -n "$LLAMA_BUILD_DIR" ]; then
  echo "bundle-linux-solibs: Copying llama-cpp libs from $LLAMA_BUILD_DIR"
  find "$LLAMA_BUILD_DIR" \( -name "*.so" -o -name "*.so.*" \) -exec cp -Lf {} "$LIBS_DIR/" \; 2>/dev/null || true
else
  echo "bundle-linux-solibs: WARNING - llama-cpp-sys-2 build output not found"
  echo "If grammar LLM is enabled, the app will fail to start."
fi

# ---- Copy libonnxruntime.so -------------------------------------------------
# ort with download-binaries places libonnxruntime.so in its build output dir.
ORT_SO=""
# Try several candidate paths (ort-sys crate name may vary with version)
while IFS= read -r candidate; do
  if [ -f "$candidate" ]; then
    ORT_SO="$candidate"
    break
  fi
done < <(find "$BUILD_BASE/release/build" "$TARGET_DIR/release/build" \
    -name "libonnxruntime.so" 2>/dev/null | sort -u)

if [ -n "$ORT_SO" ]; then
  ORT_DIR="$(dirname "$ORT_SO")"
  echo "bundle-linux-solibs: Copying ORT libs from $ORT_DIR"
  # Copy the main lib and any versioned aliases (libonnxruntime.so.1.x.x etc.)
  find "$ORT_DIR" \( -name "libonnxruntime.so" -o -name "libonnxruntime.so.*" \) \
    -exec cp -Lf {} "$LIBS_DIR/" \; 2>/dev/null || true
else
  echo "bundle-linux-solibs: WARNING - libonnxruntime.so not found in build output"
  echo "Parakeet transcription will fail to load on the target system."
fi

# ---- Report -----------------------------------------------------------------
SO_COUNT=$(find "$LIBS_DIR" \( -name "*.so" -o -name "*.so.*" \) | wc -l)
if [ "$SO_COUNT" -eq 0 ]; then
  echo "bundle-linux-solibs: WARNING - No .so files found to bundle."
  echo "The app may crash at launch if these libs are not present on the target system."
  exit 0
fi

echo "bundle-linux-solibs: Bundled $SO_COUNT shared lib(s):"
ls -lh "$LIBS_DIR"/*.so "$LIBS_DIR"/*.so.* 2>/dev/null || true

# ---- Patch binary RPATH -----------------------------------------------------
# After patchelf, the binary looks for bundled .so files at:
#   $ORIGIN/../share/taurscribe   → /usr/share/taurscribe/     (.deb / .rpm)
#                                 → AppDir/usr/share/taurscribe/ (AppImage)
# $ORIGIN is the directory containing the taurscribe binary at runtime.
echo "bundle-linux-solibs: Patching RPATH on $BINARY ..."
patchelf --set-rpath '$ORIGIN/../share/taurscribe:$ORIGIN' "$BINARY"

PATCHED_RPATH=$(patchelf --print-rpath "$BINARY")
echo "bundle-linux-solibs: Verified RPATH: $PATCHED_RPATH"
echo "bundle-linux-solibs: Done."
