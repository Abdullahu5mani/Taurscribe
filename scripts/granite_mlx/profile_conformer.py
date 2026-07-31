"""Drill into the conformer block and test mx.compile.

The stage profile showed the 16-layer encoder is ~58% of latency while running
at roughly a quarter of the M3's fp16 peak, which points at dispatch overhead
rather than arithmetic. This measures each sub-module and then checks what
mx.compile recovers. Compilation changes scheduling, not math, so any win here
is free of accuracy cost.

  python scripts/granite_mlx/profile_conformer.py --frames 400 800
"""

from __future__ import annotations

import argparse
import statistics
import sys
import time
from pathlib import Path

import mlx.core as mx
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from granite_nar_mlx import MODEL_ID, load_model  # noqa: E402


def timed(fn, runs: int = 10) -> float:
    mx.eval(fn())
    samples = []
    for _ in range(runs):
        start = time.perf_counter()
        mx.eval(fn())
        samples.append(time.perf_counter() - start)
    return statistics.median(samples)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--frames", type=int, nargs="+", default=[400, 800])
    parser.add_argument("--runs", type=int, default=10)
    args = parser.parse_args()

    from huggingface_hub import snapshot_download

    model, config = load_model(Path(snapshot_download(MODEL_ID)), dtype=mx.float16)
    block = model.encoder.layers[0]
    dists = model.encoder._attention_dists
    editor_layer = model.editor.layers[0]

    for frames in args.frames:
        rng = np.random.default_rng(0)
        x = mx.array(rng.standard_normal((1, frames, 1024), dtype=np.float32)).astype(mx.float16)
        mx.eval(x)

        print(f"\n=== frames={frames} ({frames * 0.02:.1f}s) ===")
        parts = {
            "ff1": lambda: block.ff1(x),
            "attn": lambda: block.attn(x, dists),
            "conv": lambda: block.conv(x),
            "ff2": lambda: block.ff2(x),
            "post_norm": lambda: block.post_norm(x),
        }
        subtotal = 0.0
        for name, fn in parts.items():
            ms = timed(fn, args.runs) * 1000
            subtotal += ms
            print(f"  {name:12s} {ms:7.2f} ms")
        whole = timed(lambda: block(x, dists), args.runs) * 1000
        print(f"  {'full block':12s} {whole:7.2f} ms   (sum of parts {subtotal:.2f})")

        compiled_block = mx.compile(lambda t: block(t, dists))
        comp = timed(lambda: compiled_block(x), args.runs) * 1000
        print(f"  {'compiled':12s} {comp:7.2f} ms   -> {(whole - comp) / whole * 100:+.1f}%")

        # 16 conformer blocks is the real encoder cost
        print(f"  x16 uncompiled {whole * 16:7.1f} ms | x16 compiled {comp * 16:7.1f} ms")

    # Editor layer, at a sequence length typical of real speech
    for seq in (168, 260):
        rng = np.random.default_rng(1)
        h = mx.array(rng.standard_normal((1, seq, 2048), dtype=np.float32)).astype(mx.float16)
        mx.eval(h)
        plain = timed(lambda: editor_layer(h), args.runs) * 1000
        compiled_layer = mx.compile(editor_layer)
        comp = timed(lambda: compiled_layer(h), args.runs) * 1000
        print(f"\n=== editor layer, seq={seq} ===")
        print(f"  plain {plain:6.2f} ms | compiled {comp:6.2f} ms -> {(plain - comp) / plain * 100:+.1f}%")
        print(f"  x40 plain {plain * 40:6.1f} ms | x40 compiled {comp * 40:6.1f} ms")


if __name__ == "__main__":
    main()
