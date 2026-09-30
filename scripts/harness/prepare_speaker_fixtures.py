#!/usr/bin/env python3
"""Build a neural acceptance manifest from the existing local LibriSpeech corpus.

Requires the harness Python environment, macOS afconvert and test-clean under
~/.taurscribe-harness/librispeech/LibriSpeech. Does not download data or models.
"""
import argparse
import json
from pathlib import Path
from voiceprint_check import build_clip


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    rows = []
    for speaker, chapter in [("1089", "134686"), ("1089", "134691"), ("1188", "133604")]:
        clip, _ = build_clip(speaker, chapter, 20)
        rows.append({"speaker_id": speaker, "chapter": chapter, "path": str(clip.resolve())})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(rows, indent=2) + "\n")
    print(args.output.resolve())


if __name__ == "__main__":
    main()
