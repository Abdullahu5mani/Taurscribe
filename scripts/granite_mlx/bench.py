"""Latency and WER benchmark for the Granite Speech NAR MLX port.

  # latency on a single clip, all dtypes
  python scripts/granite_mlx/bench.py latency --runs 5

  # WER + latency over a LibriSpeech test-clean subset
  python scripts/granite_mlx/bench.py wer --root <LibriSpeech/test-clean> --limit 50
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

DTYPES = {"float32": mx.float32, "float16": mx.float16, "bfloat16": mx.bfloat16}


def log_mel(waveform: np.ndarray) -> np.ndarray:
    """Granite's 80-bin log-mel front end, stacked into 160-dim frames."""
    import torch
    import torchaudio

    mel = torchaudio.transforms.MelSpectrogram(
        sample_rate=16000, n_fft=512, win_length=400, hop_length=160, n_mels=80
    )
    audio = torch.from_numpy(waveform)[None]
    length = 2 * (audio.shape[-1] // 320)
    spec = mel(audio.float())[..., :length]
    logmel = spec.transpose(-1, -2).clamp_min(1e-10).log10()
    peak = logmel.amax(dim=(-2, -1), keepdim=True)
    logmel = torch.maximum(logmel, peak - 8.0).div(4).add(1)
    return logmel.reshape(1, -1, 160).numpy()


def load_audio(path: Path) -> np.ndarray:
    import soundfile as sf

    wav, sr = sf.read(str(path), dtype="float32", always_2d=True)
    if sr != 16000:
        raise SystemExit(f"expected 16 kHz, got {sr} for {path}")
    return np.ascontiguousarray(wav.mean(axis=1))


def timed_transcribe(model, features: mx.array) -> tuple[list[int], float]:
    start = time.perf_counter()
    final_ids, _ = model.transcribe_ids(features)
    return final_ids, time.perf_counter() - start


def cmd_latency(args: argparse.Namespace) -> None:
    from huggingface_hub import snapshot_download

    path = Path(snapshot_download(MODEL_ID))
    audio_path = args.audio or (path / "10226_10111_000000.wav")
    waveform = load_audio(Path(audio_path))
    duration = len(waveform) / 16000
    mel = log_mel(waveform)

    print(f"audio: {Path(audio_path).name}  {duration:.2f}s  ({mel.shape[1]} frames)\n")
    print(f"{'dtype':10s} {'load':>8s} {'p50':>9s} {'mean':>9s} {'RTF':>7s}")

    for name in args.dtypes:
        load_start = time.perf_counter()
        model, _ = load_model(path, dtype=DTYPES[name])
        load_time = time.perf_counter() - load_start

        features = mx.array(mel).astype(DTYPES[name])
        timed_transcribe(model, features)  # warm up kernels and graph

        times = [timed_transcribe(model, features)[1] for _ in range(args.runs)]
        median = statistics.median(times)
        print(
            f"{name:10s} {load_time:7.2f}s {median:8.3f}s "
            f"{statistics.mean(times):8.3f}s {median / duration:7.3f}"
        )
        del model
        mx.clear_cache()


def normalize(text: str) -> list[str]:
    keep = [c for c in text.lower() if c.isalnum() or c.isspace() or c == "'"]
    return "".join(keep).split()


def word_error_rate(ref: list[str], hyp: list[str]) -> tuple[int, int]:
    """Levenshtein edit distance over words; returns (errors, reference length)."""
    prev = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, start=1):
        cur = [i]
        for j, h in enumerate(hyp, start=1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (r != h)))
        prev = cur
    return prev[-1], len(ref)


def collect_utterances(root: Path, limit: int) -> list[tuple[str, Path, str]]:
    items: list[tuple[str, Path, str]] = []
    for trans in sorted(root.rglob("*.trans.txt")):
        for line in trans.read_text().splitlines():
            utt_id, _, text = line.partition(" ")
            flac = trans.parent / f"{utt_id}.flac"
            if flac.exists():
                items.append((utt_id, flac, text))
            if len(items) >= limit:
                return items
    return items


def cmd_wer(args: argparse.Namespace) -> None:
    from huggingface_hub import snapshot_download
    from transformers import AutoTokenizer

    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID, trust_remote_code=True)
    utterances = collect_utterances(args.root, args.limit)
    if not utterances:
        raise SystemExit(f"no utterances under {args.root}")

    model, _ = load_model(Path(snapshot_download(MODEL_ID)), dtype=DTYPES[args.dtype])
    print(f"dtype={args.dtype}  utterances={len(utterances)}\n")

    rows = []
    total_errors = total_words = 0
    total_audio = total_time = 0.0

    for index, (utt_id, flac, reference) in enumerate(utterances, start=1):
        waveform = load_audio(flac)
        features = mx.array(log_mel(waveform)).astype(DTYPES[args.dtype])
        if index == 1:
            timed_transcribe(model, features)  # warm up before timing

        final_ids, elapsed = timed_transcribe(model, features)
        hypothesis = tokenizer.decode(final_ids, skip_special_tokens=True)

        errors, words = word_error_rate(normalize(reference), normalize(hypothesis))
        total_errors += errors
        total_words += words
        duration = len(waveform) / 16000
        total_audio += duration
        total_time += elapsed
        rows.append((utt_id, duration, elapsed, errors / max(words, 1), hypothesis))

        if index % 10 == 0 or index == len(utterances):
            print(
                f"  [{index:4d}/{len(utterances)}] "
                f"WER={total_errors / max(total_words, 1):.4f}  "
                f"RTF={total_time / max(total_audio, 1e-9):.3f}"
            )

    latencies = [r[2] for r in rows]
    print(f"\n  utterances     {len(rows)}")
    print(f"  audio          {total_audio:.1f}s")
    print(f"  aggregate WER  {total_errors / max(total_words, 1):.4f}")
    print(f"  weighted RTF   {total_time / max(total_audio, 1e-9):.4f}")
    print(f"  mean latency   {statistics.mean(latencies):.3f}s")
    print(f"  p50 latency    {statistics.median(latencies):.3f}s")
    print(f"  p95 latency    {sorted(latencies)[int(0.95 * len(latencies)) - 1]:.3f}s")

    if args.out:
        import csv

        with open(args.out, "w", newline="") as handle:
            writer = csv.writer(handle)
            writer.writerow(["utt_id", "audio_s", "latency_s", "wer", "hypothesis"])
            writer.writerows(rows)
        print(f"\n  wrote {args.out}")


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)

    lat = sub.add_parser("latency")
    lat.add_argument("--runs", type=int, default=5)
    lat.add_argument("--audio", default=None)
    lat.add_argument("--dtypes", nargs="+", default=["float16", "bfloat16", "float32"])
    lat.set_defaults(func=cmd_latency)

    wer = sub.add_parser("wer")
    wer.add_argument("--root", type=Path, required=True)
    wer.add_argument("--limit", type=int, default=50)
    wer.add_argument("--dtype", default="float16", choices=list(DTYPES))
    wer.add_argument("--out", type=Path, default=None)
    wer.set_defaults(func=cmd_wer)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
