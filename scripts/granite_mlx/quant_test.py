"""Does quantization buy SPEED here (not just memory), and at what token cost?

Measures full transcribe_ids latency for fp16 vs 8-bit vs 4-bit weights on a real
utterance, and counts how many output tokens change against the fp16 reference.
For a GEMM-bound workload at these sequence lengths, quantized matmul may or may
not beat fp16 — this settles it with numbers instead of a guess.

  python scripts/granite_mlx/quant_test.py --runs 8
"""

from __future__ import annotations

import argparse
import statistics
import sys
import time
from pathlib import Path

import mlx.core as mx
import mlx.nn as nn
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from bench import load_audio, log_mel  # noqa: E402
from granite_nar_mlx import MODEL_ID, load_model  # noqa: E402


def bench(model, features, runs):
    model.transcribe_ids(features)
    ids, _ = model.transcribe_ids(features)
    times = []
    for _ in range(runs):
        t = time.perf_counter()
        model.transcribe_ids(features)
        times.append(time.perf_counter() - t)
    return ids, statistics.median(times)


def token_diff(a, b):
    n = max(len(a), len(b))
    a = a + [None] * (n - len(a))
    b = b + [None] * (n - len(b))
    return sum(x != y for x, y in zip(a, b))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=8)
    args = ap.parse_args()

    from huggingface_hub import snapshot_download

    path = Path(snapshot_download(MODEL_ID))
    wav = load_audio(path / "10226_10111_000000.wav")
    duration = len(wav) / 16000
    features16 = mx.array(log_mel(wav)).astype(mx.float16)
    print(f"utterance {duration:.2f}s  ({features16.shape[1]} frames), runs={args.runs}\n")

    model, _ = load_model(path, dtype=mx.float16)
    ref_ids, base_p50 = bench(model, features16, args.runs)
    del model
    mx.clear_cache()

    print(f"{'variant':16s} {'p50':>9s} {'RTF':>7s} {'vs fp16':>9s} {'tokens Δ':>9s}")
    print(f"{'fp16':16s} {base_p50:8.3f}s {base_p50/duration:7.3f} {'—':>9s} {'ref':>9s}")

    # Only layers whose input dim is divisible by the group size can be quantized;
    # the editor + embeddings (the weight mass) qualify, a few tiny encoder
    # projections (e.g. 1024x160) do not and stay fp16.
    def eligible(group):
        def pred(_path, m):
            if isinstance(m, (nn.Linear, nn.Embedding)):
                w = m.weight
                return w.shape[-1] % group == 0
            return False
        return pred

    for bits, group in [(8, 64), (4, 64)]:
        model, _ = load_model(path, dtype=mx.float16)
        nn.quantize(model, group_size=group, bits=bits, class_predicate=eligible(group))
        feats = features16
        ids, p50 = bench(model, feats, args.runs)
        speed = (base_p50 - p50) / base_p50 * 100
        print(f"{f'{bits}-bit g{group}':16s} {p50:8.3f}s {p50/duration:7.3f} "
              f"{speed:+8.1f}% {token_diff(ref_ids, ids):9d}")
        del model
        mx.clear_cache()

    print(f"\n  (fp16 reference: {len(ref_ids)} tokens)")


if __name__ == "__main__":
    main()
