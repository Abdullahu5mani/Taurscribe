#!/usr/bin/env python3
"""Full UI sweep of Taurscribe, driven through the macOS accessibility tree.

Every control is pressed in the real app window and the result is read back
from what the app shows (plus the control server where there is one). Checks
are soft: a section records every failure and carries on, so one broken control
does not hide the rest. Anything a section changes is put back.

    python test_ui_full.py                 # all sections
    python test_ui_full.py --section nav quick settings

Requires Taurscribe running in test mode (tauri dev).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import traceback
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

import ax_driver

HARNESS_DIR = Path(__file__).resolve().parent
BASE = "http://127.0.0.1:8766"
APP_SUPPORT = Path.home() / "Library" / "Application Support" / "Taurscribe"
SETTINGS_JSON = APP_SUPPORT / "settings.json"
SETTINGS_TABS = ["Models", "Recording", "Grammar", "Text", "App", "About"]


# ── Report ───────────────────────────────────────────────────────────────────

class UIReport:
    def __init__(self) -> None:
        self.dir = HARNESS_DIR / "reports" / f"ui-{time.strftime('%Y%m%d-%H%M%S')}"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.results: List[Dict[str, Any]] = []
        self.section = ""
        self.t0 = time.time()

    def log(self, msg: str) -> None:
        print(f"  [{time.time() - self.t0:6.1f}s] {msg}")

    def check(self, ok: bool, name: str, detail: str = "") -> bool:
        self.results.append({"section": self.section, "name": name, "ok": bool(ok), "detail": detail})
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))
        return bool(ok)

    def finish(self) -> int:
        failed = [r for r in self.results if not r["ok"]]
        (self.dir / "report.json").write_text(json.dumps(self.results, indent=2))
        print("\n" + "=" * 78)
        by_section: Dict[str, List[bool]] = {}
        for r in self.results:
            by_section.setdefault(r["section"], []).append(r["ok"])
        for sec, oks in by_section.items():
            print(f"  {sec:<22} {sum(oks):>3}/{len(oks)} passed")
        print(f"\n  TOTAL {len(self.results) - len(failed)}/{len(self.results)} passed")
        for r in failed:
            print(f"  ✗ [{r['section']}] {r['name']} — {r['detail']}")
        print(f"  Report: {self.dir}\n" + "=" * 78)
        return 1 if failed else 0


# ── App driver ───────────────────────────────────────────────────────────────

KEYPOST = HARNESS_DIR / "bin" / "keypost"


def keypost(pid: int, *keys: str) -> None:
    """Key presses delivered to Taurscribe only (never the frontmost app)."""
    src = HARNESS_DIR / "keypost.swift"
    if not KEYPOST.exists() or KEYPOST.stat().st_mtime < src.stat().st_mtime:
        subprocess.run(["swiftc", "-O", str(src), "-o", str(KEYPOST)], check=True, capture_output=True)
    subprocess.run([str(KEYPOST), str(pid), *keys], check=True, capture_output=True, timeout=20)


def api(path: str) -> Any:
    req = urllib.request.Request(BASE + path, headers={"Authorization": "Bearer " + os.environ["TAURSCRIBE_CONTROL_TOKEN"]})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read().decode())


class App:
    """The Taurscribe window as Decider sees it."""

    def __init__(self, report: UIReport) -> None:
        self.report = report
        self.pid = self._find_pid()

    @staticmethod
    def _find_pid() -> int:
        out = subprocess.run(["pgrep", "-f", "target/debug/taurscribe$"], capture_output=True, text=True).stdout.split()
        if not out:
            raise SystemExit("Taurscribe is not running")
        return int(out[0])

    def wait_ready(self, timeout: float = 90) -> None:
        """After a launch the webview publishes its accessibility tree late."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                self.pid = self._find_pid()
                if len(self.snap()["elements"]) > 10:
                    return
            except Exception:
                pass
            time.sleep(2)
        raise RuntimeError("app window never became readable")

    def snap(self) -> Dict[str, Any]:
        # The window briefly drops out of the accessibility tree while the app
        # swaps windows (e.g. the recording overlay); retry before giving up.
        for attempt in range(8):
            try:
                return ax_driver.snapshot(self.pid)
            except ax_driver.AXError:
                if attempt == 7:
                    raise
                time.sleep(0.5)
                self.pid = self._find_pid()
        raise RuntimeError("unreachable")

    def elements(self) -> List[Dict[str, Any]]:
        return self.snap()["elements"]

    def find(self, text: str | re.Pattern, role: Optional[str] = None, exact: bool = False) -> Optional[Dict[str, Any]]:
        for e in self.elements():
            t = e["text"]
            hit = (text.search(t) if isinstance(text, re.Pattern)
                   else (t == text if exact else t.startswith(text)))
            if hit and (role is None or e["role"] == role):
                return e
        return None

    def has(self, text: str | re.Pattern, role: Optional[str] = None) -> bool:
        return self.find(text, role) is not None

    def press(self, text: str | re.Pattern, role: Optional[str] = None, exact: bool = False,
              settle: float = 1.0) -> bool:
        e = self.find(text, role, exact)
        if not e:
            self.report.log(f"  (control not found: {text!r})")
            return False
        if e["text"] in ("Close", "Minimize", "Maximize") and text not in ("Close", "Minimize", "Maximize"):
            return False  # never hit the window's own buttons by accident
        try:
            ax_driver.press(self.pid, e["index"], expect=e["text"])
        except ax_driver.StaleElement:
            # Status text in the engine picker can change between AX snapshots
            # (LOADING -> READY). Re-resolve the intended control once.
            e = self.find(text, role, exact)
            if not e:
                raise
            ax_driver.press(self.pid, e["index"], expect=e["text"])
        time.sleep(settle)
        return True

    def type_into(self, field: str, value: str) -> bool:
        e = self.find(field)
        if not e:
            return False
        ax_driver.set_value(self.pid, e["index"], value, expect=e["text"])
        time.sleep(0.6)
        return True

    def choose(self, popup: str, option: str) -> bool:
        """Opens a picker and chooses one of its options."""
        e = self.find(popup, "AXPopUpButton")
        if not e:
            return False
        current = e["value"].split(" - installed", 1)[0]
        if current == option:
            return True
        ax_driver.press(self.pid, e["index"], expect=e["text"])
        time.sleep(0.8)
        # Native select menus are outside the window's accessibility tree.
        # Arrow navigation is reliable for this fixed-order tier picker;
        # type-ahead can instead dismiss the menu and trigger app shortcuts.
        if popup == "Whisper model tier":
            tiers = ["Tiny", "Base", "Small", "Medium", "Large"]
            if current not in tiers or option not in tiers:
                keypost(self.pid, "escape")
                return False
            delta = tiers.index(option) - tiers.index(current)
            keys = (["down"] if delta > 0 else ["up"]) * abs(delta) + ["return"]
        else:
            keys = [c for c in option.lower() if c.isalnum()][:6] + ["return"]
        keypost(self.pid, *keys)
        ok = self.wait(lambda: (self.find(popup, "AXPopUpButton") or {}).get("value", "").split(" - installed", 1)[0] == option, 4)
        if not ok:
            keypost(self.pid, "escape")
        return bool(ok)

    def checked(self, text: str, role: Optional[str] = None) -> Optional[bool]:
        e = self.find(text, role)
        return None if e is None else (e["value"] == "1" or e["checked"])

    def wait(self, cond: Callable[[], Any], timeout: float = 10, every: float = 0.5) -> Any:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                v = cond()
                if v:
                    return v
            except Exception:
                pass
            time.sleep(every)
        return None

    def screenshot(self, label: str) -> None:
        out = self.report.dir / f"{len(self.report.results):03d}_{label}.png"
        try:
            ax_driver.screenshot(self.pid, out)
        except Exception:
            pass

    # navigation
    def go(self, section: str) -> bool:
        label = {"mic": "Microphone dictation mode",
                 "meetings": "Meeting detection and dual-channel recording mode",
                 "files": "File transcription mode"}[section]
        self.close_settings()
        self.press(label, "AXRadioButton")
        return bool(self.wait(lambda: self.checked(label, "AXRadioButton"), 5))

    def open_settings(self, tab: str = "Models") -> bool:
        if not self.has("Close settings"):
            self.press("Open Settings", "AXButton", exact=True)
            self.wait(lambda: self.has("Close settings"), 5)
        self.press(tab, "AXRadioButton", exact=True)
        return bool(self.wait(lambda: self.checked(tab, "AXRadioButton"), 5))

    def close_settings(self) -> None:
        if self.has("Close settings"):
            self.press("Close settings", exact=True)
            self.wait(lambda: not self.has("Close settings"), 5)


FLIPPING_LABELS = {"Mute sounds": "Unmute sounds", "Mute sound effects": "Unmute sound effects"}


def toggle_roundtrip(app: App, report: UIReport, label: str, reopen: Callable[[], None]) -> None:
    """Flip a checkbox, confirm it shows flipped after reopening its screen, flip back."""
    if label in FLIPPING_LABELS:
        return flip_label_roundtrip(app, report, label, FLIPPING_LABELS[label], reopen)
    before = app.checked(label, "AXCheckBox")
    if before is None:
        report.check(False, f"'{label}' is present")
        return
    app.press(label, "AXCheckBox")
    flipped = app.wait(lambda: app.checked(label, "AXCheckBox") == (not before), 4)
    reopen()
    kept = app.checked(label, "AXCheckBox") == (not before)
    app.press(label, "AXCheckBox")
    restored = app.wait(lambda: app.checked(label, "AXCheckBox") == before, 4)
    report.check(bool(flipped) and kept and bool(restored), f"'{label}' toggles, persists, and restores",
                 f"{before} → {not before} (kept after reopen: {kept}) → {before}")


def flip_label_roundtrip(app: App, report: UIReport, off_label: str, on_label: str,
                         reopen: Callable[[], None]) -> None:
    """A toggle whose label names the action ("Mute" / "Unmute"): press it, the
    label must flip and stay flipped after reopening; press again to restore."""
    start = off_label if app.has(off_label, "AXCheckBox") else on_label
    other = on_label if start == off_label else off_label
    if not app.has(start, "AXCheckBox"):
        report.check(False, f"'{off_label}' is present")
        return
    app.press(start, "AXCheckBox", exact=True)
    flipped = app.wait(lambda: app.find(other, "AXCheckBox", exact=True), 4)
    reopen()
    kept = app.find(other, "AXCheckBox", exact=True) is not None
    app.press(other, "AXCheckBox", exact=True)
    restored = app.wait(lambda: app.find(start, "AXCheckBox", exact=True), 4)
    report.check(bool(flipped) and kept and bool(restored), f"'{off_label}' toggles, persists, and restores",
                 f"'{start}' → '{other}' (kept after reopen: {kept}) → '{start}'")


# ── Sections ─────────────────────────────────────────────────────────────────

def section_nav(app: App, report: UIReport) -> None:
    """Window chrome and the three sections."""
    for name in ("Close", "Minimize", "Maximize", "Cycle logo animation", "Open Settings"):
        report.check(app.find(name, exact=True) is not None, f"Title bar shows '{name}'")
    for sec, marker in (("meetings", "Speaker Vault"), ("files", "Browse audio files"), ("mic", "Start recording (REC)")):
        ok = app.go(sec)
        report.check(ok and app.has(marker), f"{sec.title()} section opens", f"marker '{marker}' visible: {app.has(marker)}")
        app.screenshot(f"nav_{sec}")
    # Only one section is selected at a time.
    radios = [e for e in app.elements() if e["role"] == "AXRadioButton" and e["text"].endswith("mode")]
    report.check(sum(1 for e in radios if e["value"] == "1") == 1, "Exactly one section is active",
                 str([(e["text"][:20], e["value"]) for e in radios]))
    app.press("Cycle logo animation", exact=True)
    report.check(app.has("TAURSCRIBE"), "Logo button works without breaking the header")


def section_quick(app: App, report: UIReport) -> None:
    """Quick Settings panel toggles (shown next to every section)."""
    app.go("mic")
    for label in ("Background noise reduction", "Floating transcript overlay", "Mute mic background audio", "Mute sounds"):
        toggle_roundtrip(app, report, label, reopen=lambda: (app.go("files"), app.go("mic")))
    # Tone styles need the grammar model: disabled without it.
    tones = [e for e in app.elements() if e["text"].endswith("transcription style")]
    grammar_on = app.checked("Grammar LLM post-processing", "AXCheckBox")
    report.check(len(tones) == 5, "Five tone styles are listed", f"{len(tones)} found")
    if not grammar_on:
        report.check(all(e["disabled"] for e in tones), "Tone styles are disabled while grammar is off")
    report.check(app.has("Open Dictionary settings"), "Dictionary shortcut shown")
    report.check(app.has("Open Snippets settings"), "Snippets shortcut shown")
    app.press("Open Dictionary settings")
    report.check(app.wait(lambda: app.checked("Text", "AXRadioButton"), 5), "Dictionary shortcut opens Settings → Text")
    app.close_settings()


def section_settings(app: App, report: UIReport) -> None:
    """Every Settings tab opens, and each simple control persists and restores."""
    for tab in SETTINGS_TABS:
        report.check(app.open_settings(tab), f"Settings → {tab} opens")
        app.screenshot(f"settings_{tab.lower()}")

    def reopen(tab: str) -> Callable[[], None]:
        return lambda: (app.close_settings(), app.open_settings(tab))

    app.open_settings("Recording")
    for label in ("Recording overlay HUD", "RNNoise background noise reduction", "Mute system audio during recording",
                  "Auto-detect meetings", "Auto-record detected meetings"):
        toggle_roundtrip(app, report, label, reopen("Recording"))

    # Radio pairs: pick the other option, reopen, confirm, restore.
    for tab, a, b in (("Recording", "Hold to record mode", "Click to toggle record mode"),
                      ("Recording", "Standard Microphone", "Dual-Channel (Mic + Call)"),
                      ("App", "Minimise to tray", "Quit app")):
        app.open_settings(tab)
        orig = a if app.checked(a, "AXRadioButton") else b
        other = b if orig == a else a
        app.press(other, "AXRadioButton", exact=True)
        reopen(tab)()
        kept = app.checked(other, "AXRadioButton")
        app.press(orig, "AXRadioButton", exact=True)
        restored = app.wait(lambda: app.checked(orig, "AXRadioButton"), 4)
        report.check(bool(kept) and bool(restored), f"'{a}' / '{b}' switch persists and restores",
                     f"{orig} → {other} (kept: {kept}) → {orig}")

    app.open_settings("Recording")
    mic = app.find("Microphone input device", "AXPopUpButton")
    report.check(mic is not None and bool(mic["value"]), "Microphone picker shows a device", mic["value"] if mic else "")
    report.check(app.has("Change global hotkey binding"), "Global hotkey can be changed")

    app.open_settings("App")
    toggle_roundtrip(app, report, "Mute sound effects", reopen("App"))

    # Models tab: memory retention choices and the model sections.
    app.open_settings("Models")
    for opt in ("Immediately", "5 minutes", "15 minutes", "30 minutes", "1 hour", "Never"):
        report.check(app.has(opt, "AXButton"), f"Memory retention option '{opt}'")
    for sec in ("WHISPER", "PARAKEET", "QWEN3-ASR", "POST-PROCESSING", "SPEAKER RECOGNITION", "BUILT INTO THE APP"):
        report.check(app.has(sec), f"Models tab lists {sec}")
    for gone in ("GRANITE", "Granite", "Parakeet TDT"):
        report.check(not app.has(gone), f"Models tab no longer offers {gone}")
    for pick in ("Whisper model tier", "Whisper model language", "Whisper model quantization"):
        e = app.find(pick, "AXPopUpButton")
        report.check(e is not None and bool(e["value"]), f"'{pick}' picker shows a value", e["value"] if e else "missing")
    report.check(app.has(re.compile(r"RNNoise")), "Built-in RNNoise card shown")

    # About tab.
    app.open_settings("About")
    for b in ("Open Models storage folder", "Open Recordings storage folder", "Open Settings storage folder"):
        report.check(app.has(b), f"About shows '{b}'")
    # Factory reset: open the confirmation, then cancel. Never confirm.
    app.press("Factory reset application data")
    confirm = app.wait(lambda: app.find(re.compile(r"Confirm factory reset")), 4)
    cancel = app.find("Cancel factory reset")
    report.check(bool(confirm) and cancel is not None, "Factory reset asks for confirmation first")
    if cancel:
        app.press("Cancel factory reset")
    report.check(app.wait(lambda: app.has("Factory reset application data") and not app.has(re.compile(r"Confirm factory reset")), 4),
                 "Cancelling factory reset leaves everything in place")
    app.close_settings()
    report.check(not app.has("Close settings"), "Settings closes")


def section_text(app: App, report: UIReport) -> None:
    """Custom vocabulary, dictionary and snippet entries add and remove."""
    app.open_settings("Text")
    word = "Zyqrotex"
    app.type_into("Technical term or name", word)
    add = app.find("Add custom vocabulary term")
    report.check(add is not None and not add["disabled"], "Add-term button enables once text is typed")
    app.press("Add custom vocabulary term")
    report.check(app.wait(lambda: app.has(f"Remove term {word}"), 4), f"Vocabulary term '{word}' is added")
    app.close_settings(); app.open_settings("Text")
    report.check(app.has(f"Remove term {word}"), "Vocabulary term persists after reopening")
    app.press(f"Remove term {word}")
    report.check(app.wait(lambda: not app.has(f"Remove term {word}"), 4), "Vocabulary term is removed")

    app.type_into("Sounds like word or phrase", "tore scribe")
    app.type_into("Correct spelling word or phrase", "Taurscribe")
    app.press("Add dictionary entry")
    entry = app.wait(lambda: app.find(re.compile(r"(?i)remove.*tore scribe|tore scribe.*remove|delete.*tore scribe")), 4)
    report.check(bool(entry), "Dictionary entry is added", entry["text"] if entry else "")
    if entry:
        app.press(entry["text"], exact=True)
        report.check(app.wait(lambda: not app.find(re.compile(r"(?i)tore scribe")), 4), "Dictionary entry is removed")

    app.type_into("Snippet trigger phrase", "sig block")
    app.type_into("Snippet expands to text", "Best regards, Taurscribe QA")
    app.press("Add snippet entry")
    snip = app.wait(lambda: app.find(re.compile(r"(?i)remove.*sig block|delete.*sig block")), 4)
    report.check(bool(snip), "Snippet is added", snip["text"] if snip else "")
    if snip:
        app.press(snip["text"], exact=True)
        report.check(app.wait(lambda: not app.find(re.compile(r"(?i)remove.*sig block|delete.*sig block")), 4), "Snippet is removed")
    toggle_roundtrip(app, report, "Toggle active app contextual biasing", lambda: (app.close_settings(), app.open_settings("Text")))
    app.close_settings()


HISTORY_DB = APP_SUPPORT / "transcript_history.db"


def history_rows() -> List[tuple]:
    import sqlite3
    con = sqlite3.connect(HISTORY_DB)
    try:
        return con.execute("SELECT id, transcript, engine, model_id FROM transcriptions ORDER BY id").fetchall()
    finally:
        con.close()


def engine_status(app: App) -> str:
    e = app.find("Switch engine or model", "AXPopUpButton")
    return (e["text"].split("status", 1)[-1].strip() if e else "").upper()


def installed_models(app: App, engine: str) -> List[str]:
    """Model radio labels listed under an engine in the picker."""
    if not app.has("Engine picker"):
        app.press("Switch engine or model")
    if app.has("Back to engine list"):
        app.press("Back to engine list")
    if not app.press(f"Engine {engine}"):
        return []
    labels = [e["text"] for e in app.elements() if e["role"] == "AXRadioButton" and e["text"].startswith("Select model")]
    return labels


def load_model(app: App, report: UIReport, engine: str, label: str, timeout: float = 240) -> bool:
    installed_models(app, engine)
    app.press(label, "AXRadioButton", exact=True)
    if app.has("Engine picker"):
        app.press("Switch engine or model")  # close the picker
    if "LOAD REQUIRED" in engine_status(app) or app.has("Load model"):
        app.press("Load model", exact=True)
    t0 = time.time()
    ready = app.wait(lambda: not app.has("Load model") and not re.search(r"LOAD|LOADING|WARM", engine_status(app)),
                     timeout, every=2)
    report.log(f"  {label[13:70]} → status '{engine_status(app)}' after {time.time() - t0:.0f}s")
    return bool(ready)


def frontmost_name() -> str:
    front = subprocess.run(["lsappinfo", "front"], capture_output=True, text=True).stdout.strip()
    info = subprocess.run(["lsappinfo", "info", "-only", "name", front], capture_output=True, text=True).stdout
    m = re.search(r'"LSDisplayName"="([^"]*)"', info)
    return m.group(1) if m else ""


def bring_to_front(pid: int) -> None:
    subprocess.run(["osascript", "-e",
                    f'tell application "System Events" to set frontmost of (first process whose unix id is {pid}) to true'],
                   capture_output=True)
    time.sleep(0.6)


def safe_paste_target(app: App) -> None:
    """Every dictation ends by pasting into the app in front (by design). Tests
    drive Taurscribe in the background, so put Taurscribe in front first; never
    let a test transcript land in whatever the user is working in."""
    bring_to_front(app.pid)
    if frontmost_name().lower() != "taurscribe":
        raise RuntimeError(f"refusing to finish a dictation: '{frontmost_name()}' is in front and would receive the paste")


def dictate(app: App, clip: Path) -> Optional[tuple]:
    """REC → the reader speaks into the virtual mic → REC; returns the new history row."""
    from voiceprint_check import speak_locally_path
    before = {r[0] for r in history_rows()}
    app.press("Start recording (REC)", "AXCheckBox", settle=1.5)
    proc = speak_locally_path(clip)
    proc.wait(timeout=120)
    time.sleep(1.5)
    safe_paste_target(app)
    app.press(re.compile(r"(Stop|Start) recording"), "AXCheckBox", settle=1.0)
    row = app.wait(lambda: next((r for r in history_rows() if r[0] not in before), None), 120, every=1.5)
    return row


def section_dictation(app: App, report: UIReport) -> None:
    """Each installed model: load it in the UI, dictate a real reader, check the result."""
    from recording_check import coverage, restore_default_input, use_local_mic
    from voiceprint_check import build_clip
    clip, words = build_clip("1089", "134686", 8)

    class _R:  # use_local_mic logs through a report with .log(actor, msg)
        def log(self, actor, msg): report.log(msg)
    app.go("mic")
    use_local_mic(_R())
    try:
        for engine in ("Whisper", "Granite", "Qwen3-ASR"):
            for label in installed_models(app, engine):
                name = label[len("Select model "):].split(", size")[0]
                if not load_model(app, report, engine, label):
                    report.check(False, f"{engine} · {name}: model loads", engine_status(app))
                    continue
                report.check(True, f"{engine} · {name}: model loads")
                t0 = time.time()
                row = dictate(app, clip)
                took = time.time() - t0
                if not row:
                    report.check(False, f"{engine} · {name}: dictation produces a transcript", "no history row within 120s")
                    continue
                engine_key = {"Qwen3-ASR": "qwen3"}.get(engine, engine.lower())
                report.check(engine_key in (row[2] or "").lower(), f"{engine} · {name}: saved under the selected engine",
                             f"engine={row[2]} model={row[3]}")
                cov = coverage(words, row[1])
                report.check(cov >= 0.6, f"{engine} · {name}: transcript is accurate",
                             f"{cov:.0%} of the reader's words in {took:.0f}s: \"{row[1][:90]}\"")
                shown = app.wait(lambda: app.find(re.compile(re.escape(row[1][:40]))), 10)
                report.check(bool(shown), f"{engine} · {name}: transcript shows in the feed")
                # Copy, then delete the entry from the UI.
                subprocess.run(["pbcopy"], input=b"", check=False)
                copy_btn = next((e for e in app.elements() if e["text"] == "Copy transcript to clipboard"), None)
                if copy_btn:
                    ax_driver.press(app.pid, copy_btn["index"])
                    time.sleep(0.8)
                    clip_text = subprocess.run(["pbpaste"], capture_output=True, text=True).stdout
                    report.check(clip_text.strip()[:30] == row[1].strip()[:30], f"{engine} · {name}: Copy puts the transcript on the clipboard",
                                 clip_text[:60])
                delete = next((e for e in app.elements() if e["text"].startswith("Delete transcript from")), None)
                if delete:
                    ax_driver.press(app.pid, delete["index"], expect=delete["text"])
                    gone = app.wait(lambda: row[0] not in {r[0] for r in history_rows()}, 6)
                    if not gone and app.find(re.compile(r"(?i)confirm")):
                        app.press(re.compile(r"(?i)confirm"))
                        gone = app.wait(lambda: row[0] not in {r[0] for r in history_rows()}, 6)
                    report.check(bool(gone), f"{engine} · {name}: Delete removes the entry")
                app.screenshot(f"dictation_{engine}")
    finally:
        restore_default_input()
        if app.has("Engine picker"):
            app.press("Switch engine or model")


MODELS_DIR = APP_SUPPORT / "models"


def card_elements(app: App, card: str) -> List[Dict[str, Any]]:
    """Elements belonging to one model card (from its label to the next card)."""
    els = app.elements()
    start = next((i for i, e in enumerate(els) if e["text"] == f"Model {card}"), None)
    if start is None:
        return []
    out = []
    for e in els[start + 1:]:
        if e["text"].startswith("Model ") and e["role"] == "AXGroup":
            break
        out.append(e)
    return out


def card_state(app: App, card: str) -> str:
    texts = [e["text"] for e in card_elements(app, card)]
    joined = " | ".join(texts)
    if any(t.endswith("is Verified") for t in texts):
        return "verified"
    if re.search(r"Verifying", joined):
        return "verifying"
    if re.search(r"Downloading|Starting download|Finalizing|Extracting|Cancel download of", joined):
        return "downloading"
    if any(t.startswith("Download ") for t in texts):
        return "not-downloaded"
    return "unknown: " + joined[:120]


def card_percent(app: App, card: str) -> Optional[int]:
    for e in card_elements(app, card):
        if e["role"] == "AXProgressIndicator" and e["value"]:
            try:
                return int(float(e["value"]))
            except ValueError:
                pass
        m = re.fullmatch(r"(\d+)%", e["text"])
        if m:
            return int(m.group(1))
    return None


def press_in_card(app: App, card: str, prefix: str) -> bool:
    e = next((e for e in card_elements(app, card) if e["text"].startswith(prefix)), None)
    if not e:
        return False
    ax_driver.press(app.pid, e["index"], expect=e["text"])
    time.sleep(1.0)
    return True


def download_model(app: App, report: UIReport, card: str, timeout: float = 3600) -> bool:
    app.open_settings("Models")
    state = card_state(app, card)
    if state == "verified":
        report.check(True, f"{card}: already downloaded and verified")
        return True
    if not press_in_card(app, card, "Download "):
        report.check(False, f"{card}: download button present", state)
        return False
    t0, last_pct, saw_progress, last_log = time.time(), -1, False, 0.0
    while time.time() - t0 < timeout:
        state = card_state(app, card)
        pct = card_percent(app, card)
        if state in ("downloading", "verifying"):
            saw_progress = True
        if pct is not None and pct != last_pct and time.time() - last_log > 30:
            report.log(f"  {card}: {state} {pct}% ({time.time() - t0:.0f}s)")
            last_pct, last_log = pct, time.time()
        if state == "verified":
            break
        if state == "not-downloaded" and time.time() - t0 > 10:
            break  # fell back: failed
        errs = [e["text"] for e in app.elements() if re.search(r"(?i)download failed|error", e["text"])]
        if errs and time.time() - t0 > 5:
            report.log(f"  {card}: error shown: {errs[:2]}")
        time.sleep(3)
    took = time.time() - t0
    report.check(saw_progress, f"{card}: shows download progress")
    return report.check(state == "verified", f"{card}: downloads and verifies", f"{state} after {took:.0f}s")


def section_downloads(app: App, report: UIReport) -> None:
    """Download through the Models tab: two Whisper sizes, cancel mid-way, then every other model."""
    app.open_settings("Models")
    # Whisper Base (English, Q5_1) + its Neural Engine encoder, then Small (English, Q5_1).
    app.choose("Whisper model language", "English")
    app.choose("Whisper model quantization", "Quantized")
    for tier, card in (("Base", "Whisper Base (English, Q5_1)"), ("Small", "Whisper Small (English, Q5_1)")):
        report.check(app.choose("Whisper model tier", tier), f"Whisper size picker switches to {tier}")
        report.check(app.wait(lambda: app.has(f"Model {card}"), 5) is not None and app.has(f"Model {card}"),
                     f"Models tab shows the {card} card")
        download_model(app, report, card)
        if tier == "Base":
            enc = next((e["text"] for e in app.elements() if e["role"] == "AXGroup" and "CoreML" in e["text"]), None)
            report.log(f"  matching encoder card: {enc}")
            if enc:
                download_model(app, report, enc[len("Model "):])

    # Cancel part-way: partial files must be removed and the card must reset.
    card = "Parakeet Nemotron Streaming (INT4)"
    folder = MODELS_DIR / "parakeet-nemotron"
    if card_state(app, card) != "verified":
        press_in_card(app, card, "Download ")
        grew = app.wait(lambda: (card_percent(app, card) or 0) >= 3, 180, every=2)
        report.check(bool(grew), f"{card}: download starts", f"{card_percent(app, card)}%")
        report.check(press_in_card(app, card, "Cancel download of"), f"{card}: cancel button works")
        reset = app.wait(lambda: card_state(app, card) == "not-downloaded", 30, every=1)
        report.check(bool(reset), f"{card}: card returns to 'Download' after cancel", card_state(app, card))
        leftovers = [str(p.relative_to(MODELS_DIR)) for p in folder.rglob("*") if p.is_file()] if folder.exists() else []
        partials = [p for p in MODELS_DIR.rglob("*.part")] + [p for p in MODELS_DIR.rglob("*.partial")] + [p for p in MODELS_DIR.rglob("*.download")]
        report.check(not leftovers and not partials, f"{card}: partial files removed after cancel",
                     f"left: {leftovers[:5] + [str(p) for p in partials[:5]]}")

    for card in ("Parakeet Nemotron Streaming (INT4)", "Parakeet Nemotron Streaming (Apple Silicon MLX)",
                 "Qwen3 Apple Silicon (MLX)", "LLM FlowScribe Qwen 2.5 0.5B V2"):
        download_model(app, report, card)
    report.check(not app.has("Model Qwen3 Universal (ONNX)"),
                 "Unpublished Qwen3 ONNX model is not offered for download")
    app.close_settings()


def delete_model(app: App, report: UIReport, card: str, folder: Optional[Path] = None) -> bool:
    app.open_settings("Models")
    if not press_in_card(app, card, "Delete "):
        return report.check(False, f"{card}: delete button present", card_state(app, card))
    confirm = app.wait(lambda: next((e for e in card_elements(app, card) if e["text"].startswith("Confirm delete")), None), 5)
    report.check(bool(confirm), f"{card}: delete asks for confirmation")
    if confirm:
        press_in_card(app, card, "Confirm delete")
    gone = app.wait(lambda: card_state(app, card) == "not-downloaded", 60, every=1)
    report.check(bool(gone), f"{card}: card returns to 'Download' after deleting", card_state(app, card))
    if folder is not None:
        report.check(not folder.exists() or not any(folder.rglob("*")), f"{card}: model files removed from disk", str(folder))
    return bool(gone)


def section_download_cancel(app: App, report: UIReport) -> None:
    """Delete a model, start it again, cancel at once, then download it for real."""
    card, folder = "Parakeet Nemotron Streaming (INT4)", MODELS_DIR / "parakeet-nemotron"
    if card_state_open(app, card) == "verified":
        delete_model(app, report, card, folder)
    press_in_card(app, card, "Download ")
    cancel = app.wait(lambda: next((e for e in card_elements(app, card) if e["text"].startswith("Cancel download of")), None),
                      20, every=0.2)
    report.check(bool(cancel), f"{card}: shows a cancel button while downloading")
    if cancel:
        ax_driver.press(app.pid, cancel["index"], expect=cancel["text"])
    reset = app.wait(lambda: card_state(app, card) == "not-downloaded", 30, every=0.5)
    report.check(bool(reset), f"{card}: card returns to 'Download' after cancel", card_state(app, card))
    time.sleep(2)
    left = [str(p.relative_to(MODELS_DIR)) for p in folder.rglob("*") if p.is_file()] if folder.exists() else []
    staging = [p.name for p in MODELS_DIR.iterdir()
               if p.name.startswith("parakeet-nemotron") and p.name not in ("parakeet-nemotron", "parakeet-nemotron-mlx")]
    report.check(not left and not staging, f"{card}: no partial files left after cancel", f"{left[:5]} {staging}")
    download_model(app, report, card)
    app.close_settings()


def card_state_open(app: App, card: str) -> str:
    app.open_settings("Models")
    return card_state(app, card)


PANEL_SERVICE = "openAndSavePanelService"
LIBRI = Path.home() / ".taurscribe-harness" / "librispeech" / "LibriSpeech" / "test-clean"


def panel_pid() -> Optional[int]:
    out = subprocess.run(["pgrep", "-f", "-i", PANEL_SERVICE], capture_output=True, text=True).stdout.split()
    return int(out[-1]) if out else None


FILE_FIXTURES = Path("/Volumes/ExternalSSD/TaurscribeData/file-fixtures")
FIXTURE_TRAIL = ("ExternalSSD", "TaurscribeData", "file-fixtures")  # sidebar item, then folders


def open_via_dialog(app: App, report: UIReport, path: Path, browse_label: str) -> bool:
    """The real macOS open panel, driven through accessibility only (no keys, no
    focus change): sidebar → folders → file → Open. The file must sit in
    FILE_FIXTURES (the panel hides dot-folders)."""
    if not app.press(browse_label, "AXButton", settle=1.5):
        return False
    if not app.wait(lambda: app.find("Open", "AXButton", exact=True), 8):
        return report.check(False, "Open-file dialog appears")

    def pick(name: str, role: Optional[str]) -> bool:
        def attempt():
            for e in app.elements():
                if (e["value"] == name or e["text"] == name) and (role is None or e["role"] == role):
                    ax_driver.select_row(app.pid, e["index"])
                    return True
            return False
        ok = app.wait(attempt, 6)
        time.sleep(1.0)
        return bool(ok)

    ok = pick(FIXTURE_TRAIL[0], "AXStaticText")
    for folder in FIXTURE_TRAIL[1:]:
        ok = ok and pick(folder, "AXTextField")
    ok = ok and pick(path.name, "AXTextField")
    op = app.find("Open", "AXButton", exact=True)
    if ok and op and not op["disabled"]:
        ax_driver.press(app.pid, op["index"])
    else:
        app.press("Cancel", "AXButton", exact=True)
        return report.check(False, f"Dialog selects {path.name}")
    return bool(app.wait(lambda: app.find(f"Remove {path.name}"), 10))


def file_done(app: App, name: str) -> bool:
    return app.find(f"Show transcript for {name}") is not None or app.find(f"Copy transcript for {name}") is not None


def section_files(app: App, report: UIReport) -> None:
    """File transcription through the real open dialog: formats, queue, cancel, actions."""
    from recording_check import coverage
    from voiceprint_check import build_clip
    work = HARNESS_DIR / "reports" / "file_fixtures"
    work.mkdir(parents=True, exist_ok=True)
    wav, wav_words = build_clip("1089", "134691", 20)
    flac = LIBRI / "1089" / "134691" / "1089-134691-0000.flac"
    flac_words = next(l.split(" ", 1)[1] for l in (flac.parent / "1089-134691.trans.txt").read_text().splitlines()
                      if l.startswith(flac.stem)).lower()
    m4a = work / "reader_1188.m4a"
    src_1188, m4a_words = build_clip("1188", "133604", 20)
    subprocess.run(["afconvert", "-f", "m4af", "-d", "aac", str(src_1188), str(m4a)], check=True, capture_output=True)
    long_wav = work / "long_60s.wav"
    long_src, _ = build_clip("121", "127105", 60)
    long_wav.write_bytes(long_src.read_bytes())
    FILE_FIXTURES.mkdir(parents=True, exist_ok=True)
    import shutil as _sh
    staged = {}
    for f in (wav, flac, m4a, long_wav):
        dst = FILE_FIXTURES / f.name
        dst.unlink(missing_ok=True)  # LibriSpeech sources are read-only, and so are old copies
        _sh.copy(f, dst)
        staged[f] = dst
    wav, flac, m4a, long_wav = staged[wav], staged[flac], staged[m4a], staged[long_wav]

    app.go("files")
    for f, words, kind in ((wav, wav_words, "WAV"), (flac, flac_words, "FLAC"), (m4a, m4a_words, "M4A")):
        browse = "Browse more audio files" if app.has("Browse more audio files") else "Browse audio files"
        report.check(open_via_dialog(app, report, f, browse), f"{kind}: file opens through the dialog", f.name)
        done = app.wait(lambda: file_done(app, f.name), 240, every=2)
        report.check(bool(done), f"{kind}: transcription finishes")
        if not done:
            continue
        app.press(f"Show transcript for {f.name}")
        text = " ".join(e["text"] for e in app.elements() if e["role"] in ("AXStaticText", "AXTextArea") and len(e["text"]) > 40)
        cov = coverage(words, text)
        report.check(cov >= 0.6, f"{kind}: transcript is accurate", f"{cov:.0%} of the reader's words")
        subprocess.run(["pbcopy"], input=b"", check=False)
        app.press(f"Copy transcript for {f.name}", settle=0.8)
        clip = subprocess.run(["pbpaste"], capture_output=True, text=True).stdout
        report.check(coverage(words, clip) >= 0.6, f"{kind}: Copy puts the transcript on the clipboard", clip[:60])
    app.screenshot("files_done")

    # Re-transcribe the WAV.
    rt = app.find(f"Re-transcribe {wav.name}")
    if rt:
        ax_driver.press(app.pid, rt["index"], expect=rt["text"])
        busy = app.wait(lambda: app.find(re.compile(r"Transcribing|Cancel transcription for " + re.escape(wav.name))), 10, every=0.3)
        again = app.wait(lambda: file_done(app, wav.name), 240, every=2)
        report.check(bool(again), "Re-transcribe runs again and finishes", f"saw busy state: {bool(busy)}")

    # Queue while busy, then cancel the long one.
    report.check(open_via_dialog(app, report, long_wav, "Browse more audio files"), "Long file opens")
    cancel = app.wait(lambda: app.find(f"Cancel transcription for {long_wav.name}"), 20, every=0.5)
    report.check(bool(cancel), "A running transcription can be cancelled (button shown)")
    if cancel:
        ax_driver.press(app.pid, cancel["index"], expect=cancel["text"])
        stopped = app.wait(lambda: not app.find(f"Cancel transcription for {long_wav.name}"), 20)
        report.check(bool(stopped) and not file_done(app, long_wav.name), "Cancel stops the transcription",
                     "finished anyway" if file_done(app, long_wav.name) else "")

    # Remove every entry.
    for name in (wav.name, flac.name, m4a.name, long_wav.name):
        if app.find(f"Remove {name}"):
            app.press(f"Remove {name}")
            report.check(app.wait(lambda: not app.find(f"Remove {name}"), 5) is not None and not app.find(f"Remove {name}"),
                         f"Remove clears {name}")


REVIEW_PERSON = "Peter Bobbe (UI test)"


def stereo_call(you: Path, caller: Path, out: Path) -> Path:
    """Two-channel call: you on the left, then the caller on the right."""
    import struct, wave as _w
    def rd(p):
        w = _w.open(str(p)); n = w.getnframes()
        return list(struct.unpack(f"<{n}h", w.readframes(n))), w.getframerate()
    a, r = rd(you); b, _ = rd(caller)
    L = [0] * (r // 2) + a + [0] * len(b)
    R = [0] * (r // 2) + [0] * len(a) + b
    inter = [v for pair in zip(L, R) for v in pair]
    w = _w.open(str(out), "wb"); w.setnchannels(2); w.setsampwidth(2); w.setframerate(r)
    w.writeframes(struct.pack(f"<{len(inter)}h", *inter)); w.close()
    return out.resolve()


def review_modal_checks(app: App, report: UIReport, meeting_id: int) -> None:
    """The review modal that opens after a call is processed."""
    shown = app.wait(lambda: app.has("WHO WAS ON THIS CALL?"), 15)
    report.check(bool(shown), "Review modal opens after a call is processed")
    if not shown:
        return
    report.check(not app.has("Review call recording and finalize discussion points"),
                 "No made-up action item is shown as 'detected'")
    app.press("Planning", "AXButton", exact=True, settle=0.6)
    app.type_into("Speaker name", REVIEW_PERSON)
    report.check(app.press("▶ 3s Clip", settle=0.8), "Review modal plays the caller's voice clip")
    app.press("Save & View Notes", "AXButton", settle=2.5)
    report.check(app.wait(lambda: not app.has("WHO WAS ON THIS CALL?"), 6) is not None, "Save & View Notes closes the modal")
    d = api(f"/api/meetings/{meeting_id}")
    callers = {t["speaker_name"] for t in d["turns"] if t["channel"] == 1}
    report.check(callers == {REVIEW_PERSON}, "Name given in the review modal is saved on the caller's turns", str(callers))
    report.check((d.get("category") or "").lower() == "planning", "Category chosen in the review modal is saved", str(d.get("category")))


def list_titles(app: App) -> List[str]:
    return [e["text"] for e in app.elements() if e["role"] == "AXHeading" and e["text"].startswith("UI test:")]


def section_meetings(app: App, report: UIReport) -> None:
    """Meetings area: list, platform filters, search, detail, playback, speakers, vault, copy, export, delete.
    Uses two meetings made through the app's own pipeline ("UI test: …")."""
    import sqlite3
    from voiceprint_check import build_clip, post
    app.go("meetings")
    # Two calls through the app's own pipeline: you (reader 121) then a caller.
    W = HARNESS_DIR / "reports" / "meeting_fixtures"
    W.mkdir(parents=True, exist_ok=True)
    you, yt = build_clip("121", "121726", 8)
    made = {}
    for key, (spk, chap, title, plat) in {"m0": ("1089", "134686", "UI test: Roadmap sync", "meet"),
                                           "m1": ("1188", "133604", "UI test: Design review", "zoom")}.items():
        caller, ct = build_clip(spk, chap, 20)
        wav = stereo_call(you, caller, W / f"{key}.wav")
        res = post("/api/simulate/feed-meeting", {"wav_path": str(wav), "transcript": yt + " " + ct, "title": title, "platform": plat})
        made[title] = res["data"]["meeting_id"]
        if key == "m0":
            review_modal_checks(app, report, made[title])
    report.log(f"  test meetings: {made}")
    titles = list_titles(app)
    report.check(set(titles) >= {"UI test: Roadmap sync", "UI test: Design review"}, "Both test meetings are listed", str(titles))
    chips = [e["text"] for e in app.elements() if e["role"] == "AXButton" and re.fullmatch(r"(All|Google Meet|Zoom|Microsoft Teams|Teams|meet|zoom|teams) \d+", e["text"])]
    report.check("Google Meet 1" in chips and "Zoom 1" in chips and not any(c.split()[0].islower() for c in chips),
                 "Platform filters use proper names, no duplicates", str(chips))

    app.press("Google Meet 1", "AXButton", exact=True, settle=1.2)
    report.check(list_titles(app) == ["UI test: Roadmap sync"], "Google Meet filter shows only the Meet call", str(list_titles(app)))
    app.press("Zoom 1", "AXButton", exact=True, settle=1.2)
    report.check(list_titles(app) == ["UI test: Design review"], "Zoom filter shows only the Zoom call", str(list_titles(app)))
    app.press(re.compile(r"^All \d+$"), "AXButton", settle=1.2)

    app.type_into("Search meetings...", "Roadmap")
    time.sleep(1.2)
    report.check(list_titles(app) == ["UI test: Roadmap sync"], "Search finds a meeting by title", str(list_titles(app)))
    app.type_into("Search meetings...", "titian holbein")
    time.sleep(1.2)
    report.check(list_titles(app) == ["UI test: Design review"], "Search finds a meeting by spoken words", str(list_titles(app)))
    if app.has("Clear search"):
        app.press("Clear search", settle=1.2)
    else:
        app.type_into("Search meetings...", "")
        time.sleep(1.2)
    report.check(len(list_titles(app)) >= 2, "Clearing the search shows every meeting again")

    # Open a meeting.
    app.press("UI test: Roadmap sync", "AXHeading", exact=True, settle=1.2)
    title_field = app.find("Click to rename meeting title")
    report.check(bool(title_field) and title_field["value"] == "UI test: Roadmap sync", "Selecting a meeting opens it",
                 title_field["value"] if title_field else "")
    texts = [e["text"] for e in app.elements()] + [e["value"] for e in app.elements()]
    report.check("You" in texts and any(t in texts for t in ("Remote Participant", REVIEW_PERSON)), "Transcript shows both speakers")
    report.check(any("stew for dinner" in t for t in texts), "Transcript shows the caller's words")

    # Playback: play → clock moves → pause → speed.
    def clock():
        e = next((e for e in app.elements() if re.fullmatch(r"\d+:\d\d", e["text"])), None)
        return e["text"] if e else None
    app.press("Play Recording", settle=0.3)
    moved = app.wait(lambda: clock() not in (None, "0:00") and clock(), 6)
    report.check(bool(moved), "Play starts playback (clock moves)", f"clock {clock()}")
    pause = app.find(re.compile(r"(?i)^pause"))
    report.check(pause is not None, "Pause control appears while playing")
    if pause:
        ax_driver.press(app.pid, pause["index"])
        time.sleep(0.8)
        c1 = clock(); time.sleep(1.5); c2 = clock()
        report.check(c1 == c2, "Pause stops the clock", f"{c1} → {c2}")
    speed = app.find(re.compile(r"^\d(\.\d+)?x$"), "AXButton")
    if speed:
        before = speed["text"]
        ax_driver.press(app.pid, speed["index"]); time.sleep(0.6)
        after = (app.find(re.compile(r"^\d(\.\d+)?x$"), "AXButton") or {}).get("text")
        report.check(after and after != before, "Speed button changes playback speed", f"{before} → {after}")
        while after and after != before:
            e = app.find(re.compile(r"^\d(\.\d+)?x$"), "AXButton")
            ax_driver.press(app.pid, e["index"]); time.sleep(0.4)
            after = (app.find(re.compile(r"^\d(\.\d+)?x$"), "AXButton") or {}).get("text")

    # Speaker clip and alternate samples.
    report.check(app.press("3s Clip", settle=1.0), "Speaker's 3 s voice clip plays")
    s1 = (app.find(re.compile(r"^Sample ?\d/\d")) or {}).get("text")
    app.press(re.compile(r"^Sample ?\d/\d"), settle=1.2)
    s2 = (app.find(re.compile(r"^Sample ?\d/\d")) or {}).get("text")
    report.check(bool(s1) and s1 != s2, "'Sample' cycles to another voice sample", f"{s1} → {s2}")

    # Inline rename on the other call, then the vault.
    person = "Duncan Murrell (UI test)"
    app.press("UI test: Design review", "AXHeading", exact=True, settle=1.2)
    report.check(not app.find("Speaker name (Enter", "AXTextField"), "No rename field left open after switching meetings")
    app.press("Rename speaker", settle=1.0)
    app.type_into("Speaker name", person)
    app.press("Rename speaker", settle=2.0)
    report.check(app.wait(lambda: app.has(person), 8) is not None and app.has(person), "Renaming a speaker updates the transcript")
    app.press("Speaker Vault", settle=1.5)
    report.check(app.wait(lambda: app.has(person) and app.has(REVIEW_PERSON), 6) is not None,
                 "Both named callers appear in the Speaker Vault", f"{app.has(person)} / {app.has(REVIEW_PERSON)}")
    app.screenshot("speaker_vault")
    app.press("Close modal", settle=1.0)

    # Copy and export.
    subprocess.run(["pbcopy"], input=b"", check=False)
    app.press("UI test: Roadmap sync", "AXHeading", exact=True, settle=1.2)
    app.press("Copy Transcript", "AXButton", settle=0.8)
    clip = subprocess.run(["pbpaste"], capture_output=True, text=True).stdout
    report.check("stew for dinner" in clip and "contrivance" in clip, "Copy Transcript copies both speakers", clip[:80])
    downloads = Path.home() / "Downloads"
    before = set(downloads.iterdir())
    app.press("Export", "AXButton", exact=True, settle=3.0)
    new = [p for p in downloads.iterdir() if p not in before]
    report.check(bool(new), "Export saves a transcript file", str([p.name for p in new]) or "no file appeared in ~/Downloads")
    for f in new:
        if "Roadmap" in f.name:
            f.unlink()

    # Delete both test meetings through the UI.
    db = APP_SUPPORT / "transcript_history.db"
    for title in ("UI test: Roadmap sync", "UI test: Design review"):
        app.press(title, "AXHeading", exact=True, settle=1.0)
        con = sqlite3.connect(db)
        audio = [r[0] for r in con.execute("SELECT audio_path FROM meetings WHERE title = ?", (title,))]
        con.close()
        app.press("Delete this meeting recording", settle=1.0)
        confirm = app.find(re.compile(r"(?i)^(confirm|delete)(?!.*this meeting recording)"), "AXButton")
        if confirm:
            ax_driver.press(app.pid, confirm["index"]); time.sleep(1.2)
        gone = app.wait(lambda: title not in list_titles(app), 6)
        report.check(bool(gone), f"Delete removes '{title}' from the list")
        report.check(all(not Path(a).exists() for a in audio if a), f"Delete removes '{title}' audio from disk", str(audio))
    # The vault person this test created.
    con = sqlite3.connect(db)
    for name in (person, REVIEW_PERSON):
        con.execute("DELETE FROM speaker_vault WHERE name = ?", (name,))
    con.commit(); con.close()


def section_grammar(app: App, report: UIReport) -> None:
    """Grammar LLM (FlowScribe) post-processing: enable it, tones unlock, dictation is cleaned up."""
    import sqlite3
    from recording_check import coverage, restore_default_input, use_local_mic
    from voiceprint_check import build_clip
    clip, words = build_clip("1089", "134686", 8)
    app.go("mic")
    was_on = app.checked("Grammar LLM post-processing", "AXCheckBox")
    if not was_on:
        app.press("Grammar LLM post-processing", "AXCheckBox", settle=3)
    on = app.wait(lambda: app.checked("Grammar LLM post-processing", "AXCheckBox"), 30, every=1)
    report.check(bool(on), "Grammar LLM turns on (model installed)")
    unlocked = app.wait(lambda: (lambda t: t and not any(e["disabled"] for e in t))(
        [e for e in app.elements() if e["text"].endswith("transcription style")]), 10)
    report.check(bool(unlocked), "Tone styles unlock when grammar is on")

    class _R:
        def log(self, actor, msg): report.log(msg)
    use_local_mic(_R())
    try:
        # A fast engine for the raw transcript.
        wl = [l for l in installed_models(app, "Whisper") if "Tiny" in l]
        if wl:
            load_model(app, report, "Whisper", wl[0])
        for tone in ("Professional", "Casual"):
            app.press(f"{tone} transcription style", "AXRadioButton", settle=1)
            row = dictate(app, clip)
            if not row:
                report.check(False, f"{tone}: dictation with grammar produces text")
                continue
            con = sqlite3.connect(HISTORY_DB)
            used = con.execute("SELECT grammar_llm_used FROM transcriptions WHERE id = ?", (row[0],)).fetchone()[0]
            con.close()
            report.check(used == 1, f"{tone}: transcript went through the grammar model", f"grammar_llm_used={used}")
            cov = coverage(words, row[1])
            report.check(cov >= 0.6 and row[1][:1].isupper() and row[1].rstrip()[-1:] in ".!?",
                         f"{tone}: output keeps the words and is cleaned up",
                         f"{cov:.0%} words: \"{row[1][:100]}\"")
            d = next((e for e in app.elements() if e["text"].startswith("Delete transcript from")), None)
            if d:
                ax_driver.press(app.pid, d["index"]); time.sleep(1)
    finally:
        restore_default_input()
        if not was_on and app.checked("Grammar LLM post-processing", "AXCheckBox"):
            app.press("Grammar LLM post-processing", "AXCheckBox", settle=1)


def textedit_text() -> str:
    r = subprocess.run(["osascript", "-e", 'tell application "TextEdit" to get text of document 1'],
                       capture_output=True, text=True)
    return r.stdout.strip()


def section_hotkey(app: App, report: UIReport) -> None:
    """Global hold-to-record hotkey (Left Ctrl + Left Option), and the result pasted
    into the app in front (a blank TextEdit document)."""
    import sqlite3
    from recording_check import coverage, restore_default_input, use_local_mic
    from voiceprint_check import build_clip, speak_locally_path
    binding = json.loads(SETTINGS_JSON.read_text()).get("hotkey_binding", {})
    report.check(binding.get("keys") == ["ControlLeft", "AltLeft"] and binding.get("mode") == "hold",
                 "Hotkey is hold Left Ctrl + Left Option", str(binding))
    clip, words = build_clip("1089", "134686", 8)
    app.go("mic")
    wl = [l for l in installed_models(app, "Whisper") if "Tiny" in l]
    if wl:
        load_model(app, report, "Whisper", wl[0])
    if app.has("Engine picker"):
        app.press("Switch engine or model")
    was_running = bool(subprocess.run(["pgrep", "-x", "TextEdit"], capture_output=True).stdout)
    subprocess.run(["osascript", "-e", 'tell application "TextEdit" to activate',
                    "-e", 'tell application "TextEdit" to make new document'], capture_output=True)
    time.sleep(1.5)

    class _R:
        def log(self, actor, msg): report.log(msg)
    use_local_mic(_R())
    before = {r[0] for r in history_rows()}
    try:
        hold = subprocess.Popen([str(HARNESS_DIR / "bin" / "hotkeyhold"), str(clip_len(clip) + 2.0), "ctrl", "alt"],
                                stdout=subprocess.PIPE, text=True)
        hold.stdout.readline()  # keys are down
        recording = app.wait(lambda: api("/api/status")["recording"]["is_recording"], 3, every=0.2)
        report.check(bool(recording), "Holding the hotkey starts recording (app in the background)")
        speak_locally_path(clip).wait(timeout=60)
        if frontmost_name() != "TextEdit":
            subprocess.run(["osascript", "-e", 'tell application "TextEdit" to activate'], capture_output=True)
            time.sleep(0.5)
        hold.wait(timeout=30)
        row = app.wait(lambda: next((r for r in history_rows() if r[0] not in before), None), 60, every=1)
        report.check(bool(row), "Releasing the hotkey transcribes", row[1][:80] if row else "no transcript")
        pasted = app.wait(lambda: textedit_text() or None, 10)
        report.check(bool(pasted) and coverage(words, pasted) >= 0.6,
                     "The transcript is pasted into the app in front (TextEdit)", (pasted or "nothing pasted")[:90])
        if row:
            con = sqlite3.connect(HISTORY_DB); con.execute("DELETE FROM transcriptions WHERE id = ?", (row[0],)); con.commit(); con.close()
    finally:
        restore_default_input()
        subprocess.run(["osascript", "-e", 'tell application "TextEdit" to close every document saving no'], capture_output=True)
        if not was_running:
            subprocess.run(["osascript", "-e", 'tell application "TextEdit" to quit'], capture_output=True)


def clip_len(path: Path) -> float:
    import wave as _w
    w = _w.open(str(path)); n = w.getnframes() / w.getframerate(); w.close()
    return n


AX_BIN = HARNESS_DIR / "bin" / "ax_snapshot"


def tray(pid: int, press: Optional[str] = None) -> Any:
    out = subprocess.run([str(AX_BIN), "tray", str(pid)] + ([press] if press else []),
                         capture_output=True, text=True, timeout=20)
    if out.returncode != 0:
        raise ax_driver.AXError(out.stderr.strip())
    return json.loads(out.stdout) if not press else True


def app_windows(pid: int) -> List[Dict[str, Any]]:
    out = subprocess.run([str(AX_BIN), "cgwindows", str(pid)], capture_output=True, text=True, timeout=20)
    return json.loads(out.stdout or "[]")


def section_tray(app: App, report: UIReport) -> None:
    """Tray menu, hide/show via tray, and the recording overlay."""
    app.go("mic")
    items = tray(app.pid)
    report.check(items[-1] == "Exit" and "" not in [i.replace(" (disabled)", "") for i in items[:-1] if i != " (disabled)"],
                 "Tray menu ends with Exit (no stray entry)", str(items))
    report.check(items[0] == "Show Taurscribe", "Tray menu offers Show Taurscribe first", str(items))
    model_item = items[1]
    if model_item == "Unload Model":
        tray(app.pid, "Unload Model"); time.sleep(2)
        report.check(app.wait(lambda: "LOAD REQUIRED" in engine_status(app), 10) is not None and "LOAD REQUIRED" in engine_status(app),
                     "Tray 'Unload Model' unloads the model", engine_status(app))
        items = app.wait(lambda: (lambda i: i if "Load Model" in i else None)(tray(app.pid)), 8) or tray(app.pid)
        report.check("Load Model" in items, "Tray then offers 'Load Model'", str(items))
    if "Load Model" in tray(app.pid):
        tray(app.pid, "Load Model")
        ok = app.wait(lambda: "LOAD REQUIRED" not in engine_status(app) and not app.has("Load model"), 120, every=2)
        report.check(bool(ok), "Tray 'Load Model' loads the model", engine_status(app))

    # Hide with the window's close button (close behaviour: tray), show from the tray.
    report.check(json.loads(SETTINGS_JSON.read_text()).get("close_behavior") == "tray", "Close button is set to minimise to tray")
    app.press("Close", "AXButton", exact=True, settle=1.5)
    hidden = not [w for w in app_windows(app.pid) if w["layer"] == 0 and w["w"] > 300]
    alive = bool(subprocess.run(["pgrep", "-f", "target/debug/taurscribe$"], capture_output=True).stdout)
    report.check(hidden and alive, "Close hides the window and keeps the app running in the tray",
                 f"hidden={hidden} running={alive}")
    tray(app.pid, "Show Taurscribe")
    back = app.wait(lambda: [w for w in app_windows(app.pid) if w["layer"] == 0 and w["w"] > 300], 6)
    report.check(bool(back), "Tray 'Show Taurscribe' brings the window back")
    app.wait_ready(30)

    # Overlay: a floating panel during hotkey recordings (the in-app REC button
    # records with the window already in view, so no overlay there by design).
    report.check(bool(app.checked("Floating transcript overlay", "AXCheckBox")), "Overlay setting is on")
    idle = app_windows(app.pid)
    safe_paste_target(app)
    hold = subprocess.Popen([str(HARNESS_DIR / "bin" / "hotkeyhold"), "3", "ctrl", "alt"], stdout=subprocess.PIPE, text=True)
    hold.stdout.readline()
    time.sleep(1.5)
    extra = [w for w in app_windows(app.pid) if w not in idle]
    hold.wait(timeout=20)
    report.check(bool(extra), "Overlay window appears during a hotkey recording", str(extra)[:120])
    gone = app.wait(lambda: len(app_windows(app.pid)) <= len(idle), 15, every=1)
    report.check(bool(gone), "Overlay goes away after the recording")


SECTIONS: Dict[str, Callable[[App, UIReport], None]] = {
    "nav": section_nav,
    "quick": section_quick,
    "settings": section_settings,
    "text": section_text,
    "dictation": section_dictation,
    "downloads": section_downloads,
    "download_cancel": section_download_cancel,
    "files": section_files,
    "meetings": section_meetings,
    "grammar": section_grammar,
    "hotkey": section_hotkey,
    "tray": section_tray,
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--section", nargs="*", default=list(SECTIONS))
    args = parser.parse_args()
    report = UIReport()
    app = App(report)
    app.wait_ready()
    for name in args.section:
        report.section = name
        print(f"\n>>> {name.upper()}")
        try:
            SECTIONS[name](app, report)
        except Exception as e:
            report.check(False, f"{name} section ran to the end", f"{type(e).__name__}: {e}")
            traceback.print_exc()
            app.screenshot(f"{name}_crash")
    return report.finish()


if __name__ == "__main__":
    sys.exit(main())
