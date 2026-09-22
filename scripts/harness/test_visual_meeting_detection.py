#!/usr/bin/env python3
"""Visual Verification Suite for Taurscribe Meeting Detection UI.

Validates:
1. Dynamic discovery of Taurscribe desktop window ID via macOS Quartz / CoreGraphics.
2. Baseline capture of idle state: no meeting banner, standard titlebar.
3. Live meeting injection: triggers both TitleBar MeetingHeaderPill and MeetingBanner.
4. Computer-vision pixel analysis (Pillow):
   - Header Pill: detects active green status indicator (R < 60, G > 160, B < 80) in TitleBar.
   - Meeting Banner: detects presence of red record CTA button (R > 190, G < 80, B < 80).
   - Structural visual delta: confirms significant UI layout change (> 5,000 modified pixels).
5. Meeting leave teardown: verifies banner and header pill cleanly demount.
6. Exports high-resolution before/during/after PNG artifacts to conversation directory.
"""

from __future__ import annotations
import json
import os
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from PIL import Image, ImageChops

HARNESS_DIR = Path(__file__).resolve().parent
ROOT_DIR = HARNESS_DIR.parent.parent
ARTIFACTS_DIR = Path("/Users/abdullahusmani/.gemini/antigravity/brain/842a84bb-eafe-4f04-94bd-93f838fba900")
ARTIFACTS_DIR.mkdir(parents=True, exist_ok=True)

CONTROL_URL = "http://127.0.0.1:8766"
PROOF_TOKEN = os.environ["TAURSCRIBE_CONTROL_TOKEN"]


def get_taurscribe_window_id() -> int:
    """Finds the CoreGraphics window ID for the running Taurscribe desktop window."""
    swift_cmd = """
    import Cocoa
    import CoreGraphics

    let options = CGWindowListOption(arrayLiteral: .optionOnScreenOnly, .excludeDesktopElements)
    let windows = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
    for w in windows {
        let name = w[kCGWindowOwnerName as String] as? String ?? ""
        let wid = w[kCGWindowNumber as String] as? CGWindowID ?? 0
        let bounds = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
        let h = bounds["Height"] as? Int ?? 0
        if name.lowercased().contains("taurscribe") && h > 200 {
            print("\(wid)")
            break
        }
    }
    """
    res = subprocess.run(["swift", "-e", swift_cmd], capture_output=True, text=True)
    out = res.stdout.strip()
    if not out.isdigit():
        raise RuntimeError(f"Could not find Taurscribe window ID (output: '{out}')")
    return int(out)


def capture_window(wid: int, dest_path: Path):
    """Captures an exact window screenshot via macOS screencapture."""
    res = subprocess.run(["screencapture", "-l", str(wid), str(dest_path)], capture_output=True)
    if res.returncode != 0:
        raise RuntimeError(f"screencapture failed: {res.stderr.decode('utf-8')}")
    if not dest_path.exists() or dest_path.stat().st_size == 0:
        raise RuntimeError(f"Screenshot file was not created at {dest_path}")


def send_control(path: str, data: dict | None = None) -> dict:
    url = f"{CONTROL_URL}{path}"
    headers = {"Authorization": f"Bearer {PROOF_TOKEN}"}
    body = None
    if data is not None:
        headers["Content-Type"] = "application/json"
        body = json.dumps(data).encode("utf-8")
    req = urllib.request.Request(url, data=body, headers=headers, method="POST" if data is not None else "GET")
    with urllib.request.urlopen(req, timeout=5) as resp:
        return json.loads(resp.read().decode("utf-8"))


def count_color_matches(img: Image.Image, r_range, g_range, b_range, box=None) -> int:
    """Counts pixels matching specific RGB ranges within an optional bounding box."""
    target_img = img.crop(box) if box else img
    pixels = target_img.convert("RGB").getdata()
    count = 0
    for r, g, b in pixels:
        if (r_range[0] <= r <= r_range[1] and
            g_range[0] <= g <= g_range[1] and
            b_range[0] <= b <= b_range[1]):
            count += 1
    return count


def run_visual_verification():
    print("=" * 75)
    print(" TAURSCRIBE VISUAL MEETING DETECTION TEST SUITE")
    print("=" * 75)

    # 1. Verify backend control server
    print("\n[STEP 1] Checking Taurscribe in-process control server...")
    health = send_control("/api/health")
    print(f"[PASS] Connected to {health['app']} v{health['version']} (pid: {health['pid']})")

    # Ensure clean starting state
    send_control("/api/simulate/meeting-leave", {})
    time.sleep(1.0)

    # 2. Locate window ID
    print("\n[STEP 2] Discovering macOS CoreGraphics Window ID...")
    wid = get_taurscribe_window_id()
    print(f"[PASS] Located Taurscribe desktop window (Window ID: {wid})")

    # 3. Capture baseline idle state
    print("\n[STEP 3] Capturing baseline IDLE state...")
    img_idle_path = ARTIFACTS_DIR / "visual_verify_1_idle.png"
    capture_window(wid, img_idle_path)
    img_idle = Image.open(img_idle_path)
    w, h = img_idle.size
    print(f"[PASS] Baseline saved to {img_idle_path.name} (Resolution: {w}x{h}, {img_idle_path.stat().st_size // 1024} KB)")

    # Verify no red record buttons in idle state
    # Red record button color: R > 200, G < 80, B < 80
    idle_red_pixels = count_color_matches(img_idle, (200, 255), (0, 80), (0, 80), box=(0, 0, w, int(h * 0.35)))
    print(f"[INFO] Red CTA pixels in upper 35% of idle window: {idle_red_pixels}")
    assert idle_red_pixels < 20, f"Unexpected red record button detected in idle state ({idle_red_pixels} pixels)"

    # 4. Trigger meeting detection
    print("\n[STEP 4] Dispatching meeting-detected event (Sprint Architecture Review)...")
    meeting_payload = {
        "title": "Sprint Architecture Review - Google Meet",
        "platform": "meet",
        "app_name": "Google Chrome",
        "url": "https://meet.google.com/out-wekh-iwf"
    }
    join_res = send_control("/api/simulate/meeting-join", meeting_payload)
    assert join_res.get("success") is True, "Meeting join request accepted"
    print(f"[PASS] Injected meeting: '{join_res['data']['title']}' (pid: {join_res['data']['pid']})")

    # Allow React component mount & CSS animations to settle
    time.sleep(1.5)

    # 5. Capture meeting detected state
    print("\n[STEP 5] Capturing DETECTED state...")
    img_active_path = ARTIFACTS_DIR / "visual_verify_2_meeting_detected.png"
    capture_window(wid, img_active_path)
    img_active = Image.open(img_active_path)
    print(f"[PASS] Active state saved to {img_active_path.name} ({img_active_path.stat().st_size // 1024} KB)")

    # 6. Computer Vision Visual Analysis
    print("\n[STEP 6] Analyzing visual layout & pixel differences...")

    # A. Header Pill green status check (R < 60, G > 160, B < 120 in titlebar region)
    titlebar_box = (0, 0, w, int(h * 0.12))
    active_green_pixels = count_color_matches(img_active, (0, 60), (160, 255), (0, 120), box=titlebar_box)
    print(f"[VISUAL_CV] Green meeting status pill pixels in TitleBar: {active_green_pixels}")
    assert active_green_pixels >= 15, f"Expected active green meeting indicator in TitleBar, found {active_green_pixels} pixels"
    print("[PASS] TitleBar MeetingHeaderPill successfully detected visually!")

    # B. MeetingBanner red record CTA check (R > 200, G < 80, B < 80 in banner region)
    banner_box = (0, int(h * 0.08), w, int(h * 0.25))
    active_red_pixels = count_color_matches(img_active, (200, 255), (0, 80), (0, 80), box=banner_box)
    print(f"[VISUAL_CV] Red 'Record Call' CTA pixels in MeetingBanner: {active_red_pixels}")
    assert active_red_pixels >= 100, f"Expected red record button in MeetingBanner, found {active_red_pixels} pixels"
    print("[PASS] MeetingBanner '[ 🔴 Record Call ]' CTA successfully detected visually!")

    # C. Full image difference verification
    diff = ImageChops.difference(img_idle, img_active)
    # Count pixels that changed by more than a threshold
    diff_pixels = sum(1 for p in diff.convert("L").getdata() if p > 20)
    print(f"[VISUAL_CV] Total modified pixels between idle and meeting state: {diff_pixels:,}")
    assert diff_pixels >= 5000, f"Expected significant visual UI change, found only {diff_pixels} modified pixels"
    print(f"[PASS] Meeting detection produces clear visual contrast ({diff_pixels:,} modified pixels)!")

    # 7. Teardown: Meeting Leave
    print("\n[STEP 7] Dispatching meeting-ended event...")
    leave_res = send_control("/api/simulate/meeting-leave", {})
    assert leave_res.get("success") is True
    time.sleep(1.2)

    # 8. Capture post-leave state
    print("\n[STEP 8] Capturing POST-LEAVE state...")
    img_cleared_path = ARTIFACTS_DIR / "visual_verify_3_meeting_cleared.png"
    capture_window(wid, img_cleared_path)
    img_cleared = Image.open(img_cleared_path)
    print(f"[PASS] Cleared state saved to {img_cleared_path.name}")

    # Verify banner and pill have been removed
    cleared_red_pixels = count_color_matches(img_cleared, (200, 255), (0, 80), (0, 80), box=banner_box)
    cleared_green_pixels = count_color_matches(img_cleared, (0, 60), (160, 255), (0, 120), box=titlebar_box)
    print(f"[VISUAL_CV] Post-leave red banner CTA pixels: {cleared_red_pixels}")
    print(f"[VISUAL_CV] Post-leave green titlebar pixels: {cleared_green_pixels}")
    assert cleared_red_pixels < 20, f"MeetingBanner still visible after leave ({cleared_red_pixels} red pixels)"
    assert cleared_green_pixels < 10, f"MeetingHeaderPill still visible after leave ({cleared_green_pixels} green pixels)"
    print("[PASS] Visual UI successfully reverted to idle state!")

    print("\n" + "=" * 75)
    print(" ALL VISUAL VERIFICATION TESTS PASSED CLEANLY!")
    print("=" * 75)
    print(f"Artifacts exported to:")
    print(f"  1. Baseline Idle:       {img_idle_path}")
    print(f"  2. Meeting Detected:    {img_active_path}")
    print(f"  3. Reverted Post-Leave: {img_cleared_path}")


if __name__ == "__main__":
    run_visual_verification()
