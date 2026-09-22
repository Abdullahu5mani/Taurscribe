#!/usr/bin/env python3
"""Decider-driven, real-meeting E2E test for Taurscribe meeting detection.

Nothing is simulated. Decider drives every step by reading indexed UI snapshots
and choosing what to click, and the pass/fail verdict comes from Decider reading
the Taurscribe window itself:

  Host   — the user's real, signed-in Chrome. Observed/actuated through the macOS
           accessibility tree (ax_snapshot) because Chrome refuses CDP on the
           default profile.
  Guest  — an isolated Chrome profile driven over CDP (snapshot.js), joining
           anonymously with a scripted WebAudio mic.
  App    — the Taurscribe WKWebView, observed through the accessibility tree.

Pass criteria:
  Phase 1  meeting wasn't there   → app shows no detection
  Phase 2  meeting was there      → app shows detection (correct platform)
  Phase 3  meeting stopped        → app detection clears

The API (/api/status) is only a secondary cross-check, read after Decider has
judged the UI, because /api/status forces a detector rescan and must not be what
makes the UI show a meeting.

Usage:
  scripts/harness/.venv/bin/python scripts/harness/test_decider_meeting_detection.py [--platform meet|teams|all]

Requires: Taurscribe running in test mode (control server on :8766), Google Chrome
signed in to a Google account that can create meetings, and Accessibility
permission for the terminal running this script.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

HARNESS_DIR = Path(__file__).resolve().parent
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import ax_driver  # noqa: E402
from cdp_guest import ChromeGuestSession, RemoteChromeGuestSession  # noqa: E402  (re-execs into .venv if needed)
from decider import DeciderAction, MeetingDecider  # noqa: E402
from platforms import PLATFORMS, NotSignedIn, PlatformAdapter  # noqa: E402
from recording_check import GuestSpeaker, HostSpeaker, ensure_model_loaded, restore_default_input, verify_recording  # noqa: E402
from voiceprint_check import verify_voiceprints  # noqa: E402

CONTROL_PORT = int(os.environ.get("TAURSCRIBE_CONTROL_PORT", "8766"))
BASE_URL = f"http://127.0.0.1:{CONTROL_PORT}"

DETECT_TIMEOUT = 45.0     # meeting live → app shows it
CLEAR_TIMEOUT = 45.0      # meeting stopped → app clears it
STABLE_WINDOW = 6.0       # a verdict must hold this long to count
JOIN_TIMEOUT = 90.0       # guest knock + host admit + guest in call
GUEST_CDP_PORT = 9455
HOST_CDP_PORT = 9333
# Persistent, signed-in profile for platforms whose host UI cannot be driven from
# the user's own Chrome (Zoom hides its toolbar until the pointer hovers).
HOST_PROFILE = Path.home() / ".taurscribe-harness" / "host-chrome"
VM_HOST_LOCAL_PORT = 9456  # Mac end of the SSH tunnel to a VM-hosted meeting host


# ── Reporting ────────────────────────────────────────────────────────────────

class Report:
    def __init__(self, platform: str):
        stamp = time.strftime("%Y%m%d-%H%M%S")
        self.dir = HARNESS_DIR / "reports" / f"{platform}-{stamp}"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.steps: List[Dict[str, Any]] = []
        self.checks: List[Dict[str, Any]] = []
        self.t0 = time.time()

    def log(self, actor: str, msg: str, **data):
        t = time.time() - self.t0
        print(f"  [{t:6.1f}s] {actor:<7} {msg}")
        self.steps.append({"t": round(t, 2), "actor": actor, "msg": msg, **data})

    def check(self, ok: bool, name: str, detail: str = ""):
        self.checks.append({"name": name, "ok": ok, "detail": detail})
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))
        if not ok:
            raise AssertionError(f"{name}: {detail}")

    def screen(self, label: str) -> Optional[Path]:
        """Full-screen capture, for seeing why a step is stuck."""
        out = self.dir / f"screen_{len(self.steps):03d}_{label}.png"
        subprocess.run(["screencapture", "-x", str(out)], capture_output=True, timeout=10)
        if out.exists():
            self.log("REPORT", f"screenshot → {out.name}")
            return out
        return None

    def save_snapshot(self, label: str, snapshot: Dict[str, Any]):
        (self.dir / f"{label}.txt").write_text(snapshot.get("table", ""))

    def finish(self, passed: bool, error: str = ""):
        (self.dir / "report.json").write_text(json.dumps({
            "passed": passed, "error": error, "checks": self.checks, "steps": self.steps,
        }, indent=2))
        print(f"\n  Report: {self.dir}")


# ── Observers / actors ───────────────────────────────────────────────────────

def api_get(path: str) -> Dict[str, Any]:
    req = urllib.request.Request(BASE_URL + path, headers={"Authorization": "Bearer " + os.environ["TAURSCRIBE_CONTROL_TOKEN"]})
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.loads(resp.read().decode())


def chrome_pids() -> Dict[str, List[int]]:
    """Main (non-helper) Chrome processes, split into the user's and ours."""
    out = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    user, harness = [], []
    for line in out.splitlines():
        pid_s, _, cmd = line.strip().partition(" ")
        if not cmd.startswith("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"):
            continue
        ours = "--user-data-dir=/tmp/taurscribe" in cmd or str(HOST_PROFILE) in cmd
        (harness if ours else user).append(int(pid_s))
    return {"user": user, "harness": harness}


def osascript(script: str, timeout: float = 30.0) -> str:
    res = subprocess.run(["osascript", "-e", script], capture_output=True, text=True, timeout=timeout)
    if res.returncode != 0:
        raise RuntimeError(res.stderr.strip())
    return res.stdout.strip()


class AppObserver:
    """Decider's eyes on the Taurscribe window."""

    def __init__(self, pid: int, decider: MeetingDecider, report: Report):
        self.pid = pid
        self.decider = decider
        self.report = report

    def look(self, expected_platform: str) -> Dict[str, Any]:
        snap = ax_driver.snapshot(self.pid)
        verdict = self.decider.assess_meeting_detection(snap, expected_platform)
        verdict["_snapshot"] = snap
        return verdict

    def wait_for(self, want_detected: bool, expected_platform: str, timeout: float) -> Dict[str, Any]:
        """Polls the UI until Decider's verdict equals want_detected and holds for STABLE_WINDOW."""
        deadline = time.time() + timeout
        stable_since: Optional[float] = None
        verdict: Dict[str, Any] = {}
        last_state = None
        while time.time() < deadline:
            verdict = self.look(expected_platform)
            state = bool(verdict["detected"])
            if state != last_state:
                self.report.log("DECIDER", f"app says detected={state} ({verdict['reason']})",
                                evidence=verdict.get("evidence"), mode=verdict.get("mode"))
                last_state = state
            if state == want_detected:
                stable_since = stable_since or time.time()
                if time.time() - stable_since >= STABLE_WINDOW:
                    verdict["latency_s"] = round(stable_since - (deadline - timeout), 1)
                    return verdict
            else:
                stable_since = None
            time.sleep(1.0)
        verdict["timed_out"] = True
        return verdict

    def screenshot(self, label: str):
        ax_driver.screenshot(self.pid, self.report.dir / f"app_{label}.png")


class ChromeHostAX:
    """Host session in the user's real Chrome, driven by Decider over accessibility.

    Platform adapters (platforms.py) tell it what to open and set `window` to a
    substring of the host window's title so snapshots target the right window.
    """

    def __init__(self, decider: MeetingDecider, report: Report):
        self.decider = decider
        self.report = report
        pids = chrome_pids()["user"]
        if not pids:
            # Opening a window launches Chrome; its pid shows up afterwards.
            osascript('tell application "Google Chrome" to activate')
            time.sleep(3)
            pids = chrome_pids()["user"]
        if not pids:
            raise RuntimeError("Could not find the user's Chrome process")
        self.pid: int = pids[0]
        self.window: Optional[str] = None
        self.window_id: Optional[str] = None

    def open_window(self, url: str):
        self.window_id = osascript(f'''
        tell application "Google Chrome"
            set w to make new window
            set URL of active tab of w to "{url}"
            return id of w
        end tell''')
        self.report.log("HOST", f"opened {url} in your Chrome (pid {self.pid})")

    def _by_id(self, action: str) -> str:
        """AppleScript that runs `action` on our window. Chrome window ids exceed
        AppleScript's integer range, so `window id N` breaks; compare as text."""
        return f'''
        tell application "Google Chrome"
            repeat with w in windows
                if (id of w as text) is "{self.window_id}" then
                    {action}
                    return ""
                end if
            end repeat
        end tell'''

    def wait_front_url(self, pattern: re.Pattern, timeout: float, hint: str = "") -> str:
        deadline = time.time() + timeout
        url = ""
        while time.time() < deadline:
            url = osascript(self._by_id("return URL of active tab of w"))
            if pattern.search(url):
                return url
            time.sleep(1)
        raise RuntimeError(f"Host URL never matched {pattern.pattern} (last: {url}). {hint}")

    def wait_window(self, timeout: float = 20.0):
        ax_driver.wait_for_window(self.pid, self.window, timeout=timeout)

    def raise_window(self):
        """Keeps the test's own window frontmost so title matching picks it over
        any other window of the same app the user has open."""
        if self.window_id:
            try:
                osascript(self._by_id("set index of w to 1"), timeout=5)
            except Exception:
                pass

    def snapshot(self) -> Dict[str, Any]:
        self.raise_window()
        return ax_driver.snapshot(self.pid, window=self.window)

    def act(self, action: DeciderAction):
        if action.operation == "CLICK" and action.target is not None:
            self.raise_window()
            # The reason carries the label Decider chose, as "...: 'label'".
            m = re.search(r": '(.*)'\)?$", action.reason or "")
            try:
                ax_driver.press(self.pid, action.target, window=self.window, expect=m.group(1) if m else None)
            except ax_driver.StaleElement as e:
                self.report.log("HOST", f"screen changed before the click ({e}); re-reading")

    def step(self, decide: Callable[[Dict[str, Any]], DeciderAction]) -> DeciderAction:
        snap = self.snapshot()
        action = decide(snap)
        if action.operation == "CLICK":
            self.report.log("HOST", f"Decider → {action}")
        self.act(action)
        return action

    def leave(self, timeout: float = 20.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            action = self.step(self.decider.decide_leave)
            if action.operation == "DONE":
                self.report.log("HOST", "Decider confirms host left the call")
                return
            time.sleep(1.5)
        raise RuntimeError("Host could not leave the call")

    def close(self):
        """Closes only the window this test opened."""
        if not self.window_id:
            return
        try:
            osascript(self._by_id("close w"))
        except Exception as e:
            print(f"  [CLEANUP] host close warning: {e}")
        self.window_id = None


def quit_harness_chrome():
    """Quits the harness host Chrome if it is running. While it runs, AppleScript's
    "Google Chrome" can resolve to it instead of the user's Chrome, which the
    accessibility-driven hosts depend on. The profile (and its sign-ins) stays on disk."""
    try:
        urllib.request.urlopen(f"http://127.0.0.1:{HOST_CDP_PORT}/json/version", timeout=1)
    except Exception:
        return
    from cdp_guest import CDPPage
    try:
        CDPPage(HOST_CDP_PORT).call("Browser.close")
    except Exception:
        pass
    deadline = time.time() + 10
    while time.time() < deadline:
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{HOST_CDP_PORT}/json/version", timeout=1)
            time.sleep(0.5)
        except Exception:
            return


class ChromeHostCDP:
    """Host session in a dedicated harness Chrome profile, driven over CDP.

    Same interface as ChromeHostAX, so platform adapters work with either. CDP can
    move the pointer, which reveals hover-only toolbars (Zoom's Leave/End).
    """

    def __init__(self, decider: MeetingDecider, report: Report, remote: bool = False):
        self.decider = decider
        self.report = report
        self.window: Optional[str] = "cdp"
        self.window_id = None
        self.page = None
        self.proc = None
        # remote=True: the browser runs in the Linux VM with a persistent, signed-in
        # profile (the remote participant); otherwise the Mac's harness profile.
        self.remote_vm = None
        if remote:
            from vm_remote import RemoteChromium
            self.remote_vm = RemoteChromium(VM_HOST_LOCAL_PORT, remote_port=9223,
                                            profile="taurscribe-zoom-host", fresh=False)
        # Regex for a child frame URL that holds the real UI (Zoom runs the meeting
        # inside a same-origin iframe of its PWA shell). None = the top document.
        self.frame_hint: Optional[re.Pattern] = None
        self._worlds: Dict[str, int] = {}

    def _frame(self) -> Optional[Dict[str, Any]]:
        if not self.frame_hint:
            return None
        found = []

        def walk(node):
            f = node["frame"]
            if f.get("parentId") and self.frame_hint.search(f.get("url", "")):
                found.append(f)
            for c in node.get("childFrames", []):
                walk(c)
        walk(self.page.call("Page.getFrameTree")["result"]["frameTree"])
        return found[0] if found else None

    def _eval(self, expression: str):
        """Evaluates in the UI frame when there is one, else in the top document."""
        frame = self._frame()
        if not frame:
            return self.page.evaluate(expression)
        ctx = self._worlds.get(frame["id"])
        if ctx is None:
            ctx = self.page.call("Page.createIsolatedWorld", {"frameId": frame["id"], "worldName": "decider"})["result"]["executionContextId"]
            self._worlds[frame["id"]] = ctx
        res = self.page.call("Runtime.evaluate", {"expression": expression, "contextId": ctx,
                                                   "returnByValue": True, "awaitPromise": True})
        if "exceptionDetails" in res.get("result", {}) or "error" in res:
            self._worlds.pop(frame["id"], None)  # frame reloaded; recreate next time
            return None
        return res["result"]["result"].get("value")

    def _frame_offset(self):
        """Top-page position of the UI frame's <iframe> (asked of Chrome directly;
        the element's src can differ from where the frame has navigated since)."""
        frame = self._frame()
        if not frame:
            return 0, 0
        owner = self.page.call("DOM.getFrameOwner", {"frameId": frame["id"]}).get("result", {})
        node = owner.get("backendNodeId")
        if not node:
            return 0, 0
        box = self.page.call("DOM.getBoxModel", {"backendNodeId": node}).get("result", {}).get("model")
        return (box["content"][0], box["content"][1]) if box else (0, 0)

    def _connect(self, url: str):
        from cdp_guest import CDPPage, CHROME_BIN, MICROPHONE_SHIM
        if self.remote_vm:
            self.remote_vm.start()
            self.page = CDPPage(VM_HOST_LOCAL_PORT)
            self.page.call("Page.enable")
            # The remote host also speaks in the recording phase (scripted mic).
            self.page.call("Page.addScriptToEvaluateOnNewDocument", {"source": MICROPHONE_SHIM})
            return
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{HOST_CDP_PORT}/json", timeout=1)
        except Exception:
            HOST_PROFILE.mkdir(parents=True, exist_ok=True)
            self.proc = subprocess.Popen([
                CHROME_BIN, f"--remote-debugging-port={HOST_CDP_PORT}", f"--user-data-dir={HOST_PROFILE}",
                "--no-first-run", "--no-default-browser-check",
                "--use-fake-ui-for-media-stream",  # auto-grant mic; the real mic is used
                "--window-size=1300,850", "about:blank",
            ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.page = CDPPage(HOST_CDP_PORT)
        self.page.call("Page.enable")

    def open_window(self, url: str):
        if not self.page:
            self._connect(url)
        self.page.call("Page.navigate", {"url": url})
        self.report.log("HOST", f"opened {url} in the harness Chrome ({HOST_PROFILE})")
        time.sleep(3)

    def url(self) -> str:
        return str(self.page.evaluate("location.href"))

    def wait_front_url(self, pattern: re.Pattern, timeout: float, hint: str = "") -> str:
        deadline = time.time() + timeout
        url = ""
        while time.time() < deadline:
            url = self.url()
            if pattern.search(url):
                return url
            time.sleep(1)
        raise RuntimeError(f"Host URL never matched {pattern.pattern} (last: {url}). {hint}")

    def wait_window(self, timeout: float = 20.0):
        deadline = time.time() + timeout
        while time.time() < deadline and self.page.evaluate("document.readyState") != "complete":
            time.sleep(0.5)

    def hover(self):
        """Nudges the pointer over the page so auto-hiding toolbars render."""
        size = self.page.evaluate("[innerWidth, innerHeight]") or [1200, 800]
        for dx in (0, 7):
            self.page.call("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": size[0] // 2 + dx, "y": size[1] // 2})

    def snapshot(self) -> Dict[str, Any]:
        from cdp_guest import SNAPSHOT_JS
        snap = self._eval(SNAPSHOT_JS)
        snap = snap if isinstance(snap, dict) else {"elements": [], "table": ""}
        # Only wake the toolbar when it is hidden: moving the pointer away also
        # closes menus a previous click opened (Zoom's End menu).
        if not self.decider.has_leave_control(snap):
            self.hover()
            again = self._eval(SNAPSHOT_JS)
            if isinstance(again, dict):
                snap = again
        return snap

    def act(self, action: DeciderAction):
        if action.operation != "CLICK" or action.target is None:
            return
        # A real pointer click at the element's centre (React apps ignore el.click()).
        box = self._eval(f"""
        (() => {{
            const el = document.querySelector('[data-jev-index="{action.target}"]');
            if (!el) return null;
            el.scrollIntoView({{block: 'center'}});
            const r = el.getBoundingClientRect();
            return [r.left + r.width / 2, r.top + r.height / 2];
        }})()""")
        if not box:
            return
        ox, oy = self._frame_offset()
        x, y = box[0] + ox, box[1] + oy
        # `buttons` must be set on the press or pointer-event UIs (Zoom) ignore it.
        for kind, buttons in (("mouseMoved", 0), ("mousePressed", 1), ("mouseReleased", 0)):
            self.page.call("Input.dispatchMouseEvent", {"type": kind, "x": x, "y": y, "button": "left",
                                                         "buttons": buttons, "clickCount": 1})

    def step(self, decide: Callable[[Dict[str, Any]], DeciderAction]) -> DeciderAction:
        snap = self.snapshot()
        action = decide(snap)
        if action.operation == "CLICK":
            self.report.log("HOST", f"Decider → {action}")
        self.act(action)
        return action

    def leave(self, timeout: float = 20.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.step(self.decider.decide_leave).operation == "DONE":
                self.report.log("HOST", "Decider confirms host left the call")
                return
            time.sleep(1.5)
        raise RuntimeError("Host could not leave the call")

    def close(self):
        """Parks the harness Chrome on a blank page; the profile (and sign-in) is kept.
        A VM host's browser is shut down (its profile stays in the VM)."""
        if self.page:
            try:
                self.page.call("Page.navigate", {"url": "about:blank"})
            except Exception:
                pass
        if self.remote_vm:
            try:
                self.page and self.page.close()
            except Exception:
                pass
            self.page = None
            self.remote_vm.stop()

    def import_login_from_mac(self, domain_suffix: str) -> int:
        """Copies one site's login cookies from the Mac's signed-in harness profile
        into this (VM) browser. Returns how many cookies were copied."""
        mac = ChromeHostCDP(self.decider, self.report)
        mac._connect("")
        cookies = mac.page.call("Storage.getCookies")["result"]["cookies"]
        quit_harness_chrome()
        keep = ("name", "value", "domain", "path", "secure", "httpOnly", "sameSite", "expires")
        wanted = [{k: c[k] for k in keep if k in c and not (k == "expires" and c[k] < 0)}
                  for c in cookies if c["domain"].lstrip(".").endswith(domain_suffix)]
        if wanted:
            self.page.call("Storage.setCookies", {"cookies": wanted})
        self.report.log("HOST", f"copied {len(wanted)} {domain_suffix} login cookies from the Mac's test profile into the VM")
        return len(wanted)


class LocalAttendeeAX:
    """You, in your own Chrome on this Mac, joining a meeting a remote participant
    hosts (Zoom: the signed-in host runs in the VM). Duck-types GuestCDP."""

    def __init__(self, adapter: PlatformAdapter, url: str, report: Report, decider: MeetingDecider):
        self.adapter = adapter
        self.url = url
        self.report = report
        self.ax = ChromeHostAX(decider, report)
        self.name = "Local attendee"
        self.where = "mac"
        self.session = None
        self.joined = threading.Event()
        self.error: Optional[str] = None
        self._thread: Optional[threading.Thread] = None

    def start_joining(self, timeout: float):
        self.report.log("YOU", f"joining from your Chrome on this Mac: {self.url}")

        def run():
            try:
                self.adapter.local_join(self.ax, self.url, timeout)
                self.report.log("YOU", "in the call")
                self.joined.set()
            except Exception as e:
                self.error = f"{type(e).__name__}: {e}"

        self._thread = threading.Thread(target=run, daemon=True)
        self._thread.start()

    def leave(self, decider: MeetingDecider):
        # Hover-only toolbars (Zoom) keep Leave out of the accessibility tree, but an
        # attendee may simply close the window; the host ends the meeting for all.
        self.ax.close()
        self.report.log("YOU", "left the call (closed the meeting window)")

    def stop(self):
        self.ax.close()


class GuestCDP:
    """Anonymous guest in an isolated Chrome, driven by Decider over CDP."""

    def __init__(self, url: str, report: Report, name: str = "Decider Test Guest", where: str = "vm"):
        # "vm": the guest's browser runs in the Linux VM (a real remote machine);
        # "local": an isolated Chrome on this Mac (older setup, kept as a fallback).
        session_cls = RemoteChromeGuestSession if where == "vm" else ChromeGuestSession
        self.session = session_cls(port=GUEST_CDP_PORT, url=url)
        self.where = where
        self.report = report
        self.name = name
        self.joined = threading.Event()
        self.error: Optional[str] = None
        self._thread: Optional[threading.Thread] = None

    def start_joining(self, timeout: float):
        self.session.debug_dir = self.report.dir
        self.session.start()
        where = f"VM {self.session.remote.ip}" if self.where == "vm" else f"isolated Chrome pid {self.session.proc.pid}"
        self.report.log("GUEST", f"guest browser up ({where}), Decider joining as '{self.name}'")

        def run():
            try:
                # The guest's mic is the scripted shim (silent until a clip plays), so
                # it joins unmuted: the recording phase must not depend on an unmute click.
                if self.session.auto_join(display_name=self.name, timeout=timeout, keep_mic_on=True):
                    text = self.session.page.evaluate("(document.body && document.body.innerText || '').slice(0, 400)")
                    self.report.log("GUEST", "guest page claims in-call", page_text=text)
                    (self.report.dir / "guest_in_call_page.txt").write_text(str(text))
                    self.joined.set()
            except Exception as e:
                self.error = f"{type(e).__name__}: {e}"

        self._thread = threading.Thread(target=run, daemon=True)
        self._thread.start()

    def leave(self, decider: MeetingDecider):
        """Decider leaves the call (real pointer click, confirmed), then the guest's
        browser is closed so no guest tab lingers in the room. A lingering tab is a
        live meeting on this Mac and would rightly keep the app's detection on."""
        from cdp_guest import SNAPSHOT_JS
        from recording_check import guest_click
        left = False
        try:
            page = self.session.page
            size = page.evaluate("[innerWidth, innerHeight]") or [1200, 800]
            for _ in range(4):
                page.call("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": size[0] // 2, "y": size[1] // 2})
                action = decider.decide_leave(page.evaluate(SNAPSHOT_JS) or {})
                if action.operation != "CLICK":
                    left = True
                    break
                self.report.log("GUEST", f"Decider → {action}")
                guest_click(self.session, action.target)
                time.sleep(2.0)
            left = left or not self.session.is_in_call()
        except Exception as e:
            print(f"  [GUEST] leave warning: {e}")
        self.report.log("GUEST", "left the call" if left else "leave not confirmed; closing the guest browser")
        self.stop()

    def stop(self):
        try:
            self.session.stop()
        except Exception:
            pass


# ── The test ─────────────────────────────────────────────────────────────────

def host_admit_step(host: ChromeHostAX, adapter: PlatformAdapter) -> DeciderAction:
    """One host tick while the guest is joining: clear popups first, then admit."""
    snap = host.snapshot()
    action = host.decider.decide_plan(snap, adapter.common_interrupts)
    if action.operation != "CLICK":
        action = host.decider.decide_host(snap)
    if action.operation != "CLICK" and adapter.prepare_admit(host, snap):
        (host.report.dir / f"host_after_prepare_{int(time.time())}.txt").write_text(host.snapshot().get("table", ""))
        return DeciderAction("CLICK", reason="prepare_admit")
    if action.operation == "CLICK":
        host.report.log("HOST", f"Decider → {action}")
        host.act(action)
    return action


def run(platform_key: str, check_recording: bool = True, guest_where: str = "vm",
        check_voiceprints: bool = True) -> str:
    """Returns 'passed', 'failed' or 'skipped'."""
    adapter = PLATFORMS[platform_key]
    report = Report(platform_key)
    decider = MeetingDecider()
    host: Optional[ChromeHostAX] = None
    guest: Optional[GuestCDP] = None
    app: Optional[AppObserver] = None
    mode = "LLM @ " + decider.api_base if decider.prefer_local_runner else "System-1 rules (no LLM runner configured)"

    print("=" * 78)
    print(f" DECIDER MEETING DETECTION E2E — {adapter.display_name}")
    print(f" Decider mode: {mode}")
    print("=" * 78)

    try:
        # ── Pre-flight ────────────────────────────────────────────────────────
        print("\n>>> PRE-FLIGHT")
        health = api_get("/api/health")
        report.check(health.get("status") == "ok", "Taurscribe control server is up", f"pid {health.get('pid')}")
        app = AppObserver(int(health["pid"]), decider, report)
        # The first accessibility request switches WebKit's AX tree on; a freshly
        # launched window can take a few seconds to publish it.
        deadline = time.time() + 15
        snap = ax_driver.snapshot(app.pid)
        while len(snap.get("elements", [])) <= 5 and time.time() < deadline:
            time.sleep(1)
            snap = ax_driver.snapshot(app.pid)
        report.check(len(snap.get("elements", [])) > 5, "Decider can read the Taurscribe window",
                     f"{len(snap['elements'])} elements via accessibility")

        if check_recording:
            ensure_model_loaded(app, decider, report)

        stale = []
        for pid in chrome_pids()["user"]:
            stale += [t for t in ax_driver.windows(pid) if adapter.is_meeting_window(t)]
        report.check(not stale, f"No {adapter.display_name} window already open in Chrome",
                     f"close these first: {stale}" if stale else "")

        # ── Phase 1: no meeting ───────────────────────────────────────────────
        print(f"\n>>> PHASE 1: meeting wasn't there → no detection")
        v = app.wait_for(False, adapter.display_name, timeout=CLEAR_TIMEOUT)
        report.save_snapshot("phase1_app", v["_snapshot"])
        app.screenshot("phase1_no_meeting")
        report.check(not v["detected"] and not v.get("timed_out"),
                     "Decider sees no meeting in the app", v["reason"])
        active = api_get("/api/status")["detector"]["active_meetings"]
        report.check(len(active) == 0, "API cross-check: no active meetings", f"{len(active)} active")

        # ── Phase 2: meeting live ─────────────────────────────────────────────
        print(f"\n>>> PHASE 2: meeting was there → detection")
        if adapter.host_kind == "vm":
            host = ChromeHostCDP(decider, report, remote=True)
        elif adapter.host_kind == "cdp":
            host = ChromeHostCDP(decider, report)
        else:
            quit_harness_chrome()
            host = ChromeHostAX(decider, report)
        join_url = adapter.start_meeting(host)
        report.log("HOST", f"Decider confirms host is in the call; guest link {join_url}")

        if adapter.local_role == "guest":
            guest = LocalAttendeeAX(adapter, join_url, report, decider)
        else:
            guest = GuestCDP(join_url, report, where=guest_where)
        guest.start_joining(timeout=JOIN_TIMEOUT)
        # Joined means both sides agree: the guest's page is past the waiting
        # screen AND (where the platform shows one) the host's roster counts them.
        deadline = time.time() + JOIN_TIMEOUT
        count = None
        host_saw_guest: Optional[bool] = None
        last_progress = time.time()
        while time.time() < deadline and not guest.error:
            if time.time() - last_progress > 20:
                report.screen("stuck_joining")
                last_progress = time.time()
            if host_admit_step(host, adapter).operation == "CLICK":
                last_progress = time.time()
            snap = host.snapshot()
            new_count = adapter.participant_count(snap)
            if new_count != count:
                report.log("HOST", f"host roster shows {new_count} participant(s)")
                count = new_count
            seen = adapter.host_sees_guest(snap, guest.name)
            if seen and not host_saw_guest:
                report.log("HOST", "host sees the guest in the call")
            host_saw_guest = host_saw_guest or seen  # join toasts vanish; keep the proof
            if guest.joined.is_set() and host_saw_guest:
                break
            if guest.joined.is_set() and host_saw_guest is None and time.time() > deadline - JOIN_TIMEOUT + 45:
                break  # platform never shows host-side evidence
            time.sleep(1.5)
        report.check(guest.joined.is_set(), "Guest page reports it is in the call",
                     guest.error or ("in call" if guest.joined.is_set() else "timed out"))
        if host_saw_guest is None:
            report.log("HOST", "platform shows no host-side evidence; relying on the guest page")
        else:
            report.check(bool(host_saw_guest), "Host sees the guest in the call",
                         f"participants={count}" if count is not None else "join notice seen")

        v = app.wait_for(True, adapter.display_name, timeout=DETECT_TIMEOUT)
        report.save_snapshot("phase2_app", v["_snapshot"])
        app.screenshot("phase2_meeting_live")
        report.check(v["detected"] and not v.get("timed_out"), "Decider sees the meeting in the app",
                     f"{v.get('evidence')} (stable after {v.get('latency_s')}s)")
        report.check((v.get("platform") or "").lower() == adapter.display_name.lower(),
                     f"App names the platform '{adapter.display_name}'", f"app says '{v.get('platform')}'")
        active = api_get("/api/status")["detector"]["active_meetings"]
        match = [m for m in active if m.get("platform") == adapter.key]
        report.check(bool(match), f"API cross-check: active meeting with platform '{adapter.key}'",
                     json.dumps([{k: m.get(k) for k in ("platform", "title", "url", "confidence", "pid")} for m in active]))

        # ── Phase 2b: the live call is recorded properly ──────────────────────
        if check_recording or check_voiceprints:
            if guest._thread:
                guest._thread.join(timeout=10)  # the join thread owns the guest's CDP socket
            remote = HostSpeaker(host) if adapter.local_role == "guest" else GuestSpeaker(guest)
        if check_recording:
            print(f"\n>>> PHASE 2b: guest speaks into the meeting → Taurscribe records it")
            verify_recording(app, remote, adapter, decider, report)

        # ── Phase 2c: callers recognised by voice (real voices, LibriSpeech) ──
        if check_voiceprints:
            print(f"\n>>> PHASE 2c: real voices over the call → speaker recognised / not confused")
            verify_voiceprints(app, remote, adapter, decider, report)

        # ── Phase 3: meeting stopped ──────────────────────────────────────────
        print(f"\n>>> PHASE 3: meeting stopped → detection stopped")
        guest.leave(decider)
        adapter.leave_host(host)
        v = app.wait_for(False, adapter.display_name, timeout=CLEAR_TIMEOUT)
        report.save_snapshot("phase3_app", v["_snapshot"])
        app.screenshot("phase3_meeting_stopped")
        report.check(not v["detected"] and not v.get("timed_out"), "Decider sees detection cleared in the app",
                     f"{v['reason']} (stable after {v.get('latency_s')}s)")
        active = api_get("/api/status")["detector"]["active_meetings"]
        report.check(len(active) == 0, "API cross-check: no active meetings", f"{len(active)} active")

        print("\n" + "=" * 78)
        print(f" ALL PHASES PASSED — {adapter.display_name}")
        print("  ✓ meeting wasn't there → no detection")
        print("  ✓ meeting was there    → detection")
        if check_recording:
            print("  ✓ you + guest spoke    → each on their own channel, transcribed, no leakage")
        if check_voiceprints:
            print("  ✓ real voices          → named caller recognised later, other voice not confused")
        print("  ✓ meeting stopped      → detection stopped")
        print("=" * 78)
        report.finish(True)
        return "passed"

    except NotSignedIn as e:
        print(f"\n{'=' * 78}\n SKIPPED — {e}\n Sign in to {adapter.display_name} in Chrome and re-run.\n{'=' * 78}")
        report.finish(False, f"skipped: {e}")
        return "skipped"

    except Exception as e:
        err = f"{type(e).__name__}: {e}"
        print(f"\n{'=' * 78}\n TEST FAILED — {err}\n{'=' * 78}")
        report.screen("FAILURE")
        if app:
            try:
                app.screenshot("FAILURE")
                report.save_snapshot("FAILURE_app", ax_driver.snapshot(app.pid))
            except Exception:
                pass
        if host and host.window:
            try:
                report.save_snapshot("FAILURE_host", host.snapshot())
            except Exception:
                pass
        if guest and guest.session and guest.session.page and not (guest._thread and guest._thread.is_alive()):
            try:
                (report.dir / "FAILURE_guest.txt").write_text(str(guest.session.page.evaluate(
                    "(document.body && document.body.innerText || '').slice(0, 3000)")))
            except Exception:
                pass
        report.finish(False, err)
        return "failed"

    finally:
        print("\n[CLEANUP]")
        restore_default_input()  # never leave BlackHole as the user's default mic
        # Never leave the app recording after a failed run.
        try:
            if api_get("/api/status")["recording"]["is_recording"]:
                req = urllib.request.Request(BASE_URL + "/api/capture/stop", method="POST",
                                             headers={"Authorization": "Bearer " + os.environ["TAURSCRIBE_CONTROL_TOKEN"]})
                urllib.request.urlopen(req, timeout=400).read()
                print("  [CLEANUP] stopped a recording left running by the test")
        except Exception as e:
            print(f"  [CLEANUP] recording stop warning: {e}")
        if guest:
            guest.stop()
        if host:
            try:
                if host.window and decider.has_leave_control(host.snapshot()):
                    adapter.leave_host(host)
            except Exception:
                pass
            host.close()
            if isinstance(host, ChromeHostCDP):
                quit_harness_chrome()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--platform", choices=sorted(PLATFORMS) + ["all"], default="meet")
    parser.add_argument("--guest", choices=["vm", "local"], default="vm",
                        help="where the remote participant's browser runs (default: the Linux VM)")
    parser.add_argument("--no-voiceprints", action="store_true",
                        help="skip phase 2c (LibriSpeech voices over the call, speaker recognition)")
    parser.add_argument("--no-recording", action="store_true",
                        help="skip phase 2b (guest speaks, Taurscribe records, transcript checked)")
    args = parser.parse_args()
    keys = sorted(PLATFORMS) if args.platform == "all" else [args.platform]
    results = {}
    for key in keys:
        results[key] = run(key, check_recording=not args.no_recording, guest_where=args.guest,
                           check_voiceprints=not args.no_voiceprints)
        time.sleep(3)
    if len(keys) > 1:
        print("\n" + "=" * 78 + "\n SUMMARY")
        for key, res in results.items():
            print(f"  {PLATFORMS[key].display_name:<18} {res.upper()}")
        print("=" * 78)
    sys.exit(1 if "failed" in results.values() else 0)


if __name__ == "__main__":
    main()
