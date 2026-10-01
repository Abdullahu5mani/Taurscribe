#!/usr/bin/env python3
"""Explicitly download the pinned model to a user-selected cache directory."""
import argparse
from pathlib import Path

from decider_benchmark import PINNED_REVISION


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache-dir', type=Path, required=True)
    args = parser.parse_args()
    from huggingface_hub import snapshot_download
    path = snapshot_download('Mapika/decider-2b', revision=PINNED_REVISION,
                             cache_dir=args.cache_dir, allow_patterns=['*.json', '*.safetensors', '*.jinja'],
                             ignore_patterns=['eval_results.json'])
    print(path)


if __name__ == '__main__':
    main()
