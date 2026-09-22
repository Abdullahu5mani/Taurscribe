#!/usr/bin/env python3
"""Local Meeting WebRTC/WebAudio Simulation Server for Taurscribe.

Serves an interactive meeting HTML page that plays real audio through Chrome's
audio subsystem, allowing the real CoreAudio meeting detector and dual-channel
system loopback capture to run 100% offline without requiring Google sign-in or CAPTCHA.
"""

from __future__ import annotations
import http.server
import socketserver
import threading
from pathlib import Path

PORT = 8999

HTML_CONTENT = """<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>Google Meet - Architecture Sync (meet.google.com/taur-sync-live)</title>
  <style>
    body {
      background: #202124;
      color: #fff;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      height: 100vh;
      margin: 0;
    }
    .call-card {
      background: #303134;
      padding: 32px 48px;
      border-radius: 12px;
      text-align: center;
      box-shadow: 0 4px 16px rgba(0,0,0,0.5);
    }
    h1 { margin: 0 0 12px; font-size: 22px; color: #8ab4f8; }
    p { margin: 0 0 24px; color: #9aa0a6; font-size: 14px; }
    .status-badge {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      background: #1e3a1e;
      color: #81c995;
      padding: 6px 14px;
      border-radius: 20px;
      font-size: 13px;
      font-weight: 500;
    }
    .pulse {
      width: 8px; height: 8px; border-radius: 50%; background: #34a853;
      animation: pulse 1.5s infinite;
    }
    @keyframes pulse { 0% { transform: scale(0.9); opacity: 0.7; } 50% { transform: scale(1.3); opacity: 1; } 100% { transform: scale(0.9); opacity: 0.7; } }
    .btn-join {
      background: #1a73e8; color: #fff; border: none; padding: 10px 24px;
      border-radius: 24px; font-size: 14px; font-weight: 600; cursor: pointer;
      margin-top: 20px;
    }
    .btn-join:hover { background: #1b66c9; }
  </style>
</head>
<body>
  <div class="call-card">
    <div class="status-badge"><span class="pulse"></span> Connected · Live Call</div>
    <h1 id="meeting-title" style="margin-top: 16px;">Architecture Sync</h1>
    <p>Google Meet call active · Audio playing via WebAudio destination</p>
    <button id="join-btn" class="btn-join" onclick="startAudio()">Join now</button>
  </div>

  <script>
    let audioCtx = null;
    let osc = null;
    let gain = null;

    function startAudio() {
      if (!audioCtx) {
        audioCtx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 48000 });
      }
      if (audioCtx.state !== 'running') {
        audioCtx.resume();
      }
      if (!osc) {
        osc = audioCtx.createOscillator();
        gain = audioCtx.createGain();
        gain.gain.value = 0.04;
        osc.type = 'sine';
        osc.frequency.setValueAtTime(440, audioCtx.currentTime);
        osc.connect(gain);
        gain.connect(audioCtx.destination);
        osc.start();
        document.getElementById('join-btn').innerText = 'Call in progress';
        document.getElementById('join-btn').disabled = true;
        console.log('[LOCAL_MEET] Continuous WebAudio oscillator active');
      }
    }

    // Expose WAV player for scripted test clips
    window.__playClipBase64 = async function(base64) {
      if (!audioCtx) {
        audioCtx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 48000 });
      }
      if (audioCtx.state !== 'running') await audioCtx.resume();
      const binary = atob(base64);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) {
        bytes[i] = binary.charCodeAt(i);
      }
      const buffer = await audioCtx.decodeAudioData(bytes.buffer);
      const src = audioCtx.createBufferSource();
      src.buffer = buffer;
      src.connect(audioCtx.destination);
      src.start();
      console.log('[LOCAL_MEET] Played audio clip (' + buffer.duration.toFixed(2) + 's)');
      return buffer.duration;
    };

    // Auto-start on load if requested via query param
    if (window.location.search.includes('autojoin=1')) {
      window.addEventListener('load', () => {
        setTimeout(startAudio, 200);
      });
    }
  </script>
</body>
</html>
"""


class MeetingHandler(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith("/meeting") or self.path == "/":
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(HTML_CONTENT.encode())))
            self.end_headers()
            self.wfile.write(HTML_CONTENT.encode())
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, format, *args):
        # Suppress noisy HTTP logs
        pass


def start_server(port: int = PORT) -> socketserver.TCPServer:
    socketserver.TCPServer.allow_reuse_address = True
    httpd = socketserver.TCPServer(("127.0.0.1", port), MeetingHandler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    return httpd


if __name__ == "__main__":
    print(f"[LOCAL_MEETING_SERVER] Serving on http://127.0.0.1:{PORT}/meeting")
    httpd = start_server(PORT)
    try:
        import time
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        httpd.shutdown()
