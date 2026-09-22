#!/usr/bin/env python3
"""Speaker recognition over a real call, with real human voices (LibriSpeech).

Runs inside the meeting E2E after the recording phase, in the same live call.
Three recordings, each processed and saved by the app:

  R1  Caller = LibriSpeech reader 1089 (male), you = reader 121 (female, on
      this Mac's mic). The user names the caller -> the vault learns his
      voiceprint from what actually came through the meeting (codec, jitter
      buffer, the platform's audio processing).
  R2  Same caller, a different book chapter (other words, other session)
      -> the app names him automatically.
  R3  A different male reader, 1188 -> must NOT be taken for 1089.

Everything this phase adds (its meetings, clips, voice samples and the vault
person) is removed afterwards; the call-channel voice samples are copied into
the report first.

LibriSpeech test-clean (openslr.org/12, CC BY 4.0) is expected under
~/.taurscribe-harness/librispeech/LibriSpeech/test-clean.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import sqlite3
import subprocess
import time
import wave
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import ax_driver
from decider import MeetingDecider
from recording_check import (HISTORY_DB, LOCAL_MIC_DEVICE, PLAYER_BIN, PLAYER_SRC, PROCESSING_TIMEOUT, api,
                             coverage, restore_default_input, use_local_mic)

LIBRI = Path.home() / ".taurscribe-harness" / "librispeech" / "LibriSpeech" / "test-clean"
CACHE = Path.home() / ".taurscribe-harness" / "librispeech" / "clips"

CALLER = ("1089", "Peter Bobbe")      # LibriSpeech speaker id, reader name (SPEAKERS.TXT)
IMPOSTOR = ("1188", "Duncan Murrell")
YOU = "121"

# (speaker, chapter, seconds of speech)
R1_CALLER = (CALLER[0], "134686", 20)
R2_CALLER = (CALLER[0], "134691", 20)
R3_CALLER = (IMPOSTOR[0], "133604", 20)
YOU_LINES = (YOU, "121726", 8)
GAP_SECONDS = 0.4


def build_clip(speaker: str, chapter: str, seconds: float, skip: int = 0) -> Tuple[Path, str]:
    """Concatenates a reader's utterances (in order) into a 16 kHz mono WAV of
    at least `seconds`; returns the file and what was said."""
    CACHE.mkdir(parents=True, exist_ok=True)
    out = CACHE / f"{speaker}_{chapter}_{int(seconds)}s_{skip}.wav"
    txt = out.with_suffix(".txt")
    if out.exists() and txt.exists():
        return out, txt.read_text()
    folder = LIBRI / speaker / chapter
    if not folder.exists():
        raise AssertionError(f"LibriSpeech not found at {folder} (download test-clean from openslr.org/12)")
    trans = {}
    for line in (folder / f"{speaker}-{chapter}.trans.txt").read_text().splitlines():
        uid, _, words = line.partition(" ")
        trans[uid] = words.lower()
    pcm = bytearray()
    said: List[str] = []
    gap = b"\x00\x00" * int(16000 * GAP_SECONDS)
    for flac in sorted(folder.glob("*.flac"))[skip:]:
        tmp = CACHE / "tmp.wav"
        subprocess.run(["afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", str(flac), str(tmp)],
                       check=True, capture_output=True)
        pcm += _pcm16_data(tmp) + gap
        said.append(trans.get(flac.stem, ""))
        if len(pcm) / 2 / 16000 >= seconds:
            break
    (CACHE / "tmp.wav").unlink(missing_ok=True)
    with wave.open(str(out), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(bytes(pcm))
    txt.write_text(" ".join(said))
    return out, " ".join(said)


def _pcm16_data(path: Path) -> bytes:
    """The sample bytes of a 16-bit WAV (afconvert writes WAVE_FORMAT_EXTENSIBLE,
    which the wave module rejects)."""
    import struct
    data = path.read_bytes()
    i = 12
    while i + 8 <= len(data):
        cid, size = data[i:i + 4], struct.unpack("<I", data[i + 4:i + 8])[0]
        if cid == b"data":
            return data[i + 8:i + 8 + size]
        i += 8 + size + (size & 1)
    raise ValueError(f"no data chunk in {path}")


def speak_locally_path(path: Path) -> subprocess.Popen:
    """Plays a WAV into the virtual mic (non-blocking)."""
    if not PLAYER_BIN.exists() or PLAYER_BIN.stat().st_mtime < PLAYER_SRC.stat().st_mtime:
        PLAYER_BIN.parent.mkdir(exist_ok=True)
        subprocess.run(["swiftc", "-O", str(PLAYER_SRC), "-o", str(PLAYER_BIN)], check=True, capture_output=True)
    return subprocess.Popen([str(PLAYER_BIN), LOCAL_MIC_DEVICE, str(path)],
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def clip_seconds(path: Path) -> float:
    with wave.open(str(path), "rb") as w:
        return w.getnframes() / w.getframerate()


def vault() -> List[Dict[str, Any]]:
    return api("/api/vault")


def post(path: str, body: dict) -> dict:
    import urllib.request
    req = urllib.request.Request("http://127.0.0.1:8766" + path, data=json.dumps(body).encode(), method="POST",
                                 headers={"Authorization": "Bearer " + os.environ["TAURSCRIBE_CONTROL_TOKEN"], "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read().decode())


def record_once(app, remote, decider: MeetingDecider, report, label: str,
                caller_clip: Path, you_clip: Optional[Path]) -> Dict[str, Any]:
    """Decider records one stretch of the call from the app, the caller (and
    optionally you) talk, Decider stops it; returns the saved meeting."""
    before = {m["id"] for m in api("/api/meetings")}
    snap = ax_driver.snapshot(app.pid)
    record = decider.find_record_call_control(snap)
    report.check(record is not None, f"{label}: Decider finds 'record this call'",
                 record["text"] if record else "no record control visible")
    ax_driver.press(app.pid, record["index"])
    state: Dict[str, Any] = {}
    deadline = time.time() + 20
    while time.time() < deadline:
        state = decider.assess_recording(ax_driver.snapshot(app.pid))
        if state["recording"]:
            break
        time.sleep(1)
    report.check(state["recording"], f"{label}: app is recording the call", str(state.get("evidence")))
    report.check(remote.unmute(decider, report), f"{label}: caller's mic is on")
    time.sleep(1.0)

    if you_clip:
        proc = speak_locally_path(you_clip)
        report.log("YOU", f"{label}: reader {YOU} speaking on this Mac's mic ({clip_seconds(you_clip):.0f}s)")
        proc.wait(timeout=120)
        time.sleep(1.0)
    seconds = remote.play(caller_clip)
    report.log("CALLER", f"{label}: {caller_clip.stem} speaking in the meeting ({seconds:.0f}s)")
    time.sleep(seconds + 3.0)  # network + jitter buffer tail

    stop = decider.find_stop_recording_control(ax_driver.snapshot(app.pid))
    report.check(stop is not None, f"{label}: Decider finds the stop control", stop["text"] if stop else "")
    ax_driver.press(app.pid, stop["index"])

    new_id = None
    deadline = time.time() + PROCESSING_TIMEOUT
    while time.time() < deadline:
        new = [m for m in api("/api/meetings") if m["id"] not in before]
        busy = any(e["text"].startswith("Processing meeting") for e in ax_driver.snapshot(app.pid)["elements"])
        if new and not busy:
            new_id = max(m["id"] for m in new)
            break
        time.sleep(2)
    report.check(new_id is not None, f"{label}: recording processed and saved",
                 f"meeting #{new_id}" if new_id else f"nothing saved within {PROCESSING_TIMEOUT:.0f}s")
    return api(f"/api/meetings/{new_id}")


def caller_turns(meeting: Dict[str, Any]) -> List[Dict[str, Any]]:
    return [t for t in meeting.get("turns", []) if t.get("channel") == 1]


def verify_voiceprints(app, remote, adapter, decider: MeetingDecider, report) -> None:
    engine = (api("/api/status").get("voiceprints") or {}).get("engine")
    report.check(engine == "neural", "Speaker recognition model is installed (voice engine: neural)", str(engine))

    r1, r1_text = build_clip(*R1_CALLER)
    r2, r2_text = build_clip(*R2_CALLER)
    r3, _ = build_clip(*R3_CALLER)
    you, _ = build_clip(*YOU_LINES)
    person = f"{CALLER[1]} (LibriSpeech {CALLER[0]})"

    before_people = {v["id"] for v in vault()}
    created: List[int] = []

    def people() -> List[Dict[str, Any]]:
        return [v for v in vault() if v["id"] not in before_people]

    def summary() -> str:
        return json.dumps([(p["name"], p["meeting_count"], p["sample_count"]) for p in people()])

    use_local_mic(report)
    try:
        # R1: the caller is new; the user names him.
        m1 = record_once(app, remote, decider, report, "R1", r1, you)
        created.append(m1["id"])
        turns = caller_turns(m1)
        names = {t["speaker_name"] for t in turns}
        report.log("REPORT", f"R1 turns: {[(t['channel'], t['speaker_name'], t['text'][:40]) for t in m1.get('turns', [])]}")
        report.check(bool(turns), "R1: the caller's speech is on the call channel", f"{len(turns)} caller turn(s)")
        cov = coverage(r1_text, " ".join(t["text"] for t in turns))
        report.check(cov >= 0.4, "R1: the caller's words were transcribed", f"{cov:.0%} of what he read")
        report.check(not people() and not any(n == person for n in names),
                     "R1: an unknown caller is left unnamed", f"caller named {names}")
        post("/api/simulate/rename-speaker", {"meeting_id": m1["id"], "speaker_id": turns[0]["speaker_id"],
                                               "new_name": person, "update_vault": True})
        ppl = people()
        report.check(len(ppl) == 1 and ppl[0]["name"] == person, f"R1: naming the caller enrolls '{person}'", summary())
        report.check(bool(ppl) and ppl[0]["sample_count"] >= 1,
                     "R1: a voiceprint was learned from the call audio", summary())

        # R2: same man, other chapter -> recognized.
        m2 = record_once(app, remote, decider, report, "R2", r2, None)
        created.append(m2["id"])
        names = {t["speaker_name"] for t in caller_turns(m2)}
        report.log("REPORT", f"R2 caller named {names}")
        report.check(names == {person}, "R2: the same voice in a new recording is recognized by name",
                     f"caller named {names}")
        ppl = people()
        report.check(len(ppl) == 1 and ppl[0]["meeting_count"] == 2, "R2: still one person, now 2 calls", summary())

        # R3: another man -> not him.
        m3 = record_once(app, remote, decider, report, "R3", r3, None)
        created.append(m3["id"])
        names = {t["speaker_name"] for t in caller_turns(m3)}
        report.log("REPORT", f"R3 caller named {names}")
        report.check(bool(caller_turns(m3)) and person not in names,
                     f"R3: a different man ({IMPOSTOR[1]}) is not taken for {CALLER[1]}", f"caller named {names}")
        ppl = people()
        report.check(len(ppl) == 1 and ppl[0]["meeting_count"] == 2, "R3: vault unchanged", summary())
    finally:
        restore_default_input()
        _cleanup(report, created, before_people)


def _cleanup(report, created: List[int], before_people: set) -> None:
    con = sqlite3.connect(HISTORY_DB)
    try:
        q = ",".join("?" * len(created))
        snips = [r[0] for r in con.execute(f"SELECT snippet_path FROM meeting_turns WHERE meeting_id IN ({q})", created)
                 if r[0]] if created else []
        audio = [r[0] for r in con.execute(f"SELECT audio_path FROM meetings WHERE id IN ({q})", created)
                 if r[0]] if created else []
        voice = [str(Path(f).with_name(re.sub(r"(_cand_\d+)?\.wav$", "_voice.wav", Path(f).name))) for f in snips]
        keep = report.dir / "voiceprint_samples"
        keep.mkdir(exist_ok=True)
        for f in voice + audio:
            if Path(f).exists():
                shutil.copy(f, keep / Path(f).name)
        if created:
            con.execute(f"DELETE FROM meeting_turns WHERE meeting_id IN ({q})", created)
            con.execute(f"DELETE FROM meetings WHERE id IN ({q})", created)
        for v in vault():
            if v["id"] not in before_people:
                con.execute("DELETE FROM speaker_vault WHERE id = ?", (v["id"],))
        con.commit()
    finally:
        con.close()
    for f in snips + voice + audio:
        Path(f).unlink(missing_ok=True)
    report.log("REPORT", f"voiceprint phase cleanup: removed {len(created)} meetings and the test vault person "
                         f"(voice samples and recordings kept in {keep})")
