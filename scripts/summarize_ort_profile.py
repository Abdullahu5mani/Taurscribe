#!/usr/bin/env python3
"""Summarize ONNX Runtime Chrome-trace JSON files by provider/op/node."""

from __future__ import annotations

import argparse
import collections
import json
from pathlib import Path


def ms(us: float) -> float:
    return us / 1000.0


def summarize_file(path: Path, top: int) -> dict:
    with path.open("r", encoding="utf-8") as f:
        events = json.load(f)

    node_events = [event for event in events if event.get("cat") == "Node"]
    by_provider: collections.Counter[str] = collections.Counter()
    by_op: collections.Counter[str] = collections.Counter()
    by_node: collections.Counter[str] = collections.Counter()

    for event in node_events:
        dur = float(event.get("dur") or 0.0)
        args = event.get("args") or {}
        by_provider[str(args.get("provider", "?"))] += dur
        by_op[str(args.get("op_name", "?"))] += dur
        by_node[str(event.get("name", "?"))] += dur

    total = sum(by_op.values())

    def top_rows(counter: collections.Counter[str]) -> list[dict]:
        rows = []
        for name, dur in counter.most_common(top):
            rows.append(
                {
                    "name": name,
                    "total_ms": round(ms(dur), 3),
                    "pct": round((dur / total * 100.0) if total else 0.0, 2),
                }
            )
        return rows

    return {
        "file": str(path),
        "node_events": len(node_events),
        "node_total_ms": round(ms(total), 3),
        "providers": top_rows(by_provider),
        "ops": top_rows(by_op),
        "nodes": top_rows(by_node),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("paths", nargs="+", type=Path)
    parser.add_argument("--top", type=int, default=10)
    parser.add_argument("--json-out", type=Path)
    args = parser.parse_args()

    files: list[Path] = []
    for path in args.paths:
        if path.is_dir():
            files.extend(sorted(path.glob("*.json")))
        elif any(ch in str(path) for ch in "*?["):
            files.extend(sorted(path.parent.glob(path.name)))
        else:
            files.append(path)

    summaries = [summarize_file(path, args.top) for path in files]
    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(summaries, indent=2), encoding="utf-8")

    for summary in summaries:
        print(f"\n{Path(summary['file']).name}")
        print(
            f"node_events={summary['node_events']} "
            f"node_total_ms={summary['node_total_ms']}"
        )
        for key in ("providers", "ops", "nodes"):
            print(key)
            for row in summary[key]:
                print(f"  {row['total_ms']:10.3f} ms {row['pct']:6.2f}%  {row['name']}")


if __name__ == "__main__":
    main()
