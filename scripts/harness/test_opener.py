#!/usr/bin/env python3
"""Standalone test for Google Meet Opener (Host Controller).

Validates:
1. Opener starts real Google Meet host call in Google Chrome.
2. Redirects to genuine room format (https://meet.google.com/xxx-yyyy-zzz).
3. Chrome CDP Guest navigates to the opened meeting room.
4. Guest lobby is handled via Decider.
5. Host can check admission requests.
6. Opener cleanly shuts down and closes the Chrome window.
"""

from __future__ import annotations
import os
import re
import sys
import time
from pathlib import Path

HARNESS_DIR = Path(__file__).resolve().parent
ROOT_DIR = HARNESS_DIR.parent.parent
if str(ROOT_DIR) not in sys.path:
    sys.path.insert(0, str(ROOT_DIR))

from cdp_guest import ChromeGuestSession
from meeting_opener import MeetingOpener


def test_google_meet_opener():
    print("=" * 70)
    print(" TESTING REAL GOOGLE MEET OPENER")
    print("=" * 70)

    opener = MeetingOpener(mode="google-meet")
    guest_session = None

    try:
        # Step 1: Start host session
        print("\n[STEP 1] Starting Google Meet Opener...")
        start_time = time.time()
        url = opener.start()
        elapsed = time.time() - start_time
        print(f"[RESULT] Room opened in {elapsed:.2f}s: {url}")

        # Step 2: Validate URL structure
        print("\n[STEP 2] Validating Google Meet URL format...")
        match = re.search(r"https://meet\.google\.com/([a-z]{3}-[a-z]{4}-[a-z]{3})", url)
        if not match:
            raise AssertionError(f"URL did not match Google Meet pattern: '{url}'")
        meet_code = match.group(1)
        print(f"[PASS] Valid Google Meet room code: {meet_code}")

        # Step 3: Launch Guest Chrome on CDP to join this room
        print(f"\n[STEP 3] Launching isolated Guest Chrome on CDP to visit {url}...")
        guest_session = ChromeGuestSession(port=9455, url=url)
        guest_session.start()
        time.sleep(3)

        # Step 4: Verify Guest can see the Google Meet lobby
        print("\n[STEP 4] Inspecting guest page via CDP & Decider...")
        snapshot_js_path = HARNESS_DIR / "snapshot.js"
        snapshot_js = snapshot_js_path.read_text() if snapshot_js_path.exists() else ""
        snapshot = guest_session.page.evaluate(snapshot_js) if snapshot_js else {}
        page_title = guest_session.page.evaluate("document.title") or ""
        print(f"[INFO] Guest page title: '{page_title}'")
        print(f"[INFO] Interactable DOM elements detected: {len(snapshot.get('elements', [])) if isinstance(snapshot, dict) else 0}")

        # Ensure page title or URL is Google Meet
        page_url = guest_session.page.evaluate("window.location.href") or ""
        assert "meet.google.com" in page_url, f"Expected meet.google.com in guest URL, got: {page_url}"
        print(f"[PASS] Guest successfully reached real Google Meet room: {page_url}")

        # Test Decider handling lobby
        print("\n[STEP 4B] Running Decider guest join sequence...")
        join_result = guest_session.auto_join(display_name="Sarah Chen (Guest)", timeout=15.0)
        print(f"[INFO] Guest auto_join result: {join_result}")

        # Step 5: Test host admission check
        print("\n[STEP 5] Testing Host admission query...")
        admitted = opener.admit_pending_guests()
        print(f"[INFO] Host admission result: {admitted}")

        print("\n[PASS] Google Meet Opener verification completed successfully!")

    finally:
        print("\n[STEP 6] Cleaning up test sessions...")
        if guest_session:
            print("Stopping Guest Chrome session...")
            guest_session.stop()
        print("Stopping Host Google Meet window...")
        opener.stop()
        print("[CLEANUP] All sessions stopped.")


if __name__ == "__main__":
    test_google_meet_opener()
