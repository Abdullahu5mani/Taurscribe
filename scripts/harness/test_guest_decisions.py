"""The guest driver must respect WAIT and required-model failure."""
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import cdp_guest
from meeting_decider import DeciderAction


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


if __name__ == "__main__":
    unittest.main()
