"""The guest driver must respect WAIT and required-model failure."""
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import cdp_guest
from meeting_decider import DeciderAction
from test_decider_meeting_detection import GuestCDP
from platforms import GoogleMeet


class GuestDecisionTests(unittest.TestCase):
    def guest(self):
        page = Mock()
        page.evaluate.return_value = {"elements": [{"index": 1, "role": "button", "text": "Ask to join"}]}
        return SimpleNamespace(page=page, debug_dir=None, is_in_call=lambda: False)

    def test_wait_does_not_fall_through_to_legacy_clicks_or_typing(self):
        guest = self.guest()
        decider = SimpleNamespace(require_model=False, decide=lambda **kwargs: DeciderAction("WAIT"))
        clock = SimpleNamespace(now=100.0)
        with patch("meeting_decider.MeetingDecider", return_value=decider), \
             patch.object(cdp_guest.time, "time", side_effect=lambda: clock.now), \
             patch.object(cdp_guest.time, "sleep", side_effect=lambda seconds: setattr(clock, "now", clock.now + seconds)):
            self.assertFalse(cdp_guest.ChromeGuestSession.auto_join(guest, timeout=1))
        guest.page.click.assert_not_called()
        guest.page.type_text.assert_not_called()
        self.assertIs(guest.decider, decider)

    def test_required_model_failure_reaches_the_test_runner(self):
        guest = self.guest()
        decider = SimpleNamespace(require_model=True, decide=Mock(side_effect=RuntimeError("model unavailable")))
        with patch("meeting_decider.MeetingDecider", return_value=decider):
            with self.assertRaisesRegex(RuntimeError, "model unavailable"):
                cdp_guest.ChromeGuestSession.auto_join(guest, timeout=1)
        guest.page.click.assert_not_called()

    def leave_guest(self):
        page = Mock()
        page.evaluate.side_effect = lambda script: [1200, 800] if script == '[innerWidth, innerHeight]' else {}
        return SimpleNamespace(session=SimpleNamespace(page=page, is_in_call=lambda: False),
                               report=SimpleNamespace(log=Mock()), stop=Mock())

    def test_leave_wait_and_empty_observation_never_count_as_confirmation(self):
        guest = self.leave_guest()
        decider = SimpleNamespace(decide_leave=Mock(return_value=DeciderAction('WAIT')))
        with patch('test_decider_meeting_detection.time.sleep'), patch('recording_check.guest_click') as click:
            GuestCDP.leave(guest, decider)
        click.assert_not_called()
        self.assertEqual(decider.decide_leave.call_count, 5)
        guest.report.log.assert_called_once_with('GUEST', 'leave not confirmed; closing the guest browser')
        guest.stop.assert_called_once()

    def test_leave_click_requires_observed_completion(self):
        guest = self.leave_guest()
        decider = SimpleNamespace(decide_leave=Mock(side_effect=[DeciderAction('CLICK', 2), DeciderAction('WAIT'), DeciderAction('DONE')]))
        with patch('test_decider_meeting_detection.time.sleep'), patch('recording_check.guest_click') as click:
            GuestCDP.leave(guest, decider)
        click.assert_called_once_with(guest.session, 2)
        guest.report.log.assert_any_call('GUEST', 'left the call')
        guest.stop.assert_called_once()

    def test_failed_leave_observation_still_closes_only_the_test_guest(self):
        guest = self.leave_guest()
        decider = SimpleNamespace(decide_leave=Mock(side_effect=RuntimeError('unreadable')))
        GuestCDP.leave(guest, decider)
        guest.report.log.assert_called_once_with('GUEST', 'leave not confirmed; closing the guest browser')
        guest.stop.assert_called_once()


class MeetRosterEvidenceTests(unittest.TestCase):
    def test_named_in_call_roster_proves_guest_without_numerical_count(self):
        snapshot = {'elements': [{'role': 'AXGroup', 'text': 'Guest'},
                                 {'role': 'AXButton', 'text': "Mute Guest's microphone"}]}
        self.assertTrue(GoogleMeet().host_sees_guest(snapshot, 'Guest'))

    def test_pending_admission_name_does_not_prove_join(self):
        snapshot = {'elements': [{'role': 'AXGroup', 'text': 'Guest'},
                                 {'role': 'AXButton', 'text': 'Admit Guest'}]}
        self.assertIsNone(GoogleMeet().host_sees_guest(snapshot, 'Guest'))

    def test_other_guests_microphone_does_not_prove_the_requested_guest(self):
        snapshot = {'elements': [{'role': 'AXGroup', 'text': 'Guest'},
                                 {'role': 'AXButton', 'text': "Mute Someone Else's microphone"}]}
        self.assertIsNone(GoogleMeet().host_sees_guest(snapshot, 'Guest'))


if __name__ == "__main__":
    unittest.main()
