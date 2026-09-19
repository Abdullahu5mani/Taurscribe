#!/usr/bin/env bash
# ==============================================================================
# Taurscribe Linux ARM64 (aarch64) Native Execution Runner
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "==============================================================================="
echo "       Taurscribe Linux ARM64 Native Containerized Execution Suite             "
echo "==============================================================================="
echo "Host Machine: $(uname -sm)"
echo "Docker Architecture: $(docker info --format '{{.Architecture}}')"

# Run inside native Ubuntu 24.04 ARM64 container
docker run --rm \
    --platform linux/arm64 \
    -v "$ROOT_DIR:/workspace" \
    -w /workspace \
    ubuntu:24.04 \
    bash -c '
        set -euo pipefail
        echo "Updating apt cache and installing build-essential..."
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq && apt-get install -y -qq build-essential libasound2-dev file > /dev/null

        echo "Compiling linux_arm64_test_harness.c with native aarch64 gcc..."
        gcc -O3 -march=armv8-a+simd scripts/tests/linux_arm64_test_harness.c -lm -o /tmp/linux_arm64_test_harness

        echo "Inspecting compiled ELF binary:"
        file /tmp/linux_arm64_test_harness

        echo ""
        echo "Executing Linux ARM64 native harness:"
        /tmp/linux_arm64_test_harness
    '

echo ""
echo "✓ [PASS] Linux ARM64 native execution completed cleanly!"
