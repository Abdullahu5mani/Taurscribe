#!/usr/bin/env python3
"""Unit tests for MeetingDecider and Jev Ultrafast DOM action selection."""

import unittest
from decider import MeetingDecider, DeciderAction


class TestMeetingDecider(unittest.TestCase):
    def setUp(self):
        self.decider = MeetingDecider()

    def test_in_call_done(self):
        snapshot = {"is_in_call": True, "elements": []}
        action = self.decider.decide(snapshot)
        self.assertEqual(action.operation, "DONE")

    def test_dismiss_warning_popup(self):
        snapshot = {
            "is_in_call": False,
            "elements": [
                {"index": 1, "tag": "button", "role": "button", "text": "Continue without microphone", "disabled": False, "checked": False},
                {"index": 2, "tag": "button", "role": "button", "text": "Join now", "disabled": True, "checked": False},
            ]
        }
        action = self.decider.decide(snapshot)
        self.assertEqual(action.operation, "CLICK")
        self.assertEqual(action.target, 1)

    def test_mute_mic_and_cam(self):
        snapshot = {
            "is_in_call": False,
            "elements": [
                {"index": 1, "tag": "button", "role": "button", "text": "Turn off microphone", "disabled": False, "checked": False},
                {"index": 2, "tag": "button", "role": "button", "text": "Turn off camera", "disabled": False, "checked": False},
                {"index": 3, "tag": "input", "role": "textbox", "tag": "input", "type": "text", "text": "Your name", "value": "Dr. Sarah Chen", "disabled": False, "checked": False},
                {"index": 4, "tag": "button", "role": "button", "text": "Ask to join", "disabled": False, "checked": False},
            ]
        }
        action = self.decider.decide(snapshot, guest_name="Dr. Sarah Chen")
        self.assertEqual(action.operation, "CLICK")
        self.assertEqual(action.target, 1)

    def test_fill_empty_name(self):
        snapshot = {
            "is_in_call": False,
            "elements": [
                {"index": 1, "tag": "input", "role": "textbox", "type": "text", "text": "Your name", "value": "", "disabled": False, "checked": False},
                {"index": 2, "tag": "button", "role": "button", "text": "Ask to join", "disabled": True, "checked": False},
            ]
        }
        action = self.decider.decide(snapshot, guest_name="Dr. Sarah Chen")
        self.assertEqual(action.operation, "TYPE_TEXT")
        self.assertEqual(action.target, 1)
        self.assertEqual(action.text, "Dr. Sarah Chen")

    def test_click_ask_to_join(self):
        snapshot = {
            "is_in_call": False,
            "elements": [
                {"index": 1, "tag": "input", "role": "textbox", "type": "text", "text": "Your name", "value": "Dr. Sarah Chen", "disabled": False, "checked": False},
                {"index": 2, "tag": "button", "role": "button", "text": "Ask to join", "disabled": False, "checked": False},
            ]
        }
        action = self.decider.decide(snapshot, guest_name="Dr. Sarah Chen")
        self.assertEqual(action.operation, "CLICK")
        self.assertEqual(action.target, 2)

    def test_host_admit_guest(self):
        snapshot = {
            "is_in_call": False,
            "admit_request": {"text": "Participant waiting to join", "admit_target": 5},
            "elements": [
                {"index": 5, "tag": "button", "role": "button", "text": "Admit", "disabled": False, "checked": False},
                {"index": 6, "tag": "button", "role": "button", "text": "Deny", "disabled": False, "checked": False},
            ]
        }
        action = self.decider.decide(snapshot)
        self.assertEqual(action.operation, "CLICK")
        self.assertEqual(action.target, 5)


if __name__ == "__main__":
    unittest.main()
