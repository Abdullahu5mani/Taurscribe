"""Per-stage timing breakdown, to find where the latency actually goes.

Each stage is forced to evaluate with mx.eval() before the clock is read,
because MLX is lazy and would otherwise attribute all the work to whichever
call first needs a concrete value.

  python scripts/granite_mlx/profile_stages.py --seconds 2 8 16
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
from granite_nar_mlx import MODEL_ID, _add_insertion_slots, _ctc_greedy, load_model  # noqa: E402


def timed(fn, runs: int) -> tuple[float, object]:
    result = fn()
    mx.eval(result)
    samples = []
    for _ in range(runs):
        start = time.perf_counter()
        out = fn()
        mx.eval(out)
        samples.append(time.perf_counter() - start)
        result = out
    return statistics.median(samples), result


def profile(model, config, frames: int, runs: int) -> None:
    rng = np.random.default_rng(0)
    features = mx.array(rng.standard_normal((1, frames, 160), dtype=np.float32)).astype(mx.float16)

    enc_cfg = config["encoder_config"]
    total = 0.0

    t_enc, enc_out = timed(
        lambda: model.encoder(features, config["encoder_layer_indices"]), runs
    )
    bpe_logits, multilayer = enc_out
    pooled_len = -(-frames // enc_cfg["bpe_pooling_window"])
    bpe_logits = bpe_logits[:, :pooled_len]
    total += t_enc

    t_argmax, ids = timed(lambda: mx.argmax(bpe_logits[0], axis=-1), runs)
    total += t_argmax
    ctc_ids = _ctc_greedy(ids, config["blank_token_id"])

    t_proj, audio_embeds = timed(lambda: model.projector(multilayer), runs)
    total += t_proj

    audio_embeds = audio_embeds / config["text_config"]["embedding_multiplier"]
    audio_len = frames // config["projector_config"]["downsample_rate"]
    audio_embeds = audio_embeds[:, :audio_len]

    slots = _add_insertion_slots(
        ctc_ids, config["blank_token_id"], config["min_edit_sequence_length"]
    )
    text_embeds = model.editor.embed_tokens(mx.array(slots))[None]
    embeds = mx.concatenate([audio_embeds, text_embeds], axis=1)
    seq = embeds.shape[1]

    # Editor split into backbone vs the vocabulary projection, because only the
    # text tail of the sequence is ever read from the projection.
    def backbone():
        x = embeds * config["text_config"]["embedding_multiplier"]
        for layer in model.editor.layers:
            x = layer(x)
        return mx.fast.rms_norm(x, model.editor.norm_w, model.editor.eps)

    t_body, hidden = timed(backbone, runs)
    total += t_body

    t_head_all, _ = timed(lambda: model.editor._project_to_vocab(hidden), runs)
    t_head_text, _ = timed(lambda: model.editor._project_to_vocab(hidden[:, audio_len:]), runs)
    total += t_head_all

    print(f"\n  frames={frames} ({frames * 0.02:.1f}s audio)  editor_seq={seq} "
          f"(audio={audio_len}, text={len(slots)})")
    print(f"    {'stage':26s} {'ms':>8s}  {'%':>5s}")
    for name, value in [
        ("conformer encoder x16", t_enc),
        ("ctc argmax", t_argmax),
        ("projector", t_proj),
        ("editor backbone x40", t_body),
        ("lm_head (full seq)", t_head_all),
    ]:
        print(f"    {name:26s} {value * 1000:8.1f}  {value / total * 100:5.1f}")
    print(f"    {'TOTAL':26s} {total * 1000:8.1f}")
    saved = t_head_all - t_head_text
    print(f"    lm_head (text only)      {t_head_text * 1000:8.1f} ms "
          f"-> saves {saved * 1000:.1f} ms ({saved / total * 100:.1f}% of total)")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, nargs="+", default=[2.0, 8.0, 16.0])
    parser.add_argument("--runs", type=int, default=5)
    args = parser.parse_args()

    from huggingface_hub import snapshot_download

    model, config = load_model(Path(snapshot_download(MODEL_ID)), dtype=mx.float16)
    for seconds in args.seconds:
        profile(model, config, int(seconds / 0.02), args.runs)


if __name__ == "__main__":
    main()
