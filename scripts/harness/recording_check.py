#!/usr/bin/env python3
"""Phase 2b of the Decider meeting E2E: does Taurscribe record the call properly?

With the meeting live and detected, Decider starts a dual-channel recording from
the Taurscribe UI, the guest speaks known sentences INTO the meeting (its mic shim
never plays locally), Decider stops the recording from the UI, and the saved
meeting is checked against the script:

  * saved as a meeting with the right platform
  * transcript covers the script
  * the call channel (Ch2, system loopback) carries the guest's speech
  * the saved WAV is stereo with real signal on the call channel
"""

from __future__ import annotations

import json
import os
import re
import struct
import shutil
import subprocess
import time
import urllib.request
from pathlib import Path
from typing import Any, Dict, List, Optional

import ax_driver
from decider import MeetingDecider, PlanStep
from cdp_guest import SNAPSHOT_JS

HARNESS_DIR = Path(__file__).resolve().parent
AUDIO_DIR = HARNESS_DIR / "audio"

# Clips from generate_test_audio.py, with the exact synthesized text.
HOST_INTRO = ("01_host_intro.wav", "Welcome to the Taurscribe architecture review. Today we are verifying dual channel recording and vocal isolation.")
HOST_CROSSTALK = ("03a_crosstalk_host.wav", "Let me interrupt for a moment to discuss the architecture.")
GUEST_CROSSTALK = ("03b_crosstalk_rem.wav", "I am continuing to speak while the host speaks at the exact same time.")
CLIPS = [  # the guest's clean lines
    ("02_remote1_clean1.wav", "Thank you. I am speaker one, and I am speaking clearly from the remote conference room."),
    ("04_remote1_clean2.wav", "Here is another clean statement from speaker one, perfect for candidate snippet cycling."),
]

# "You" speak through a virtual mic: audio played into BlackHole's output comes
# out of its input, which the test makes the macOS default input (Taurscribe's
# mic channel always records the default input).
LOCAL_MIC_DEVICE = "BlackHole 2ch"
PLAYER_SRC = HARNESS_DIR / "play_to_device.swift"
PLAYER_BIN = HARNESS_DIR / "bin" / "play_to_device"
MAX_CROSS_LEAK = 0.25  # share of one side's distinctive words allowed on the other channel
_original_input: Optional[str] = None

MIN_TRANSCRIPT_COVERAGE = 0.6   # share of script words found in the transcript
MIN_CALL_CHANNEL_COVERAGE = 0.4  # share of script words found in channel-1 turns
PROCESSING_TIMEOUT = 240.0  # debug builds transcribe slowly
# "Unmute", "Unmute mic", "unmute my microphone", "Unmute (⌘+Shift+M)", "Turn on microphone"
UNMUTE = PlanStep("Unmute to speak", r"\bunmute\b|^turn on microphone\b", plain_only=False)


def app_snapshot(pid: int) -> Dict[str, Any]:
    """Retry transient WebKit accessibility misses under heavy ASR inference."""
    for attempt in range(3):
        try:
            snap = ax_driver.snapshot(pid)
            if snap.get("elements"):
                return snap
        except Exception:
            if attempt == 2:
                raise
        subprocess.run(
            ["osascript", "-e", f'tell application "System Events" to set frontmost of (first process whose unix id is {pid}) to true'],
            capture_output=True,
            timeout=5,
        )
        time.sleep(0.8)
    raise RuntimeError("Taurscribe accessibility tree remained empty")


def words(text: str) -> List[str]:
    return re.findall(r"[a-z0-9']+", text.lower())


def coverage(expected: str, actual: str) -> float:
    want = words(expected)
    have = set(words(actual))
    return sum(1 for w in want if w in have) / max(len(want), 1)


def api(path: str) -> Any:
    req = urllib.request.Request(f"http://127.0.0.1:8766{path}", headers={"Authorization": "Bearer " + os.environ["TAURSCRIBE_CONTROL_TOKEN"]})
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.loads(r.read().decode())


def read_wav(path: str):
    """Minimal RIFF reader: PCM16/24/32 and IEEE float32, including
    WAVE_FORMAT_EXTENSIBLE (65534), which Python's wave module rejects."""
    data = Path(path).read_bytes()
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError(f"not a WAV file: {path}")
    pos, fmt, pcm = 12, None, None
    while pos + 8 <= len(data):
        cid, size = data[pos:pos + 4], struct.unpack("<I", data[pos + 4:pos + 8])[0]
        body = data[pos + 8:pos + 8 + size]
        if cid == b"fmt ":
            tag, ch, rate, _, _, bits = struct.unpack("<HHIIHH", body[:16])
            if tag == 0xFFFE and len(body) >= 26:
                tag = struct.unpack("<H", body[24:26])[0]  # sub-format GUID starts with the real tag
            fmt = (tag, ch, rate, bits)
        elif cid == b"data":
            pcm = body
        pos += 8 + size + (size & 1)
    if not fmt or pcm is None:
        raise ValueError(f"WAV missing fmt/data chunk: {path}")
    tag, ch, rate, bits = fmt
    if tag == 3 and bits == 32:
        samples = struct.unpack(f"<{len(pcm) // 4}f", pcm[:len(pcm) // 4 * 4])
    elif tag == 1 and bits == 16:
        samples = [x / 32768.0 for x in struct.unpack(f"<{len(pcm) // 2}h", pcm[:len(pcm) // 2 * 2])]
    elif tag == 1 and bits == 32:
        samples = [x / 2147483648.0 for x in struct.unpack(f"<{len(pcm) // 4}i", pcm[:len(pcm) // 4 * 4])]
    else:
        raise ValueError(f"unsupported WAV encoding tag={tag} bits={bits}")
    return ch, rate, bits, samples


def channel_rms(path: str) -> Dict[str, Any]:
    """Per-channel RMS (0..1 full scale) plus basic format facts."""
    ch, rate, bits, samples = read_wav(path)
    rms = []
    for c in range(ch):
        chan = samples[c::ch]
        rms.append((sum(x * x for x in chan) / max(len(chan), 1)) ** 0.5)
    frames = len(samples) // max(ch, 1)
    return {"channels": ch, "rate": rate, "bits": bits, "seconds": round(frames / max(rate, 1), 1),
            "rms": [round(r, 5) for r in rms]}


def _switch_audio(*args: str) -> str:
    res = subprocess.run(["SwitchAudioSource", *args], capture_output=True, text=True, timeout=10)
    if res.returncode != 0:
        raise RuntimeError(f"SwitchAudioSource {' '.join(args)} failed: {res.stderr.strip()}")
    return res.stdout.strip()


def use_local_mic(report) -> None:
    """Makes BlackHole the default input for the recording (remembering the old one)."""
    global _original_input
    inputs = _switch_audio("-a", "-t", "input").splitlines()
    if LOCAL_MIC_DEVICE not in inputs:
        raise AssertionError(f"'{LOCAL_MIC_DEVICE}' is not an input device. Install BlackHole 2ch "
                             "(brew install blackhole-2ch) and run: sudo killall coreaudiod")
    if _original_input is None:
        _original_input = _switch_audio("-c", "-t", "input")
    _switch_audio("-t", "input", "-s", LOCAL_MIC_DEVICE)
    report.log("REPORT", f"default input: '{_original_input}' → '{LOCAL_MIC_DEVICE}' for the recording")


def restore_default_input() -> None:
    """Puts the user's input device back. Safe to call any number of times."""
    global _original_input
    if _original_input:
        try:
            _switch_audio("-t", "input", "-s", _original_input)
        finally:
            _original_input = None


def speak_locally(clip: str) -> subprocess.Popen:
    """Plays a clip into the virtual mic (non-blocking)."""
    if not PLAYER_BIN.exists() or PLAYER_BIN.stat().st_mtime < PLAYER_SRC.stat().st_mtime:
        PLAYER_BIN.parent.mkdir(exist_ok=True)
        subprocess.run(["swiftc", "-O", str(PLAYER_SRC), "-o", str(PLAYER_BIN)], check=True, capture_output=True)
    return subprocess.Popen([str(PLAYER_BIN), LOCAL_MIC_DEVICE, str(AUDIO_DIR / clip)],
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


HISTORY_DB = Path.home() / "Library" / "Application Support" / "Taurscribe" / "transcript_history.db"


def dictation_history_count() -> int:
    """Rows in the dictation (Mic) history. Meeting recordings must not add any."""
    import sqlite3
    con = sqlite3.connect(f"file:{HISTORY_DB}?mode=ro", uri=True)
    try:
        return con.execute("SELECT COUNT(*) FROM transcriptions").fetchone()[0]
    finally:
        con.close()


def channel_correlation(path: str) -> float:
    """Pearson correlation between the two channels (1.0 = the same signal)."""
    ch, _, _, samples = read_wav(path)
    if ch != 2:
        return 0.0
    a, b = samples[0::2], samples[1::2]
    n = min(len(a), len(b))
    if n == 0:
        return 0.0
    ma, mb = sum(a[:n]) / n, sum(b[:n]) / n
    cov = sum((x - ma) * (y - mb) for x, y in zip(a[:n], b[:n]))
    va = sum((x - ma) ** 2 for x in a[:n])
    vb = sum((y - mb) ** 2 for y in b[:n])
    return cov / ((va * vb) ** 0.5) if va > 0 and vb > 0 else 0.0


def guest_click(session, index: int):
    """Real pointer click on a snapshot.js-indexed element in the guest page."""
    box = session.page.evaluate(f"""
    (() => {{
        const el = document.querySelector('[data-jev-index="{index}"]');
        if (!el) return null;
        el.scrollIntoView({{block: 'center'}});
        const r = el.getBoundingClientRect();
        return [r.left + r.width / 2, r.top + r.height / 2];
    }})()""")
    if not box:
        return
    for kind, buttons in (("mouseMoved", 0), ("mousePressed", 1), ("mouseReleased", 0)):
        session.page.call("Input.dispatchMouseEvent", {"type": kind, "x": box[0], "y": box[1],
                                                      "button": "left", "buttons": buttons, "clickCount": 1})


def guest_mute_shortcut(session):
    """⌘⇧M / Ctrl+Shift+M, the mute toggle in Teams and Zoom web (isolated guest only)."""
    session.page.evaluate("document.body && document.body.focus()")
    for mods, key_mod in ((12, "Meta"), (10, "Control")):  # 4=meta|8=shift, 2=ctrl|8=shift
        for kind in ("keyDown", "keyUp"):
            session.page.call("Input.dispatchKeyEvent", {
                "type": kind, "modifiers": mods, "key": "M", "code": "KeyM",
                "windowsVirtualKeyCode": 77, "nativeVirtualKeyCode": 46})
        time.sleep(1.2)
        after = session.page.evaluate(SNAPSHOT_JS) or {}
        if not any(re.search(r"\bunmute\b|^turn on microphone", e["text"], re.I) for e in after.get("elements", [])):
            return key_mod
    return None


def guest_unmute(session, decider: MeetingDecider, report) -> bool:
    """Decider unmutes the guest (it muted itself while joining)."""
    size = session.page.evaluate("[innerWidth, innerHeight]") or [1200, 800]
    for attempt in range(6):
        if attempt == 2:
            # Two clicks did not take (split buttons can swallow them); use the shortcut.
            used = guest_mute_shortcut(session)
            if used:
                report.log("GUEST", f"unmuted with the {used}+Shift+M shortcut")
                return True
        session.page.call("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": size[0] // 2, "y": size[1] // 2})
        snap = session.page.evaluate(SNAPSHOT_JS) or {}
        action = decider.decide_plan(snap, [UNMUTE])
        if action.operation != "CLICK":
            muted_hint = any(re.search(r"\bunmute\b|^turn on microphone", e["text"], re.I) for e in snap.get("elements", []))
            if muted_hint:
                (report.dir / "guest_unmute_snapshot.txt").write_text(snap.get("table", ""))
            return not muted_hint
        target = session.page.evaluate(f"""(() => {{ const el = document.querySelector('[data-jev-index="{action.target}"]');
            return el ? el.outerHTML.slice(0, 160) : null; }})()""")
        report.log("GUEST", f"Decider → {action}", element=target)
        guest_click(session, action.target)
        # A toggle: wait for its label to change before clicking again, or a
        # slow UI (Teams) gets flipped back off by the next click.
        deadline = time.time() + 6
        while time.time() < deadline:
            time.sleep(0.75)
            after = session.page.evaluate(SNAPSHOT_JS) or {}
            if decider.decide_plan(after, [UNMUTE]).operation != "CLICK":
                break
        try:
            session.screenshot(report.dir / f"guest_unmute_{int(time.time())}.png")
        except Exception:
            pass
    return False


class GuestSpeaker:
    """The remote participant is the guest (Meet, Teams): a CDP guest session."""

    def __init__(self, guest):
        self.guest = guest

    def unmute(self, decider: MeetingDecider, report) -> bool:
        return guest_unmute(self.guest.session, decider, report)

    def play(self, path: Path) -> float:
        return self.guest.session.play_audio(path)


class HostSpeaker:
    """The remote participant is the host (Zoom in the VM): a ChromeHostCDP whose
    meeting UI lives in an iframe, so the mic shim is driven inside that frame."""

    def __init__(self, host):
        self.host = host

    MUTE_VISIBLE = re.compile(r"^(mute my microphone|mute mic|mute)\b", re.I)

    def _look(self) -> Dict[str, Any]:
        # Zoom's toolbar is hover-only: reveal it before reading the mic state.
        self.host.hover()
        time.sleep(0.6)
        return self.host.snapshot()

    def unmute(self, decider: MeetingDecider, report) -> bool:
        """Only positive evidence counts: a visible "mute my microphone" control.
        A missing "unmute" control can just mean the toolbar is hidden."""
        for _ in range(8):
            snap = self._look()
            if any(self.MUTE_VISIBLE.search(e["text"]) for e in snap.get("elements", [])):
                return True
            action = decider.decide_plan(snap, [UNMUTE])
            if action.operation == "CLICK":
                report.log("HOST", f"Decider → {action}")
                self.host.act(action)
                time.sleep(2.0)
            else:
                time.sleep(1.0)
        return False

    def play(self, path: Path) -> float:
        # The meeting frame is evaluated through an isolated world, which cannot
        # call the shim's page function; the shim also listens for this DOM event.
        import base64
        b64 = base64.b64encode(Path(path).read_bytes()).decode()
        self.host._eval(f"window.dispatchEvent(new CustomEvent('taurscribe-play', {{detail: '{b64}'}})); true")
        ch, rate, _, samples = read_wav(str(path))
        return len(samples) / max(ch, 1) / rate


def _clock_seconds(text: str) -> Optional[int]:
    m = re.match(r"^(\d+):(\d\d)$", text)
    return int(m.group(1)) * 60 + int(m.group(2)) if m else None


MAX_PLAYBACK_BYTES_PER_SECOND = 3_500  # 16 kbps Opus is 2,000 B/s plus container


def check_playback_file(report, audio_path: str, raw_seconds: float) -> Dict[str, Any]:
    """The saved meeting audio is the small playback copy, not the raw capture."""
    path = Path(audio_path)
    report.check(path.exists(), "Meeting playback file exists", audio_path)
    report.check(path.suffix == ".webm", "Playback copy is compressed (WebM/Opus), not the raw WAV", path.name)
    report.check(not path.with_suffix(".wav").exists(), "Raw WAV removed after processing",
                 str(path.with_suffix(".wav")))
    size = path.stat().st_size if path.exists() else 0
    bps = size / raw_seconds if raw_seconds else 0
    raw_bytes = raw_seconds * 48_000 * 2 * 4
    report.check(0 < bps <= MAX_PLAYBACK_BYTES_PER_SECOND, "Playback copy is small",
                 f"{size / 1024:.0f} KB for {raw_seconds:.0f}s = {bps:.0f} B/s "
                 f"(~{raw_bytes / max(size, 1):.0f}x smaller than the raw capture)")
    info: Dict[str, Any] = {"bytes": size}
    if shutil.which("ffprobe") and path.exists():
        dur = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
                             capture_output=True, text=True).stdout.strip()
        vol = subprocess.run(["ffmpeg", "-v", "info", "-i", str(path), "-af", "volumedetect", "-f", "null", "-"],
                             capture_output=True, text=True).stderr
        m = re.search(r"mean_volume: (-?[\d.]+) dB", vol)
        info.update(duration=float(dur or 0), mean_db=float(m.group(1)) if m else None)
        report.check(abs(info["duration"] - raw_seconds) < 0.5, "Playback copy has the full length",
                     f"{info['duration']:.1f}s vs {raw_seconds:.1f}s recorded")
        report.check(info["mean_db"] is not None and info["mean_db"] > -45, "Playback copy decodes to audible audio",
                     f"mean volume {info['mean_db']} dB")
    return info


def verify_playback(app, report, expected_seconds: float) -> None:
    """Decider presses Play Recording on the displayed meeting: the position must
    advance and the player must know the recording's real length."""
    def player(snap):
        els = snap["elements"]
        i = next((k for k, e in enumerate(els) if e["text"] in ("Play Recording", "Pause Audio")), None)
        return (els[i], [e["text"] for e in els[i + 1:i + 3]]) if i is not None else (None, [])

    btn, _ = player(app_snapshot(app.pid))
    report.check(btn is not None and btn["text"] == "Play Recording", "Decider finds Play Recording on the meeting",
                 btn["text"] if btn else "no player visible")
    ax_driver.press(app.pid, btn["index"], expect="Play Recording")
    positions, length = [], None
    for _ in range(4):
        time.sleep(1.0)
        _, clocks = player(app_snapshot(app.pid))
        if len(clocks) == 2:
            positions.append(_clock_seconds(clocks[0]) or 0)
            length = _clock_seconds(clocks[1])
    stop, _ = player(app_snapshot(app.pid))
    if stop and stop["text"] == "Pause Audio":
        ax_driver.press(app.pid, stop["index"], expect="Pause Audio")
    report.check(len(positions) >= 2 and positions[-1] > positions[0], "Meeting recording plays back in the app",
                 f"position over 4s: {positions}")
    report.check(length is not None and abs(length - expected_seconds) <= 2,
                 "Player shows the recording's real length", f"player says {length}s, file is {expected_seconds:.0f}s")


def ensure_model_loaded(app, decider: MeetingDecider, report) -> None:
    """Pre-flight (normal screen, before any meeting): Decider loads a model if needed.
    Meeting mode hides the engine picker, so this cannot wait until the call."""
    snap = app_snapshot(app.pid)
    if not any("status" in e["text"].lower() for e in snap["elements"]):
        dictation = next((e for e in snap["elements"] if e["text"] == "Microphone dictation mode"), None)
        if dictation:
            report.log("DECIDER", "app is in meeting mode (engine picker hidden); switching to dictation view")
            ax_driver.press(app.pid, dictation["index"])
            time.sleep(1.5)
            snap = app_snapshot(app.pid)
    load = decider.find_load_model_control(snap)
    if load:
        report.log("DECIDER", f"app says LOAD REQUIRED; pressing '{load['text']}'")
        ax_driver.press(app.pid, load["index"])
        deadline = time.time() + 120
        while time.time() < deadline and decider.find_load_model_control(app_snapshot(app.pid)):
            time.sleep(2)
    snap = app_snapshot(app.pid)
    status = next((e["text"] for e in snap["elements"] if "status" in e["text"].lower()), "")
    report.check("status ready" in status.lower(), "Transcription model is loaded in the app", status or "no engine status visible")


def call_meter(snapshot: Dict[str, Any]) -> Optional[int]:
    """Call-channel level: the telemetry panel's 'Right: Meeting Callers' reading,
    else the footer's compact 'DUAL-CH M n % · S n %' meter."""
    els = snapshot.get("elements", [])
    for i, e in enumerate(els):
        if re.search(r"meeting callers", e["text"], re.I):
            for nxt in els[i + 1:i + 4]:
                m = re.match(r"^(\d+)%$", nxt["text"])
                if m:
                    return int(m.group(1))
    for i, e in enumerate(els):
        if e["text"] == "DUAL-CH":
            tail = [x["text"] for x in els[i:i + 9]]
            if "S" in tail:
                j = tail.index("S")
                if j + 1 < len(tail) and tail[j + 1].isdigit():
                    return int(tail[j + 1])
    return None


def mic_meter(snapshot: Dict[str, Any]) -> Optional[int]:
    """Mic-channel level: 'Left: Your Microphone' in the panel, else footer 'M n %'."""
    els = snapshot.get("elements", [])
    for i, e in enumerate(els):
        if re.search(r"your microphone", e["text"], re.I):
            for nxt in els[i + 1:i + 4]:
                m = re.match(r"^(\d+)%$", nxt["text"])
                if m:
                    return int(m.group(1))
    for i, e in enumerate(els):
        if e["text"] == "DUAL-CH":
            tail = [x["text"] for x in els[i:i + 9]]
            if "M" in tail:
                j = tail.index("M")
                if j + 1 < len(tail) and tail[j + 1].isdigit():
                    return int(tail[j + 1])
    return None


def open_meetings_view(app, report) -> None:
    """Decider switches the app to its meetings area (telemetry, processing
    indicator, meetings list) the way a user would, if it isn't showing."""
    snap = app_snapshot(app.pid)
    if any(re.search(r"meeting callers", e["text"], re.I) for e in snap["elements"]):
        return
    tab = next((e for e in snap["elements"] if e["text"] == "Meeting detection and dual-channel recording mode"), None)
    if tab:
        report.log("DECIDER", "opening the app's meetings view")
        ax_driver.press(app.pid, tab["index"])
        time.sleep(1.5)


def verify_recording(app, remote, adapter, decider: MeetingDecider, report) -> None:
    """Runs the recording phase; raises AssertionError via report.check on failure."""
    open_meetings_view(app, report)
    snap = app_snapshot(app.pid)
    # The mic channel records the default input, so point it at the virtual mic
    # before the recording opens it. Always restored (here and in the test's cleanup).
    use_local_mic(report)
    try:
        _record_conversation(app, remote, adapter, decider, report, snap)
    finally:
        restore_default_input()


def _record_conversation(app, remote, adapter, decider: MeetingDecider, report, snap) -> None:
    # 2. Decider starts the dual-channel recording from the meeting pill.
    before = {m["id"] for m in api("/api/meetings")}
    history_before = dictation_history_count()
    record = decider.find_record_call_control(snap)
    report.check(record is not None, "Decider finds the app's 'record this call' control",
                 record["text"] if record else "no record control visible")
    report.log("DECIDER", f"pressing app control '{record['text'][:70]}'")
    ax_driver.press(app.pid, record["index"])
    state: Dict[str, Any] = {}
    deadline = time.time() + 20
    while time.time() < deadline:
        state = decider.assess_recording(app_snapshot(app.pid))
        if state["recording"]:
            break
        time.sleep(1)
    report.check(state["recording"], "Decider sees the app recording the call", str(state.get("evidence")))
    rec_status = api("/api/status")["recording"]
    report.check(rec_status["is_recording"] and rec_status["is_dual_channel"],
                 "API cross-check: dual-channel recording running", json.dumps(rec_status))

    # 3. A two-person conversation: "you" on the local (virtual) mic, the guest in
    #    the meeting, one overlapping exchange, then the guest again.
    report.check(remote.unmute(decider, report), "Remote participant's mic is on (unmuted)")
    peaks = {"mic_while_you": 0, "call_while_guest": 0}

    def watch(seconds: float, key: str, meter):
        end = time.time() + seconds
        while time.time() < end:  # Decider watches the app's live meters
            peaks[key] = max(peaks[key], meter(app_snapshot(app.pid)) or 0)
            time.sleep(0.3)

    def you_say(clip, text):
        proc = speak_locally(clip)
        report.log("YOU", f"speaking on this Mac's mic: \"{text}\"")
        watch(4.0, "mic_while_you", mic_meter)
        proc.wait(timeout=30)
        time.sleep(1.0)

    def guest_says(clip, text):
        seconds = remote.play(AUDIO_DIR / clip)
        report.log("GUEST", f"speaking in the meeting ({seconds:.1f}s): \"{text}\"")
        watch(seconds + 1.2, "call_while_guest", call_meter)

    time.sleep(1.0)
    you_say(*HOST_INTRO)
    guest_says(*CLIPS[0])
    # Crosstalk: both talk at once.
    proc = speak_locally(HOST_CROSSTALK[0])
    seconds = remote.play(AUDIO_DIR / GUEST_CROSSTALK[0])
    report.log("BOTH", f"talking over each other: you \"{HOST_CROSSTALK[1]}\" / guest \"{GUEST_CROSSTALK[1]}\"")
    proc.wait(timeout=30)
    time.sleep(seconds + 1.0)
    guest_says(*CLIPS[1])
    time.sleep(3.0)  # network + jitter buffer tail

    report.check(peaks["mic_while_you"] >= 10, "App's live meter shows your mic while you talk",
                 f"'Your Microphone' peaked at {peaks['mic_while_you']}%")
    report.check(peaks["call_while_guest"] >= 10, "App's live meter shows call audio while the guest talks",
                 f"'Meeting Callers' peaked at {peaks['call_while_guest']}%")

    you_script = " ".join([HOST_INTRO[1], HOST_CROSSTALK[1]])
    guest_script = " ".join([t for _, t in CLIPS] + [GUEST_CROSSTALK[1]])
    script = " ".join([HOST_INTRO[1], CLIPS[0][1], HOST_CROSSTALK[1], GUEST_CROSSTALK[1], CLIPS[1][1]])

    # What the capture did (target, callers watchdog, tap restarts).
    capture = api("/api/status").get("capture") or {}
    report.log("REPORT", f"capture: {json.dumps(capture)}")
    if str(capture.get("target", "")).startswith("Process"):
        report.check(bool(capture.get("watchdog_app")), "Callers-track watchdog is armed on the meeting app",
                     f"watchdog_app={capture.get('watchdog_app')} restarts={capture.get('tap_restarts')}")

    # 4. Decider stops the recording from the app.
    snap = app_snapshot(app.pid)
    stop = decider.find_stop_recording_control(snap)
    report.check(stop is not None, "Decider finds the app's stop-recording control",
                 stop["text"] if stop else "\n".join(snap["table"].splitlines()[:40]))
    report.log("DECIDER", f"pressing app control '{stop['text'][:70]}'")
    ax_driver.press(app.pid, stop["index"])

    # 5. The meetings area must show that the call is being processed.
    indicator = None
    deadline = time.time() + 10
    while time.time() < deadline and not indicator:
        snap = app_snapshot(app.pid)
        indicator = next((e["text"] for e in snap["elements"] if e["text"].startswith("Processing meeting")), None)
        time.sleep(0.5)
    if indicator:
        app.screenshot("phase2b_processing_indicator")
    report.check(indicator is not None, "Decider sees the 'Processing meeting' indicator in the meetings area",
                 indicator or "no processing indicator within 10s of stopping")
    started = time.time()

    # ...and clear it once the meeting is saved.
    new_id = None
    still_processing = True
    deadline = time.time() + PROCESSING_TIMEOUT
    while time.time() < deadline:
        new = [m for m in api("/api/meetings") if m["id"] not in before]
        snap = app_snapshot(app.pid)
        still_processing = any(e["text"].startswith("Processing meeting") for e in snap["elements"])
        failed = next((e["text"] for e in snap["elements"] if e["text"] == "Meeting processing failed"), None)
        report.check(not failed, "App reports no processing failure", failed or "")
        if new and not still_processing:
            new_id = max(m["id"] for m in new)
            break
        time.sleep(2)
    report.log("REPORT", f"processing took {time.time() - started:.0f}s")
    report.check(not still_processing, "Processing indicator clears when done",
                 "still showing" if still_processing else "cleared")
    report.check(new_id is not None, "Recording was saved as a meeting",
                 f"meeting #{new_id}" if new_id else f"nothing saved within {PROCESSING_TIMEOUT:.0f}s")
    detail = api(f"/api/meetings/{new_id}")
    (report.dir / "recorded_meeting.json").write_text(json.dumps(detail, indent=2))
    transcript = detail.get("transcript_raw", "")
    report.log("REPORT", f"saved transcript: \"{transcript[:300]}\"")

    report.check(detail.get("platform") == adapter.key, f"Meeting saved with platform '{adapter.key}'",
                 f"saved as '{detail.get('platform')}' / '{detail.get('title')}'")
    cov = coverage(script, transcript)
    report.check(cov >= MIN_TRANSCRIPT_COVERAGE, "Transcript contains both speakers",
                 f"{cov:.0%} of the conversation's words present (need {MIN_TRANSCRIPT_COVERAGE:.0%})")

    turns = detail.get("turns", [])
    turn_list = [(t.get("channel"), t.get("speaker_name"), t.get("text", "")[:50]) for t in turns]
    you_text = " ".join(t["text"] for t in turns if t.get("channel") == 0)
    call_text = " ".join(t["text"] for t in turns if t.get("channel") == 1)
    report.log("REPORT", f"turns: {turn_list}")

    you_cov = coverage(HOST_INTRO[1], you_text)
    report.check(you_cov >= MIN_CALL_CHANNEL_COVERAGE, "Your speech is on the mic channel (Ch1, 'You')",
                 f"{you_cov:.0%} of your intro in channel-0 turns")
    call_cov = coverage(" ".join(t for _, t in CLIPS), call_text)
    report.check(call_cov >= MIN_CALL_CHANNEL_COVERAGE, "Guest speech is on the call channel (Ch2, 'Remote Participant')",
                 f"{call_cov:.0%} of the guest's lines in channel-1 turns")

    # Separation: words only one side said must not show up on the other channel.
    you_only = " ".join(w for w in words(you_script) if w not in set(words(guest_script)))
    guest_only = " ".join(w for w in words(guest_script) if w not in set(words(you_script)))
    you_leak = coverage(you_only, call_text)
    guest_leak = coverage(guest_only, you_text)
    report.check(you_leak <= MAX_CROSS_LEAK, "Your words do not leak into the call channel",
                 f"{you_leak:.0%} of your distinctive words appear in channel-1 turns (max {MAX_CROSS_LEAK:.0%})")
    report.check(guest_leak <= MAX_CROSS_LEAK, "Guest words do not leak into your channel",
                 f"{guest_leak:.0%} of the guest's distinctive words appear in channel-0 turns (max {MAX_CROSS_LEAK:.0%})")

    # Crosstalk: both overlapping lines survive, each on its own side.
    xt_you = coverage(HOST_CROSSTALK[1], you_text)
    xt_guest = coverage(GUEST_CROSSTALK[1], call_text)
    report.check(xt_you >= 0.3 and xt_guest >= 0.3, "Crosstalk: both overlapping lines captured on their own channels",
                 f"your line {xt_you:.0%} in Ch1, guest line {xt_guest:.0%} in Ch2")

    # The raw two-channel WAV only lives until processing finishes; the app
    # reports what it measured on it, then keeps a small playback copy.
    proc = api("/api/status").get("processing") or {}
    report.log("REPORT", f"processing: {json.dumps({k: v for k, v in proc.items() if k != 'playback_path'})}")
    report.check(proc.get("raw_channels") == 2, "Recorded audio was dual-channel (mic + call)",
                 f"{proc.get('raw_channels')} channel(s)")
    rms_ = proc.get("channel_rms") or []
    report.check(len(rms_) == 2 and min(rms_) > 0.002, "Both channels had real signal", f"channel RMS {rms_}")
    corr = proc.get("channel_correlation")
    report.check(corr is not None and corr < 0.5, "Mic and call channels carried different audio",
                 f"channel correlation {corr} (1.00 would mean one signal recorded twice)")

    audio_path = detail.get("audio_path") or ""
    raw_seconds = float(proc.get("raw_seconds") or 0)
    playback = check_playback_file(report, audio_path, raw_seconds)

    # The transcript belongs to the meetings area only: nothing in dictation history.
    history_after = dictation_history_count()
    report.check(history_after == history_before, "Meeting transcript stays out of the dictation (Mic) history",
                 f"dictation history rows {history_before} → {history_after}")

    # 6. Decider finds the meeting and the guest's words in the app's transcript area.
    title = detail.get("title", "")
    seen: Dict[str, Any] = {}
    deadline = time.time() + 20
    while time.time() < deadline:
        snap = app_snapshot(app.pid)
        texts = [e["text"] for e in snap["elements"]]
        seen["listed"] = any(t == title for t in texts)
        area = " ".join(texts[next((i for i, t in enumerate(texts) if t.startswith("MEETING TRANSCRIPT")), len(texts)):])
        seen["coverage"] = coverage(script, area)
        if seen["listed"] and seen["coverage"] >= MIN_CALL_CHANNEL_COVERAGE:
            break
        time.sleep(2)
    report.save_snapshot("phase2b_app_after_recording", snap)
    verify_playback(app, report, raw_seconds)
    report.check(seen.get("listed"), "Decider sees the new meeting in the app's meetings list", f"title '{title}'")
    report.check(seen.get("coverage", 0) >= MIN_CALL_CHANNEL_COVERAGE, "Decider reads the conversation in the app's transcript area",
                 f"{seen.get('coverage', 0):.0%} of script words visible under MEETING TRANSCRIPT")
