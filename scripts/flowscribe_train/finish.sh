#!/bin/bash
# After training: merge the LoRA adapter, convert to F16 GGUF, evaluate vs v2.
set -euo pipefail
cd "$(dirname "$0")"
export HF_HOME=/Volumes/ExternalSSD/TaurscribeData/hf
ADAPTERS=${1:-adapters/v1c}
BASE=$(ls -d "$HF_HOME"/hub/models--Qwen--Qwen3.5-0.8B/snapshots/* | head -1)
CONVERT=/Volumes/ExternalSSD/TaurscribeData/llama.cpp-convert
OUT=/Volumes/ExternalSSD/TaurscribeData/flowscribe
GGUF="$OUT/flowscribe-v3-f16.gguf"

echo "== 1/4 merge adapter (MLX layout)"
rm -rf fused/v1 hf/v1
.venv/bin/python -m mlx_lm fuse --model "$BASE" --adapter-path "$ADAPTERS" --save-path fused/v1
echo "== 2/4 rebuild as a standard Hugging Face checkpoint"
"$CONVERT/.venv/bin/python" merge_hf.py fused/v1 hf/v1
echo "== 3/4 convert to F16 GGUF"
"$CONVERT/.venv/bin/python" "$CONVERT/convert_hf_to_gguf.py" hf/v1 --outtype f16 --outfile "$GGUF" 2>&1 | grep "n_tensors\|successfully"
echo "== 4/4 evaluate v3 and v2 on the held-out test set (app inference code)"
.venv/bin/python eval.py --gguf "$GGUF" --out eval/v3 > /dev/null
.venv/bin/python eval.py --v2 --out eval/v2 > /dev/null
.venv/bin/python eval.py --report eval/v3 eval/v2
