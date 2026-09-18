#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Compiling and Running Native Apple Silicon Metal GPU Compute Suite ==="

# 1. Compile Metal shader source into AIR bytecode
xcrun -sdk macosx metal -c "$REPO_ROOT/scripts/tests/metal_compute_shader.metal" -o /tmp/metal_compute_shader.air

# 2. Compile AIR into metallib
xcrun -sdk macosx metallib /tmp/metal_compute_shader.air -o /tmp/metal_compute_shader.metallib

# 3. Compile Objective-C Metal test harness
clang -O2 -framework Foundation -framework Metal "$REPO_ROOT/scripts/tests/metal_compute_test.m" -o /tmp/metal_compute_test

# 4. Run harness on physical Apple Silicon Metal GPU
/tmp/metal_compute_test

# 5. Clean up temporary artifacts
rm -f /tmp/metal_compute_shader.air /tmp/metal_compute_shader.metallib /tmp/metal_compute_test
