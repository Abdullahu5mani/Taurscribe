#!/usr/bin/env bash
# =============================================================================
# macOS dylib bundling for Taurscribe .app distribution
# =============================================================================
#
# WHY THIS SCRIPT EXISTS:
# llama-cpp-2 uses dynamic-link, so the binary depends on libllama.dylib (and
# possibly libggml.dylib). Tauri only copies the main binary into the .app;
# it does not automatically bundle these dylibs. Without them, users get:
#   dyld: Library not loaded: @rpath/libllama.dylib
#
# HOW IT WORKS:
# 1. dylibbundler finds all dylibs the binary depends on
# 2. Copies them to macos-dylibs/ and rewrites paths to @executable_path/../Frameworks
# 3. tauri.conf.json bundle.macOS.frameworks lists these paths; Tauri copies them
#    into Contents/Frameworks/ when creating the .app
#
# WHY dylibbundler (not manual copy):
# - Handles transitive deps (dylibs that depend on other dylibs)
# - Fixes @rpath in the binary so it finds libs in the app bundle
# - Standard tool for macOS app distribution (e.g. brew install dylibbundler)
#
# CI: release.yml and build.yml install dylibbundler on macOS runners.
# Local: brew install dylibbundler before building for macOS.
#
set -e

# Only run on macOS; no-op on Windows/Linux (beforeBundleCommand runs on all platforms)
if [ "$(uname)" != "Darwin" ]; then
  exit 0
fi

# Resolve paths relative to project root (where tauri build runs from)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SRC_TAURI="$PROJECT_ROOT/src-tauri"
TARGET_DIR="${CARGO_TARGET_DIR:-$SRC_TAURI/target}"
TARGET_TRIPLE="${TAURI_BUILD_TARGET:-$(rustc -vV 2>/dev/null | grep 'host:' | cut -d' ' -f2)}"

# Default to host target if not set (e.g. when building for current machine)
if [ -z "$TARGET_TRIPLE" ]; then
  TARGET_TRIPLE=$(rustc -vV 2>/dev/null | grep 'host:' | cut -d' ' -f2)
fi

# Cargo puts host builds at target/release/; cross-builds at target/$TRIPLE/release/
# IMPORTANT: Check the cross path first. CI uses `tauri build --target aarch64-apple-darwin`
# which always uses the triple-prefixed directory, even if it matches the host.
BINARY_HOST="$TARGET_DIR/release/taurscribe"
BINARY_CROSS="$TARGET_DIR/$TARGET_TRIPLE/release/taurscribe"
DYLIB_DIR="$SRC_TAURI/macos-dylibs"

if [ -f "$BINARY_CROSS" ]; then
  BINARY="$BINARY_CROSS"
elif [ -f "$BINARY_HOST" ]; then
  BINARY="$BINARY_HOST"
else
  echo "bundle-macos-dylibs: Binary not found at $BINARY_CROSS or $BINARY_HOST, skipping"
  exit 0
fi

echo "bundle-macos-dylibs: Using binary at $BINARY"
BINARY_DIR="$(dirname "$BINARY")"

# dylibbundler looks for dylibs next to the binary (or in system paths). In CI and when
# using --target, llama-cpp-sys-2 builds libggml-base*.dylib and libllama.dylib into
# target/<triple>/release/build/llama-cpp-sys-2-*/out/lib/, so they are not beside the binary.
# Copy them into BINARY_DIR so dylibbundler can find them and avoid interactive "Please
# specify the directory" prompts (which break CI).
LLAMA_LIB_DIR=""
for candidate in "$TARGET_DIR/$TARGET_TRIPLE/release/build/llama-cpp-sys-2-"*/out/lib "$TARGET_DIR/release/build/llama-cpp-sys-2-"*/out/lib; do
  if [ -d "$candidate" ]; then
    LLAMA_LIB_DIR="$candidate"
    break
  fi
done
if [ -n "$LLAMA_LIB_DIR" ]; then
  echo "bundle-macos-dylibs: Copying llama-cpp dylibs from $LLAMA_LIB_DIR to $BINARY_DIR"
  cp -f "$LLAMA_LIB_DIR"/*.dylib "$BINARY_DIR/" 2>/dev/null || true
  ls -la "$BINARY_DIR"/lib{ggml,llama}*.dylib 2>/dev/null || true
fi

# dylibbundler required on macOS; without it the app crashes at launch (libggml-base, libllama)
if ! command -v dylibbundler &>/dev/null; then
  echo "bundle-macos-dylibs: ERROR - dylibbundler not found."
  echo "The app will crash at launch without bundled dylibs. Install with: brew install dylibbundler"
  exit 1
fi

mkdir -p "$DYLIB_DIR"
rm -f "$DYLIB_DIR"/*.dylib 2>/dev/null || true

# Build search-path flags for dylibbundler.
# We pass -s for both:
#   (a) the llama-cpp-sys-2 build output dir (canonical location of libggml-*, libllama)
#   (b) the BINARY_DIR (where we copied the dylibs above)
# This ensures dylibbundler can resolve both direct and transitive @rpath deps
# without prompting interactively (which hangs CI).
# Build search flags as an array so paths with spaces are handled correctly.
SEARCH_FLAGS=(-s "$BINARY_DIR")
if [ -n "$LLAMA_LIB_DIR" ]; then
  SEARCH_FLAGS+=(-s "$LLAMA_LIB_DIR")
fi

echo "bundle-macos-dylibs: Running dylibbundler with search flags: ${SEARCH_FLAGS[*]}"

# -od: use @executable_path; -b: bundle (copy) deps; -x: binary; -d: output dir; -p: rpath prefix
# -s: additional search path for dylibs that aren't in standard locations
dylibbundler -od -b -x "$BINARY" -d "$DYLIB_DIR" -p "@executable_path/../Frameworks" "${SEARCH_FLAGS[@]}"

# Generate tauri.macos.conf.json with framework paths (Tauri validates these at build time;
# we create this file here so it only exists when dylibs exist, avoiding "Library not found").
if ls "$DYLIB_DIR"/*.dylib 1>/dev/null 2>&1; then
  echo "bundle-macos-dylibs: Bundled dylibs:"
  ls -la "$DYLIB_DIR"/*.dylib
  FRAMEWORKS_JSON="["
  for f in "$DYLIB_DIR"/*.dylib; do
    [ -f "$f" ] || continue
    bn=$(basename "$f")
    FRAMEWORKS_JSON="$FRAMEWORKS_JSON\"./macos-dylibs/$bn\","
  done
  FRAMEWORKS_JSON="${FRAMEWORKS_JSON%,}]"
  MACOS_CONF="$SRC_TAURI/tauri.macos.conf.json"
  echo "{\"bundle\":{\"macOS\":{\"frameworks\":$FRAMEWORKS_JSON}}}" > "$MACOS_CONF"
  echo "bundle-macos-dylibs: Wrote $MACOS_CONF"
fi
