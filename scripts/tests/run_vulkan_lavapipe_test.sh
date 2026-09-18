#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Running Vulkan Software GPU Emulation Suite (Mesa Lavapipe) ==="

docker run --rm --platform linux/amd64 -v "$REPO_ROOT":/workspace -w /workspace ubuntu:24.04 sh -c '
  set -e
  echo "[1/4] Installing Mesa Lavapipe, Vulkan SDK, and glslangValidator..."
  apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
    mesa-vulkan-drivers \
    libvulkan-dev \
    vulkan-tools \
    glslang-tools \
    build-essential >/dev/null

  echo "[2/4] Inspecting Vulkan Lavapipe driver..."
  vulkaninfo --summary || true

  echo "[3/4] Compiling GLSL compute shader to SPIR-V..."
  glslangValidator -V scripts/tests/vulkan_compute.comp -o scripts/tests/vulkan_compute.spv

  echo "[4/4] Compiling and running Vulkan Lavapipe compute test..."
  gcc -O2 scripts/tests/vulkan_lavapipe_test.c -lvulkan -lm -o /tmp/vulkan_lavapipe_test
  /tmp/vulkan_lavapipe_test scripts/tests/vulkan_compute.spv
'
