"""Backend wire/validation tests. Fake responses test adapters, not model accuracy."""
import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from types import SimpleNamespace
from unittest.mock import patch

from meeting_decider import DeciderAction, MeetingDecider
from test_decider_regressions import element


class BackendTests(unittest.TestCase):
    def setUp(self):
        env = patch.dict("os.environ", {}, clear=True)
        env.start()
        self.addCleanup(env.stop)
        self.snapshot = {"elements": [element(0, "Join now"), element(1, "Chat", "AXStaticText")]}

    def test_systemone_uses_the_documented_choice_endpoint(self):
        requests = []

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                requests.append((self.path, body))
                choices = body["questions"]["next_action"]["criteria"]
                choice = next(k for k in choices if k.startswith("CLICK [0]"))
                self.send_response(200)
                self.end_headers()
                self.wfile.write(json.dumps({"answers": {"next_action": {"choice": choice}}}).encode())

        server = HTTPServer(("127.0.0.1", 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            decider = MeetingDecider(api_base=f"http://127.0.0.1:{server.server_port}/v1", backend="systemone", require_model=True)
            action = decider.decide(self.snapshot)
            self.assertEqual((action.operation, action.target, action.mode), ("CLICK", 0, "systemone"))
            self.assertEqual(requests[0][0], "/v1/systemone")
            self.assertNotIn("messages", requests[0][1])
            self.assertEqual(decider.decision_counts, {"systemone": 1})
        finally:
            server.shutdown()
            worker.join(timeout=2)
            server.server_close()

    def test_native_uses_choice_not_prediction_and_includes_wait(self):
        with patch.object(MeetingDecider, "_check_available_runners"):
            decider = MeetingDecider(backend="native", require_model=True)
        native_calls = []

        def native(context, questions):
            native_calls.append(questions)
            return [{"choice": "CLICK [0] Join now", "confidence": 0.9}]

        decider._decider_lib = SimpleNamespace(decide=native)
        action = decider.decide(self.snapshot)
        self.assertEqual((action.target, action.mode), (0, "native"))
        self.assertIn("WAIT", native_calls[0][0]["options"])
        self.assertNotIn("CLICK [1] Chat", native_calls[0][0]["options"])

    def test_native_abstention_means_wait(self):
        with patch.object(MeetingDecider, "_check_available_runners"):
            decider = MeetingDecider(backend="native")
        decider._decider_lib = SimpleNamespace(decide=lambda *args: [{"choice": None}])
        self.assertEqual(decider.decide(self.snapshot).operation, "WAIT")

    def test_invalid_model_actions_are_rejected_and_fallback_is_reported(self):
        invalid = [
            DeciderAction("CLICK", 999), DeciderAction("CLICK", "0"),
            DeciderAction("CLICK", True), DeciderAction("CLICK", 1),
            DeciderAction("CLICK", 0, text="unexpected"), DeciderAction("TYPE_TEXT", 0, text="name"),
            DeciderAction("WAIT", 0), DeciderAction("DONE"), DeciderAction("DELETE", 0),
        ]
        for response in invalid:
            with self.subTest(response=response):
                decider = MeetingDecider(backend="chat")
                with patch.object(decider, "_ask_http_action", return_value=response):
                    action = decider.decide(self.snapshot)
                self.assertEqual((action.target, action.mode), (0, "system1"))
                self.assertEqual(decider.fallbacks, ["ValueError"])
                self.assertNotIn("chat", decider.decision_counts)

    def test_disabled_and_duplicate_indices_cannot_be_clicked(self):
        for elements in ([dict(element(0, "Join now"), disabled=True)], [element(0, "Join now"), element(0, "Leave call")]):
            with self.subTest(elements=elements):
                with self.assertRaises(ValueError):
                    self.decider_validate(DeciderAction("CLICK", 0), {"elements": elements})

    def decider_validate(self, action, snapshot):
        MeetingDecider()._validate_action(action, snapshot)

    def test_required_model_fails_instead_of_silently_using_rules(self):
        decider = MeetingDecider(backend="chat", require_model=True)
        with patch.object(decider, "_ask_http_action", side_effect=TimeoutError):
            with self.assertRaisesRegex(RuntimeError, "decision failed"):
                decider.decide(self.snapshot)
        self.assertEqual(decider.decision_counts, {})

    def test_timeout_fallback_records_actual_mode(self):
        decider = MeetingDecider(backend="systemone")
        with patch.object(decider, "_decide_choices_http", side_effect=TimeoutError):
            action = decider.decide(self.snapshot)
        self.assertEqual(action.to_dict()["mode"], "system1")
        self.assertEqual(decider.fallbacks, ["TimeoutError"])

    def test_host_cannot_declare_done_with_a_pending_admission(self):
        decider = MeetingDecider(backend="chat", require_model=True)
        snap = {"elements": [element(0, "Admit Guest"), element(2, "Leave call")]}
        with patch.object(decider, "_ask_http_action", return_value=DeciderAction("DONE")):
            with self.assertRaises(RuntimeError):
                decider.decide_host(snap)

    def test_host_done_requires_observed_call_control(self):
        decider = MeetingDecider(backend="chat", require_model=True)
        with patch.object(decider, "_ask_http_action", return_value=DeciderAction("DONE")):
            action = decider.decide_host({"elements": [element(0, "Leave call")]})
        self.assertEqual(action.mode, "chat")

    def test_ui_assertions_do_not_ask_a_model_for_a_verdict(self):
        decider = MeetingDecider(backend="chat", require_model=True)
        with patch.object(decider, "_chat", side_effect=AssertionError("Must not consult model")):
            verdict = decider.assess_meeting_detection({"elements": [element(0, "Active call detected: Google Meet. Click to record")]}, "meet")
        self.assertTrue(verdict["detected"])
        self.assertEqual(verdict["mode"], "system1")

    def test_empty_observation_is_unknown_not_clear(self):
        self.assertIsNone(MeetingDecider().assess_meeting_detection({"elements": []}, "meet")["detected"])

    def test_configuration_mistakes_fail_early(self):
        with self.assertRaises(ValueError):
            MeetingDecider(backend="made-up")
        with self.assertRaises(ValueError):
            MeetingDecider(backend="rules", require_model=True)

    def test_malformed_chat_replies_do_not_count_as_model_success(self):
        for reply in ("{}", "[]", "no JSON here", '{"operation": 5}', '{"operation":"DELETE","target":0}'):
            with self.subTest(reply=reply):
                decider = MeetingDecider(backend="chat", require_model=True)
                with patch.object(decider, "_chat", return_value=reply):
                    with self.assertRaises(RuntimeError):
                        decider.decide(self.snapshot)
                self.assertEqual(decider.decision_counts, {})

    def test_chat_reply_can_type_a_name_into_a_native_input(self):
        decider = MeetingDecider(backend="chat", require_model=True)
        snap = {"elements": [{"index": 0, "role": "AXTextField", "text": "Your name"}]}
        with patch.object(decider, "_chat", return_value='{"operation":"TYPE_TEXT","target":0,"text":"Guest"}') as chat:
            action = decider.decide(snap, guest_name="Guest")
        self.assertEqual((action.operation, action.text, action.mode), ("TYPE_TEXT", "Guest", "chat"))
        self.assertIn("TYPE_TEXT", chat.call_args[0][0])


if __name__ == "__main__":
    unittest.main()
