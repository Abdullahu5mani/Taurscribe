#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "==============================================================================="
echo "        TAURSCRIBE CROSS-PLATFORM GPU HARDWARE EMULATION & FAULT SUITE        "
echo "==============================================================================="
echo "Target Architectures: Linux x86_64 / ARM64, Windows x86_64 / ARM64"
echo "Acceleration APIs:    Vulkan Compute (Lavapipe), DirectML (WARP), CUDA (Mock)"
echo "Host System:          $(uname -s) $(uname -m)"
echo "Timestamp:            $(date)"
echo "==============================================================================="

FAILED=0

# Tier 1: Vulkan + Mesa Lavapipe (Linux Software Compute)
echo ""
echo ">>> [TIER 1/3] Vulkan 1.3 Mesa Lavapipe Compute Emulation (Linux/CPU)..."
if bash "$REPO_ROOT/scripts/tests/run_vulkan_lavapipe_test.sh"; then
    TIER1_RESULT="PASS"
else
    TIER1_RESULT="FAIL"
    FAILED=1
fi

# Tier 2: DirectML + WARP (Windows Software D3D12 under Wine64)
echo ""
echo ">>> [TIER 2/3] DirectML & Direct3D 12 WARP Software Emulation (Win32/Wine64)..."
if bash "$REPO_ROOT/scripts/tests/run_directml_warp_test.sh"; then
    TIER2_RESULT="PASS"
else
    TIER2_RESULT="FAIL"
    FAILED=1
fi

# Tier 3: CUDA Mock & Fault Injection (NVIDIA Driver & Runtime Fault Emulation)
echo ""
echo ">>> [TIER 3/3] CUDA Mock & Memory Guard Fault-Injection Suite..."
if bash "$REPO_ROOT/scripts/tests/run_cuda_mock_test.sh"; then
    TIER3_RESULT="PASS"
else
    TIER3_RESULT="FAIL"
    FAILED=1
fi

echo ""
echo "==============================================================================="
echo "                        GPU EMULATION SCORECARD                               "
echo "==============================================================================="
echo "  Tier 1 - Vulkan + Mesa Lavapipe Compute:    [$TIER1_RESULT]"
echo "  Tier 2 - DirectML + WARP D3D12 under Wine:  [$TIER2_RESULT]"
echo "  Tier 3 - CUDA Mock & Fault Injection:       [$TIER3_RESULT]"
echo "==============================================================================="

if [ "$FAILED" -eq 0 ]; then
    echo "✓ ALL GPU EMULATION AND HARDWARE SIMULATION TIERS PASSED!"
    exit 0
else
    echo "✗ SOME GPU EMULATION TESTS FAILED!"
    exit 1
fi
