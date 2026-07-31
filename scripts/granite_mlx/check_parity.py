"""Stage-by-stage numerical parity: MLX port vs the PyTorch reference.

Feeds the reference's own input_features into the MLX model so the mel
front-end is excluded, isolating the ported graph. Compares every stage
boundary, so drift is localized to the layer that introduced it rather
than only showing up as a wrong transcript.

  python scripts/granite_mlx/check_parity.py --ref ref.npz [--dtype float32]
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import mlx.core as mx
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from granite_nar_mlx import load_model  # noqa: E402

# Tolerances are per-stage: activations accumulate error through 16 conformer
# blocks and 40 editor layers, and fp16 has ~1e-3 relative resolution.
STAGES = [
    ("enc_input_linear", 2e-3),
    ("enc_layer_4", 5e-3),
    ("enc_layer_8", 5e-3),
    ("enc_out_mid_logits", 1e-2),
    ("enc_layer_12", 1e-2),
    ("enc_layer_16", 1e-2),
    ("enc_bpe_logits", 2e-2),
    ("audio_embeds", 2e-2),
    ("editor_logits", 5e-2),
]


def compare(name: str, got: np.ndarray, want: np.ndarray, tol: float) -> tuple[bool, str]:
    if got.shape != want.shape:
        return False, f"shape {got.shape} != reference {want.shape}"
    diff = np.abs(got.astype(np.float64) - want.astype(np.float64))
    scale = max(float(np.abs(want).max()), 1e-6)
    rel = float(diff.max()) / scale
    corr = float(np.corrcoef(got.ravel().astype(np.float64), want.ravel().astype(np.float64))[0, 1])
    ok = rel <= tol
    return ok, f"max|Δ|/scale={rel:.2e} (tol {tol:.0e})  corr={corr:.6f}"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ref", type=Path, default=Path("ref.npz"))
    parser.add_argument("--dtype", default="float32", choices=["float32", "float16", "bfloat16"])
    args = parser.parse_args()

    ref = np.load(args.ref, allow_pickle=True)
    dtype = {"float32": mx.float32, "float16": mx.float16, "bfloat16": mx.bfloat16}[args.dtype]

    model, config = load_model(dtype=dtype)
    features = mx.array(ref["input_features"]).astype(dtype)

    capture: dict[str, mx.array] = {}
    final_ids, ctc_ids = model.transcribe_ids(features, capture)
    mx.eval(list(capture.values()))

    print(f"dtype: {args.dtype}\n")
    failures = 0
    for name, tol in STAGES:
        if name not in ref:
            print(f"  {name:22s} SKIP (not in reference)")
            continue
        got = np.array(capture[name].astype(mx.float32))
        want = ref[name]
        if name == "enc_bpe_logits" and want.ndim == 2 and got.ndim == 2:
            want = want[: got.shape[0]]
        ok, detail = compare(name, got, want, tol)
        failures += 0 if ok else 1
        print(f"  {'PASS' if ok else 'FAIL'}  {name:22s} {detail}")

    print()
    ref_ctc = ref["ctc_ids"].tolist()
    ref_final = ref["final_ids"].tolist()
    print(f"  ctc ids   {'MATCH' if ctc_ids == ref_ctc else 'DIFFER'}  ({len(ctc_ids)} vs {len(ref_ctc)})")
    print(f"  final ids {'MATCH' if final_ids == ref_final else 'DIFFER'}  ({len(final_ids)} vs {len(ref_final)})")

    from transformers import AutoTokenizer

    tokenizer = AutoTokenizer.from_pretrained(
        "ibm-granite/granite-speech-4.1-2b-nar", trust_remote_code=True
    )
    text = tokenizer.decode(final_ids, skip_special_tokens=True)
    print(f"\n  reference : {ref['text']}")
    print(f"  mlx       : {text}")

    if failures or final_ids != ref_final:
        sys.exit(1)


if __name__ == "__main__":
    main()
