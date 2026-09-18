#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Compiling and Running AMD ROCm / HIP True Parallel Local Suite ==="

docker run --platform linux/amd64 --rm \
  -v "$REPO_ROOT":/workspace -w /workspace \
  ubuntu:24.04 bash -c "
    apt-get update -qq > /dev/null && \
    apt-get install -y -qq g++ libtbb-dev > /dev/null && \
    g++ -O2 -std=c++17 -Iscripts/tests/HIP-CPU/include scripts/tests/test_rocm_hip_harness.cpp -ltbb -o /tmp/test_rocm_hip_harness && \
    /tmp/test_rocm_hip_harness
  "
