"""Turn flowscribe_data build output into mlx-lm chat-format files.

The system prompt here must match what the app sends at inference time.
"""
import json
import sys
from pathlib import Path

SYSTEM = (
    "You are FlowScribe. Rewrite the dictation in <text> as the speaker meant it, "
    "following the tags. Output only the result."
)

src = Path(sys.argv[1] if len(sys.argv) > 1 else "../flowscribe_data/out/dataset-v1")
dst = Path(sys.argv[2] if len(sys.argv) > 2 else "data/v1")
dst.mkdir(parents=True, exist_ok=True)
for split, name in (("train", "train"), ("val", "valid"), ("test", "test")):
    rows = [json.loads(l) for l in (src / f"{split}.jsonl").read_text().splitlines() if l.strip()]
    with (dst / f"{name}.jsonl").open("w") as f:
        for r in rows:
            f.write(json.dumps({"messages": [
                {"role": "system", "content": SYSTEM},
                {"role": "user", "content": r["prompt"]},
                {"role": "assistant", "content": r["completion"]},
            ]}, ensure_ascii=False) + "\n")
    print(f"{name}: {len(rows)}")
