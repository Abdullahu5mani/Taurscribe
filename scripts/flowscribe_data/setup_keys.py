#!/usr/bin/env python3
"""Paste API keys into .env interactively. Input is hidden; press Enter to keep
an existing key (or skip one you don't have)."""

import getpass
import os
from pathlib import Path

ENV = Path(__file__).resolve().parent / ".env"
KEYS = [
    ("GEMINI_API_KEY", "Google AI Studio (Gemini)"),
    ("DEEPSEEK_API_KEY", "DeepSeek"),
    ("OPENAI_API_KEY", "OpenAI"),
]


def read_env() -> dict:
    values = {}
    if ENV.exists():
        for line in ENV.read_text().splitlines():
            if "=" in line and not line.lstrip().startswith("#"):
                k, v = line.split("=", 1)
                values[k.strip()] = v.strip()
    return values


def main() -> None:
    values = read_env()
    print("Paste each key and press Enter (typing is hidden).")
    print("Press Enter on its own to keep the current value or skip.\n")
    for name, label in KEYS:
        current = values.get(name, "")
        hint = f"set, ends in …{current[-4:]}" if current else "not set"
        entered = getpass.getpass(f"{label} key [{hint}]: ").strip()
        if entered:
            values[name] = entered
    ENV.write_text("".join(f"{k}={values.get(k, '')}\n" for k, _ in KEYS))
    os.chmod(ENV, 0o600)  # readable only by you
    print(f"\nSaved to {ENV}")
    for name, label in KEYS:
        print(f"  {label}: {'set' if values.get(name) else 'missing'}")


if __name__ == "__main__":
    main()
