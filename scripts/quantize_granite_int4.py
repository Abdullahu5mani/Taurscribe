#!/usr/bin/env python3
"""Create an experimental INT4 Granite ONNX bundle.

This is intentionally conservative: it copies the source bundle, then applies
ONNX Runtime's weight-only 4-bit quantizer to MatMul nodes only. Embedding
Gather quantization is left out for runtime compatibility while testing.
"""

from __future__ import annotations

import argparse
import logging
import shutil
import tempfile
from pathlib import Path

import onnx
from onnxruntime.quantization.matmul_nbits_quantizer import MatMulNBitsQuantizer


MODEL_FILES = (
    "encoder.onnx",
    "projector.onnx",
    "editor.onnx",
)

PRESERVE_SUFFIXES = {
    ".json",
    ".onnx",
    ".txt",
    ".md",
    ".model",
    ".yaml",
    ".yml",
}


def copy_bundle(src: Path, dst: Path, overwrite: bool) -> None:
    if dst.exists():
        if not overwrite:
            raise SystemExit(f"Destination already exists: {dst}")
        shutil.rmtree(dst)
    shutil.copytree(src, dst)


def quantize_model(path: Path, block_size: int, symmetric: bool) -> None:
    quantizer = MatMulNBitsQuantizer(
        str(path),
        bits=4,
        block_size=block_size,
        is_symmetric=symmetric,
        op_types_to_quantize=("MatMul",),
    )
    quantizer.process()

    with tempfile.TemporaryDirectory(prefix=f"{path.stem}-int4-") as temp_dir:
        temp_path = Path(temp_dir) / path.name
        temp_data = Path(str(temp_path) + ".data")
        quantizer.model.save_model_to_file(str(temp_path), use_external_data_format=True)

        old_data = Path(str(path) + ".data")
        if old_data.exists():
            old_data.unlink()
        path.unlink()
        temp_path.replace(path)
        if temp_data.exists():
            temp_data.replace(old_data)


def referenced_external_data_files(model_path: Path) -> set[str]:
    model = onnx.load(str(model_path), load_external_data=False)
    refs: set[str] = set()

    def scan_tensor(tensor: onnx.TensorProto) -> None:
        for entry in tensor.external_data:
            if entry.key == "location":
                refs.add(entry.value)

    def scan_graph(graph: onnx.GraphProto) -> None:
        for initializer in graph.initializer:
            scan_tensor(initializer)
        for sparse_initializer in graph.sparse_initializer:
            scan_tensor(sparse_initializer.values)
            scan_tensor(sparse_initializer.indices)
        for node in graph.node:
            for attr in node.attribute:
                if attr.HasField("t"):
                    scan_tensor(attr.t)
                for tensor in attr.tensors:
                    scan_tensor(tensor)
                if attr.HasField("g"):
                    scan_graph(attr.g)
                for subgraph in attr.graphs:
                    scan_graph(subgraph)

    scan_graph(model.graph)
    return refs


def prune_unreferenced_external_data(dst: Path) -> None:
    keep = {path.name for path in dst.glob("*.onnx")}
    for model_path in dst.glob("*.onnx"):
        keep.update(referenced_external_data_files(model_path))

    removed = 0
    removed_bytes = 0
    for path in dst.iterdir():
        if not path.is_file():
            continue
        if path.name in keep or path.suffix.lower() in PRESERVE_SUFFIXES:
            continue
        removed += 1
        removed_bytes += path.stat().st_size
        path.unlink()

    print(
        "Pruned "
        f"{removed} stale external-data files "
        f"({removed_bytes / 1024 / 1024 / 1024:.2f} GiB)"
    )


def main() -> None:
    logging.getLogger("onnxruntime.quantization.matmul_nbits_quantizer").setLevel(logging.WARNING)

    parser = argparse.ArgumentParser()
    parser.add_argument("--src", required=True, type=Path)
    parser.add_argument("--dst", required=True, type=Path)
    parser.add_argument("--overwrite", action="store_true")
    parser.add_argument("--block-size", type=int, default=128)
    parser.add_argument("--symmetric", action="store_true")
    parser.add_argument(
        "--prune-only",
        action="store_true",
        help="Only remove unreferenced external-data files from --dst.",
    )
    args = parser.parse_args()

    src = args.src.expanduser().resolve()
    dst = args.dst.expanduser().resolve()
    if not args.prune_only and not src.exists():
        raise SystemExit(f"Source does not exist: {src}")

    if args.prune_only:
        if not dst.exists():
            raise SystemExit(f"Destination does not exist: {dst}")
    else:
        print(f"Copying bundle: {src} -> {dst}")
        copy_bundle(src, dst, args.overwrite)

        for name in MODEL_FILES:
            path = dst / name
            if not path.exists():
                print(f"Skipping missing model: {path}")
                continue
            before = path.stat().st_size + (Path(str(path) + ".data").stat().st_size if Path(str(path) + ".data").exists() else 0)
            print(f"Quantizing {name} ({before / 1024 / 1024:.1f} MiB)...")
            quantize_model(path, args.block_size, args.symmetric)
            after = path.stat().st_size + (Path(str(path) + ".data").stat().st_size if Path(str(path) + ".data").exists() else 0)
            print(f"  -> {after / 1024 / 1024:.1f} MiB")

    prune_unreferenced_external_data(dst)

    total = sum(p.stat().st_size for p in dst.rglob("*") if p.is_file())
    print(f"Done. Bundle size: {total / 1024 / 1024 / 1024:.2f} GiB")


if __name__ == "__main__":
    main()
