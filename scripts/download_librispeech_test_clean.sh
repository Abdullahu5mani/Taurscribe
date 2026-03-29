#!/usr/bin/env bash
# LibriSpeech test-clean (OpenSLR SLR12, CC BY 4.0).
# LIBRISPEECH_ROOT: parent directory for LibriSpeech/ (default: repo/taurscribe-runtime/librispeech).
set -euo pipefail

EXPECTED_MD5="32fa31d27d2e1cad72775fee3f4849a9"
URL="https://www.openslr.org/resources/12/test-clean.tar.gz"
TARBALL="test-clean.tar.gz"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEFAULT_ROOT="$REPO_ROOT/taurscribe-runtime/librispeech"
DEST_ROOT="${LIBRISPEECH_ROOT:-$DEFAULT_ROOT}"

mkdir -p "$DEST_ROOT"
ARCHIVE="$DEST_ROOT/$TARBALL"

if [[ ! -f "$ARCHIVE" ]]; then
  echo "Downloading $URL ..."
  curl -L -o "$ARCHIVE" "$URL"
else
  echo "Archive already present: $ARCHIVE"
fi

echo "Verifying MD5..."
if command -v md5sum >/dev/null 2>&1; then
  HASH=$(md5sum "$ARCHIVE" | awk '{print $1}')
elif command -v md5 >/dev/null 2>&1; then
  HASH=$(md5 -q "$ARCHIVE")
else
  echo "WARN: no md5sum/md5; skipping checksum"
  HASH="$EXPECTED_MD5"
fi
if [[ "$HASH" != "$EXPECTED_MD5" ]]; then
  echo "MD5 mismatch: got $HASH expected $EXPECTED_MD5"
  exit 1
fi

MARKER="$DEST_ROOT/LibriSpeech/test-clean"
if [[ -d "$MARKER" ]]; then
  echo "Corpus already extracted: $MARKER"
  exit 0
fi

echo "Extracting..."
tar -xzf "$ARCHIVE" -C "$DEST_ROOT"
if [[ ! -d "$MARKER" ]]; then
  echo "Extraction failed: expected $MARKER"
  exit 1
fi
echo "Done. test-clean at: $MARKER"
echo "Build manifest: cargo run --manifest-path src-tauri/Cargo.toml --bin librispeech_manifest -- --root \"$MARKER\" --out \"$DEST_ROOT/eval_manifest.jsonl\""
