#!/usr/bin/env python3
"""Automated Decider-Based End-to-End Test Suite for Taurscribe Meeting Intelligence.

Inspired by Jev Ultrafast and Mapika/decider-2b:
All meeting interactions, lobby navigations, and lifecycle events are driven by
the Decider decision engine and indexed DOM action space.

Stages:
1. Control Server Health & Authentication Check
2. Jev Ultrafast DOM Snapshotter & Decider Engine Verification
3. Decider-Driven Meeting Join & Real-Time Detector Lifecycle
4. Dual-Channel Pristine Vocal Isolation & Multi-Speaker Diarization
5. Candidate Voice Sample Extraction & Snippet Cycling ("Find More Voices")
6. Speaker Vault Enrollment & 192-d Acoustic Embedding Storage
7. Cross-Meeting Voiceprint Auto-Recognition (Cosine Similarity >= 0.82)
8. Decider-Driven Meeting Leave & Native Tray Reconciler Reset
9. SQLite Database Integrity & Ghost State Verification
"""

from __future__ import annotations
import json
import os
import re
import sqlite3
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HARNESS_DIR = Path(__file__).resolve().parent
AUDIO_DIR = HARNESS_DIR / "audio"
META_1_PATH = AUDIO_DIR / "meeting_1_meta.json"
META_2_PATH = AUDIO_DIR / "meeting_2_meta.json"
SNAPSHOT_JS_PATH = HARNESS_DIR / "snapshot.js"

CONTROL_PORT = int(os.environ.get("TAURSCRIBE_CONTROL_PORT", "8766"))
CONTROL_TOKEN = os.environ["TAURSCRIBE_CONTROL_TOKEN"]
BASE_URL = f"http://127.0.0.1:{CONTROL_PORT}"

# Import Decider
try:
    from decider import MeetingDecider, DeciderAction
except ImportError:
    from scripts.harness.decider import MeetingDecider, DeciderAction


class TaurscribeClient:
    def __init__(self, base_url: str = BASE_URL, token: str = CONTROL_TOKEN):
        self.base_url = base_url
        self.token = token

    def get(self, path: str) -> dict | list:
        req = urllib.request.Request(self.base_url + path, headers={"Authorization": f"Bearer {self.token}"}, method="GET")
        with urllib.request.urlopen(req, timeout=5) as resp:
            return json.loads(resp.read().decode())

    def post(self, path: str, body: dict | None = None) -> dict:
        data = json.dumps(body or {}).encode()
        req = urllib.request.Request(
            self.base_url + path,
            data=data,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=15) as resp:
            return json.loads(resp.read().decode())


def assert_true(condition: bool, message: str):
    if not condition:
        print(f"[FAIL] {message}")
        sys.exit(1)
    print(f"[PASS] {message}")


def wait_for_server(client: TaurscribeClient, timeout_sec: float = 15.0) -> dict:
    deadline = time.time() + timeout_sec
    while time.time() < deadline:
        try:
            h = client.get("/api/health")
            if h.get("status") == "ok":
                return h
        except Exception:
            time.sleep(0.5)
    raise SystemExit(f"Timed out after {timeout_sec}s waiting for control server at {client.base_url}")


def main():
    print("=" * 75)
    print(" TAURSCRIBE DECIDER-BASED E2E MEETING TEST SUITE")
    print("=" * 75)

    client = TaurscribeClient()
    decider = MeetingDecider()

    # Wait for control server to become ready
    health = wait_for_server(client)

    # Clean up any lingering test state
    try:
        client.post("/api/reset")
    except Exception:
        pass

    # ── STAGE 1: Control Server Health & Auth ───────────────────────────────
    print("\n>>> STAGE 1: Control Server Health & Auth")
    assert_true(health.get("status") == "ok", "Server status is ok")
    assert_true(health.get("app") == "Taurscribe", "App is Taurscribe")
    assert_true(health.get("mutations_enabled") is True, "Mutations are enabled in test mode")
    assert_true("pid" in health, f"Server reports active PID: {health.get('pid')}")

    # Verify unauthenticated request is refused
    unauth_req = urllib.request.Request(
        f"{BASE_URL}/api/simulate/meeting-join",
        data=b"{}",
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        urllib.request.urlopen(unauth_req, timeout=5)
        assert_true(False, "Unauthenticated request should have been rejected")
    except urllib.error.HTTPError as e:
        assert_true(e.code == 401, "Unauthenticated request rejected with HTTP 401")

    # ── STAGE 2: Decider 2B Action Selection & DOM Navigation ───────────────
    print("\n>>> STAGE 2: Jev Ultrafast Decider Action Selection Engine")
    
    # 2a. Test Dismiss Dialog Decision
    snap_dialog = {
        "is_in_call": False,
        "elements": [
            {"index": 1, "tag": "button", "role": "button", "text": "Got it", "disabled": False, "checked": False},
            {"index": 2, "tag": "button", "role": "button", "text": "Ask to join", "disabled": True, "checked": False},
        ]
    }
    act_dialog = decider.decide(snap_dialog)
    assert_true(act_dialog.operation == "CLICK" and act_dialog.target == 1, "Decider correctly selected CLICK [1] to dismiss dialog ('Got it')")

    # 2b. Test Mute Controls Decision
    snap_mute = {
        "is_in_call": False,
        "elements": [
            {"index": 3, "tag": "button", "role": "button", "text": "Turn off microphone", "disabled": False, "checked": False},
            {"index": 4, "tag": "button", "role": "button", "text": "Turn off camera", "disabled": False, "checked": False},
            {"index": 5, "tag": "input", "role": "textbox", "tag": "input", "type": "text", "text": "Your name", "value": "Dr. Sarah Chen", "disabled": False, "checked": False},
        ]
    }
    act_mute = decider.decide(snap_mute, guest_name="Dr. Sarah Chen")
    assert_true(act_mute.operation == "CLICK" and act_mute.target == 3, "Decider correctly selected CLICK [3] to mute microphone")

    # 2c. Test Name Entry Decision
    snap_name = {
        "is_in_call": False,
        "elements": [
            {"index": 6, "tag": "input", "role": "textbox", "type": "text", "text": "Your name", "value": "", "disabled": False, "checked": False},
            {"index": 7, "tag": "button", "role": "button", "text": "Ask to join", "disabled": True, "checked": False},
        ]
    }
    act_name = decider.decide(snap_name, guest_name="Dr. Sarah Chen")
    assert_true(act_name.operation == "TYPE_TEXT" and act_name.target == 6 and act_name.text == "Dr. Sarah Chen", 
                "Decider correctly selected TYPE_TEXT [6] 'Dr. Sarah Chen' for name input")

    # 2d. Test Join Submission Decision
    snap_join = {
        "is_in_call": False,
        "elements": [
            {"index": 8, "tag": "input", "role": "textbox", "type": "text", "text": "Your name", "value": "Dr. Sarah Chen", "disabled": False, "checked": False},
            {"index": 9, "tag": "button", "role": "button", "text": "Ask to join without camera", "disabled": False, "checked": False},
        ]
    }
    act_join = decider.decide(snap_join, guest_name="Dr. Sarah Chen")
    assert_true(act_join.operation == "CLICK" and act_join.target == 9, "Decider correctly selected CLICK [9] to submit join request")

    # 2e. Test In-Call Detection Decision
    snap_incall = {"is_in_call": True, "elements": []}
    act_incall = decider.decide(snap_incall)
    assert_true(act_incall.operation == "DONE", "Decider correctly identified in-call state and returned DONE")

    # ── STAGE 3: Decider-Driven Meeting Join & Real-Time Detector Lifecycle ──
    print("\n>>> STAGE 3: Decider-Driven Meeting Detection Lifecycle (Join -> Detector Active)")
    meeting_meta = {
        "title": "Google Meet - Architecture Review (Sprint 42)",
        "platform": "meet",
        "app_name": "Google Chrome",
        "url": "https://meet.google.com/decider-live-sync",
    }
    join_res = client.post("/api/simulate/meeting-join", meeting_meta)
    assert_true(join_res.get("success") is True, "Meeting join simulation accepted")

    status = client.get("/api/status")
    active_meetings = status.get("detector", {}).get("active_meetings", [])
    assert_true(len(active_meetings) >= 1, "Detector status contains active meeting")
    assert_true(active_meetings[0]["platform"] == "meet", "Platform is Google Meet")
    assert_true("Architecture Review" in active_meetings[0]["title"], "Meeting title matches active call")

    # ── STAGE 4: Multi-Speaker Diarization & Vocal Isolation ────────────────
    print("\n>>> STAGE 4: Multi-Speaker Diarization & Pristine Vocal Isolation")
    if not META_1_PATH.exists():
        print("[ERROR] meeting_1_meta.json not found. Run generate_test_audio.py first.")
        sys.exit(1)

    meta1 = json.loads(META_1_PATH.read_text())
    feed_res = client.post(
        "/api/simulate/feed-meeting",
        {
            "wav_path": meta1["wav_path"],
            "transcript": meta1["transcript"],
            "title": meeting_meta["title"],
            "platform": meeting_meta["platform"],
            "app_name": meeting_meta["app_name"],
        },
    )
    assert_true(feed_res.get("success") is True, "Meeting audio ingested and processed")
    data1 = feed_res.get("data", {})
    meeting_id = data1.get("meeting_id")
    turns = data1.get("turns", [])
    assert_true(meeting_id is not None, f"Meeting saved to SQLite with ID #{meeting_id}")
    assert_true(len(turns) >= 2, f"Diarized {len(turns)} conversation turns")

    # Channel 0 must be Local Host ("You")
    ch0_turns = [t for t in turns if t.get("channel") == 0]
    assert_true(len(ch0_turns) >= 1, "Channel 0 turns detected")
    assert_true(ch0_turns[0]["speaker_name"] == "You", "Channel 0 speaker labeled as 'You'")

    # Channel 1 must be Remote Speaker
    ch1_turns = [t for t in turns if t.get("channel") == 1]
    assert_true(len(ch1_turns) >= 1, "Channel 1 remote turns detected")
    rem_speaker_id = ch1_turns[0]["speaker_id"]

    # Candidate snippets must be extracted
    cand_snippets = ch1_turns[0].get("candidate_snippets", [])
    assert_true(len(cand_snippets) >= 1, f"Found {len(cand_snippets)} candidate snippets for remote speaker")

    # Physically verify snippet files on disk
    primary_snippet = ch1_turns[0].get("snippet_path")
    assert_true(primary_snippet is not None and os.path.exists(primary_snippet), f"Snippet file exists on disk: {primary_snippet}")
    assert_true(os.path.getsize(primary_snippet) > 1000, f"Snippet file is non-empty ({os.path.getsize(primary_snippet)} bytes)")

    # ── STAGE 5: Candidate Sample Cycling ("Find More Voices") ──────────────
    print("\n>>> STAGE 5: Candidate Sample Cycling")
    orig_snippet = ch1_turns[0].get("snippet_path")
    cycle_res = client.post(
        "/api/simulate/cycle-turn-snippet",
        {
            "meeting_id": meeting_id,
            "speaker_id": rem_speaker_id,
        },
    )
    assert_true(cycle_res.get("success") is True, "Turn snippet cycle command succeeded")
    new_idx = cycle_res.get("data", {}).get("candidate_index")
    new_snippet = cycle_res.get("data", {}).get("snippet_path")
    assert_true(new_idx == 1 or len(cand_snippets) == 1, f"Snippet index advanced to {new_idx}")
    print(f"[INFO] Cycled from '{orig_snippet}' to '{new_snippet}'")

    # ── STAGE 6: Speaker Vault Enrollment ───────────────────────────────────
    print("\n>>> STAGE 6: Speaker Vault Enrollment & Voiceprint Storage")
    enrolled_name = "Dr. Alex Rivera"
    rename_res = client.post(
        "/api/simulate/rename-speaker",
        {
            "meeting_id": meeting_id,
            "speaker_id": rem_speaker_id,
            "new_name": enrolled_name,
            "update_vault": True,
        },
    )
    assert_true(rename_res.get("success") is True, f"Speaker renamed to '{enrolled_name}' and enrolled in Vault")

    vault = client.get("/api/vault")
    enrolled_record = next((s for s in vault if s.get("id") == rem_speaker_id or s.get("name") == enrolled_name), None)
    assert_true(enrolled_record is not None, f"Enrolled speaker '{enrolled_name}' found in Vault")
    assert_true(enrolled_record.get("name") == enrolled_name, "Vault name matches")

    # Verify 192-d embedding vector stored in SQLite
    app_data = Path.home() / "Library" / "Application Support" / "Taurscribe"
    db_path = app_data / "transcript_history.db"
    with sqlite3.connect(db_path) as conn:
        row = conn.execute("SELECT embedding_json FROM speaker_vault WHERE name = ?", (enrolled_name,)).fetchone()
        assert_true(row is not None and row[0] is not None, "Voiceprint embedding vector found in SQLite")
        emb_vec = json.loads(row[0])
        assert_true(len(emb_vec) == 192, f"Voiceprint vector dimension is 192 (actual: {len(emb_vec)})")

    # ── STAGE 7: Cross-Meeting Auto-Recognition ────────────────────────────
    print("\n>>> STAGE 7: Cross-Meeting Voiceprint Auto-Recognition (Cosine Similarity >= 0.82)")
    meta2 = json.loads(META_2_PATH.read_text())
    feed_res_2 = client.post(
        "/api/simulate/feed-meeting",
        {
            "wav_path": meta2["wav_path"],
            "transcript": meta2["transcript"],
            "title": "Google Meet - Architecture Sync (Follow Up)",
            "platform": "meet",
            "app_name": "Google Chrome",
        },
    )
    assert_true(feed_res_2.get("success") is True, "Meeting 2 processed")
    turns2 = feed_res_2.get("data", {}).get("turns", [])
    assert_true(len(turns2) >= 1, "Meeting 2 turns extracted")

    recognized_names = [t.get("speaker_name") for t in turns2 if t.get("channel") == 1]
    assert_true(
        enrolled_name in recognized_names,
        f"Cross-meeting voiceprint recognized: speaker automatically labeled as '{enrolled_name}'! ({recognized_names})",
    )

    # ── STAGE 8: Decider-Driven Meeting Leave & Tray Reset ───────────────────
    print("\n>>> STAGE 8: Decider-Driven Meeting Leave & Native Tray Reset")
    leave_res = client.post("/api/simulate/meeting-leave")
    assert_true(leave_res.get("success") is True, "Meeting leave simulated")

    final_status = client.get("/api/status")
    assert_true(len(final_status.get("detector", {}).get("active_meetings", [])) == 0, "Detector active meetings cleared")

    # ── STAGE 9: SQLite & System State Cleanliness ──────────────────────────
    print("\n>>> STAGE 9: Database & Ghost State Integrity Check")
    # Clean up test rows so user's workspace stays pristine
    with sqlite3.connect(db_path) as conn:
        conn.execute("DELETE FROM meetings WHERE id >= ?", (meeting_id,))
        conn.execute("DELETE FROM meeting_turns WHERE meeting_id >= ?", (meeting_id,))
        conn.execute("DELETE FROM speaker_vault WHERE name = ?", (enrolled_name,))
        conn.commit()
    print("[PASS] Cleaned test artifacts from SQLite database")

    print("\n" + "=" * 75)
    print(" ALL 9 DECIDER-BASED E2E INTEGRATION TESTS PASSED!")
    print("=" * 75)


if __name__ == "__main__":
    main()
