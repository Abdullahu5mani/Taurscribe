#!/usr/bin/env python3
"""Score FlowScribe outputs against the held-out test set.

Runs the app's own inference code (src-tauri tools/flowscribe_compare) so the
numbers reflect what users get, then compares with the targets.

    python eval.py --gguf /path/flowscribe-v3-f16.gguf --out eval/v3     # v3
    python eval.py --v2 --out eval/v2                                    # installed v2
    python eval.py --report eval/v3 eval/v2                              # side by side

Metrics (per model, overall / per level / per app):
  exact     output == target (whitespace-normalised)
  wer       word error rate vs target (case and punctuation ignored)
  invented  share of outputs containing a word found in neither input nor
            target, i.e. made-up content (the failure that matters most)
  ms        latency p50 / p95
"""

import argparse
import collections
import json
import os
import re
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TOOL = ROOT / "src-tauri" / "target" / "release" / "flowscribe_compare"
TEST = HERE.parent / "flowscribe_data" / "out" / "dataset-v1" / "test.jsonl"

TAG = re.compile(r"<(engine|level|app|vocab)=([^>]*)>")


def parse_prompt(prompt: str) -> dict:
    tags = dict(TAG.findall(prompt.split("\n", 1)[0]))
    prev = re.search(r"<prev>(.*?)</prev>", prompt, re.S)
    text = re.search(r"<text>(.*?)</text>", prompt, re.S)
    return {
        "engine": tags.get("engine", "whisper"),
        "level": tags.get("level", "clean"),
        "app": tags.get("app", "generic"),
        "vocab": [v for v in tags.get("vocab", "").split("; ") if v],
        "prev": prev.group(1) if prev else None,
        "text": text.group(1) if text else "",
    }


def norm_words(s: str) -> list[str]:
    return re.findall(r"[a-z0-9@._/'-]+", s.lower().replace("\n", " "))


def wer(ref: list[str], hyp: list[str]) -> float:
    d = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        prev, d[0] = d[0], i
        for j, h in enumerate(hyp, 1):
            prev, d[j] = d[j], min(d[j] + 1, d[j - 1] + 1, prev + (r != h))
    return d[len(hyp)] / max(len(ref), 1)


def invented(inp: str, target: str, out: str, vocab: list[str]) -> bool:
    def pieces(s: str) -> set[str]:
        words = norm_words(s)
        return set(words) | {p for w in words for p in re.split(r"[-/.]", w) if p}
    allowed = pieces(inp) | pieces(target) | pieces(" ".join(vocab))
    # Joined forms ("getuserbyid", "jane@acme.com") count if their pieces were said.
    allowed_text = " ".join(allowed)
    for w in norm_words(out):
        if w in allowed or w.strip(".'-/") in allowed or all(p in allowed for p in re.split(r"[-/.]", w) if p):
            continue
        if len(w) > 3 and w.replace(".", "").replace("@", "") in allowed_text.replace(" ", ""):
            continue
        return True
    return False


def run(args) -> None:
    rows = [json.loads(l) for l in TEST.read_text().splitlines() if l.strip()]
    if args.limit:
        rows = rows[: args.limit]
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    inp = out_dir / "in.jsonl"
    with inp.open("w") as f:
        for i, r in enumerate(rows):
            p = parse_prompt(r["prompt"])
            f.write(json.dumps({**p, "id": i, "target": r["completion"], "meta": r["meta"],
                                "style": "Casual"}, ensure_ascii=False) + "\n")
    env = dict(os.environ)
    env.pop("FLOWSCRIBE_V3_GGUF", None)
    if args.gguf:
        env["FLOWSCRIBE_V3_GGUF"] = args.gguf
    subprocess.run([str(TOOL), str(inp), str(out_dir / "out.jsonl")], env=env, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    report([out_dir])


def score(rows: list[dict]) -> dict:
    n = len(rows)
    if not n:
        return {}
    ms = sorted(r["ms"] for r in rows)
    return {
        "n": n,
        "exact": sum(" ".join(r["output"].split()) == " ".join(r["target"].split()) for r in rows) / n,
        "wer": sum(wer(norm_words(r["target"]), norm_words(r["output"])) for r in rows) / n,
        "invented": sum(invented(r["text"], r["target"], r["output"], r["vocab"]) for r in rows) / n,
        "p50_ms": ms[n // 2],
        "p95_ms": ms[min(n - 1, int(n * 0.95))],
    }


def fmt(s: dict) -> str:
    return (f"n={s['n']:<4} exact {s['exact']:6.1%}  WER {s['wer']:6.1%}  invented {s['invented']:6.1%}  "
            f"latency p50 {s['p50_ms']} ms / p95 {s['p95_ms']} ms")


def report(dirs: list[Path]) -> None:
    lines = []
    for d in dirs:
        rows = [json.loads(l) for l in (d / "out.jsonl").read_text().splitlines() if l.strip()]
        version = rows[0]["version"] if rows else "?"
        lines.append(f"\n## {d.name} ({version})\nall        {fmt(score(rows))}")
        by = collections.defaultdict(list)
        for r in rows:
            by[("level", r["level"])].append(r)
            by[("app", r["app"])].append(r)
        for (kind, key), rs in sorted(by.items()):
            lines.append(f"{kind}={key:<14} {fmt(score(rs))}")
        bad = [r for r in rows if invented(r["text"], r["target"], r["output"], r["vocab"])][:8]
        if bad:
            lines.append("\nExamples with invented words:")
            for r in bad:
                lines.append(f"  [{r['level']}/{r['app']}] in: {r['text'][:120]}\n     out: {r['output'][:120]!r}\n  target: {r['target'][:120]!r}")
    text = "\n".join(lines)
    print(text)
    (dirs[0] / "report.md").write_text(text)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--gguf", help="v3 GGUF to evaluate")
    ap.add_argument("--v2", action="store_true", help="evaluate the installed v2 model")
    ap.add_argument("--out")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--report", nargs="+", help="only print reports for these eval dirs")
    args = ap.parse_args()
    if args.report:
        report([Path(d) for d in args.report])
    else:
        run(args)


if __name__ == "__main__":
    main()
