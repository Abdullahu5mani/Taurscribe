#!/usr/bin/env python3
"""Live dashboard for data generation: refreshes every few seconds.

    python status.py            # watch everything under out/
    python status.py out/ds1    # only some run dirs
"""

import collections
import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "out"
LEVELS = 3  # each accepted item becomes verbatim/clean/formatted records


def jsonl(path: Path) -> list[dict]:
    if not path.exists():
        return []
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            pass  # a line being written right now
    return rows


def run_dirs(args: list[str]) -> list[Path]:
    if args:
        return [Path(a) for a in args]
    found = {p.parent for p in OUT.rglob("items.jsonl")} | {p.parent for p in OUT.rglob("rejects.jsonl")}
    return sorted(d for d in found if "bakeoff" not in d.parts)  # trial runs aren't training data


def last_log_line(run: Path) -> str:
    # Logs sit next to run dirs as <parent>/<something>-<name>.log or <run>.log.
    candidates = list(run.parent.glob(f"*{run.name}.log")) + [run.with_suffix(".log")]
    for log in candidates:
        if log.exists():
            lines = [l for l in log.read_text(errors="replace").splitlines() if l.strip()]
            if lines:
                return lines[-1].strip()[:90]
    return ""


def live_spend(run: Path) -> float:
    """Spend of the session still running, read from its progress lines
    (usage.json is only written when a session ends)."""
    import re
    log = run.with_suffix(".log")
    if not log.exists():
        return 0.0
    lines = log.read_text(errors="replace").splitlines()
    # Progress since the last finished session ("this session" line).
    tail = []
    for line in reversed(lines):
        if "this session" in line or "judged " in line:
            break
        tail.append(line)
    for line in tail:
        m = re.search(r"ok / \d+ rejected, \$([0-9.]+)", line)
        if m:
            return float(m.group(1))
    return 0.0


TRAIN_LOG = HERE.parent / "flowscribe_train" / "train_v1c.log"
TOTAL_ITERS = 4000
RESUMED_AT = 0  # v1c counts its own 4000 steps (continues v1 from step 1200)


def training_lines() -> list[str]:
    import re
    if not TRAIN_LOG.exists():
        return []
    text = TRAIN_LOG.read_text(errors="replace").replace("\r", "\n")
    iters = re.findall(r"Iter (\d+): Train loss ([0-9.]+).*?It/sec ([0-9.]+).*?Peak mem ([0-9.]+) GB", text)
    vals = re.findall(r"Iter (\d+): Val loss ([0-9.]+)", text)
    out = ["", "Training (Qwen3.5-0.8B LoRA)"]
    if "Error" in text or "Traceback" in text:
        err = [l for l in text.splitlines() if "Error" in l][-1:]
        out.append(f"  !! STOPPED: {err[0][:100] if err else 'see train_v1.log'}")
    if iters:
        it, loss, speed, mem = iters[-1]
        it, speed = int(it) + RESUMED_AT, float(speed)
        eta_h = (TOTAL_ITERS - it) / speed / 3600 if speed else 0
        out.append(f"  step {it}/{TOTAL_ITERS} ({it / TOTAL_ITERS:.0%})  train loss {loss}  {speed} steps/s  peak {mem} GB  ~{eta_h:.1f} h left")
    else:
        out.append("  starting…")
    if vals:
        out.append("  val loss: " + "  ".join(f"@{i} {v}" for i, v in vals[-5:]))
    ckpts = sorted((HERE.parent / "flowscribe_train" / "adapters" / "v1c").glob("*_adapters.safetensors"))
    if ckpts:
        out.append(f"  last checkpoint: {ckpts[-1].name}")
    return out


def render(runs: list[Path]) -> str:
    out = [f"FlowScribe v3 data · {time.strftime('%H:%M:%S')}  (Ctrl+C to close)", ""]
    header = f"{'run':<24}{'items':>7}{'rejected':>10}{'judged ok':>11}{'records':>9}{'spent':>9}"
    out += [header, "─" * len(header)]
    tot = collections.Counter()
    reasons = collections.Counter()
    apps, phen, teachers = collections.Counter(), collections.Counter(), collections.Counter()
    for run in runs:
        items, rejects = jsonl(run / "items.jsonl"), jsonl(run / "rejects.jsonl")
        judged = jsonl(run / "judged.jsonl")
        usage = json.loads((run / "usage.json").read_text()) if (run / "usage.json").exists() else {}
        usage = {**usage, "usd": usage.get("usd", 0) + live_spend(run)}
        ok_j = sum(1 for j in judged if j.get("judge_ok"))
        usable = ok_j if judged else len(items)
        name = str(run.relative_to(OUT)) if run.is_relative_to(OUT) else str(run)
        judged_col = f"{ok_j}/{len(judged)}" if judged else "–"
        out.append(f"{name:<24}{len(items):>7}{len(rejects):>10}{judged_col:>11}{usable * LEVELS:>9}{'$' + format(usage.get('usd', 0), '.3f'):>9}")
        status = last_log_line(run)
        if status and "this session" not in status:
            out.append(f"   ↳ {status}")
        tot.update(items=len(items), rejects=len(rejects), usable=usable)
        tot["usd"] += usage.get("usd", 0)
        reasons.update(r.get("reason", "?") for r in rejects)
        reasons.update("judge: " + (j.get("judge_reason") or "failed")[:50] for j in judged if not j.get("judge_ok"))
        for it in items:
            apps[it["spec"]["app"]] += 1
            phen.update(it["spec"]["phenomena"])
            teachers[it.get("teacher", "?").split(":")[0]] += 1
    out += ["─" * len(header),
            f"{'TOTAL':<24}{tot['items']:>7}{tot['rejects']:>10}{'':>11}{tot['usable'] * LEVELS:>9}{'$' + format(tot['usd'], '.3f'):>9}",
            "",
            f"Training records so far: {tot['usable'] * LEVELS:,}  ({tot['usable']:,} usable items × {LEVELS} levels)"]
    if tot["items"] + tot["rejects"]:
        rate = tot["items"] / (tot["items"] + tot["rejects"]) * 100
        out.append(f"Accept rate: {rate:.0f}%   Cost per 1k items: ${tot['usd'] / max(tot['items'], 1) * 1000:.2f}")
    if teachers:
        out += ["", "Teachers:  " + "  ".join(f"{k} {v}" for k, v in teachers.most_common())]
    if apps:
        out.append("Apps:      " + "  ".join(f"{k} {v}" for k, v in apps.most_common()))
    if phen:
        out.append("Phenomena: " + "  ".join(f"{k} {v}" for k, v in phen.most_common()))
    if reasons:
        out += ["", "Top reject reasons:"] + [f"  {n:>4}  {r}" for r, n in reasons.most_common(5)]
    out += training_lines()
    return "\n".join(out)


def main() -> None:
    args = sys.argv[1:]
    try:
        while True:
            text = render(run_dirs(args))
            sys.stdout.write("\033[2J\033[H" + text + "\n")
            sys.stdout.flush()
            time.sleep(3)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
