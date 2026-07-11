#!/usr/bin/env python3
"""Build the Granite portable (non-NVIDIA) bundle from the INT4 argmax bundle.

The stock Granite encoder export cannot run on DirectML for two reasons:

1. Its windowed attention uses rank-5 MatMul (`[1,4,8,200,128] x [1,4,8,128,200]`).
   The DirectML execution provider rejects these at kernel creation with
   `80070057 E_INVALIDARG` (observed on both AMD and NVIDIA adapters).
2. Odd-numbered conformer layers compute their Reshape/Slice shape operands at
   runtime through Shape -> Gather -> Unsqueeze -> Concat chains. DirectML
   crashes with a native access violation when it compiles partitions around
   those dynamically-shaped Reshape/Slice nodes.

Both are graph-shape problems, not quantization problems (the plain FP32
export fails identically). Because the app always pads encoder input to a
fixed [1, 800, 160] bucket, both are fixable offline:

- every attention MatMul is flattened to rank 3 by merging the static
  (1, 4 windows, 8 heads) batch dims into 32, sandwiched between Reshapes;
- every shape-slot operand (Reshape target, Slice bounds, Pad pads, ...) is
  evaluated once on CPU with a zero input and baked in as an initializer,
  after which the dead Shape/Gather/Concat chains are pruned (~950 nodes).

Output parity vs. the source encoder is bit-noise only (max rel diff ~4e-5,
from the reassociated batched matmul), and the pruned graph is also slightly
faster on CPU. Projector / embed_tokens / editor are copied through unchanged
— the editor's dynamic sequence dimension already works on DirectML.

Usage:
  python scripts/make_granite_portable_dml.py \
    --src <granite-speech-4.1-2b-nar-int4-argmax dir> \
    --dst <output bundle dir> [--overwrite]
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
from pathlib import Path

import numpy as np
import onnx
from onnx import helper, numpy_helper
import onnxruntime as ort

ENCODER_FRAMES = 800
ATTN_WINDOWS = 4
ATTN_HEADS = 8
ATTN_WINDOW_LEN = 200
ATTN_HEAD_DIM = 128
FLAT_BATCH = ATTN_WINDOWS * ATTN_HEADS

ATTN_MM = re.compile(r"^/layers\.\d+/attn/(MatMul|MatMul_1)$")

# lhs/rhs/out trailing dims for the two attention matmuls
MM_DIMS = {
    "MatMul": ((ATTN_WINDOW_LEN, ATTN_HEAD_DIM), (ATTN_HEAD_DIM, ATTN_WINDOW_LEN),
               (ATTN_WINDOW_LEN, ATTN_WINDOW_LEN)),
    "MatMul_1": ((ATTN_WINDOW_LEN, ATTN_WINDOW_LEN), (ATTN_WINDOW_LEN, ATTN_HEAD_DIM),
                 (ATTN_WINDOW_LEN, ATTN_HEAD_DIM)),
}

SHAPE_SLOTS = {
    "Reshape": [1],
    "Slice": [1, 2, 3, 4],
    "Pad": [1],
    "Expand": [1],
    "Unsqueeze": [1],
    "Squeeze": [1],
    "Tile": [1],
    "ConstantOfShape": [0],
    "Range": [0, 1, 2],
}

COPY_FILES = (
    "projector.onnx",
    "projector.onnx.data",
    "embed_tokens.onnx",
    "embed_tokens.onnx.data",
    "editor.onnx",
    "editor.onnx.data",
    "tokenizer.json",
    "tokenizer_config.json",
    "preprocessor_config.json",
    "processor_config.json",
    "generation_config.json",
)


def flatten_attention_matmuls(model: onnx.ModelProto) -> int:
    graph = model.graph
    new_inits: dict[str, onnx.TensorProto] = {}

    def shape_init(dims: list[int]) -> str:
        key = "ts_shape_" + "_".join(str(d) for d in dims)
        if key not in new_inits:
            new_inits[key] = numpy_helper.from_array(
                np.array(dims, dtype=np.int64), name=key)
        return key

    insertions = []
    count = 0
    for idx, node in enumerate(graph.node):
        if node.op_type != "MatMul":
            continue
        match = ATTN_MM.match(node.name)
        if not match:
            continue
        (la, lb), (ra, rb), (oa, ob) = MM_DIMS[match.group(1)]
        lhs, rhs = node.input
        out = node.output[0]
        lhs_flat = f"{node.name}_lhs_flat"
        rhs_flat = f"{node.name}_rhs_flat"
        out_flat = f"{node.name}_out_flat"
        pre = [
            helper.make_node("Reshape", [lhs, shape_init([FLAT_BATCH, la, lb])],
                             [lhs_flat], name=f"{node.name}_flatten_lhs"),
            helper.make_node("Reshape", [rhs, shape_init([FLAT_BATCH, ra, rb])],
                             [rhs_flat], name=f"{node.name}_flatten_rhs"),
        ]
        post = helper.make_node(
            "Reshape",
            [out_flat, shape_init([1, ATTN_WINDOWS, ATTN_HEADS, oa, ob])],
            [out], name=f"{node.name}_unflatten_out")
        node.input[0] = lhs_flat
        node.input[1] = rhs_flat
        node.output[0] = out_flat
        insertions.append((idx, pre, post))
        count += 1

    for idx, pre, post in reversed(insertions):
        for p in reversed(pre):
            graph.node.insert(idx, p)
        graph.node.insert(idx + len(pre) + 1, post)
    graph.initializer.extend(new_inits.values())
    return count


def replace_glu_splits(model: onnx.ModelProto) -> int:
    """Replace two-output GLU Split nodes with two Slice nodes.

    DirectML's graph fusion mis-executes multi-output Split inside fused
    partitions (observed as garbage GLU output on the Radeon 780M and RTX
    4070 alike, ORT 1.22–1.24), so the portable encoder avoids the op.
    """
    graph = model.graph
    producers = {out: node for node in graph.node for out in node.output}
    init_dims = {t.name: list(t.dims) for t in graph.initializer}
    new_inits: dict[str, onnx.TensorProto] = {}

    def index_init(name: str, values: list[int]) -> str:
        if name not in new_inits:
            new_inits[name] = numpy_helper.from_array(
                np.array(values, dtype=np.int64), name=name)
        return name

    replaced = 0
    insertions = []
    removals = []
    for idx, node in enumerate(graph.node):
        if node.op_type != "Split" or len(node.output) != 2 or len(node.input) != 1:
            continue
        axis = next((a.i for a in node.attribute if a.name == "axis"), 0)
        producer = producers.get(node.input[0])
        channels = None
        if producer is not None and producer.op_type == "Conv" and len(producer.input) >= 2:
            dims = init_dims.get(producer.input[1])
            if dims:
                channels = dims[0]
        if channels is None or channels % 2 != 0:
            print(f"  skipping Split {node.name}: cannot derive channel count")
            continue
        half = channels // 2
        axes = index_init("ts_split_axes_" + str(axis), [axis])
        slice_a = helper.make_node(
            "Slice",
            [node.input[0], index_init("ts_split_start_0", [0]),
             index_init(f"ts_split_end_{half}", [half]), axes],
            [node.output[0]], name=f"{node.name}_slice_lo")
        slice_b = helper.make_node(
            "Slice",
            [node.input[0], index_init(f"ts_split_end_{half}", [half]),
             index_init(f"ts_split_end_{channels}", [channels]), axes],
            [node.output[1]], name=f"{node.name}_slice_hi")
        insertions.append((idx, [slice_a, slice_b]))
        removals.append(idx)
        replaced += 1

    for idx, nodes in reversed(insertions):
        del graph.node[idx]
        for n in reversed(nodes):
            graph.node.insert(idx, n)
    graph.initializer.extend(new_inits.values())
    return replaced


def bake_shape_slots(model: onnx.ModelProto, work_dir: Path) -> tuple[int, int]:
    graph = model.graph
    init_names = {t.name for t in graph.initializer}
    const_outputs = {n.output[0] for n in graph.node if n.op_type == "Constant"}
    producers = {out: node for node in graph.node for out in node.output}

    candidates = set()
    for node in graph.node:
        for s in SHAPE_SLOTS.get(node.op_type, ()):  # noqa: B905
            if s >= len(node.input):
                continue
            t = node.input[s]
            if t and t not in init_names and t not in const_outputs and t in producers:
                candidates.add(t)
    if not candidates:
        return 0, 0

    eval_model = onnx.ModelProto()
    eval_model.CopyFrom(model)
    del eval_model.graph.output[:]
    for t in sorted(candidates):
        eval_model.graph.output.append(helper.make_empty_tensor_value_info(t))
    eval_path = work_dir / "_bake_eval.onnx"
    onnx.save(eval_model, str(eval_path))

    so = ort.SessionOptions()
    so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_DISABLE_ALL
    so.log_severity_level = 3
    session = ort.InferenceSession(str(eval_path), sess_options=so,
                                   providers=["CPUExecutionProvider"])
    feeds = {"input_features": np.zeros((1, ENCODER_FRAMES, 160), dtype=np.float32)}
    values = session.run(None, feeds)
    names = [o.name for o in session.get_outputs()]
    del session
    eval_path.unlink(missing_ok=True)

    baked = dict(zip(names, values))
    for name, value in baked.items():
        arr = np.asarray(value)
        graph.initializer.append(numpy_helper.from_array(arr, name=name + "_baked"))

    for node in graph.node:
        for s in SHAPE_SLOTS.get(node.op_type, ()):  # noqa: B905
            if s < len(node.input) and node.input[s] in baked:
                node.input[s] = node.input[s] + "_baked"

    needed = set()
    stack = [o.name for o in graph.output]
    while stack:
        t = stack.pop()
        if t in needed:
            continue
        needed.add(t)
        node = producers.get(t)
        if node:
            stack.extend(i for i in node.input if i)

    keep_nodes = [n for n in graph.node if any(o in needed for o in n.output)]
    pruned = len(graph.node) - len(keep_nodes)
    del graph.node[:]
    graph.node.extend(keep_nodes)

    used = set()
    for n in graph.node:
        used.update(i for i in n.input if i)
    keep_inits = [t for t in graph.initializer if t.name in used]
    del graph.initializer[:]
    graph.initializer.extend(keep_inits)
    return len(baked), pruned


def verify_parity(src_encoder: Path, dst_encoder: Path) -> float:
    rng = np.random.default_rng(3)
    feeds = {"input_features": rng.standard_normal((1, ENCODER_FRAMES, 160),
                                                   dtype=np.float32)}
    so = ort.SessionOptions()
    so.log_severity_level = 3
    worst_rel = 0.0
    a = ort.InferenceSession(str(src_encoder), sess_options=so,
                             providers=["CPUExecutionProvider"])
    outs_a = a.run(None, feeds)
    del a
    b = ort.InferenceSession(str(dst_encoder), sess_options=so,
                             providers=["CPUExecutionProvider"])
    outs_b = b.run(None, feeds)
    del b
    for ta, tb in zip(outs_a, outs_b):
        diff = float(np.max(np.abs(ta - tb)))
        rel = diff / (float(np.max(np.abs(ta))) + 1e-9)
        worst_rel = max(worst_rel, rel)
    return worst_rel


def update_manifest(dst: Path) -> None:
    manifest_path = dst / "taurscribe_granite_nar_manifest.json"
    manifest = {}
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest.update(
        {
            "format": "taurscribe-granite-nar-onnx-bundle",
            "variant": "int4-argmax-dml-static",
            "export_dtype": "int4-matmulnbits-weights",
            "fixed_encoder_frames": ENCODER_FRAMES,
            "encoder_dml_safe": True,
            "execution_provider_preference": [
                "CPUExecutionProvider",
                "DmlExecutionProvider",
            ],
        }
    )
    notes = manifest.setdefault("notes", [])
    note = ("Portable bundle: encoder attention MatMuls flattened to rank 3 and "
            "shape chains baked for the fixed 800-frame bucket so the full "
            "encoder loads on DirectML; CPU remains the default backend.")
    if note not in notes:
        notes.append(note)
    if isinstance(manifest.get("graphs"), dict):
        editor = manifest["graphs"].get("editor")
        if isinstance(editor, dict):
            editor["outputs"] = ["token_ids"]
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--src", required=True, type=Path,
                        help="INT4 argmax source bundle directory")
    parser.add_argument("--dst", required=True, type=Path,
                        help="output portable bundle directory")
    parser.add_argument("--overwrite", action="store_true")
    args = parser.parse_args()

    src = args.src.expanduser().resolve()
    dst = args.dst.expanduser().resolve()
    if not src.is_dir():
        raise SystemExit(f"Source does not exist: {src}")
    if dst.exists():
        if not args.overwrite:
            raise SystemExit(f"Destination already exists: {dst}")
        shutil.rmtree(dst)
    dst.mkdir(parents=True)

    print(f"Loading encoder from {src}")
    model = onnx.load(str(src / "encoder.onnx"), load_external_data=False)

    flattened = flatten_attention_matmuls(model)
    print(f"Flattened {flattened} attention MatMuls to rank 3")

    replaced = replace_glu_splits(model)
    print(f"Replaced {replaced} GLU Split nodes with Slice pairs")

    # Baking needs the external weights resolvable next to the eval graph.
    shutil.copy2(src / "encoder.onnx.data", dst / "encoder.onnx.data")
    baked, pruned = bake_shape_slots(model, dst)
    print(f"Baked {baked} shape tensors, pruned {pruned} dead nodes")

    onnx.save(model, str(dst / "encoder.onnx"))

    for name in COPY_FILES:
        path = src / name
        if path.exists():
            shutil.copy2(path, dst / name)
    if (src / "taurscribe_granite_nar_manifest.json").exists():
        shutil.copy2(src / "taurscribe_granite_nar_manifest.json",
                     dst / "taurscribe_granite_nar_manifest.json")
    update_manifest(dst)

    worst_rel = verify_parity(src / "encoder.onnx", dst / "encoder.onnx")
    print(f"Encoder CPU parity worst relative diff: {worst_rel:.2e}")
    if worst_rel > 1e-3:
        raise SystemExit("Parity check failed; not shipping this bundle")

    total = sum(p.stat().st_size for p in dst.rglob("*") if p.is_file())
    print(f"Done. Bundle size: {total / 1024 / 1024 / 1024:.2f} GiB")


if __name__ == "__main__":
    main()
