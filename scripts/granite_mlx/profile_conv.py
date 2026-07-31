"""Is the depthwise Conv1d the encoder bottleneck, and can we beat it?

The stage profile put the conformer conv module above ff1 despite fewer FLOPs.
depth_conv is a groups==channels Conv1d, the case MLX's conv kernel handles
worst. A depthwise conv is algebraically just sum_k w[:,k] * shift(x, k), a
chain of elementwise ops the GPU fuses well. This measures both and checks they
agree to fp16 precision — any win is free of accuracy cost.

  python scripts/granite_mlx/profile_conv.py
"""

from __future__ import annotations

import statistics
import sys
import time
from pathlib import Path

import mlx.core as mx
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from granite_nar_mlx import MODEL_ID, load_model  # noqa: E402


def timed(fn, runs=30):
    mx.eval(fn())
    s = []
    for _ in range(runs):
        t = time.perf_counter()
        mx.eval(fn())
        s.append(time.perf_counter() - t)
    return statistics.median(s) * 1000


def depthwise_shift(x, weight, bias, kernel):
    """weight is MLX Conv1d layout (channels, kernel, 1)."""
    pad = kernel // 2
    xp = mx.pad(x, [(0, 0), (pad, pad), (0, 0)])
    length = x.shape[1]
    acc = mx.zeros_like(x)
    for k in range(kernel):
        acc = acc + xp[:, k : k + length, :] * weight[:, k, 0]
    return acc + bias


def main():
    from huggingface_hub import snapshot_download

    model, _ = load_model(Path(snapshot_download(MODEL_ID)), dtype=mx.float16)
    conv = model.encoder.layers[0].conv
    w = conv.depth_conv.weight
    bias = conv.depth_conv.bias
    print(f"depth_conv weight shape {w.shape}, groups=depthwise, kernel={w.shape[1]}")

    for frames in (400, 800):
        rng = np.random.default_rng(0)
        x = mx.array(rng.standard_normal((1, frames, 1024), dtype=np.float32)).astype(mx.float16)
        h = conv.up_conv(conv.norm(x))
        a, b = mx.split(h, 2, axis=-1)
        gated = a * mx.sigmoid(b)
        mx.eval(gated)

        print(f"\n=== frames={frames} ({frames*0.02:.1f}s) ===")
        t_full = timed(lambda: conv(x))
        t_up = timed(lambda: conv.up_conv(conv.norm(x)))
        t_dw = timed(lambda: conv.depth_conv(gated))
        t_down = timed(lambda: conv.down_conv(gated))
        print(f"  norm+up_conv   {t_up:6.2f} ms")
        print(f"  depth_conv     {t_dw:6.2f} ms   <-- grouped/depthwise")
        print(f"  down_conv      {t_down:6.2f} ms")
        print(f"  FULL module    {t_full:6.2f} ms")

        t_shift = timed(lambda: depthwise_shift(gated, w, bias, 15))
        win = (t_dw - t_shift) / t_dw * 100
        print(f"  depthwise via shifts  {t_shift:6.2f} ms  ->  {win:+.1f}% vs grouped conv")

        # module-level impact if we swap the implementation
        def conv_shift(t):
            h = conv.up_conv(conv.norm(t))
            aa, bb = mx.split(h, 2, axis=-1)
            g = aa * mx.sigmoid(bb)
            g = mx.fast.silu(depthwise_shift(g, w, bias, 15)) if hasattr(mx.fast, "silu") \
                else (depthwise_shift(g, w, bias, 15))
            return conv.down_conv(g)
        t_mod = timed(lambda: conv_shift(x))
        print(f"  FULL module (shift)   {t_mod:6.2f} ms  ->  {(t_full-t_mod)/t_full*100:+.1f}% "
              f"| x16 saves ~{(t_full-t_mod)*16:.0f} ms/utterance")

        ref = conv.depth_conv(gated)
        alt = depthwise_shift(gated, w, bias, 15)
        mx.eval(ref, alt)
        r = np.array(ref.astype(mx.float32)); al = np.array(alt.astype(mx.float32))
        scale = float(np.abs(r).max())
        print(f"  agreement: max|delta|/scale = {float(np.abs(r-al).max())/scale:.2e}")


if __name__ == "__main__":
    main()
