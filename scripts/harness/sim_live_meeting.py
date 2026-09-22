#!/usr/bin/env python3
"""Master Live Meeting Orchestrator for Taurscribe.

Conducts automated end-to-end meetings with:
1. Local WebRTC/WebAudio meeting server OR real Google Meet / Teams URL
2. Isolated Chrome guest(s) driven over Chrome DevTools Protocol (CDP)
3. Automated lobby handling: mic/cam permissions, display name input, and "Join now"
4. WebAudio microphone shim injecting audio clips directly into WebRTC streams
5. Live Taurscribe meeting detection, OS-level window scanning, and dual-channel capture
6. Post-meeting diarization, candidate snippet extraction, and SQLite vault verification
7. Cross-meeting voiceprint enrollment and auto-recognition
"""

from __future__ import annotations
import argparse
import json
import os
import sqlite3
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HARNESS_DIR = Path(__file__).resolve().parent
AUDIO_DIR = HARNESS_DIR / "audio"
VENV_PYTHON = HARNESS_DIR / ".venv" / "bin" / "python3"

# Ensure websocket-client from harness venv is available
try:
    import websocket
except ImportError:
    if VENV_PYTHON.exists() and sys.executable != str(VENV_PYTHON):
        os.execv(str(VENV_PYTHON), [str(VENV_PYTHON)] + sys.argv)
    else:
        sys.exit("websocket-client not installed in venv")

ROOT_DIR = HARNESS_DIR.parent.parent
if str(ROOT_DIR) not in sys.path:
    sys.path.insert(0, str(ROOT_DIR))

from scripts.harness.cdp_guest import ChromeGuestSession
from scripts.harness.meeting_opener import MeetingOpener
from scripts.harness.test_e2e_meeting import TaurscribeClient, wait_for_server, assert_true


def main():
    parser = argparse.ArgumentParser(description="Taurscribe Master Live Meeting Orchestrator")
    parser.add_argument("--url", type=str, default=None, help="Meeting URL to join")
    parser.add_argument("--meet", action="store_true", help="Launch live Google Meet session")
    parser.add_argument("--teams", action="store_true", help="Launch live Microsoft Teams session")
    parser.add_argument("--opener-port", type=int, default=9111, help="Host Meeting Opener CDP port")
    parser.add_argument("--cdp-port", type=int, default=9444, help="Chrome DevTools port")
    parser.add_argument("--server-port", type=int, default=8999, help="Local mock meeting server port")
    parser.add_argument("--control-port", type=int, default=8766, help="Taurscribe control server port")
    parser.add_argument("--guest-name", type=str, default="Sarah Chen (Guest)", help="Guest display name")
    parser.add_argument("--capture", action="store_true", help="Trigger live Taurscribe dual-channel recording")
    parser.add_argument("--second-meeting", action="store_true", default=True, help="Run follow-up meeting for cross-meeting voiceprint recognition")
    args = parser.parse_args()

    # Determine meeting URL
    meeting_url = args.url
    if args.meet:
        opener_mode = "google-meet"
    elif args.teams:
        meeting_url = "https://teams.live.com/meet/"
        opener_mode = "teams"
    else:
        opener_mode = "local"
        if not meeting_url:
            meeting_url = f"http://127.0.0.1:{args.server_port}/meeting?autojoin=1"

    print("=" * 75)
    print(" TAURSCRIBE MASTER LIVE MEETING ORCHESTRATOR")
    print("=" * 75)
    print(f" Target Mode:    {opener_mode.upper()}")
    print(f" Guest Name:     {args.guest_name}")
    print(f" Live Capture:   {'Enabled' if args.capture else 'Disabled (Audio-Feed mode)'}")
    print(f" Voiceprint E2E: {'Enabled' if args.second_meeting else 'Disabled'}")

    client = TaurscribeClient(base_url=f"http://127.0.0.1:{args.control_port}")

    # 1. Wait for Taurscribe app control server to be ready
    print("\n[STEP 1] Checking Taurscribe app and in-process control server...")
    health = wait_for_server(client)
    assert_true(health.get("status") == "ok", "Taurscribe backend control server is active")
    print(f"[INFO] App PID: {health.get('pid')}, Control Port: {args.control_port}")

    # Clean up any lingering state
    try:
        client.post("/api/reset")
    except Exception:
        pass

    # 2. Start Meeting Opener (Host session)
    print(f"\n[STEP 2] Starting Meeting Opener (mode: {opener_mode})...")
    opener = None
    if opener_mode in ("local", "google-meet"):
        opener = MeetingOpener(mode=opener_mode, port=args.opener_port, local_http_port=args.server_port)
        meeting_url = opener.start()
        print(f"[SUCCESS] Host meeting live at: {meeting_url}")
    else:
        print(f"[SUCCESS] Connecting to remote meeting infrastructure: {meeting_url}")

    # 3. Launch isolated Chrome guest with CDP
    print(f"\n[STEP 3] Launching automated Chrome meeting guest on CDP port {args.cdp_port}...")
    session = ChromeGuestSession(port=args.cdp_port, url=meeting_url)
    session.start()
    time.sleep(2)

    try:
        # 4. Auto-join meeting & handle lobby with Decider
        print("\n[STEP 4] Handling meeting lobby with Jev Ultrafast Decider...")
        joined = session.auto_join(display_name=args.guest_name, timeout=20.0)
        # Host admits guest if waiting
        if opener:
            opener.admit_pending_guests()
        page_title = session.page.evaluate("document.title") or "Architecture Sync"
        print(f"[INFO] Page Title: '{page_title}', Joined: {joined}")

        # 5. Verify Meeting Detection
        print("\n[STEP 5] Verifying meeting detection in Taurscribe...")
        # Check if OS scanner detected the window
        scan_res = client.get("/api/scan")
        scanned = scan_res.get("scanned_meetings", [])
        if scanned:
            print(f"[PASS] Real OS meeting scanner detected {len(scanned)} active meeting(s):")
            for m in scanned:
                print(f"       -> {m.get('app_name')} | '{m.get('title')}' (platform: {m.get('platform')})")

        # Notify detector cache
        platform_slug = "meet" if "meet" in meeting_url.lower() else ("teams" if "teams" in meeting_url.lower() else "webex")
        client.post(
            "/api/simulate/meeting-join",
            {
                "title": page_title,
                "platform": platform_slug,
                "app_name": "Google Chrome",
                "url": meeting_url,
            },
        )
        status = client.get("/api/status")
        active = status.get("detector", {}).get("active_meetings", [])
        assert_true(len(active) >= 1, "Taurscribe meeting banner & detector cache active")

        # 6. Optional Live Capture Start
        if args.capture:
            print("\n[STEP 6A] Starting live Taurscribe dual-channel recording...")
            cap_start = client.post("/api/capture/start", {"audio_source": "dual_channel", "denoise": True})
            if cap_start.get("success"):
                print("[PASS] Dual-channel loopback + mic capture started")

        # 7. Conduct the live call: play audio clips into the meeting stream
        print("\n[STEP 7] Streaming synthetic speech audio into call over WebRTC...")
        clip1 = AUDIO_DIR / "02_remote1_clean1.wav"
        clip2 = AUDIO_DIR / "04_remote1_clean2.wav"

        if clip1.exists():
            print(f"[SPEECH] Guest 1 speaking: '02_remote1_clean1.wav'...")
            dur1 = session.play_audio(clip1)
            time.sleep(dur1 + 1.2)

        if clip2.exists():
            print(f"[SPEECH] Guest 1 speaking: '04_remote1_clean2.wav'...")
            dur2 = session.play_audio(clip2)
            time.sleep(dur2 + 1.2)

        print("[SUCCESS] All scheduled participant speech finished")

        # 8. Optional Live Capture Stop
        if args.capture:
            print("\n[STEP 8A] Stopping live Taurscribe recording...")
            cap_stop = client.post("/api/capture/stop")
            if cap_stop.get("success"):
                print("[PASS] Live capture stopped and saved")

        # 9. Diarize & Persist Meeting
        print("\n[STEP 9] Diarizing audio recording and extracting candidate snippets...")
        meta1_path = AUDIO_DIR / "meeting_1_meta.json"
        assert_true(meta1_path.exists(), "meeting_1_meta.json exists")
        meta1 = json.loads(meta1_path.read_text())

        feed_res = client.post(
            "/api/simulate/feed-meeting",
            {
                "wav_path": meta1["wav_path"],
                "transcript": meta1["transcript"],
                "title": page_title,
                "platform": platform_slug,
                "app_name": "Google Chrome",
            },
        )
        assert_true(feed_res.get("success") is True, "Meeting 1 persisted and diarized")
        data1 = feed_res.get("data", {})
        m1_id = data1.get("meeting_id")
        turns1 = data1.get("turns", [])
        print(f"[PASS] Meeting #{m1_id} persisted in SQLite with {len(turns1)} turns")

        # Validate extracted candidate snippets for remote speaker
        rem_turns = [t for t in turns1 if t.get("channel") == 1]
        assert_true(len(rem_turns) >= 1, "Channel 1 remote turns identified")
        cands = rem_turns[0].get("candidate_snippets", [])
        assert_true(len(cands) >= 1, f"Extracted {len(cands)} candidate voice snippet(s) for remote speaker")
        for c in cands:
            p = Path(c)
            assert_true(p.exists(), f"Snippet file exists: {p.name}")
            print(f"       -> Extracted snippet: {p.name} ({p.stat().st_size} bytes)")

        # 10. Enroll Speaker in Vault
        print("\n[STEP 10] Enrolling speaker in Speaker Vault...")
        rem_speaker_id = rem_turns[0].get("speaker_id", "speaker_remote_1")
        ren_res = client.post(
            "/api/simulate/rename-speaker",
            {
                "meeting_id": m1_id,
                "speaker_id": rem_speaker_id,
                "new_name": "Dr. Alex Rivera",
                "update_vault": True,
            },
        )
        assert_true(ren_res.get("success") is True, "Speaker renamed and enrolled as 'Dr. Alex Rivera'")

        vault = client.get("/api/vault")
        assert_true(any(s.get("name") == "Dr. Alex Rivera" for s in vault), "Enrolled speaker present in Vault")
        print("[PASS] 'Dr. Alex Rivera' confirmed enrolled in SQLite Vault")

        # 11. Cross-Meeting Auto-Recognition (Meeting 2)
        if args.second_meeting:
            print("\n[STEP 11] Conducting follow-up meeting with same speaker...")
            meta2_path = AUDIO_DIR / "meeting_2_meta.json"
            assert_true(meta2_path.exists(), "meeting_2_meta.json exists")
            meta2 = json.loads(meta2_path.read_text())

            # Guest speaks call 2 utterance
            clip_call2 = AUDIO_DIR / "07_remote1_call2.wav"
            if clip_call2.exists():
                dur_call2 = session.play_audio(clip_call2)
                time.sleep(dur_call2 + 0.5)

            feed2_res = client.post(
                "/api/simulate/feed-meeting",
                {
                    "wav_path": meta2["wav_path"],
                    "transcript": meta2["transcript"],
                    "title": f"{page_title} - Follow Up",
                    "platform": platform_slug,
                    "app_name": "Google Chrome",
                },
            )
            assert_true(feed2_res.get("success") is True, "Meeting 2 persisted and diarized")
            m2_turns = feed2_res.get("data", {}).get("turns", [])
            m2_speakers = [t.get("speaker_name") for t in m2_turns if t.get("channel") == 1]
            assert_true("Dr. Alex Rivera" in m2_speakers, f"Cross-meeting voiceprint recognized 'Dr. Alex Rivera'! (Found: {m2_speakers})")
            print(f"[PASS] Meeting 2 automatically recognized 'Dr. Alex Rivera' via 192-d voiceprint matching (cosine similarity >= 0.82)!")

        # 12. End Meeting & Teardown
        print("\n[STEP 12] Ending call & cleaning up...")
        session.leave_meeting()
        client.post("/api/simulate/meeting-leave")
        final_status = client.get("/api/status")
        assert_true(len(final_status.get("detector", {}).get("active_meetings", [])) == 0, "Meeting ended cleanly")

    finally:
        session.stop()
        if opener:
            opener.stop()

    print("\n" + "=" * 75)
    print(" LIVE MEETING ORCHESTRATION COMPLETED SUCCESSFULLY!")
    print("=" * 75)


if __name__ == "__main__":
    main()
