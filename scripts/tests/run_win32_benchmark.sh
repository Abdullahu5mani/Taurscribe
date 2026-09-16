#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Building / Ensuring taurscribe-win32 Docker Image ==="
docker build --platform linux/amd64 -f "$REPO_ROOT/scripts/tests/Dockerfile.win32" -t taurscribe-win32:latest "$REPO_ROOT"

echo "=== Compiling win32_affinity_test.exe via MinGW x86_64 ==="
docker run --rm -v "$REPO_ROOT":/workspace taurscribe-win32:latest sh -c \
  "x86_64-w64-mingw32-gcc -O2 scripts/tests/win32_thread_affinity_test.c -o scripts/tests/win32_affinity_test.exe"

echo "=== Executing Win32 Affinity & Hybrid CPU Test Suite under Wine64 ==="
docker run --rm -v "$REPO_ROOT":/workspace taurscribe-win32:latest wine scripts/tests/win32_affinity_test.exe
