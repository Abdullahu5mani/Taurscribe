#!/usr/bin/env python3
"""Run only hermetic harness tests; never discover live/resetting E2E scripts."""
import unittest

MODULES = ["test_decider", "test_decider_regressions", "test_decider_backends",
           "test_meeting_observer", "test_recording_check", "test_guest_decisions", "test_asr_quality",
           "test_file_model_selection", "test_decider_model_acceptance", "test_decider_policy"]

if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromNames(MODULES))
    raise SystemExit(0 if result.wasSuccessful() else 1)
