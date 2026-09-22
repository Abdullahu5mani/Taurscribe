#!/usr/bin/env python3
"""Python wrapper around bin/ax_snapshot (macOS accessibility observer/actuator).

Gives Decider the same indexed-element snapshot for native windows (the Taurscribe
WKWebView, the user's signed-in Chrome) that snapshot.js gives it for CDP pages.
The Swift binary is compiled on first use and rebuilt when its source changes.
"""

from __future__ import annotations

import json
import subprocess
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

HARNESS_DIR = Path(__file__).resolve().parent
SWIFT_SRC = HARNESS_DIR / "ax_snapshot.swift"
AX_BIN = HARNESS_DIR / "bin" / "ax_snapshot"


class AXError(RuntimeError):
    pass


def ensure_built() -> Path:
    if AX_BIN.exists() and AX_BIN.stat().st_mtime >= SWIFT_SRC.stat().st_mtime:
        return AX_BIN
    AX_BIN.parent.mkdir(exist_ok=True)
    res = subprocess.run(
        ["swiftc", "-O", str(SWIFT_SRC), "-o", str(AX_BIN)],
        capture_output=True, text=True,
    )
    if res.returncode != 0:
        raise AXError(f"Failed to compile ax_snapshot.swift:\n{res.stderr}")
    return AX_BIN


def _run(args: List[str], timeout: float = 20.0) -> str:
    res = subprocess.run([str(ensure_built())] + args, capture_output=True, text=True, timeout=timeout)
    if res.returncode != 0:
        raise AXError(res.stderr.strip() or f"ax_snapshot exited {res.returncode}")
    return res.stdout


def _window_args(window: Optional[str]) -> List[str]:
    return ["--window", window] if window else []


def snapshot(pid: int, window: Optional[str] = None) -> Dict[str, Any]:
    return json.loads(_run(["snapshot", str(pid)] + _window_args(window)))


class StaleElement(AXError):
    """The UI changed since the snapshot; re-read the screen and decide again."""


def press(pid: int, index: int, window: Optional[str] = None, expect: Optional[str] = None) -> None:
    args = ["press", str(pid), str(index)] + _window_args(window)
    if expect is not None:
        args += ["--expect", expect]
    try:
        _run(args)
    except AXError as e:
        if "expected '" in str(e) or "index out of range" in str(e):
            raise StaleElement(str(e)) from None
        raise


def set_value(pid: int, index: int, text: str, window: Optional[str] = None, expect: Optional[str] = None) -> None:
    """Types into an accessibility text field (sets its value)."""
    args = ["setvalue", str(pid), str(index), text] + _window_args(window)
    if expect is not None:
        args += ["--expect", expect]
    try:
        _run(args)
    except AXError as e:
        if "expected '" in str(e) or "index out of range" in str(e):
            raise StaleElement(str(e)) from None
        raise


def select_row(pid: int, index: int, expect: Optional[str] = None) -> None:
    """Selects the table/browser row that holds element `index`."""
    _run(["select", str(pid), str(index)] + (["--expect", expect] if expect else []))


def menu_pick(pid: int, title: str) -> None:
    """Chooses an item in the popup menu the app currently has open."""
    _run(["menupick", str(pid), title])


def scroll_to(pid: int, index: int, window: Optional[str] = None) -> None:
    """Scrolls an element into view (AXScrollToVisible)."""
    _run(["scrollto", str(pid), str(index)] + _window_args(window))


def windows(pid: int) -> List[str]:
    return json.loads(_run(["windows", str(pid)]))


def frame(pid: int, window: Optional[str] = None) -> Dict[str, int]:
    return json.loads(_run(["frame", str(pid)] + _window_args(window)))


def screenshot(pid: int, out_path: Path, window: Optional[str] = None) -> Optional[Path]:
    """Captures the window itself (by CGWindowID, so it works even when covered);
    falls back to the window's screen rectangle."""
    if window is None:
        try:
            wid = _run(["windowid", str(pid)]).strip()
            subprocess.run(["screencapture", "-x", "-o", "-l", wid, str(out_path)], capture_output=True, timeout=10)
            if out_path.exists():
                return out_path
        except Exception:
            pass
    try:
        f = frame(pid, window)
        subprocess.run(
            ["screencapture", "-x", "-R", f"{f['x']},{f['y']},{f['w']},{f['h']}", str(out_path)],
            capture_output=True, timeout=10,
        )
        return out_path if out_path.exists() else None
    except Exception:
        return None


def wait_for_window(pid: int, substring: str, timeout: float = 20.0) -> str:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            for title in windows(pid):
                if substring.lower() in title.lower():
                    return title
        except AXError:
            pass
        time.sleep(0.5)
    raise AXError(f"No window containing '{substring}' appeared for pid {pid} within {timeout}s")
