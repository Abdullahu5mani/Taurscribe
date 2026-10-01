#!/usr/bin/env python3
"""Explicit real Decider-2B acceptance; never run in hermetic unit discovery."""
from __future__ import annotations
import argparse
import hashlib
import importlib.metadata
import json
import math
import platform
import statistics
import time
from pathlib import Path

from decider_cases import cases, variants
from meeting_decider import MeetingDecider

PINNED_REVISION = '533964dae8be954c5b5e19fa4948e48408094c1e'


def evaluate(row, action):
    expected = row['expected']
    if action.operation != expected['operation']:
        return False
    if expected['label'] is None:
        return action.target is None and action.text is None
    matches = [e for e in row['snapshot']['elements'] if e['index'] == action.target]
    return (len(matches) == 1 and matches[0]['text'] in expected.get('labels', [expected['label']])
            and (action.operation != 'TYPE_TEXT' or action.text == 'Sarah Guest'))


def summary(rows):
    timed = sorted(row['latency_s'] for row in rows if row.get('model_calls', 0))
    model_rows = [row for row in rows if row.get('model_calls', 0)]
    return {'checks': len(rows), 'passed': sum(row['ok'] for row in rows),
            'accuracy': sum(row['ok'] for row in rows) / max(1, len(rows)),
            'model_checks': len(model_rows), 'model_passed': sum(row['ok'] for row in model_rows),
            'model_accuracy': sum(row['ok'] for row in model_rows)/len(model_rows) if model_rows else None,
            'model_calls': sum(row.get('model_calls', 0) for row in rows),
            'latency_p50_s': statistics.median(timed) if timed else None,
            'latency_p95_s': timed[min(len(timed)-1, math.ceil(len(timed)*.95)-1)] if timed else None}


class ObservedRunner:
    """Wrap actual model inference to retain probability and call-count evidence."""
    def __init__(self, runner, torch):
        self.runner, self.torch = runner, torch
        self.calls, self.last = 0, None

    def decide(self, context, questions):
        if len(self.runner.m.tok.encode(context)) > 4096:
            raise ValueError('Observation exceeds explicit native context limit; narrow it instead of truncating it')
        self.last = self.runner.decide(context, questions, max_ctx_tokens=4096)
        self.calls += 1
        if len(self.last) != len(questions):
            raise ValueError('Model output count does not match the questions')
        for result, question in zip(self.last, questions):
            probs = result['probs']
            if (not math.isfinite(result['confidence']) or not 0 <= result['confidence'] <= 1
                or set(probs) != set(question['options'])
                or not all(math.isfinite(p) and 0 <= p <= 1 for p in probs.values())
                or abs(sum(probs.values())-1) > .002):
                raise ValueError('Invalid probability distribution from actual model')
        return self.last


def model_decision(decider, row):
    if row['entry'] == 'join':
        return decider.decide(row['snapshot'], goal=row['goal'], guest_name='Sarah Guest')
    if row['entry'] == 'host':
        return decider.decide_host(row['snapshot'])
    return decider.decide_ui(row['snapshot'], row['goal'])


def navigation_snapshot(snapshot):
    """Keep every control and its state; omit duplicate prose for navigation."""
    fields = ('index', 'text', 'role', 'value', 'disabled', 'checked')
    return {'elements': [{k: item[k] for k in fields if k in item}
                         for item in snapshot['elements']
                         if MeetingDecider._is_clickable(item) or MeetingDecider._is_text_input(item)]}


def check_case(decider, runner, row, torch):
    before = runner.calls
    runner.last = None
    start = time.perf_counter()
    result = {'id': row['id'], 'ok': False, 'expected': row['expected']}
    try:
        action = model_decision(decider, row)
        torch.mps.synchronize()
        result['action'] = action.to_dict()
        invoked = runner.calls - before
        source_verified = ((invoked == 0 and action.mode == 'guard' and action.operation == 'WAIT')
                           or (invoked == 1 and action.mode == 'native'))
        result['ok'] = evaluate(row, action) and source_verified
        if runner.last:
            result['model_output'] = runner.last[0]
    except Exception as error:
        result['error'] = type(error).__name__ + ': ' + str(error)
    if runner.last:
        result['model_output'] = runner.last[0]
    result.update(model_calls=runner.calls-before, latency_s=time.perf_counter()-start)
    return result


def native_ui(decider, runner, torch, app=None, on_result=None):
    """Actual AX navigation; check each answer before pressing its expected control."""
    import ax_driver
    from test_ui_full import App, UIReport
    if app is None:
        app = App(UIReport())
        app.wait_ready()
    rows = []
    original_mode = next((e['text'] for e in app.elements() if e['role'] == 'AXRadioButton' and e['checked'] and e['text'].endswith('mode')), None)
    if app.has('Close settings'):
        raise RuntimeError('Close the existing settings panel before this isolated native run')
    steps = [('Open Taurscribe settings', 'Open settings', 'Close settings'),
             ('View the Meetings settings tab', 'Meetings', 'Meetings'),
             ('View the Recording settings tab', 'Recording', 'Recording'),
             ('View the Models settings tab', 'Models', 'Models'),
             ('View the About settings tab', 'About', 'About'),
             ('Close settings and return to Taurscribe', 'Close settings', None),
             ('View file transcription mode', 'File transcription mode', 'File transcription mode'),
             ('View meeting detection and dual-channel recording mode', 'Meeting detection and dual-channel recording mode', 'Meeting detection and dual-channel recording mode'),
             ('View microphone dictation mode', 'Microphone dictation mode', 'Microphone dictation mode')]
    try:
        for i, (goal, label, post) in enumerate(steps):
            # Each checkpoint remains testable after a wrong prediction. Setup
            # is deterministic and never counted as a model action or flow success.
            if 1 <= i <= 5 and not app.has('Close settings'):
                app.press('Settings',exact=True)
                app.wait(lambda: app.has('Close settings'),5)
            if i >= 6:
                app.close_settings()
            # Every navigation checkpoint must require an actual transition.
            # A prior model WAIT can leave a later target already selected.
            if post and (post in ('Meetings', 'Recording', 'Models', 'About') or post.endswith('mode')):
                if app.checked(post, 'AXRadioButton'):
                    alternate = ('General' if not post.endswith('mode') else
                                 ('Microphone dictation mode' if post != 'Microphone dictation mode' else
                                  'Meeting detection and dual-channel recording mode'))
                    if not app.press(alternate, 'AXRadioButton', exact=True) or not app.wait(
                            lambda: app.checked(alternate, 'AXRadioButton'), 5):
                        raise RuntimeError('Could not establish unselected navigation target')
                if app.checked(post, 'AXRadioButton'):
                    raise RuntimeError('Native checkpoint target is already selected')
            snap = app.snap()
            row = dict(id=f'native-ui/{i}-{label}', entry='ui', goal=goal, snapshot=navigation_snapshot(snap), expected={'operation':'CLICK','label':label})
            if i == 0:
                # Both current Settings entry points have the same postcondition.
                row['expected']['labels'] = ['Open settings', 'Settings', 'Open Settings']
            result = check_case(decider, runner, row, torch)
            if result['ok']:
                target = next(e for e in snap['elements'] if e['index'] == result['action']['target'])
                # Swift validates the chosen label against a fresh AX tree.
                ax_driver.press(app.pid, target['index'], expect=target['text'])
                if post in ('Meetings', 'Recording', 'Models', 'About') or (post and post.endswith('mode')):
                    result['ok'] = bool(app.wait(lambda: app.checked(post, 'AXRadioButton'), 5))
                elif post:
                    result['ok'] = bool(app.wait(lambda: app.has(post), 5))
                else:
                    result['ok'] = bool(app.wait(lambda: not app.has('Close settings'), 5))
                result['postcondition_checked'] = True
            rows.append(result)
            if on_result:
                on_result(result)
            print(f"{'PASS' if result['ok'] else 'FAIL'} {row['id']}", flush=True)
            app.screenshot(f'decider_{i}')
    finally:
        app.close_settings()
        if original_mode:
            app.press(original_mode, 'AXRadioButton', exact=True)
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model-dir', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=2)
    parser.add_argument('--variants', type=int, default=3)
    parser.add_argument('--native-ui', action='store_true')
    parser.add_argument('--native-only', action='store_true', help='Run native checkpoints without repeating fixture replay')
    parser.add_argument('--unrestricted-replay', action='store_true', help='Historical raw-adapter baseline; replay only, never actuate UI')
    args = parser.parse_args()
    if args.repeats < 1 or not 1 <= args.variants <= 3:
        parser.error('repeats >= 1 and variants between 1 and 3 required')
    if args.native_only and not args.native_ui:
        parser.error('--native-only requires --native-ui')
    if args.unrestricted_replay and args.native_ui:
        parser.error('Unrestricted decisions are permitted only for non-actuating fixture replay')
    if args.model_dir.name != PINNED_REVISION or not (args.model_dir/'model.safetensors').is_file():
        parser.error('Use the pinned, complete Decider 2B snapshot, not an arbitrary model')
    import psutil
    import torch
    if not torch.backends.mps.is_available():
        parser.error('This acceptance run requires the actual macOS MPS backend')
    process = psutil.Process()
    app = None
    if args.native_ui:
        # Warm the real WebKit AX tree before loading another GPU model.
        from test_ui_full import App, UIReport
        app = App(UIReport())
        app.wait_ready()
    start = time.perf_counter()
    decider = MeetingDecider(model_name=str(args.model_dir), backend='native', require_model=True,
                             restrict_actions=not args.unrestricted_replay)
    decider.load_model()
    if decider.model_name != str(args.model_dir):
        raise ValueError('DECIDER_MODEL must not override the pinned benchmark model directory')
    runner = ObservedRunner(decider._decider_lib, torch)
    decider._decider_lib = runner
    metadata = {'model':'Mapika/decider-2b', 'revision':PINNED_REVISION,
                'action_policy': 'unrestricted-replay' if args.unrestricted_replay else 'task-and-phase',
                'model_sha256':hashlib.file_digest((args.model_dir/'model.safetensors').open('rb'), 'sha256').hexdigest(),
                'package':importlib.metadata.version('decider-ai'), 'torch':torch.__version__,
                'transformers':importlib.metadata.version('transformers'), 'platform':platform.platform(),
                'device':str(runner.runner.dev), 'dtype':str(next(runner.runner.m.parameters()).dtype),
                'model_load_s':time.perf_counter()-start, 'rss_after_load_bytes':process.memory_info().rss}
    report = {'metadata':metadata, 'results':[], 'native_ui':[], 'rules_baseline':[]}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    def save():
        report['summary'] = summary(report['results'])
        report['native_summary'] = summary(report['native_ui'])
        report['decision_counts'] = decider.decision_counts
        report['fallbacks'] = decider.fallbacks
        report['rss_peak_sampled_bytes'] = max(report.get('rss_peak_sampled_bytes',0),process.memory_info().rss)
        report['mps_driver_allocated_bytes'] = torch.mps.driver_allocated_memory()
        report['mps_tensor_allocated_bytes'] = torch.mps.current_allocated_memory()
        report['mps_driver_peak_sampled_bytes'] = max(report.get('mps_driver_peak_sampled_bytes',0), report['mps_driver_allocated_bytes'])
        report['mps_tensor_peak_sampled_bytes'] = max(report.get('mps_tensor_peak_sampled_bytes',0), report['mps_tensor_allocated_bytes'])
        args.report.write_text(json.dumps(report,indent=2))
    try:
        for repeat in range(args.repeats):
            for base in ([] if args.native_only else cases()):
                for row in variants(base, args.variants):
                    row['id'] += f'/repeat-{repeat}'
                    result = check_case(decider, runner, row, torch)
                    report['results'].append(result)
                    print(f"{'PASS' if result['ok'] else 'FAIL'} {result['id']} {result['latency_s']:.2f}s",flush=True)
                    # Only the meeting join/host cases have a comparable rules policy.
                    if row['entry'] in ('join','host'):
                        begin = time.perf_counter()
                        action = (decider._decide_host_rules(row['snapshot']) if row['entry'] == 'host'
                                  else decider._decide_system1(row['snapshot'],row['goal'],'Sarah Guest'))
                        report['rules_baseline'].append(dict(id=row['id'],ok=evaluate(row,action),latency_s=time.perf_counter()-begin,action=action.to_dict()))
                    save()
        if args.native_ui:
            def record_native(row):
                report['native_ui'].append(row)
                save()
            try:
                native_ui(decider,runner,torch,app=app,on_result=record_native)
            except Exception as error:
                report['native_error'] = type(error).__name__ + ': ' + str(error)
    finally:
        save()
    calls = report['summary']['model_calls'] + report['native_summary']['model_calls']
    ok = all(r['ok'] for r in report['results']+report['native_ui']) and not decider.fallbacks and calls > 0 and not report.get('native_error')
    if args.native_ui:
        ok = ok and len(report['native_ui']) == 9
    print(json.dumps({'replay': report['summary'], 'native': report['native_summary']},indent=2),flush=True)
    return 0 if ok else 1


if __name__ == '__main__':
    raise SystemExit(main())
