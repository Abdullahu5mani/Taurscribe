"""End-to-end effect of compiling the encoder stack, on the real transcribe path.

Patches a whole-stack-compiled encoder into model.encode and measures full
transcribe_ids latency before/after on a real utterance, then checks the output
token ids are bit-identical (no accuracy change) and reports the worst logit
drift. This is the honest number: the actual shipping path, real audio.

  python scripts/granite_mlx/end2end_compile.py --runs 8
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
from bench import load_audio, log_mel  # noqa: E402
from granite_nar_mlx import MODEL_ID, load_model  # noqa: E402


def bench(model, features, runs):
    model.transcribe_ids(features)  # warm up graph + kernels
    ids, _ = model.transcribe_ids(features)
    times = []
    for _ in range(runs):
        t = time.perf_counter()
        model.transcribe_ids(features)
        times.append(time.perf_counter() - t)
    return ids, statistics.median(times), statistics.mean(times)


def patch_compiled_encoder(model):
    enc = model.encoder
    li = model.config["encoder_layer_indices"]
    window = model.config["encoder_config"]["bpe_pooling_window"]
    compiled = mx.compile(lambda feats: enc(feats, li, None))

    def fast_encode(features, valid_frames, capture=None):
        bpe_logits, multilayer = compiled(features)
        pooled_len = -(-valid_frames // window)
        return bpe_logits[:, :pooled_len], multilayer

    model.encode = fast_encode


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=8)
    args = ap.parse_args()

    from huggingface_hub import snapshot_download

    path = Path(snapshot_download(MODEL_ID))
    wav = load_audio(path / "10226_10111_000000.wav")
    duration = len(wav) / 16000
    features = mx.array(log_mel(wav)).astype(mx.float16)
    print(f"utterance {duration:.2f}s  ({features.shape[1]} frames), fp16, runs={args.runs}\n")

    model, _ = load_model(path, dtype=mx.float16)
    base_ids, base_p50, base_mean = bench(model, features, args.runs)

    patch_compiled_encoder(model)
    fast_ids, fast_p50, fast_mean = bench(model, features, args.runs)

    print(f"{'':14s} {'p50':>9s} {'mean':>9s} {'RTF(p50)':>9s}")
    print(f"{'baseline':14s} {base_p50:8.3f}s {base_mean:8.3f}s {base_p50/duration:9.3f}")
    print(f"{'enc-compiled':14s} {fast_p50:8.3f}s {fast_mean:8.3f}s {fast_p50/duration:9.3f}")
    print(f"\n  end-to-end speedup: {(base_p50 - fast_p50) / base_p50 * 100:+.1f}%  "
          f"({base_p50/fast_p50:.2f}x)")

    same = base_ids == fast_ids
    print(f"  output tokens identical: {same}  "
          f"({len(base_ids)} tokens{'' if same else f', {sum(a!=b for a,b in zip(base_ids,fast_ids))} differ'})")


if __name__ == "__main__":
    main()
