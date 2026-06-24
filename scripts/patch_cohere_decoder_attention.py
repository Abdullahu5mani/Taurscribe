#!/usr/bin/env python3
"""Create a CUDA-compatible Cohere decoder ONNX file.

The upstream Cohere decoder uses `com.microsoft::GroupQueryAttention` with an
`attention_bias` input. ONNX Runtime's CUDA GroupQueryAttention kernel currently
rejects that input at runtime. This script rewrites those self-attention nodes to
standard ONNX opset-24 `Attention`, preserving Q/K/V, the bias/mask, KV cache
inputs, head counts, scale, and output names.

Usage:
  python scripts/patch_cohere_decoder_attention.py \
    --input "%LOCALAPPDATA%/Taurscribe/models/cohere-speech-1b/decoder_model_merged_fp16.onnx" \
    --output "%LOCALAPPDATA%/Taurscribe/models/cohere-speech-1b/decoder_model_merged_fp16_attention_causal0.onnx"

The output keeps references to the original external data file, so place it in
the same directory as `decoder_model_merged_fp16.onnx_data`.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import onnx
from onnx import helper


def patch_decoder(input_path: Path, output_path: Path) -> int:
    model = onnx.load(input_path, load_external_data=False)

    for opset in model.opset_import:
        if opset.domain == "":
            opset.version = max(opset.version, 24)

    rewritten = 0
    for node in model.graph.node:
        if node.domain != "com.microsoft" or node.op_type != "GroupQueryAttention":
            continue

        attrs = {attr.name: helper.get_attribute_value(attr) for attr in node.attribute}
        old_inputs = list(node.input)

        if len(old_inputs) <= 10 or not old_inputs[10]:
            raise ValueError(f"{node.name} does not have the expected attention_bias input")

        node.domain = ""
        node.op_type = "Attention"

        node.attribute.clear()
        node.attribute.extend(
            [
                helper.make_attribute("q_num_heads", int(attrs["num_heads"])),
                helper.make_attribute("kv_num_heads", int(attrs["kv_num_heads"])),
                helper.make_attribute("scale", float(attrs["scale"])),
                helper.make_attribute("is_causal", 0),
            ]
        )

        # GroupQueryAttention:
        #   0 Q, 1 K, 2 V, 3 past_key, 4 past_value, 10 attention_bias
        # ONNX Attention:
        #   0 Q, 1 K, 2 V, 3 attn_mask, 4 past_key, 5 past_value, 6 nonpad_kv_seqlen
        node.input.clear()
        node.input.extend(
            [
                old_inputs[0],
                old_inputs[1],
                old_inputs[2],
                old_inputs[10],
                old_inputs[3],
                old_inputs[4],
                "",
            ]
        )
        rewritten += 1

    if rewritten == 0:
        raise ValueError("No com.microsoft::GroupQueryAttention nodes were found")

    output_path.parent.mkdir(parents=True, exist_ok=True)
    onnx.save(model, output_path, save_as_external_data=False)
    return rewritten


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    rewritten = patch_decoder(args.input, args.output)
    print(f"wrote {args.output}")
    print(f"rewrote {rewritten} GroupQueryAttention nodes")


if __name__ == "__main__":
    main()
