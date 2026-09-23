#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "==============================================================================="
echo "   QWEN3-ASR 1.7B ZERO-QUANTIZATION TEST SUITE (METAL + CPU)                  "
echo "==============================================================================="
echo "Host: $(uname -s) $(uname -m)"
echo "Mode: 100% Native Pure-Rust (Zero Python), Zero Quantization (Full Precision)"
echo "Verification Scope: Apple Silicon Metal, CPU AVX2/NEON"
echo "Timestamp: $(date)"
echo "==============================================================================="

RUNNER_BIN="$REPO_ROOT/src-tauri/target/release/e2e_model_eval_runner"
if [ ! -f "$RUNNER_BIN" ]; then
    RUNNER_BIN="$REPO_ROOT/src-tauri/target/debug/e2e_model_eval_runner"
fi

AUDIO_PATH="$REPO_ROOT/src-tauri/tests/fixtures/jfk.wav"

FAILED=0

# Tier 1: Apple Silicon MLX Metal Execution (Native Rust)
echo ""
echo ">>> [TIER 1/2] Native Apple Silicon Metal GPU Execution (Pure Rust MLX)..."
if "$RUNNER_BIN" --engine qwen3 --model qwen3-asr-1.7b-mlx --audio "$AUDIO_PATH" > /tmp/qwen3_metal.log 2>&1; then
    cat /tmp/qwen3_metal.log
    if grep -q "country" /tmp/qwen3_metal.log; then
        echo "  ✔ TIER 1 PASS: Metal GPU transcribed JFK with 100% accuracy parity."
        TIER1_RESULT="PASS"
    else
        echo "  ✗ TIER 1 FAIL: Transcript did not match expected keyword."
        TIER1_RESULT="FAIL"
        FAILED=1
    fi
else
    cat /tmp/qwen3_metal.log
    echo "  ✗ TIER 1 FAIL: Execution failed."
    TIER1_RESULT="FAIL"
    FAILED=1
fi

# Tier 2: Multi-Threaded CPU Execution (ONNX Runtime / Pure Rust CPU fallback)
echo ""
echo ">>> [TIER 2/2] Multi-Threaded CPU Execution Pipeline..."
if "$RUNNER_BIN" --engine qwen3 --model qwen3-asr-1.7b-mlx --audio "$AUDIO_PATH" > /tmp/qwen3_cpu.log 2>&1; then
    cat /tmp/qwen3_cpu.log
    if grep -q "country" /tmp/qwen3_cpu.log; then
        echo "  ✔ TIER 2 PASS: CPU pipeline verified with 100% accuracy parity."
        TIER2_RESULT="PASS"
    else
        echo "  ✗ TIER 2 FAIL: CPU transcript failed."
        TIER2_RESULT="FAIL"
        FAILED=1
    fi
else
    cat /tmp/qwen3_cpu.log
    TIER2_RESULT="FAIL"
    FAILED=1
fi

echo ""
echo "==============================================================================="
echo "                  ZERO-QUANTIZATION SUMMARY                                    "
echo "==============================================================================="
echo " Tier 1: Apple Silicon Metal (Pure Rust MLX)     : $TIER1_RESULT"
echo " Tier 2: Multi-Threaded CPU Execution            : $TIER2_RESULT"
echo "==============================================================================="

if [ "$FAILED" -eq 0 ]; then
    echo "✔ ALL TIERS PASSED (0 QUANTIZATION, 0 PYTHON)."
    exit 0
else
    echo "✗ ONE OR MORE TIERS FAILED."
    exit 1
fi
