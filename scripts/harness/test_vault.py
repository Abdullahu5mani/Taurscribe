#!/usr/bin/env python3
"""Speaker voiceprint vault: cross-meeting recognition without false merges.

Feeds two-channel meetings through the app's real processing pipeline
(control server `feed-meeting`: diarization, clip extraction, voiceprints,
vault matching, saving), then checks the vault like a user would expect:

  A. "Daniel" talks ~13 s; the user names him "Daniel Guest" -> one person, 1 call.
  B. Daniel again, different words -> recognized as "Daniel Guest", 2 calls.
  C. "Karen" saying exactly Daniel's words from B -> NOT Daniel (voice, not words).
  D. "Fred", another male voice, saying Daniel's words from A -> NOT Daniel.
  E. Daniel for only a few seconds -> too little speech to recognize by voice,
     so he is left unnamed rather than guessed.

Callers are synthesized with macOS `say` so each meeting has enough speech
(the app needs 5 s of a caller's voice before it trusts a voiceprint).
Everything the test creates (meetings, clips, the vault person) is removed at
the end. Requires Taurscribe running in test mode.
"""

from __future__ import annotations

import json
import os
import re
import sqlite3
import struct
import subprocess
import sys
import time
import urllib.request
import wave
from pathlib import Path

HARNESS_DIR = Path(__file__).resolve().parent
AUDIO_DIR = HARNESS_DIR / "audio"
BASE = "http://127.0.0.1:8766"
TOKEN = os.environ["TAURSCRIBE_CONTROL_TOKEN"]
DB = Path.home() / "Library" / "Application Support" / "Taurscribe" / "transcript_history.db"
WORK = HARNESS_DIR / "reports" / "vault_fixtures"
PERSON = "Daniel Guest"

TEXT_1 = ("The quarterly numbers came in higher than we expected, mostly because the new onboarding flow "
          "cut drop off in half. We still need to look at retention for the enterprise tier, and I want a "
          "clear plan for the migration before the end of the month.")
TEXT_2 = ("I spoke with the design team yesterday and they think the settings page needs another pass. The "
          "download buttons are hard to find, and people do not understand which models are required. Let us "
          "schedule a review on Thursday afternoon.")
TEXT_SHORT = "Sounds good, talk soon."

# key -> (say voice, words)
MEETINGS = {
    "A": ("Daniel", TEXT_1),
    "B": ("Daniel", TEXT_2),
    "C": ("Karen", TEXT_2),
    "D": ("Fred", TEXT_1),
    "E": ("Daniel", TEXT_SHORT),
}

checks: list[tuple[bool, str, str]] = []


def check(ok: bool, name: str, detail: str = "") -> None:
    checks.append((ok, name, detail))
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))


def post(path: str, body: dict) -> dict:
    req = urllib.request.Request(BASE + path, data=json.dumps(body).encode(), method="POST",
                                 headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read().decode())


def get(path: str):
    req = urllib.request.Request(BASE + path, headers={"Authorization": f"Bearer {TOKEN}"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read().decode())


def say_clip(voice: str, text: str, out: Path) -> Path:
    subprocess.run(["say", "-v", voice, "-o", str(out), "--data-format=LEI16@16000", text], check=True)
    return out


def stereo_meeting(clip: Path, out: Path) -> Path:
    """Two-channel meeting: silent mic (left), the caller's clip on the call channel (right)."""
    with wave.open(str(clip), "rb") as w:
        rate, frames = w.getframerate(), w.readframes(w.getnframes())
    mono = struct.unpack(f"<{len(frames) // 2}h", frames)
    pad = [0] * (rate // 2)
    remote = pad + list(mono) + pad
    inter = []
    for s in remote:
        inter += [0, s]
    with wave.open(str(out), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack(f"<{len(inter)}h", *inter))
    return out


def caller_turns(meeting_id: int) -> list[dict]:
    return [t for t in get(f"/api/meetings/{meeting_id}")["turns"] if t["channel"] == 1]


def vault() -> list[dict]:
    return get("/api/vault")


def main() -> int:
    WORK.mkdir(parents=True, exist_ok=True)
    before_people = {v["id"] for v in vault()}
    created: list[int] = []
    engine = (get("/api/status").get("voiceprints") or {}).get("engine", "fallback")
    print("=" * 70 + f"\n SPEAKER VAULT: cross-meeting recognition (voice engine: {engine})\n" + "=" * 70)
    if engine != "neural":
        print("  (no neural speaker model: people are linked by name only, never by voice)")
    try:
        ids = {}

        def people() -> list[dict]:
            return [v for v in vault() if v["id"] not in before_people]

        def summary() -> str:
            return json.dumps([(p["name"], p["meeting_count"]) for p in people()])

        for key, (voice, text) in MEETINGS.items():
            clip = say_clip(voice, text, WORK / f"vault_{key}_mono.wav")
            wav = stereo_meeting(clip, WORK / f"vault_{key}.wav")
            res = post("/api/simulate/feed-meeting", {"wav_path": str(wav), "transcript": text,
                                                       "title": f"Vault test {key}", "platform": "meet"})
            ids[key] = res["data"]["meeting_id"]
            created.append(ids[key])
            turns = caller_turns(ids[key])
            names = {t["speaker_name"] for t in turns}
            print(f"  meeting {key} ({voice}): caller named {names}")

            if key == "A":
                check(len(turns) > 0, "Meeting A has a caller turn", f"{len(turns)} turn(s)")
                check(not people(), "An unnamed caller is not added to the vault")
                post("/api/simulate/rename-speaker", {"meeting_id": ids["A"], "speaker_id": turns[0]["speaker_id"],
                                                       "new_name": PERSON, "update_vault": True})
                ppl = people()
                check(len(ppl) == 1 and ppl[0]["name"] == PERSON, f"Naming the caller enrolls '{PERSON}'", summary())
                check(bool(ppl) and ppl[0]["meeting_count"] == 1, "New person has 1 call")

            elif key == "B":
                if engine == "neural":
                    check(names == {PERSON}, "Same voice, different words, another meeting: recognized", f"caller named {names}")
                else:
                    check(PERSON not in names, "Without a voice model, no one is auto-linked by voice", f"caller named {names}")
                    post("/api/simulate/rename-speaker", {"meeting_id": ids["B"], "speaker_id": turns[0]["speaker_id"],
                                                           "new_name": PERSON, "update_vault": True})
                ppl = people()
                check(len(ppl) == 1 and ppl[0]["meeting_count"] == 2,
                      "Still one person, now 2 calls (recognized or named)", summary())

            elif key in ("C", "D"):
                what = "A woman saying Daniel's exact words" if key == "C" else "Another male voice saying Daniel's words"
                check(PERSON not in names, f"{what} is not merged into '{PERSON}'", f"caller named {names}")
                ppl = people()
                check(len(ppl) == 1 and ppl[0]["meeting_count"] == 2, f"Vault unchanged by meeting {key}", summary())

            elif key == "E":
                check(PERSON not in names, "A caller with only a few seconds of speech is not guessed by voice",
                      f"caller named {names}")
                ppl = people()
                check(len(ppl) == 1 and ppl[0]["meeting_count"] == 2, "Vault unchanged by the short caller", summary())
    finally:
        # Remove exactly what this test created.
        con = sqlite3.connect(DB)
        try:
            snips = [r[0] for r in con.execute(
                f"SELECT snippet_path FROM meeting_turns WHERE meeting_id IN ({','.join('?' * len(created))})", created)
                     if r[0]] if created else []
            audio = [r[0] for r in con.execute(
                f"SELECT audio_path FROM meetings WHERE id IN ({','.join('?' * len(created))})", created)
                     if r[0]] if created else []
            if created:
                con.execute(f"DELETE FROM meeting_turns WHERE meeting_id IN ({','.join('?' * len(created))})", created)
                con.execute(f"DELETE FROM meetings WHERE id IN ({','.join('?' * len(created))})", created)
            new_people = [v["id"] for v in vault() if v["id"] not in before_people]
            for pid in new_people:
                con.execute("DELETE FROM speaker_vault WHERE id = ?", (pid,))
            con.commit()
        finally:
            con.close()
        voice = [str(Path(f).with_name(re.sub(r"(_cand_\d+)?\.wav$", "_voice.wav", Path(f).name))) for f in snips]
        for f in snips + voice + audio:
            Path(f).unlink(missing_ok=True)
        print(f"\n[CLEANUP] removed {len(created)} test meetings and their clips, and the test vault person")

    failed = [c for c in checks if not c[0]]
    print("=" * 70 + (f"\n ALL {len(checks)} VAULT CHECKS PASSED" if not failed else f"\n {len(failed)} VAULT CHECK(S) FAILED") + "\n" + "=" * 70)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
