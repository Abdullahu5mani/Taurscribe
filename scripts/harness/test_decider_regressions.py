"""Decision contracts: no live UI, weights, network, or sign-in required."""
import unittest
from unittest.mock import patch

from meeting_decider import MeetingDecider, PlanStep


def element(index, text, role="button", **extra):
    return dict(index=index, text=text, role=role, tag="button", type="",
                checked=False, disabled=False, **extra)


class DecisionRegressionTests(unittest.TestCase):
    def setUp(self):
        self.env = patch.dict("os.environ", {}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)
        self.decider = MeetingDecider()

    def test_empty_snapshot_is_not_proof_the_call_was_left(self):
        self.assertEqual(self.decider.decide_leave({"elements": []}).operation, "WAIT")

    def test_join_text_in_chat_is_not_a_control(self):
        snap = {"elements": [element(2, "Join now", "AXStaticText")]}
        self.assertEqual(self.decider.decide(snap).operation, "WAIT")

    def test_disabled_display_name_is_not_typed_into(self):
        field = element(0, "Your name", "textbox")
        field.update(tag="input", type="text", disabled=True)
        self.assertEqual(self.decider.decide({"elements": [field]}).operation, "WAIT")

    def test_search_placeholder_is_not_a_guest_name(self):
        field = element(0, "Search", "textbox", placeholder="Search participants")
        field.update(tag="input", type="text")
        self.assertEqual(self.decider.decide({"elements": [field]}).operation, "WAIT")

    def test_native_accessibility_input_needs_no_dom_fields(self):
        snap = {"elements": [{"index": 0, "role": "AXTextField", "text": "Your name", "value": ""}]}
        action = self.decider.decide(snap, guest_name="Guest")
        self.assertEqual((action.operation, action.target, action.text), ("TYPE_TEXT", 0, "Guest"))

    def test_retry_limit_resets_after_screen_changes(self):
        lobby = {"elements": [element(3, "Join now")]}
        self.decider.decide(lobby)
        self.decider.decide(lobby)
        self.assertEqual(self.decider.decide(lobby).operation, "WAIT")
        self.decider.decide({"elements": [element(4, "Waiting for admission", "AXStaticText")]})
        self.assertEqual(self.decider.decide(lobby).operation, "CLICK")

    def test_host_opens_popup_then_admits_plain_button(self):
        snap = {"elements": [element(3, "Admit one guest", "AXPopUpButton"), element(4, "Leave call")]}
        self.assertEqual(self.decider.decide_host(snap).target, 3)
        snap["elements"].append(element(8, "Admit Guest"))
        self.assertEqual(self.decider.decide_host(snap).target, 8)

    def test_host_waits_for_admission_panel_to_render(self):
        snap = {"elements": [element(3, "Guest wants to join", "AXStaticText"),
                             element(4, "People panel is open", "AXStaticText"), element(5, "Leave call")]}
        self.assertEqual(self.decider.decide_host(snap).operation, "WAIT")

    def test_plan_uses_furthest_visible_step(self):
        plan = [PlanStep("First", "^Open$"), PlanStep("Second", "^Join now$")]
        self.assertEqual(self.decider.decide_plan({"elements": [element(1, "Open"), element(2, "Join now")]}, plan).target, 2)

    def test_interrupt_takes_priority_over_plan_progress(self):
        plan = [PlanStep("Join", "^Join now$"), PlanStep("Permission", "^Allow once$", interrupt=True)]
        self.assertEqual(self.decider.decide_plan({"elements": [element(1, "Join now"), element(2, "Allow once")]}, plan).target, 2)

    def test_detection_does_not_trust_chat_text(self):
        verdict = self.decider.assess_meeting_detection({"elements": [element(1, "Chat: Active call detected", "AXStaticText")]}, "meet")
        self.assertFalse(verdict["detected"])

    def test_transcript_quoting_the_banner_is_not_meeting_evidence(self):
        snap = {"elements": [element(0, "Active call detected: Google Meet", "AXStaticText"),
                             *[element(i, "Navigation") for i in range(1, 7)]]}
        self.assertIs(self.decider.assess_meeting_detection(snap, "meet")["detected"], False)


if __name__ == "__main__":
    unittest.main()
