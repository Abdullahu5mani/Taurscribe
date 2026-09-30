"""Hermetic coverage for the native model-selection assertion."""
import unittest
from unittest.mock import patch

from test_file_models import selected_model_label


LABEL = "Select model Granite Speech 5 (470M), size 949 MB"


def radio(label=LABEL, checked=True):
    return {"role": "AXRadioButton", "text": label, "checked": checked}


class FakeApp:
    def __init__(self, snapshots):
        self.snapshots = iter(snapshots)
        self.last = []
        self.pressed = []

    def elements(self):
        self.last = next(self.snapshots, self.last)
        return self.last

    def wait(self, condition, timeout):
        assert timeout == 5
        for _ in range(3):
            result = condition()
            if result:
                return result
        return None

    def has(self, label):
        return label == "Engine picker"

    def press(self, label):
        self.pressed.append(label)


class ModelSelectionTests(unittest.TestCase):
    def select(self, snapshots):
        app = FakeApp(snapshots)
        with patch("test_file_models.installed_models") as opened:
            result = selected_model_label(app, "Granite")
        opened.assert_called_once_with(app, "Granite")
        self.assertEqual(app.pressed, ["Switch engine or model"])
        return result

    def test_reads_selected_model_and_ignores_other_radios(self):
        self.assertEqual(self.select([[radio("Microphone dictation mode"), radio()]]), LABEL)

    def test_waits_for_delayed_selected_accessibility_value(self):
        self.assertEqual(self.select([[], [radio(checked=False)], [radio()]]), LABEL)

    def test_missing_selection_still_fails_after_bounded_wait(self):
        self.assertIsNone(self.select([[radio(checked=False)]]))

    def test_ambiguous_selection_is_not_accepted(self):
        self.assertIsNone(self.select([[radio(), radio("Select model Other")]]))
