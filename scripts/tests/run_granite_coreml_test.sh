#!/usr/bin/env bash
# ==============================================================================
# Taurscribe Granite ONNX CoreML Hybrid Execution Provider Test Runner
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "==============================================================================="
echo "   Granite ONNX CoreML Hybrid Execution Provider (Apple Neural Engine / GPU)   "
echo "==============================================================================="
echo "Host Machine: $(uname -sm)"

cargo test --manifest-path "$ROOT_DIR/src-tauri/Cargo.toml" --test test_ort_coreml -- --nocapture

echo ""
echo "✓ [PASS] Granite CoreML Execution Provider verified for Apple Neural Engine / GPU!"
