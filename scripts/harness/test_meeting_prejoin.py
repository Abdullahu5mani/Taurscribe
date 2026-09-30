#!/usr/bin/env python3
"""Native Mac negative test: mic-enabled Meet lobbies must not trigger detection.

Supply a disposable room URL. Creates and closes only its own Chrome windows.
Does not join, reset history, record audio, or change input-device settings.
"""
import argparse
import re
import time

from meeting_decider import MeetingDecider
from test_decider_meeting_detection import AppObserver, ChromeHostAX, Report, api_get


def run(url, windows=1):
    report = Report("meet-prejoin")
    decider = MeetingDecider(backend="rules")
    hosts = []
    try:
        status = api_get("/api/status")
        report.check(not status["recording"]["is_recording"] and not status["detector"]["active_meetings"],
                     "App is idle before opening test lobbies")
        observer = AppObserver(api_get("/api/health")["pid"], decider, report)
        for index in range(windows):
            host = ChromeHostAX(decider, report)
            hosts.append(host)
            host.open_window(url)
            host.window = url.rstrip("/").rsplit("/", 1)[-1]
            host.wait_window(timeout=30)
            deadline = time.monotonic() + 30
            snap = {}
            while time.monotonic() < deadline:
                snap = host.snapshot()
                labels = [e.get("text", "") for e in snap.get("elements", [])]
                if any(re.match(r"^(join now|ask to join)\b", t, re.I) for t in labels):
                    break
                time.sleep(0.5)
            report.save_snapshot(f"lobby_{index + 1}", snap)
            report.check(any(re.match(r"^(join now|ask to join)\b", t, re.I) for t in labels),
                         f"Lobby {index + 1} is on the pre-join page")
            report.check(any(re.match(r"^turn off (microphone|mic)\b", t, re.I) for t in labels),
                         f"Lobby {index + 1} has its microphone enabled")
            report.check(not decider.has_leave_control(snap), f"Lobby {index + 1} has not joined the call")
        verdict = observer.wait_for(False, "Google Meet", timeout=20)
        report.save_snapshot("app_with_lobbies", verdict["_snapshot"])
        report.check(verdict["detected"] is False and not verdict.get("timed_out"),
                     "Native app remains clear for the whole observation window")
        status = api_get("/api/status")
        report.check(not status["detector"]["active_meetings"] and not status["detector"]["scanned_meetings"],
                     "Detector and scanner both reject pre-join pages")
        report.check(not status["recording"]["is_recording"], "No recording was started")
        report.finish(True)
        return True
    except Exception as error:
        report.log("ERROR", str(error))
        report.finish(False, str(error))
        return False
    finally:
        for host in reversed(hosts):
            host.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True, help="Disposable Google Meet room URL")
    parser.add_argument("--windows", type=int, choices=[1, 2], default=1)
    args = parser.parse_args()
    if not re.fullmatch(r"https://meet\.google\.com/[a-z]{3}-[a-z]{4}-[a-z]{3}/?", args.url):
        parser.error("Expected a Google Meet room URL")
    raise SystemExit(0 if run(args.url, args.windows) else 1)
