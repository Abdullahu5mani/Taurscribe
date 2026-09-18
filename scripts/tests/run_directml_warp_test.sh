#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Running DirectML / D3D12 WARP Software GPU Test Suite ==="

echo "[1/3] Building / Ensuring taurscribe-win32 Docker Image..."
docker build --platform linux/amd64 -f "$REPO_ROOT/scripts/tests/Dockerfile.win32" -t taurscribe-win32:latest "$REPO_ROOT"

echo "[2/3] Compiling win32_directml_warp_test.exe via MinGW x86_64..."
docker run --rm --platform linux/amd64 -v "$REPO_ROOT":/workspace taurscribe-win32:latest sh -c \
  "x86_64-w64-mingw32-gcc -O2 scripts/tests/win32_directml_warp_test.c -lole32 -o scripts/tests/win32_directml_warp_test.exe"

echo "[3/3] Executing Direct3D 12 / WARP Software GPU Suite under Wine64..."
docker run --rm --platform linux/amd64 -v "$REPO_ROOT":/workspace taurscribe-win32:latest sh -c \
  "Xvfb :99 -screen 0 1024x768x24 > /dev/null 2>&1 & export DISPLAY=:99; sleep 1; wine scripts/tests/win32_directml_warp_test.exe"

rm -f "$REPO_ROOT/scripts/tests/win32_directml_warp_test.exe"
