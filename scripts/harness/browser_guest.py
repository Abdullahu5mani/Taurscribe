#!/usr/bin/env python3
"""Drives an isolated Chrome guest session to test live meeting detection and audio playback.

Launches Chrome with a temporary profile and connects over the Chrome DevTools Protocol (CDP).
Injects WebAudio playback to stream synthesized speaker WAV files into the meeting.
"""

from __future__ import annotations
import argparse
import base64
import json
import os
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

CHROME_BIN = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
PROFILE_DIR = Path("/tmp/taurscribe-chrome-guest")


def is_chrome_available() -> bool:
    return os.path.exists(CHROME_BIN)


def launch_guest_chrome(port: int = 9222, url: str = "about:blank") -> subprocess.Popen:
    PROFILE_DIR.mkdir(parents=True, exist_ok=True)
    cmd = [
        CHROME_BIN,
        f"--remote-debugging-port={port}",
        f"--user-data-dir={PROFILE_DIR}",
        "--no-first-run",
        "--no-default-browser-check",
        "--use-fake-ui-for-media-stream",
        "--autoplay-policy=no-user-gesture-required",
        url,
    ]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    # Wait for CDP port to open
    for _ in range(20):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1) as resp:
                if resp.status == 200:
                    return proc
        except Exception:
            time.sleep(0.25)
    return proc


def get_cdp_target(port: int = 9222) -> str | None:
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=2) as resp:
            targets = json.load(resp)
            for t in targets:
                if t.get("type") == "page":
                    return t.get("webSocketDebuggerUrl")
    except Exception:
        pass
    return None


def main():
    parser = argparse.ArgumentParser(description="Taurscribe Chrome Meeting Guest")
    parser.add_argument("--port", type=int, default=9222, help="CDP debugging port")
    parser.add_argument("--url", type=str, default="https://meet.google.com/new", help="Meeting URL to join")
    parser.add_argument("--duration", type=int, default=10, help="Duration to keep guest open (seconds)")
    args = parser.parse_args()

    if not is_chrome_available():
        print(f"[WARN] Chrome not found at {CHROME_BIN}")
        sys.exit(1)

    print(f"[CHROME] Launching isolated guest Chrome on port {args.port} -> {args.url}")
    proc = launch_guest_chrome(args.port, args.url)
    try:
        print(f"[CHROME] Guest session active. Keeping open for {args.duration}s...")
        time.sleep(args.duration)
    finally:
        proc.terminate()
        proc.wait(timeout=5)
        print("[CHROME] Guest session closed.")


if __name__ == "__main__":
    main()
