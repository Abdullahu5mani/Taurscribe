#!/usr/bin/env bash
# ==============================================================================
# Taurscribe Whisper Apple Neural Engine (ANE via CoreML) Automated Test Harness
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
MODELS_DIR="$HOME/Library/Application Support/Taurscribe/models"
AUDIO_FILE="$ROOT_DIR/src-tauri/tests/fixtures/jfk.wav"

echo "==============================================================================="
echo "       Whisper CoreML Apple Neural Engine (ANE) Inference Benchmark            "
echo "==============================================================================="
echo "Host: $(uname -sm) | OS: $(sw_vers -productName) $(sw_vers -productVersion)"
echo "Models Directory: $MODELS_DIR"

mkdir -p "$MODELS_DIR"

# Ensure ggml-tiny-encoder.mlmodelc is present
if [ ! -d "$MODELS_DIR/ggml-tiny-encoder.mlmodelc" ]; then
    echo "CoreML bundle missing. Downloading ggml-tiny-encoder.mlmodelc.zip..."
    cd "$MODELS_DIR"
    curl -sL -o "ggml-tiny-encoder.mlmodelc.zip" "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-encoder.mlmodelc.zip"
    unzip -q "ggml-tiny-encoder.mlmodelc.zip"
    rm -f "ggml-tiny-encoder.mlmodelc.zip"
fi

if [ ! -f "$MODELS_DIR/ggml-tiny.bin" ]; then
    echo "Base GGML model missing. Downloading ggml-tiny.bin..."
    cd "$MODELS_DIR"
    curl -sL -o "ggml-tiny.bin" "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"
fi

echo "✓ Model assets verified:"
ls -ld "$MODELS_DIR/ggml-tiny.bin" "$MODELS_DIR/ggml-tiny-encoder.mlmodelc"

echo ""
echo "--- Running Whisper E2E Model Evaluation with CoreML ANE Offload ---"
OUTPUT=$(cargo run --manifest-path "$ROOT_DIR/src-tauri/Cargo.toml" --bin e2e_model_eval_runner -- \
    --engine whisper --model tiny --audio "$AUDIO_FILE" 2>&1)

echo "$OUTPUT"

echo ""
echo "--- Verifying Output & Inferred Backend ---"
if echo "$OUTPUT" | grep -q "Resolved compute backend: CoreML"; then
    echo "✓ [PASS] Whisper verified executing with CoreML (Apple Neural Engine) acceleration!"
else
    echo "❌ [FAIL] Whisper did not resolve CoreML backend."
    exit 1
fi

if echo "$OUTPUT" | grep -q "ask not what your country can do for you"; then
    echo "✓ [PASS] Transcript verification: 100% bit-perfect match on JFK benchmark."
else
    echo "❌ [FAIL] Transcript does not match expected output."
    exit 1
fi

echo "==============================================================================="
echo ">>> WHISPER COREML APPLE NEURAL ENGINE VERIFICATION COMPLETED SUCCESSFULLY! <<<"
echo "==============================================================================="
