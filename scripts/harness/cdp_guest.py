#!/usr/bin/env python3
"""Drives an isolated Chrome meeting guest instance over Chrome DevTools Protocol (CDP).

Connects over WebSockets to automate meeting join flows, dismiss lobbies, and inject
synthesized audio playback directly into the meeting stream.
"""

from __future__ import annotations
import argparse
import base64
import json
import os
import signal
import subprocess
import sys
import time
import urllib.request
import wave
from pathlib import Path

try:
    import websocket
except ImportError:
    # Auto-switch to harness venv if available
    venv_py = Path(__file__).resolve().parent / ".venv" / "bin" / "python3"
    if venv_py.exists() and sys.executable != str(venv_py):
        os.execv(str(venv_py), [str(venv_py)] + sys.argv)
    else:
        sys.exit("websocket-client not installed. Run: pip install websocket-client")

CHROME_BIN = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
PROFILE_ROOT = Path("/tmp/taurscribe-cdp-guests")
SNAPSHOT_JS_FILE = Path(__file__).resolve().parent / "snapshot.js"
SNAPSHOT_JS = SNAPSHOT_JS_FILE.read_text() if SNAPSHOT_JS_FILE.exists() else ""

MICROPHONE_SHIM = r"""
(() => {
  if (window.__taurscribePlay) return;
  const context = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 48000 });
  const destination = context.createMediaStreamDestination();
  const hold = context.createConstantSource();
  hold.offset.value = 0;
  hold.connect(destination);
  hold.start();

  window.__taurscribePlay = async (base64) => {
    if (context.state !== 'running') await context.resume();
    const bytes = Uint8Array.from(atob(base64), c => c.charCodeAt(0));
    const buffer = await context.decodeAudioData(bytes.buffer);
    const source = context.createBufferSource();
    source.buffer = buffer;
    // Into the call ONLY. Never to local speakers: the recorder taps all system
    // output, so local playback would let a test pass without the audio ever
    // crossing the meeting.
    source.connect(destination);
    source.start();
    return buffer.duration;
  };

  // DevTools scripts running in an isolated world cannot call page functions,
  // but DOM events cross worlds: dispatch 'taurscribe-play' with the base64 WAV.
  window.addEventListener('taurscribe-play', (e) => { window.__taurscribePlay(e.detail); });

  const devices = navigator.mediaDevices;
  if (devices && devices.getUserMedia) {
    const originalGUM = devices.getUserMedia.bind(devices);
    devices.getUserMedia = async (constraints) => {
      if (!constraints || !constraints.audio) return originalGUM(constraints);
      if (context.state !== 'running') { try { await context.resume(); } catch (e) {} }
      const stream = new MediaStream(destination.stream.getAudioTracks().map(t => t.clone()));
      if (constraints.video) {
        try {
          const orig = await originalGUM({ video: constraints.video });
          orig.getVideoTracks().forEach(t => stream.addTrack(t));
        } catch (e) {}
      }
      return stream;
    };
  }
  console.log('[TAURSCRIBE_CDP] Scripted WebAudio microphone shim installed');
})();
"""


class CDPPage:
    def __init__(self, port: int):
        self.port = port
        self.ws_url = self._get_ws_url()
        self.ws = websocket.create_connection(self.ws_url, timeout=10, suppress_origin=True)
        self.id_counter = 0

    def _get_ws_url(self) -> str:
        deadline = time.time() + 10
        while time.time() < deadline:
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/json", timeout=2) as resp:
                    targets = json.load(resp)
                    for t in targets:
                        # Skip Chrome's own UI pages (sign-in intercepts, new-tab chrome).
                        if t.get("type") == "page" and not t.get("url", "").startswith(("chrome://", "chrome-untrusted://", "devtools://")):
                            return t["webSocketDebuggerUrl"]
            except Exception:
                time.sleep(0.3)
        raise RuntimeError(f"Could not connect to Chrome on port {self.port}")

    def call(self, method: str, params: dict | None = None) -> dict:
        self.id_counter += 1
        msg_id = self.id_counter
        self.ws.send(json.dumps({"id": msg_id, "method": method, "params": params or {}}))
        while True:
            raw = self.ws.recv()
            data = json.loads(raw)
            if data.get("id") == msg_id:
                return data

    def evaluate(self, expression: str):
        res = self.call(
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True, "awaitPromise": True},
        )
        result = res.get("result", {})
        if "exceptionDetails" in result:
            return f"Error: {result['exceptionDetails']}"
        return result.get("result", {}).get("value")

    def click(self, text_regex: str) -> bool:
        script = f"""
        (() => {{
          const regex = new RegExp('{text_regex}', 'i');
          const elements = [...document.querySelectorAll('button, [role=button], a, [role=switch], input[type=button], input[type=submit]')];
          for (const el of elements) {{
            const text = (el.innerText || el.getAttribute('aria-label') || el.title || el.value || '').trim();
            if (regex.test(text)) {{
              el.click();
              return true;
            }}
          }}
          return false;
        }})()
        """
        return bool(self.evaluate(script))

    def type_text(self, selector: str, text: str) -> bool:
        escaped_text = json.dumps(text)
        script = f"""
        (() => {{
          const el = document.querySelector({json.dumps(selector)});
          if (!el) return false;
          el.focus();
          el.value = {escaped_text};
          el.dispatchEvent(new Event('input', {{ bubbles: true }}));
          el.dispatchEvent(new Event('change', {{ bubbles: true }}));
          return true;
        }})()
        """
        return bool(self.evaluate(script))

    def query_exists(self, selector: str) -> bool:
        script = f"Boolean(document.querySelector({json.dumps(selector)}))"
        return bool(self.evaluate(script))

    def play_wav(self, wav_path: str | Path) -> float:
        p = Path(wav_path)
        if not p.exists():
            raise FileNotFoundError(f"WAV not found: {p}")
        b64 = base64.b64encode(p.read_bytes()).decode()
        expr = f"window.__taurscribePlay ? window.__taurscribePlay('{b64}') : (window.__playClipBase64 ? window.__playClipBase64('{b64}') : 0)"
        dur = self.evaluate(expr)
        return float(dur) if isinstance(dur, (int, float)) else 0.0

    def close(self):
        try:
            self.ws.close()
        except Exception:
            pass


class ChromeGuestSession:
    def __init__(self, port: int = 9222, url: str = "about:blank"):
        self.port = port
        self.url = url
        self.profile = PROFILE_ROOT / f"guest-{port}"
        self.profile.mkdir(parents=True, exist_ok=True)
        self.proc: subprocess.Popen | None = None
        self.page: CDPPage | None = None
        self.debug_dir: Path | None = None  # auto_join drops screenshots here

    def start(self):
        cmd = [
            CHROME_BIN,
            f"--remote-debugging-port={self.port}",
            f"--user-data-dir={self.profile}",
            "--no-first-run",
            "--no-default-browser-check",
            "--use-fake-ui-for-media-stream",
            "--use-fake-device-for-media-stream",  # never touch the real mic/camera
            # The guest never plays what it hears: in a two-speaker test the host's
            # voice would otherwise come back out of this Mac as "call audio".
            "--mute-audio",
            "--autoplay-policy=no-user-gesture-required",
            "--window-size=1200,800",
            self.url,
        ]
        self.proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # Connect CDP
        self.page = CDPPage(self.port)
        self.page.call("Page.enable")
        self.page.call("Page.addScriptToEvaluateOnNewDocument", {"source": MICROPHONE_SHIM})
        print(f"[CDP_GUEST] Chrome guest started on port {self.port} (pid: {self.proc.pid})")

    def play_audio(self, wav_path: str | Path) -> float:
        if not self.page:
            raise RuntimeError("Guest not started")
        return self.page.play_wav(wav_path)

    def screenshot(self, path) -> bool:
        """Saves what the guest browser shows (CDP; call from the thread that owns the page)."""
        if not self.page:
            return False
        res = self.page.call("Page.captureScreenshot", {"format": "png"})
        data = res.get("result", {}).get("data")
        if not data:
            return False
        Path(path).write_bytes(base64.b64decode(data))
        return True

    def is_in_call(self) -> bool:
        if not self.page:
            return False
        check_script = r"""
        (() => {
          // Knocking/waiting screens also render a hang-up style button; they are
          // not "in call" until the host admits us.
          const body = (document.body && document.body.innerText) || '';
          if (/asking to be let in|brings you into the call|should let you in soon|let you in shortly|let people know you're waiting|waiting for someone to let you in|when someone lets you in|waiting for the host|someone in the call needs to let you in|you can't join this call|denied your request/i.test(body)) return false;
          const joinBtn = document.getElementById('join-btn');
          if (joinBtn && joinBtn.innerText.includes('Call in progress')) return true;
          const leaveBtn = document.getElementById('leave-btn');
          if (leaveBtn && leaveBtn.offsetParent !== null) return true;
          const meetLeave = document.querySelector('button[aria-label*="Leave call" i], button[data-call-action="hangup"]');
          if (meetLeave) return true;
          const teamsHangup = document.querySelector('button[data-tid="hangup-button"], button[aria-label*="Leave" i], button[aria-label*="Hang up" i]');
          if (teamsHangup) return true;
          // Teams labels its hang-up plainly "Leave" (visible text, no aria-label).
          const leaveRe = /^\s*(leave|hang up|end call)\b/i;
          for (const b of document.querySelectorAll('button, [role="button"]')) {
            const label = (b.getAttribute('aria-label') || b.innerText || '').trim();
            if (leaveRe.test(label) && b.offsetParent !== null) return true;
          }
          return false;
        })()
        """
        return bool(self.page.evaluate(check_script))

    def auto_join(self, display_name: str = "Sarah Guest", timeout: float = 25.0, keep_mic_on: bool = False) -> bool:
        """Automates entry through lobbies and joins the meeting room using Decider & indexed DOM."""
        if not self.page:
            raise RuntimeError("Guest not started")

        print(f"[DECIDER_GUEST] Attempting auto-join as '{display_name}' (timeout: {timeout}s)...")
        try:
            from decider import MeetingDecider
        except ImportError:
            from scripts.harness.decider import MeetingDecider
        decider = MeetingDecider()

        deadline = time.time() + timeout
        step = 0
        while time.time() < deadline:
            step += 1
            if self.debug_dir and step % 15 == 1:
                try:
                    self.screenshot(Path(self.debug_dir) / f"guest_step_{step:03d}.png")
                except Exception:
                    pass
            if self.is_in_call():
                print(f"[DECIDER_GUEST] In-call detected!")
                return True

            # 1. Evaluate Jev Ultrafast DOM snapshot
            if SNAPSHOT_JS:
                try:
                    snapshot = self.page.evaluate(SNAPSHOT_JS)
                    if isinstance(snapshot, dict) and snapshot.get("elements"):
                        action = decider.decide(
                            snapshot=snapshot,
                            goal=(f"Join call as guest '{display_name}'. "
                                  + ("Turn off camera, keep microphone on. " if keep_mic_on else "Mute microphone and camera. ")
                                  + "Click Ask to join or Join now."),
                            guest_name=display_name
                        )
                        print(f"[DECIDER_GUEST] Step {step}: {action}")

                        if action.operation == "DONE":
                            if self.is_in_call():
                                return True
                            time.sleep(1.0)
                            continue
                        elif action.operation == "CLICK" and action.target is not None:
                            self.page.evaluate(f"""
                            (() => {{
                                const el = document.querySelector('[data-jev-index="{action.target}"]');
                                if (el) {{
                                    el.scrollIntoView({{ block: 'center' }});
                                    el.click();
                                    return true;
                                }}
                                return false;
                            }})()
                            """)
                            self.page.evaluate("window.startAudio && window.startAudio()")
                            time.sleep(0.8)
                            continue
                        elif action.operation == "TYPE_TEXT" and action.target is not None and action.text:
                            # React forms (Teams) ignore el.value writes; focus, clear,
                            # then send real keyboard text through CDP.
                            focused = self.page.evaluate(f"""
                            (() => {{
                                const el = document.querySelector('[data-jev-index="{action.target}"]');
                                if (!el) return false;
                                el.focus();
                                el.select && el.select();
                                return true;
                            }})()
                            """)
                            if focused:
                                self.page.call("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Backspace", "code": "Backspace", "windowsVirtualKeyCode": 8})
                                self.page.call("Input.dispatchKeyEvent", {"type": "keyUp", "key": "Backspace", "code": "Backspace", "windowsVirtualKeyCode": 8})
                                self.page.call("Input.insertText", {"text": action.text})
                            time.sleep(0.5)
                            continue
                except Exception as e:
                    print(f"[DECIDER_GUEST] Snapshot evaluation warning: {e}")

            # Fallback legacy selector heuristics
            self.page.click("Join now")
            self.page.evaluate("window.startAudio && window.startAudio()")
            self.page.click("Continue without microphone|Dismiss|Got it")
            self.page.type_text("input[placeholder*='name' i]", display_name)
            self.page.type_text("input[aria-label*='name' i]", display_name)
            self.page.type_text("input[type='text']", display_name)
            self.page.click("Ask to join|Join now|Join meeting")
            self.page.click("Continue on this browser|Join on the web instead")
            self.page.type_text("input[data-tid='prejoin-display-name-input']", display_name)
            self.page.click("Join now")

            time.sleep(1.0)

        return self.is_in_call()

    def leave_meeting(self) -> bool:
        """Clicks leave / hangup button."""
        if not self.page:
            return False
        print("[CDP_GUEST] Leaving meeting...")
        clicked = self.page.click("Leave call|Hang up|Leave")
        self.page.evaluate("window.stopAudio && window.stopAudio()")
        return clicked

    def stop(self):
        if self.page:
            self.page.close()
            self.page = None
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=3)
            except Exception:
                self.proc.kill()
            self.proc = None
            print(f"[CDP_GUEST] Chrome guest stopped")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Taurscribe Chrome CDP Guest")
    parser.add_argument("--port", type=int, default=9222)
    parser.add_argument("--url", type=str, default="http://127.0.0.1:8999/meeting?autojoin=1")
    parser.add_argument("--play", type=str, help="WAV file to play")
    args = parser.parse_args()

    session = ChromeGuestSession(args.port, args.url)
    session.start()
    try:
        time.sleep(2)
        if args.play:
            dur = session.play_audio(args.play)
            print(f"Playing {args.play} ({dur:.2f}s)...")
            time.sleep(dur + 0.5)
        else:
            print("Session active. Press Ctrl+C to close.")
            while True:
                time.sleep(1)
    finally:
        session.stop()


class RemoteChromeGuestSession(ChromeGuestSession):
    """Same guest, but its browser runs in the Linux VM (see vm_remote.py): the
    Mac under test then only runs the user's own Chrome, as in real life."""

    def __init__(self, port: int = 9455, url: str = "about:blank"):
        self.port = port
        self.url = url
        self.proc = None
        self.page = None
        self.debug_dir = None
        self.remote = None

    def start(self):
        from vm_remote import RemoteChromium
        self.remote = RemoteChromium(self.port)
        self.remote.start()
        self.page = CDPPage(self.port)
        self.page.call("Page.enable")
        self.page.call("Page.addScriptToEvaluateOnNewDocument", {"source": MICROPHONE_SHIM})
        self.page.call("Page.navigate", {"url": self.url})
        print(f"[CDP_GUEST] Remote guest started in VM {self.remote.ip} (tunnel localhost:{self.port})")

    def stop(self):
        if self.page:
            self.page.close()
            self.page = None
        if self.remote:
            self.remote.stop()
            self.remote = None
            print("[CDP_GUEST] Remote guest stopped")
