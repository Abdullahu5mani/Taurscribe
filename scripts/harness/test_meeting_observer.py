"""A missing or flickering AX observation must never count as a stable verdict."""
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import test_decider_meeting_detection as suite
from meeting_decider import MeetingDecider


class ObserverTests(unittest.TestCase):
    def check_sequence(self, sequence, want=False, timeout=15):
        clock = SimpleNamespace(now=100.0)
        report = SimpleNamespace(log=lambda *args, **kwargs: None)
        observer = suite.AppObserver(123, MeetingDecider(backend="rules"), report)
        iterator = iter(sequence)
        last = sequence[-1]

        def look(_):
            value = next(iterator, last)
            return {"detected": value, "mode": "system1", "reason": "fixture"}

        observer.look = look
        with patch.object(suite.time, "time", side_effect=lambda: clock.now), \
             patch.object(suite.time, "sleep", side_effect=lambda seconds: setattr(clock, "now", clock.now + seconds)):
            return observer.wait_for(want, "meet", timeout)

    def test_unreadable_ui_times_out_instead_of_passing_clear(self):
        self.assertTrue(self.check_sequence([None], timeout=8)["timed_out"])

    def test_empty_snapshot_interrupts_stability_window(self):
        verdict = self.check_sequence([False, False, False, None, False])
        self.assertEqual(verdict["latency_s"], 4.0)
        self.assertNotIn("timed_out", verdict)

    def test_transient_detection_interrupts_clear_window(self):
        verdict = self.check_sequence([False, False, True, False])
        self.assertEqual(verdict["latency_s"], 3.0)

    def test_detected_verdict_must_hold_for_the_entire_window(self):
        verdict = self.check_sequence([False, True, True, None, True], want=True)
        self.assertEqual(verdict["latency_s"], 4.0)


if __name__ == "__main__":
    unittest.main()
