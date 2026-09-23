#!/usr/bin/env python3
"""Upload FlowScribe v3 to Hugging Face.

Log in first yourself (this script never handles tokens):
    hf auth login

Then:
    python upload_hf.py <user>/<repo> [--private]

Uploads the F16 GGUF (what Taurscribe downloads), the merged Hugging Face
checkpoint (for further fine-tuning), the LoRA adapter, the eval report and a
model card. Prints the GGUF's SHA-256 for the app's download registry.
"""

import argparse
import hashlib
import shutil
import tempfile
from pathlib import Path

from huggingface_hub import HfApi

HERE = Path(__file__).resolve().parent
GGUF = Path("/Volumes/ExternalSSD/TaurscribeData/flowscribe/flowscribe-v3-f16.gguf")
HF_CKPT = HERE / "hf" / "v1"
ADAPTERS = HERE / "adapters" / "v1c"
REPORT = HERE / "eval" / "v3" / "report.md"
CARD = HERE / "MODEL_CARD.md"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("repo", help="e.g. Abdullahu5mani/flowscribe-qwen3.5-0.8b-v3")
    ap.add_argument("--private", action="store_true")
    args = ap.parse_args()

    api = HfApi()
    who = api.whoami()["name"]  # fails clearly if not logged in
    print(f"logged in as {who}")
    for p in (GGUF, HF_CKPT, ADAPTERS, CARD):
        if not p.exists():
            raise SystemExit(f"missing {p}")

    api.create_repo(args.repo, private=args.private, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        stage = Path(tmp)
        shutil.copy(CARD, stage / "README.md")
        if REPORT.exists():
            shutil.copy(REPORT, stage / "eval_report.md")
        (stage / "hf").mkdir()
        for f in HF_CKPT.iterdir():
            shutil.copy(f, stage / "hf" / f.name)
        (stage / "lora").mkdir()
        for f in ("adapters.safetensors", "adapter_config.json"):
            shutil.copy(ADAPTERS / f, stage / "lora" / f)
        # Large file last, uploaded straight from disk.
        api.upload_folder(repo_id=args.repo, folder_path=str(stage), commit_message="FlowScribe v3: HF checkpoint, LoRA, card")
    api.upload_file(repo_id=args.repo, path_or_fileobj=str(GGUF), path_in_repo=GGUF.name, commit_message="FlowScribe v3 F16 GGUF")
    print(f"done: https://huggingface.co/{args.repo}")
    print(f"{GGUF.name} sha256 {sha256(GGUF)}")


if __name__ == "__main__":
    main()
