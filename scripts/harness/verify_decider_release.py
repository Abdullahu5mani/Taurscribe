#!/usr/bin/env python3
"""Explicit real-weight release acceptance; never included in unit discovery."""
import argparse
import json
import time
from pathlib import Path

from decider_benchmark import PINNED_REVISION
from decider_cases import control
from meeting_decider import MeetingDecider


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model-dir', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    if args.model_dir.name != PINNED_REVISION or not (args.model_dir / 'model.safetensors').is_file():
        parser.error('Existing pinned Decider snapshot required')
    import torch
    if not torch.backends.mps.is_available():
        parser.error('MPS required for this macOS acceptance check')

    def memory():
        torch.mps.synchronize()
        return {'tensor_bytes': torch.mps.current_allocated_memory(),
                'driver_bytes': torch.mps.driver_allocated_memory()}

    report = {'revision': PINNED_REVISION, 'snapshots': {}, 'checks': {}}
    try:
        d = MeetingDecider(model_name=str(args.model_dir), backend='native', require_model=True)
        report['snapshots']['before_load'] = memory()
        report['checks']['lazy_initialization'] = d._decider_lib is None
        started = time.monotonic()
        d.load_model()
        report['load_seconds'] = time.monotonic() - started
        report['snapshots']['loaded'] = memory()
        report['checks']['real_weights_loaded'] = (
            d._decider_lib is not None and
            report['snapshots']['loaded']['tensor_bytes'] - report['snapshots']['before_load']['tensor_bytes'] > 1_000_000_000)
        snapshot = {'elements': [control(2, 'Open settings'), control(8, 'Start recording')]}
        action = d.decide_ui(snapshot, 'Open Taurscribe settings')
        report['action'] = action.to_dict()
        report['checks']['real_inference'] = action.mode == 'native'
        report['checks']['release_owned_weights'] = d.suspend_model() and d._decider_lib is None
        report['snapshots']['suspended'] = memory()
        report['checks']['tensor_memory_released'] = (
            report['snapshots']['suspended']['tensor_bytes'] <=
            report['snapshots']['before_load']['tensor_bytes'] + 64 * 1024 * 1024)
        action = d.decide({'elements': [control(2, 'Join now')]})
        report['checks']['rules_during_capture'] = action.mode == 'system1' and action.target == 2
        d.load_model()
        report['checks']['suspended_cannot_reload'] = d._decider_lib is None
        d.resume_model()
        report['checks']['resume_is_lazy'] = d._decider_lib is None
        report['snapshots']['resumed_lazy'] = memory()
    except Exception as error:
        report['error'] = type(error).__name__ + ': ' + str(error)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    return 0 if 'error' not in report and all(report['checks'].values()) else 1


if __name__ == '__main__':
    raise SystemExit(main())
