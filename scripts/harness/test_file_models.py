#!/usr/bin/env python3
"""Run one real file import through every installed ASR model in the app.

Uses the native Open panel and the Files screen, not the transcription backend
directly. The test removes its file cards and restores the selected engine/model.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sqlite3
import sys
from pathlib import Path

from recording_check import coverage
from test_ui_full import (
    APP_SUPPORT,
    FILE_FIXTURES,
    HISTORY_DB,
    App,
    UIReport,
    file_done,
    installed_models,
    load_model,
    open_via_dialog,
)
from voiceprint_check import build_clip


ENGINES = ("Whisper", "Granite", "Qwen3-ASR")
ENGINE_KEYS = {"Whisper": "whisper", "Granite": "granite", "Qwen3-ASR": "qwen3"}
SETTING_KEYS = {"Whisper": "whisper_model", "Granite": "granite_model", "Qwen3-ASR": "qwen3_model"}


def history_ids() -> set[int]:
    with sqlite3.connect(HISTORY_DB) as db:
        return {row[0] for row in db.execute("SELECT id FROM transcriptions")}


def new_history_rows(before: set[int]) -> list[tuple]:
    with sqlite3.connect(HISTORY_DB) as db:
        return db.execute(
            "SELECT id, transcript, engine, model_id, audio_source FROM transcriptions ORDER BY id DESC"
        ).fetchall() if not before else db.execute(
            "SELECT id, transcript, engine, model_id, audio_source FROM transcriptions "
            "WHERE id > ? ORDER BY id DESC", (max(before),)
        ).fetchall()


def selected_model_label(app: App, engine: str) -> str | None:
    installed_models(app, engine)
    selected = next(
        (e["text"] for e in app.elements()
         if e["role"] == "AXRadioButton" and e["text"].startswith("Select model ") and e["checked"]),
        None,
    )
    if app.has("Engine picker"):
        app.press("Switch engine or model")
    return selected


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engines", nargs="+", choices=ENGINES, default=list(ENGINES))
    args = parser.parse_args()
    report = UIReport()
    app = App(report)
    app.wait_ready()
    app.go("mic")

    original_engine = json.loads((APP_SUPPORT / "settings.json").read_text()).get("active_engine", "whisper")
    original_picker = {"whisper": "Whisper", "granite": "Granite", "qwen3": "Qwen3-ASR"}.get(original_engine, "Whisper")
    original_label = selected_model_label(app, original_picker)

    source, words = build_clip("1089", "134691", 20)
    FILE_FIXTURES.mkdir(parents=True, exist_ok=True)
    fixture = FILE_FIXTURES / "qa_all_models_reader_1089.wav"
    shutil.copyfile(source, fixture)

    try:
        for engine in args.engines:
            for label in installed_models(app, engine):
                model_name = label.removeprefix("Select model ").split(", size")[0]
                report.section = f"files {engine} {model_name}"
                print(f"\n>>> {report.section}", flush=True)
                if not load_model(app, report, engine, label, timeout=300):
                    report.check(False, "Model loads", label)
                    continue
                report.check(True, "Model loads")
                selected = selected_model_label(app, engine)
                report.check(selected == label, "Requested model is selected", f"selected={selected}")
                expected_id = json.loads((APP_SUPPORT / "settings.json").read_text()).get(SETTING_KEYS[engine])
                app.go("files")
                report.check(app.has("Audio file drop zone") and not app.has("File transcription unavailable"),
                             "Files drop zone is enabled")

                before = history_ids()
                browse = "Browse more audio files" if app.has("Browse more audio files") else "Browse audio files"
                opened = open_via_dialog(app, report, fixture, browse)
                report.check(opened, "Native file picker accepts WAV")
                if not opened:
                    app.go("mic")
                    continue

                done = app.wait(lambda: file_done(app, fixture.name), 300, every=2)
                report.check(bool(done), "File transcription finishes")
                if done:
                    app.press(f"Show transcript for {fixture.name}")
                    rows = [r for r in new_history_rows(before) if r[4] == fixture.name]
                    row = rows[0] if rows else None
                    report.check(row is not None, "File result saved to history")
                    if row:
                        expected_engine = ENGINE_KEYS[engine]
                        report.check(expected_engine in (row[2] or "").lower(),
                                     "History uses selected engine", f"engine={row[2]}, model={row[3]}")
                        report.check(bool(expected_id) and row[3] == expected_id,
                                     "History uses selected model", f"expected={expected_id}, saved={row[3]}")
                        score = coverage(words, row[1])
                        report.check(score >= 0.60, "Transcript covers source speech",
                                     f"{score:.0%}: {row[1][:100]}")
                app.screenshot(f"file_{engine}_{model_name.replace(' ', '_')}")
                if app.find(f"Remove {fixture.name}"):
                    app.press(f"Remove {fixture.name}")
                app.go("mic")
    finally:
        if original_label:
            load_model(app, report, original_picker, original_label, timeout=300)
        app.go("files")
        while app.find(f"Remove {fixture.name}"):
            app.press(f"Remove {fixture.name}")
        app.go("mic")
        fixture.unlink(missing_ok=True)

    return report.finish()


if __name__ == "__main__":
    sys.exit(main())
