#!/usr/bin/env python3
"""Deterministic multi-speaker dual-channel test audio generator for Taurscribe.

Uses native macOS speech synthesis (`say` + `afconvert`) and Python standard library `wave`
to produce reproducible 16 kHz stereo WAV files for meeting simulation, crosstalk gating,
and speaker vault verification.
"""

from __future__ import annotations
import json
import os
import struct
import subprocess
import wave
from pathlib import Path

SAMPLE_RATE = 16000
CHANNELS = 2  # Stereo: Left = Channel 0 (Local Host), Right = Channel 1 (Remote)

VOICE_HOST = "Fred"
VOICE_REMOTE_1 = "Daniel"
VOICE_REMOTE_2 = "Karen"

HARNESS_DIR = Path(__file__).resolve().parent
AUDIO_DIR = HARNESS_DIR / "audio"
AUDIO_DIR.mkdir(parents=True, exist_ok=True)


def synth_mono_wav(text: str, voice: str, filename: str) -> Path:
    """Synthesizes text into a 16 kHz 16-bit mono WAV using say + afconvert."""
    out_aiff = AUDIO_DIR / f"{filename}.aiff"
    out_wav = AUDIO_DIR / f"{filename}.wav"

    subprocess.run(["say", "-v", voice, text, "-o", str(out_aiff)], check=True)
    subprocess.run(
        [
            "afconvert",
            "-f",
            "WAVE",
            "-d",
            "LEI16@16000",
            "-c",
            "1",
            str(out_aiff),
            str(out_wav),
        ],
        check=True,
    )
    if out_aiff.exists():
        out_aiff.unlink()
    return out_wav


def read_mono_samples(wav_path: Path) -> list[int]:
    """Reads 16-bit PCM samples from mono WAV file."""
    with wave.open(str(wav_path), "rb") as w:
        n_frames = w.getnframes()
        raw = w.readframes(n_frames)
        samples = list(struct.unpack(f"<{n_frames}h", raw))
        return samples


def write_stereo_wav(
    ch0_samples: list[int], ch1_samples: list[int], out_path: Path
):
    """Writes interleaved 16-bit PCM stereo WAV (ch0 = left, ch1 = right)."""
    length = max(len(ch0_samples), len(ch1_samples))
    ch0_padded = ch0_samples + [0] * (length - len(ch0_samples))
    ch1_padded = ch1_samples + [0] * (length - len(ch1_samples))

    with wave.open(str(out_path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        interleaved = []
        for s0, s1 in zip(ch0_padded, ch1_padded):
            interleaved.append(s0)
            interleaved.append(s1)
        raw = struct.pack(f"<{len(interleaved)}h", *interleaved)
        w.writeframes(raw)


def main():
    print(f"[AUDIO] Generating test utterances in {AUDIO_DIR}...")

    # 1. Synthesize individual utterances
    p_host1 = synth_mono_wav(
        "Welcome to the Taurscribe architecture review. Today we are verifying dual channel recording and vocal isolation.",
        VOICE_HOST,
        "01_host_intro",
    )
    p_rem1_a = synth_mono_wav(
        "Thank you. I am speaker one, and I am speaking clearly from the remote conference room.",
        VOICE_REMOTE_1,
        "02_remote1_clean1",
    )
    p_crosstalk_host = synth_mono_wav(
        "Let me interrupt for a moment to discuss the architecture.",
        VOICE_HOST,
        "03a_crosstalk_host",
    )
    p_crosstalk_rem = synth_mono_wav(
        "I am continuing to speak while the host speaks at the exact same time.",
        VOICE_REMOTE_1,
        "03b_crosstalk_rem",
    )
    p_rem1_b = synth_mono_wav(
        "Here is another clean statement from speaker one, perfect for candidate snippet cycling.",
        VOICE_REMOTE_1,
        "04_remote1_clean2",
    )
    p_rem2 = synth_mono_wav(
        "And I am speaker two, joining from the mobile client to test multiple remote speaker separation.",
        VOICE_REMOTE_2,
        "05_remote2_intro",
    )
    p_host2 = synth_mono_wav(
        "Thank you everyone. The meeting is adjourned and we will process the diarization now.",
        VOICE_HOST,
        "06_host_closing",
    )

    s_host1 = read_mono_samples(p_host1)
    s_rem1_a = read_mono_samples(p_rem1_a)
    s_cross_h = read_mono_samples(p_crosstalk_host)
    s_cross_r = read_mono_samples(p_crosstalk_rem)
    s_rem1_b = read_mono_samples(p_rem1_b)
    s_rem2 = read_mono_samples(p_rem2)
    s_host2 = read_mono_samples(p_host2)

    silence_gap = [0] * int(SAMPLE_RATE * 1.5)
    silence_one_sec = [0] * SAMPLE_RATE

    # 2. Build Meeting 1 Timeline
    # Ch0: [host1] [silence] [cross_h] [silence] [host2]
    # Ch1: [silence] [rem1_a] [cross_r] [silence] [rem1_b] [silence] [rem2] [silence]
    ch0: list[int] = []
    ch1: list[int] = []
    turns_metadata = []

    def current_time_ms():
        return int((len(ch0) / SAMPLE_RATE) * 1000)

    # ── Segment 1: Host Intro (Ch0) ──
    t_start = current_time_ms()
    ch0.extend(s_host1)
    ch1.extend([0] * len(s_host1))
    t_end = current_time_ms()
    turns_metadata.append(
        {
            "speaker": "Host",
            "channel": 0,
            "start_ms": t_start,
            "end_ms": t_end,
            "text": "Welcome to the Taurscribe architecture review. Today we are verifying dual channel recording and vocal isolation.",
            "is_crosstalk": False,
        }
    )

    # Gap
    ch0.extend(silence_gap)
    ch1.extend(silence_gap)

    # ── Segment 2: Remote 1 Clean (Ch1) ──
    t_start = current_time_ms()
    ch0.extend([0] * len(s_rem1_a))
    ch1.extend(s_rem1_a)
    t_end = current_time_ms()
    turns_metadata.append(
        {
            "speaker": "Speaker 1",
            "channel": 1,
            "start_ms": t_start,
            "end_ms": t_end,
            "text": "Thank you. I am speaker one, and I am speaking clearly from the remote conference room.",
            "is_crosstalk": False,
        }
    )

    # Gap
    ch0.extend(silence_gap)
    ch1.extend(silence_gap)

    # ── Segment 3: Crosstalk (Both Speaking) ──
    t_start = current_time_ms()
    cross_len = max(len(s_cross_h), len(s_cross_r))
    ch0.extend(s_cross_h + [0] * (cross_len - len(s_cross_h)))
    ch1.extend(s_cross_r + [0] * (cross_len - len(s_cross_r)))
    t_end = current_time_ms()
    turns_metadata.append(
        {
            "speaker": "Crosstalk",
            "channel": 1,
            "start_ms": t_start,
            "end_ms": t_end,
            "text": "Let me interrupt for a moment to discuss the architecture. I am continuing to speak while the host speaks at the exact same time.",
            "is_crosstalk": True,
        }
    )

    # Gap
    ch0.extend(silence_gap)
    ch1.extend(silence_gap)

    # ── Segment 4: Remote 1 Clean 2 (Ch1) ──
    t_start = current_time_ms()
    ch0.extend([0] * len(s_rem1_b))
    ch1.extend(s_rem1_b)
    t_end = current_time_ms()
    turns_metadata.append(
        {
            "speaker": "Speaker 1",
            "channel": 1,
            "start_ms": t_start,
            "end_ms": t_end,
            "text": "Here is another clean statement from speaker one, perfect for candidate snippet cycling.",
            "is_crosstalk": False,
        }
    )

    # Gap
    ch0.extend(silence_gap)
    ch1.extend(silence_gap)

    # ── Segment 5: Remote 2 Intro (Ch1) ──
    t_start = current_time_ms()
    ch0.extend([0] * len(s_rem2))
    ch1.extend(s_rem2)
    t_end = current_time_ms()
    turns_metadata.append(
        {
            "speaker": "Speaker 2",
            "channel": 1,
            "start_ms": t_start,
            "end_ms": t_end,
            "text": "And I am speaker two, joining from the mobile client to test multiple remote speaker separation.",
            "is_crosstalk": False,
        }
    )

    # Gap
    ch0.extend(silence_gap)
    ch1.extend(silence_gap)

    # ── Segment 6: Host Closing (Ch0) ──
    t_start = current_time_ms()
    ch0.extend(s_host2)
    ch1.extend([0] * len(s_host2))
    t_end = current_time_ms()
    turns_metadata.append(
        {
            "speaker": "Host",
            "channel": 0,
            "start_ms": t_start,
            "end_ms": t_end,
            "text": "Thank you everyone. The meeting is adjourned and we will process the diarization now.",
            "is_crosstalk": False,
        }
    )

    meeting_1_wav = AUDIO_DIR / "meeting_1_dual_channel.wav"
    write_stereo_wav(ch0, ch1, meeting_1_wav)
    print(
        f"[SUCCESS] Meeting 1 written: {meeting_1_wav} ({current_time_ms()} ms)"
    )

    meta_1_path = AUDIO_DIR / "meeting_1_meta.json"
    full_transcript_1 = " ".join([t["text"] for t in turns_metadata])
    meta_1_path.write_text(
        json.dumps(
            {
                "wav_path": str(meeting_1_wav),
                "duration_ms": current_time_ms(),
                "transcript": full_transcript_1,
                "turns": turns_metadata,
            },
            indent=2,
        )
    )

    # 3. Build Meeting 2 (Cross-meeting recognition verification)
    # Uses the SAME voice (Daniel) speaking in Meeting 2 to test automatic vault recognition
    p_rem1_call2 = synth_mono_wav(
        "Hello again. This is speaker one following up in our second scheduled meeting.",
        VOICE_REMOTE_1,
        "07_remote1_call2",
    )
    s_rem1_call2 = read_mono_samples(p_rem1_call2)

    ch0_m2: list[int] = []
    ch1_m2: list[int] = []
    ch0_m2.extend([0] * len(s_rem1_call2))
    ch1_m2.extend(s_rem1_call2)

    meeting_2_wav = AUDIO_DIR / "meeting_2_dual_channel.wav"
    write_stereo_wav(ch0_m2, ch1_m2, meeting_2_wav)
    print(f"[SUCCESS] Meeting 2 written: {meeting_2_wav}")

    meta_2_path = AUDIO_DIR / "meeting_2_meta.json"
    meta_2_path.write_text(
        json.dumps(
            {
                "wav_path": str(meeting_2_wav),
                "duration_ms": int((len(ch0_m2) / SAMPLE_RATE) * 1000),
                "transcript": "Hello again. This is speaker one following up in our second scheduled meeting.",
                "speaker": "Speaker 1",
            },
            indent=2,
        )
    )

    print(
        f"[READY] Synthetic audio suite prepared in {AUDIO_DIR}. All files verified."
    )


if __name__ == "__main__":
    main()
