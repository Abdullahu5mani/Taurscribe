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

BINARY="$TARGET_DIR/$TARGET_TRIPLE/release/taurscribe"
DYLIB_DIR="$SRC_TAURI/macos-dylibs"

if [ ! -f "$BINARY" ]; then
  echo "bundle-macos-dylibs: Binary not found at $BINARY, skipping"
  exit 0
fi

# dylibbundler required on macOS for proper .app bundling
if ! command -v dylibbundler &>/dev/null; then
  echo "bundle-macos-dylibs: dylibbundler not found. Install with: brew install dylibbundler"
  echo "bundle-macos-dylibs: Skipping dylib bundling - app may fail to run if it uses dynamic libs"
  exit 0
fi

mkdir -p "$DYLIB_DIR"
rm -f "$DYLIB_DIR"/*.dylib 2>/dev/null || true

# -od: use @executable_path; -b: bundle (copy) deps; -x: binary; -d: output dir; -p: rpath prefix
dylibbundler -od -b -x "$BINARY" -d "$DYLIB_DIR" -p "@executable_path/../Frameworks"

# List what we bundled (for debugging)
if ls "$DYLIB_DIR"/*.dylib 1>/dev/null 2>&1; then
  echo "bundle-macos-dylibs: Bundled dylibs:"
  ls -la "$DYLIB_DIR"/*.dylib
  echo "bundle-macos-dylibs: If the app fails to start with 'Library not loaded', add missing dylibs to tauri.conf.json bundle.macOS.frameworks"
fi
