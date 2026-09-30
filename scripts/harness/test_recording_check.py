"""Recording preflight regressions; no browser, model or AX permission needed."""
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import recording_check


class ModelPreflightTests(unittest.TestCase):
    def test_waits_for_ready_after_the_load_button_disappears(self):
        snapshots = [
            {"elements": [{"text": "Switch engine or model; status LOAD REQUIRED"},
                          {"index": 1, "text": "Load model"}]},
            {"elements": [{"text": "Switch engine or model; status LOADING"}]},
            {"elements": [{"text": "Switch engine or model; status READY"}]},
            {"elements": [{"text": "Switch engine or model; status READY"}]},
        ]
        checked = []
        decider = SimpleNamespace(find_load_model_control=lambda snap: next(
            (e for e in snap["elements"] if e["text"] == "Load model"), None))
        report = SimpleNamespace(log=lambda *args: None,
                                 check=lambda ok, *args: checked.append(ok))
        with patch.object(recording_check, "app_snapshot", side_effect=snapshots), \
             patch.object(recording_check.ax_driver, "press") as press, \
             patch.object(recording_check.time, "sleep") as sleep:
            recording_check.ensure_model_loaded(SimpleNamespace(pid=123), decider, report)
        press.assert_called_once_with(123, 1)
        sleep.assert_called_once_with(0.5)
        self.assertEqual(checked, [True])


if __name__ == "__main__":
    unittest.main()
