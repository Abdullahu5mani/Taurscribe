#!/bin/bash
# After training: merge the LoRA adapter, convert to F16 GGUF, evaluate vs v2.
set -euo pipefail
cd "$(dirname "$0")"
export HF_HOME=/Volumes/ExternalSSD/TaurscribeData/hf
BASE=$(ls -d "$HF_HOME"/hub/models--Qwen--Qwen3.5-0.8B/snapshots/* | head -1)
CONVERT=/Volumes/ExternalSSD/TaurscribeData/llama.cpp-convert
OUT=/Volumes/ExternalSSD/TaurscribeData/flowscribe
GGUF="$OUT/flowscribe-v3-ckpt1200-f16.gguf"

echo "== 1/3 merge adapter into Qwen3.5-0.8B (full precision)"
rm -rf fused/ckpt1200
.venv/bin/python -m mlx_lm fuse --model "$BASE" --adapter-path adapters/ckpt1200 --save-path fused/ckpt1200

echo "== 2/3 convert to F16 GGUF"
"$CONVERT/.venv/bin/python" "$CONVERT/convert_hf_to_gguf.py" fused/ckpt1200 --outtype f16 --outfile "$GGUF"
ls -la "$GGUF"

echo "== 3/3 evaluate on the held-out test set (app inference code)"
.venv/bin/python eval.py --gguf "$GGUF" --out eval/ckpt1200 > /dev/null
.venv/bin/python eval.py --report eval/ckpt1200 eval/v2
