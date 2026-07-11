#!/usr/bin/env python3
"""Create a Granite ONNX bundle whose editor outputs token IDs.

The original Granite editor exports full logits shaped [batch, sequence, vocab].
For Taurscribe we only need the argmax token per text timestep before CTC
collapse. This script preserves the source bundle and writes a copied bundle
where editor.onnx has one output:

    token_ids = ArgMax(logits, axis=-1, keepdims=0)

This avoids returning the huge logits tensor to Rust.
"""

from __future__ import annotations

import argparse
import shutil
from pathlib import Path

import onnx
from onnx import TensorProto, helper


def copy_bundle(src: Path, dst: Path, overwrite: bool) -> None:
    if dst.exists():
        if not overwrite:
            raise SystemExit(f"Destination already exists: {dst}")
        shutil.rmtree(dst)
    shutil.copytree(src, dst)


def replace_editor_output(editor_path: Path) -> None:
    model = onnx.load(str(editor_path), load_external_data=False)
    if not any(output.name == "logits" for output in model.graph.output):
        raise SystemExit(f"{editor_path} does not expose a logits output")
    if any(node.output and node.output[0] == "token_ids" for node in model.graph.node):
        raise SystemExit(f"{editor_path} already appears to contain token_ids")

    argmax = helper.make_node(
        "ArgMax",
        inputs=["logits"],
        outputs=["token_ids"],
        name="taurscribe_token_ids_argmax",
        axis=-1,
        keepdims=0,
        select_last_index=0,
    )
    model.graph.node.append(argmax)

    del model.graph.output[:]
    model.graph.output.append(
        helper.make_tensor_value_info(
            "token_ids",
            TensorProto.INT64,
            ["batch", "sequence"],
        )
    )

    onnx.save_model(model, str(editor_path))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--src", required=True, type=Path)
    parser.add_argument("--dst", required=True, type=Path)
    parser.add_argument("--overwrite", action="store_true")
    args = parser.parse_args()

    src = args.src.expanduser().resolve()
    dst = args.dst.expanduser().resolve()
    if not src.exists():
        raise SystemExit(f"Source does not exist: {src}")

    print(f"Copying bundle: {src} -> {dst}")
    copy_bundle(src, dst, args.overwrite)
    replace_editor_output(dst / "editor.onnx")

    total = sum(p.stat().st_size for p in dst.rglob("*") if p.is_file())
    print(f"Done. Bundle size: {total / 1024 / 1024 / 1024:.2f} GiB")


if __name__ == "__main__":
    main()
