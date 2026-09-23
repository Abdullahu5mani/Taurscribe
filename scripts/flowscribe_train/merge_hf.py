#!/usr/bin/env python3
"""Build a standard Hugging Face checkpoint of the fine-tuned model.

`mlx_lm fuse` writes weights in MLX's layout (Qwen3.5 RMSNorm weights +1,
conv1d transposed, MTP layer dropped), which llama.cpp's converter misreads.
LoRA only changes 2-D projection matrices, so this starts from the original
Hugging Face checkpoint and replaces just those with the fused values; every
other tensor (norms, convs, A_log, MTP, vision) stays exactly as shipped.

    python merge_hf.py fused/v1 hf/v1
"""

import glob
import json
import shutil
import sys
from pathlib import Path

import torch
from safetensors import safe_open
from safetensors.torch import save_file

fused_dir, out_dir = Path(sys.argv[1]), Path(sys.argv[2])
base_dir = Path(glob.glob("/Volumes/ExternalSSD/TaurscribeData/hf/hub/models--Qwen--Qwen3.5-0.8B/snapshots/*")[0])

fused = {}
for p in fused_dir.glob("*.safetensors"):
    with safe_open(p, "pt") as f:
        for k in f.keys():
            fused[k] = f.get_tensor(k)


def fused_name(base_key: str) -> str | None:
    # base: model.language_model.X  ->  fused: language_model.model.X
    if base_key.startswith("model.language_model."):
        return "language_model.model." + base_key[len("model.language_model."):]
    if base_key.startswith("lm_head."):
        return "language_model." + base_key
    return None


out_dir.mkdir(parents=True, exist_ok=True)
replaced, kept, changed = 0, 0, 0
tensors = {}
for shard in sorted(base_dir.glob("*.safetensors")):
    with safe_open(shard, "pt") as f:
        for k in f.keys():
            t = f.get_tensor(k)
            fk = fused_name(k)
            if fk and t.ndim == 2 and fk in fused and tuple(fused[fk].shape) == tuple(t.shape):
                new = fused[fk].to(t.dtype)
                changed += int(not torch.equal(new, t))
                tensors[k] = new
                replaced += 1
            else:
                tensors[k] = t
                kept += 1
save_file(tensors, str(out_dir / "model.safetensors"), metadata={"format": "pt"})
for name in ("config.json", "tokenizer.json", "tokenizer_config.json", "chat_template.jinja",
             "merges.txt", "vocab.json", "preprocessor_config.json", "video_preprocessor_config.json", "LICENSE"):
    if (base_dir / name).exists():
        shutil.copy(base_dir / name, out_dir / name)
print(f"2-D weights from the fused model: {replaced} ({changed} actually changed by LoRA); kept from base: {kept}")
if changed == 0:
    sys.exit("No weights changed: the adapter was not merged. Refusing to continue.")
