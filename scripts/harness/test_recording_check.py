"""Recording preflight regressions; no browser, model or AX permission needed."""
import unittest
import copy
from types import SimpleNamespace
from unittest.mock import Mock, patch

import recording_check


class ModelPreflightTests(unittest.TestCase):
    def test_ready_model_does_not_request_another_load(self):
        ready = {"elements": [{"text": "Engine status READY"}]}
        checked = []
        with patch.object(recording_check, "app_snapshot", return_value=ready), \
             patch.object(recording_check.ax_driver, "press") as press:
            recording_check.ensure_model_loaded(SimpleNamespace(pid=123),
                SimpleNamespace(find_load_model_control=lambda snap: None),
                SimpleNamespace(log=lambda *args: None, check=lambda ok, *args: checked.append(ok)))
        press.assert_not_called()
        self.assertEqual(checked, [True])


class ContinuationOracleTests(unittest.TestCase):
    def fixtures(self):
        before = {'id': 3, 'duration_ms': 5000, 'transcript_raw': 'Earlier architectural discussion.'}
        after = {'id': 3, 'duration_ms': 12000,
                 'transcript_raw': before['transcript_raw'] + ' ' + recording_check.CONTINUATION_CLIP[1],
                 'turns': [{'channel': 1, 'start_ms': 6000, 'text': recording_check.CONTINUATION_CLIP[1]}]}
        return before, after

    def checks(self, before, after, ids_after=None, parts_after=2):
        return recording_check.continuation_checks(before, after, {3, 4}, ids_after or {3, 4}, 1, parts_after)

    def test_valid_append_passes_all_independent_postconditions(self):
        self.assertTrue(all(self.checks(*self.fixtures()).values()))

    def test_separate_new_card_is_not_continuation(self):
        before, after = self.fixtures()
        after['id'] = 5
        self.assertFalse(all(self.checks(before, after, {3, 4, 5}).values()))

    def test_replayed_original_without_new_speech_is_not_success(self):
        before, after = self.fixtures()
        after['transcript_raw'] = before['transcript_raw']
        after['turns'] = []
        self.assertFalse(all(self.checks(before, after).values()))

    def test_losing_old_speech_is_not_success(self):
        before, after = self.fixtures()
        after['transcript_raw'] = recording_check.CONTINUATION_CLIP[1]
        self.assertFalse(all(self.checks(before, after).values()))

    def test_wrong_channel_or_pre_seam_speech_is_not_success(self):
        before, after = self.fixtures()
        for change in ({'channel': 0}, {'start_ms': 0}):
            bad = copy.deepcopy(after)
            bad['turns'][0].update(change)
            self.assertFalse(all(self.checks(before, bad).values()))

    def test_duplicate_append_or_unchanged_duration_is_not_success(self):
        before, after = self.fixtures()
        self.assertFalse(all(self.checks(before, after, parts_after=3).values()))
        after['duration_ms'] = before['duration_ms']
        self.assertFalse(all(self.checks(before, after).values()))


class SelectedMeetingOracleTests(unittest.TestCase):
    def test_similar_transcript_in_an_older_meeting_is_rejected(self):
        snapshot = {'elements': [{'role': 'AXTextField', 'text': 'Meeting title', 'value': 'Old room'}]}
        self.assertFalse(recording_check.saved_meeting_is_selected(snapshot, {'title': 'New room'}))

    def test_review_title_does_not_prove_catalog_selection(self):
        snapshot = {'elements': [
            {'role': 'AXTextField', 'text': 'Meeting title', 'value': 'Old room'},
            {'role': 'AXTextField', 'text': 'Meeting title', 'value': 'New room'},
            {'text': 'Save & View Notes'}]}
        self.assertFalse(recording_check.saved_meeting_is_selected(snapshot, {'title': 'New room'}))

    def test_exact_selected_meeting_without_review_modal_is_accepted(self):
        snapshot = {'elements': [{'role': 'AXTextField', 'text': 'Meeting title', 'value': 'New room'}]}
        self.assertTrue(recording_check.saved_meeting_is_selected(snapshot, {'title': 'New room'}))


class PauseContractTests(unittest.TestCase):
    def test_successful_but_unsupported_pause_fails_the_oracle(self):
        response = Mock(status=200)
        response.read.return_value = b'{"success": true, "detail": "Capture paused"}'
        response.__enter__ = Mock(return_value=response)
        response.__exit__ = Mock(return_value=False)
        def check(ok, *args):
            if not ok:
                raise AssertionError(args[0])
        with patch.dict('os.environ', {'TAURSCRIBE_CONTROL_TOKEN': 'unit-test-token'}), \
             patch.object(recording_check, 'api', return_value={'recording': {'is_recording': True}}), \
             patch.object(recording_check.urllib.request, 'urlopen', return_value=response):
            with self.assertRaisesRegex(AssertionError, 'honestly rejects'):
                recording_check.verify_pause_rejected(SimpleNamespace(check=check), recording=True)

class ModelLoadFailureTests(unittest.TestCase):
    def test_failed_load_is_reported_without_waiting_for_the_full_timeout(self):
        load = {"elements": [{"text": "Engine status LOAD REQUIRED"}, {"index": 0, "text": "Load model"}]}
        failed = {"elements": [{"text": "Engine status ERROR"}]}
        checked = []
        with patch.object(recording_check, "app_snapshot", side_effect=[load, failed, failed]), \
             patch.object(recording_check.ax_driver, "press"), \
             patch.object(recording_check.time, "sleep") as sleep:
            recording_check.ensure_model_loaded(SimpleNamespace(pid=123),
                SimpleNamespace(find_load_model_control=lambda snap: snap["elements"][-1]),
                SimpleNamespace(log=lambda *args: None, check=lambda ok, *args: checked.append(ok)))
        sleep.assert_not_called()
        self.assertEqual(checked, [False])

    def test_missing_engine_status_is_not_treated_as_ready(self):
        checked = []
        with patch.object(recording_check, "app_snapshot", return_value={"elements": []}):
            recording_check.ensure_model_loaded(SimpleNamespace(pid=123),
                SimpleNamespace(find_load_model_control=lambda snap: None),
                SimpleNamespace(log=lambda *args: None, check=lambda ok, *args: checked.append(ok)))
        self.assertEqual(checked, [False])

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


class MicrophonePreferenceTests(unittest.TestCase):
    def test_failed_capture_is_stopped_before_native_input_is_restored(self):
        events = []
        response = Mock()
        response.read.return_value = b'{"success": true}'
        response.__enter__ = Mock(return_value=response)
        response.__exit__ = Mock(return_value=False)
        def stop(request, **kwargs):
            events.append(('stop', request.full_url))
            return response
        with patch.dict('os.environ', {'TAURSCRIBE_CONTROL_TOKEN': 'unit-test-token'}), \
             patch.object(recording_check, 'api', side_effect=[
                 {'recording': {'selected_input_device': 'Original', 'is_recording': False}},
                 {'recording': {'is_recording': True}}]), \
             patch.object(recording_check, 'select_app_microphone', side_effect=lambda app, name, report: events.append(('input', name))), \
             patch.object(recording_check.urllib.request, 'urlopen', side_effect=stop):
            with self.assertRaisesRegex(RuntimeError, 'capture failed'):
                with recording_check.app_microphone_for_test(SimpleNamespace(), SimpleNamespace(log=Mock())):
                    raise RuntimeError('capture failed')
        self.assertEqual(events, [('input', recording_check.LOCAL_MIC_DEVICE),
                                 ('stop', 'http://127.0.0.1:8766/api/capture/stop'), ('input', 'Original')])

    def test_restores_explicit_native_preference_after_capture_failure(self):
        app, report = SimpleNamespace(), SimpleNamespace(log=Mock())
        with patch.object(recording_check, 'api', return_value={'recording': {'selected_input_device': 'Original microphone'}}), \
             patch.object(recording_check, 'select_app_microphone') as select:
            with self.assertRaisesRegex(RuntimeError, 'capture failed'):
                with recording_check.app_microphone_for_test(app, report):
                    raise RuntimeError('capture failed')
        self.assertEqual([call.args[1] for call in select.call_args_list],
                         [recording_check.LOCAL_MIC_DEVICE, 'Original microphone'])

    def test_system_default_preference_is_restored_as_default(self):
        with patch.object(recording_check, 'api', return_value={'recording': {'selected_input_device': None}}), \
             patch.object(recording_check, 'select_app_microphone') as select:
            with recording_check.app_microphone_for_test(SimpleNamespace(), SimpleNamespace(log=Mock())):
                pass
        self.assertEqual([call.args[1] for call in select.call_args_list], [recording_check.LOCAL_MIC_DEVICE, None])

    def test_failed_setup_still_attempts_original_preference(self):
        with patch.object(recording_check, 'api', return_value={'recording': {'selected_input_device': 'Original'}}), \
             patch.object(recording_check, 'select_app_microphone', side_effect=[RuntimeError('input failed'), None]) as select:
            with self.assertRaisesRegex(RuntimeError, 'input failed'):
                with recording_check.app_microphone_for_test(SimpleNamespace(), SimpleNamespace(log=Mock())):
                    self.fail('Cannot capture after failed setup')
        self.assertEqual(select.call_args_list[-1].args[1], 'Original')


if __name__ == "__main__":
    unittest.main()
