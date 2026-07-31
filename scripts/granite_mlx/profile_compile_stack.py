"""Are the encoder/editor launch-bound? Compile the WHOLE stack, not a layer.

Per-block mx.compile was a wash, but that still pays 16 (or 40) separate kernel
schedules. Compiling the entire layer loop traces one graph across all layers,
which is the case that helps when many small ops are launch-bound rather than
compute-bound. Same math, so no accuracy cost.

  python scripts/granite_mlx/profile_compile_stack.py
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


def timed(fn, runs=20):
    mx.eval(fn())
    s = []
    for _ in range(runs):
        t = time.perf_counter()
        mx.eval(fn())
        s.append(time.perf_counter() - t)
    return statistics.median(s) * 1000


def main():
    from huggingface_hub import snapshot_download

    model, cfg = load_model(Path(snapshot_download(MODEL_ID)), dtype=mx.float16)
    enc = model.encoder
    ed = model.editor
    scond = enc.cfg["self_conditioning_layer"]

    # ---- encoder: full 16-block loop with self-conditioning ----
    def enc_forward(features):
        x = enc.input_linear(features)
        dists = enc._attention_dists
        for idx, layer in enumerate(enc.layers, start=1):
            x = layer(x, dists)
            if idx == scond:
                mp = mx.softmax(enc.out(x).astype(mx.float32), axis=-1)
                x = x + enc.out_mid(mp.astype(x.dtype))
        return x

    enc_c = mx.compile(enc_forward)

    # ---- editor: full 40-layer loop + final norm ----
    def ed_forward(x):
        for layer in ed.layers:
            x = layer(x)
        return mx.fast.rms_norm(x, ed.norm_w, ed.eps)

    ed_c = mx.compile(ed_forward)

    for frames in (400, 800):
        rng = np.random.default_rng(0)
        feats = mx.array(rng.standard_normal((1, frames, 160), np.float32)).astype(mx.float16)
        mx.eval(feats)
        p = timed(lambda: enc_forward(feats))
        c = timed(lambda: enc_c(feats))
        print(f"encoder  frames={frames:4d}: plain {p:6.1f} ms | compiled {c:6.1f} ms -> {(p-c)/p*100:+.1f}%")

    for seq in (88, 168):
        rng = np.random.default_rng(1)
        h = mx.array(rng.standard_normal((1, seq, cfg["text_config"]["hidden_size"]), np.float32)).astype(mx.float16)
        mx.eval(h)
        p = timed(lambda: ed_forward(h))
        c = timed(lambda: ed_c(h))
        print(f"editor   seq={seq:5d}: plain {p:6.1f} ms | compiled {c:6.1f} ms -> {(p-c)/p*100:+.1f}%")

    # end-to-end sanity: what fraction of a real utterance would a win on each buy?
    print("\n(encoder ~58% of latency, editor ~33% at 800 frames / seq 168)")


if __name__ == "__main__":
    main()
