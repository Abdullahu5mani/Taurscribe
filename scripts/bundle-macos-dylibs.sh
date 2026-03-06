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
BINARY_HOST="$TARGET_DIR/release/taurscribe"
BINARY_CROSS="$TARGET_DIR/$TARGET_TRIPLE/release/taurscribe"
DYLIB_DIR="$SRC_TAURI/macos-dylibs"

if [ -f "$BINARY_HOST" ]; then
  BINARY="$BINARY_HOST"
elif [ -f "$BINARY_CROSS" ]; then
  BINARY="$BINARY_CROSS"
else
  echo "bundle-macos-dylibs: Binary not found at $BINARY_HOST or $BINARY_CROSS, skipping"
  exit 0
fi

# dylibbundler required on macOS; without it the app crashes at launch (libggml-base, libllama)
if ! command -v dylibbundler &>/dev/null; then
  echo "bundle-macos-dylibs: ERROR - dylibbundler not found."
  echo "The app will crash at launch without bundled dylibs. Install with: brew install dylibbundler"
  exit 1
fi

mkdir -p "$DYLIB_DIR"
rm -f "$DYLIB_DIR"/*.dylib 2>/dev/null || true

# -od: use @executable_path; -b: bundle (copy) deps; -x: binary; -d: output dir; -p: rpath prefix
dylibbundler -od -b -x "$BINARY" -d "$DYLIB_DIR" -p "@executable_path/../Frameworks"

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
