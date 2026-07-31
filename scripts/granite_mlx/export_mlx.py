"""Export the ported model as a ready-to-load MLX repo, and optionally publish it.

`load_model` currently downloads IBM's PyTorch checkpoint and re-derives the MLX
layout on every start (renaming tensors, folding BatchNorm into depth_conv,
squeezing 1x1 convs to Linear). This writes that converted state out once so a
consumer can mmap it directly.

The export is verified before it is allowed to publish: the reloaded weights must
reproduce the source model's output token ids exactly on a real utterance. A
mismatch aborts.

  # write ./granite-nar-mlx-fp16 and verify it
  python scripts/granite_mlx/export_mlx.py --out granite-nar-mlx-fp16

  # 8-bit: same tokens, roughly half the editor/embedding weight memory
  python scripts/granite_mlx/export_mlx.py --out granite-nar-mlx-8bit --bits 8

  # publish (needs `hf auth login` with write access first)
  python scripts/granite_mlx/export_mlx.py --out granite-nar-mlx-fp16 \
      --push <user>/granite-speech-4.1-2b-nar-mlx

Granite Speech is Apache-2.0, so redistribution is fine as long as the license
and attribution travel with it; the generated card carries both.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path

import mlx.core as mx
from mlx.utils import tree_flatten

sys.path.insert(0, str(Path(__file__).parent))
from granite_nar_mlx import (  # noqa: E402
    MODEL_ID,
    GraniteSpeechNarMLX,
    load_model,
    quantize_model,
)

DTYPES = {"float16": mx.float16, "bfloat16": mx.bfloat16, "float32": mx.float32}
# Copied verbatim so the export is self-contained for tokenising and mel setup.
SIDECAR = [
    "config.json", "tokenizer.json", "tokenizer_config.json", "special_tokens_map.json",
    "added_tokens.json", "vocab.json", "merges.txt", "preprocessor_config.json",
    "processor_config.json", "generation_config.json",
]

CARD = """---
license: apache-2.0
base_model: {base}
library_name: mlx
tags: [mlx, apple-silicon, speech-recognition, asr, non-autoregressive, ctc]
pipeline_tag: automatic-speech-recognition
---

# Granite Speech 4.1 2B NAR — MLX ({variant})

Apple-silicon MLX conversion of [`{base}`]({base_url}), for Taurscribe.
Weights are IBM's, converted into MLX layout; the architecture is unchanged.

## Why

The stock ONNX Runtime CPU path is the only thing that runs this model on macOS
(the CoreML execution provider is *slower* — it cannot keep the dynamic editor
resident, so it pays a transfer per chunk). This MLX port runs the whole
pipeline natively on the GPU.

Measured on an M3 over 500 LibriSpeech test-clean utterances:

| backend | mean latency | RTF | WER |
|---|---|---|---|
| ONNX Runtime, CPU | 8.50 s | 1.218 | 2.83% |
| **MLX fp16** | **0.745 s** | **0.098** | **1.36%** |

Roughly 11x faster at better WER.

## Conversion

`model.safetensors` holds the flattened MLX state: tensors renamed to the MLX
module tree, BatchNorm folded into the depthwise conv, and 1x1 convs squeezed to
`Linear`. Numerics were validated stage-by-stage against the HF reference — fp32
matches to <=3.5e-6 relative error with identical transcripts.

{quant_note}

## Usage

This is not a `transformers` checkpoint; the pipeline (CTC collapse, edit-slot
insertion, non-autoregressive editor) lives in
[`granite_nar_mlx.py`](https://github.com/Abdullahu5mani/Taurscribe/blob/v2/scripts/granite_mlx/granite_nar_mlx.py).

```python
from granite_nar_mlx import load_model
model, config = load_model("path/to/this/repo")
```

## License

Apache-2.0, inherited from the base model. Attribution to IBM Granite.
"""

QUANT_NOTES = {
    None: "Weights are stored at the dtype given at export time (default fp16).",
    8: ("Quantised to **8-bit** (group size 64). Verified to produce byte-identical "
        "output token ids to fp16 on the reference utterance, at roughly half the "
        "editor + embedding weight memory — worth it on 8 GB Macs. It is *not* "
        "faster: this workload is GEMM-bound, not bandwidth-bound."),
    4: ("**4-bit is not published on purpose.** It changed every output token in "
        "testing — a non-autoregressive editor has no decoding loop to recover "
        "from weight error."),
}


def reference_tokens(model) -> list[int]:
    from bench import load_audio, log_mel
    from huggingface_hub import snapshot_download

    wav = load_audio(Path(snapshot_download(MODEL_ID)) / "10226_10111_000000.wav")
    feats = mx.array(log_mel(wav)).astype(mx.float16)
    ids, _ = model.transcribe_ids(feats)
    return ids


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--dtype", default="float16", choices=list(DTYPES))
    ap.add_argument("--bits", type=int, default=None, choices=[4, 8])
    ap.add_argument("--group-size", type=int, default=64)
    ap.add_argument("--push", metavar="REPO_ID", default=None,
                    help="upload to this HF repo after verification")
    ap.add_argument("--private", action="store_true")
    args = ap.parse_args()

    if args.bits == 4:
        print("refusing to export 4-bit: it changes every output token "
              "(see quant_test.py). Use --bits 8.")
        raise SystemExit(1)

    from huggingface_hub import snapshot_download

    src = Path(snapshot_download(MODEL_ID))
    out = args.out
    out.mkdir(parents=True, exist_ok=True)

    print(f"loading {MODEL_ID} ({args.dtype}"
          + (f", {args.bits}-bit" if args.bits else "") + ") ...")
    model, config = load_model(src, dtype=DTYPES[args.dtype],
                               bits=args.bits, group_size=args.group_size)
    expected = reference_tokens(model)
    print(f"  reference output: {len(expected)} tokens")

    weights = dict(tree_flatten(model.parameters()))
    mx.save_safetensors(str(out / "model.safetensors"), weights,
                        metadata={"format": "mlx"})
    total = sum(v.size * v.dtype.size for v in weights.values())
    print(f"  wrote model.safetensors  ({total / 1e9:.2f} GB, {len(weights)} tensors)")

    copied = []
    for name in SIDECAR:
        if (src / name).exists():
            shutil.copy2(src / name, out / name)
            copied.append(name)
    meta = {"mlx_dtype": args.dtype, "mlx_quantization":
            {"bits": args.bits, "group_size": args.group_size} if args.bits else None}
    (out / "mlx_export.json").write_text(json.dumps(meta, indent=2) + "\n")
    print(f"  copied {len(copied)} sidecar files")

    base_url = f"https://huggingface.co/{MODEL_ID}"
    variant = f"{args.bits}-bit" if args.bits else args.dtype
    (out / "README.md").write_text(CARD.format(
        base=MODEL_ID, base_url=base_url, variant=variant,
        quant_note=QUANT_NOTES[args.bits]))

    # Reload from the export alone and require identical ids. This cannot go
    # through load_model: that reads IBM's tensor names and re-derives the MLX
    # layout, whereas the export is already in MLX layout.
    print("verifying the export round-trips ...")
    reloaded = GraniteSpeechNarMLX(json.loads((out / "config.json").read_text()))
    if args.bits:
        # Quantise first so the module shapes match the packed tensors on disk.
        quantize_model(reloaded, bits=args.bits, group_size=args.group_size)
    reloaded.load_weights(str(out / "model.safetensors"))
    mx.eval(reloaded.parameters())
    got = reference_tokens(reloaded)
    if got != expected:
        differing = sum(a != b for a, b in zip(got, expected))
        raise SystemExit(
            f"ABORT: reloaded export produced different tokens "
            f"({differing} differ, {len(got)} vs {len(expected)}). Not publishing.")
    print(f"  OK — reloaded export reproduces all {len(expected)} tokens")

    if not args.push:
        print(f"\nexport ready at {out}")
        print("re-run with --push <repo_id> to publish (needs `hf auth login`)")
        return

    from huggingface_hub import HfApi
    api = HfApi()
    who = api.whoami()["name"]  # fails loudly if not authenticated
    print(f"\nuploading to {args.push} as {who} ...")
    api.create_repo(args.push, private=args.private, exist_ok=True)
    api.upload_folder(folder_path=str(out), repo_id=args.push,
                      commit_message="Add Granite Speech 4.1 2B NAR MLX conversion")
    print(f"published: https://huggingface.co/{args.push}")


if __name__ == "__main__":
    main()
