#!/usr/bin/env python3
"""Meeting Opener (Host Controller) for Google Meet & WebRTC.

Creates or hosts the meeting session, retrieves the shareable join URL,
and automatically monitors host admissions to admit guests as they arrive.
"""

from __future__ import annotations
import argparse
import http.server
import json
import os
import re
import socketserver
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Optional

try:
    import websocket
except ImportError:
    venv_py = Path(__file__).resolve().parent / ".venv" / "bin" / "python3"
    if venv_py.exists() and sys.executable != str(venv_py):
        os.execv(str(venv_py), [str(venv_py)] + sys.argv)
    else:
        sys.exit("websocket-client not installed. Run: pip install websocket-client")

from cdp_guest import CDPPage, CHROME_BIN
from decider import MeetingDecider

SNAPSHOT_JS_PATH = Path(__file__).resolve().parent / "snapshot.js"
SNAPSHOT_JS = SNAPSHOT_JS_PATH.read_text() if SNAPSHOT_JS_PATH.exists() else ""


class LocalMeetingHandler(http.server.SimpleHTTPRequestHandler):
    """Zero-dependency local WebRTC/WebAudio meeting room simulator."""

    def do_GET(self):
        if self.path.startswith("/meeting"):
            html = """<!DOCTYPE html>
<html>
<head>
    <title>Google Meet - Architecture Sync (meet.google.com/taur-sync-live)</title>
    <style>
        body { font-family: -apple-system, sans-serif; background: #202124; color: white; display: flex; flex-direction: column; align-items: center; justify-content: center; height: 100vh; margin: 0; }
        .card { background: #303134; padding: 24px; border-radius: 12px; width: 380px; text-align: center; box-shadow: 0 4px 20px rgba(0,0,0,0.5); }
        button { background: #1a73e8; color: white; border: none; padding: 10px 20px; border-radius: 20px; font-weight: 600; cursor: pointer; margin: 8px; font-size: 14px; }
        button:hover { background: #1557b0; }
        .controls { margin-top: 16px; display: flex; justify-content: center; gap: 8px; }
        .active-pill { display: inline-block; padding: 4px 12px; border-radius: 12px; background: #137333; font-size: 12px; margin-bottom: 12px; }
        #admit-box { display: none; background: #3c4043; padding: 12px; border-radius: 8px; margin-top: 12px; }
    </style>
</head>
<body>
    <div class="card">
        <div class="active-pill" id="status-pill">Ready to join</div>
        <h2 id="room-title">Architecture Sync</h2>
        <div id="join-section">
            <input type="text" id="name-input" placeholder="Your name" style="padding: 8px; border-radius: 6px; border: 1px solid #5f6368; width: 80%; margin-bottom: 12px; background: #202124; color: white;">
            <br>
            <button id="join-btn" onclick="startCall()">Join now</button>
            <button id="mic-toggle" onclick="toggleMic()">Turn off microphone</button>
            <button id="cam-toggle" onclick="toggleCam()">Turn off camera</button>
        </div>
        <div id="in-call-section" style="display: none;">
            <p>Call in progress • Audio streaming active</p>
            <button id="leave-btn" style="background: #ea4335;" onclick="leaveCall()">Leave call</button>
            <div id="admit-box">
                <p id="admit-msg">Someone wants to join this call</p>
                <button id="admit-btn" onclick="admitGuest()">Admit</button>
                <button id="deny-btn" style="background: #5f6368;" onclick="denyGuest()">Deny</button>
            </div>
        </div>
    </div>
    <script>
        let inCall = false;
        function startCall() {
            inCall = true;
            document.getElementById('status-pill').innerText = 'In call';
            document.getElementById('join-section').style.display = 'none';
            document.getElementById('in-call-section').style.display = 'block';
            window.startAudio && window.startAudio();
        }
        function leaveCall() {
            inCall = false;
            document.getElementById('status-pill').innerText = 'Call ended';
            document.getElementById('join-section').style.display = 'block';
            document.getElementById('in-call-section').style.display = 'none';
            window.stopAudio && window.stopAudio();
        }
        function toggleMic() {
            const btn = document.getElementById('mic-toggle');
            btn.innerText = btn.innerText.includes('off') ? 'Turn on microphone' : 'Turn off microphone';
        }
        function toggleCam() {
            const btn = document.getElementById('cam-toggle');
            btn.innerText = btn.innerText.includes('off') ? 'Turn on camera' : 'Turn off camera';
        }
        function requestAdmit(name) {
            document.getElementById('admit-msg').innerText = (name || 'Guest') + ' wants to join this call';
            document.getElementById('admit-box').style.display = 'block';
        }
        function admitGuest() {
            document.getElementById('admit-box').style.display = 'none';
            console.log('[HOST] Admitted guest');
        }
        function denyGuest() {
            document.getElementById('admit-box').style.display = 'none';
        }
        if (window.location.search.includes('autojoin=1')) {
            setTimeout(startCall, 200);
        }
    </script>
</body>
</html>"""
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(html.encode("utf-8"))
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, format, *args):
        return  # Suppress HTTP access logging


class MeetingOpener:
    """Controls the Host meeting session and admits incoming participants."""

    def __init__(self, mode: str = "local", port: int = 9111, local_http_port: int = 8999):
        self.mode = mode  # 'local', 'google-meet', or 'teams'
        self.port = port
        self.local_http_port = local_http_port
        self.profile = f"/tmp/taurscribe-opener-{port}"
        self.proc: Optional[subprocess.Popen] = None
        self.page: Optional[CDPPage] = None
        self.httpd: Optional[socketserver.TCPServer] = None
        self.http_thread: Optional[threading.Thread] = None
        self.meeting_url = ""
        self.decider = MeetingDecider()

    def start(self) -> str:
        """Starts the host session and returns the active meeting URL."""
        if self.mode == "local":
            self._start_local_server()
            self.meeting_url = f"http://127.0.0.1:{self.local_http_port}/meeting?autojoin=1"
            self._start_host_browser(self.meeting_url)
            print(f"[OPENER] Local meeting room active: {self.meeting_url}")
            return self.meeting_url

        elif self.mode == "google-meet":
            print("[OPENER] Launching real Google Meet host session in Google Chrome...")
            apple_script = """
            tell application "Google Chrome"
                activate
                set newWin to make new window
                set URL of active tab of newWin to "https://meet.google.com/new"
                delay 3
                repeat 10 times
                    set curUrl to URL of active tab of newWin
                    if curUrl contains "meet.google.com/" and curUrl does not contain "/new" then
                        return curUrl
                    end if
                    delay 1
                end repeat
                return curUrl
            end tell
            """
            res = subprocess.run(["osascript", "-e", apple_script], capture_output=True, text=True)
            url = res.stdout.strip()
            if not url or "meet.google.com" not in url:
                raise RuntimeError(f"Failed to create Google Meet call: {res.stderr}")
            self.meeting_url = url
            print(f"[OPENER] Real Google Meet room live: {self.meeting_url}")
            return self.meeting_url

        else:
            raise ValueError(f"Unknown opener mode: {self.mode}")

    def _start_local_server(self):
        socketserver.TCPServer.allow_reuse_address = True
        self.httpd = socketserver.TCPServer(("127.0.0.1", self.local_http_port), LocalMeetingHandler)
        self.http_thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.http_thread.start()

    def _start_host_browser(self, initial_url: str):
        os.makedirs(self.profile, exist_ok=True)
        cmd = [
            CHROME_BIN,
            f"--remote-debugging-port={self.port}",
            f"--user-data-dir={self.profile}",
            "--no-first-run",
            "--no-default-browser-check",
            "--use-fake-ui-for-media-stream",
            "--autoplay-policy=no-user-gesture-required",
            "--window-size=1200,800",
            initial_url,
        ]
        self.proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.page = CDPPage(self.port)
        self.page.call("Page.enable")
        print(f"[OPENER] Host Chrome running on port {self.port} (pid: {self.proc.pid})")

    def get_snapshot(self) -> dict:
        if not self.page or not SNAPSHOT_JS:
            return {}
        res = self.page.evaluate(SNAPSHOT_JS)
        return res if isinstance(res, dict) else {}

    def admit_pending_guests(self) -> bool:
        """Inspects host view for guest admission dialog and clicks Admit."""
        if self.mode == "google-meet":
            admit_script = """
            tell application "System Events"
                tell process "Google Chrome"
                    try
                        set btn to (first UI element of window 1 whose name is "Admit" or name contains "Admit")
                        click btn
                        return "admitted"
                    on error
                        return "none"
                    end try
                end tell
            end tell
            """
            try:
                res = subprocess.run(["osascript", "-e", admit_script], capture_output=True, text=True)
                if "admitted" in res.stdout:
                    print("[OPENER] Clicked 'Admit' for waiting guest!")
                    return True
            except Exception:
                pass
            return False

        if not self.page:
            return False
        snapshot = self.get_snapshot()
        admit_req = snapshot.get("admit_request")
        if admit_req and admit_req.get("admit_target"):
            print("[OPENER] Guest admission request detected! Clicking 'Admit'...")
            self.page.evaluate(f"""
            (() => {{
                const el = document.querySelector('[data-jev-index="{admit_req['admit_target']}"]');
                if (el) el.click();
            }})()
            """)
            return True

        # Check for any "Admit" or "Admit all" button
        for el in snapshot.get("elements", []):
            if re.search(r"\b(admit all|admit)\b", el.get("text", ""), re.I) and not el.get("disabled"):
                print(f"[OPENER] Found admit button [{el['index']}] '{el['text']}'. Admitting...")
                self.page.evaluate(f"""
                (() => {{
                    const el = document.querySelector('[data-jev-index="{el['index']}"]');
                    if (el) el.click();
                }})()
                """)
                return True

        return False

    def is_in_call(self) -> bool:
        snapshot = self.get_snapshot()
        return bool(snapshot.get("is_in_call"))

    def stop(self):
        if self.mode == "google-meet" and self.meeting_url:
            meet_slug = self.meeting_url.replace("https://", "")
            close_script = f"""
            tell application "Google Chrome"
                repeat with w in windows
                    try
                        if URL of active tab of w contains "{meet_slug}" then
                            close w
                            exit repeat
                        end if
                    end try
                end repeat
            end tell
            """
            try:
                subprocess.run(["osascript", "-e", close_script], capture_output=True)
            except Exception:
                pass

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
        if self.httpd:
            self.httpd.shutdown()
            self.httpd = None
        print(f"[OPENER] Meeting Opener stopped")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Taurscribe Meeting Opener")
    parser.add_argument("--mode", choices=["local", "google-meet"], default="local")
    parser.add_argument("--port", type=int, default=9111)
    args = parser.parse_args()

    opener = MeetingOpener(mode=args.mode, port=args.port)
    try:
        url = opener.start()
        print(f"Meeting live at: {url}")
        print("Press Ctrl+C to stop...")
        while True:
            opener.admit_pending_guests()
            time.sleep(1.0)
    except KeyboardInterrupt:
        pass
    finally:
        opener.stop()
