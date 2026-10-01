"""Hermetic tests for the real-model benchmark and adapter guards."""
import json
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from decider_benchmark import ObservedRunner, evaluate, navigation_snapshot, summary
from decider_cases import cases, control, variants
from meeting_decider import DeciderAction, MeetingDecider


class ModelAcceptanceContracts(unittest.TestCase):
    def setUp(self):
        env = patch.dict('os.environ', {}, clear=True)
        env.start()
        self.addCleanup(env.stop)

    def native(self, response):
        with patch.object(MeetingDecider, '_check_available_runners'):
            decider = MeetingDecider(backend='native', require_model=True)
        decider._decider_lib = SimpleNamespace(decide=lambda context, questions: response)
        return decider

    def test_radio_navigation_is_a_real_candidate(self):
        snap = {'elements':[control(7,'Meetings','AXRadioButton')]}
        action = self.native([{'choice':'CLICK [7] Meetings'}]).decide_ui(snap,'View Meetings settings')
        self.assertEqual((action.operation,action.target,action.mode),('CLICK',7,'native'))

    def test_dom_radio_navigation_is_supported(self):
        snap = {'elements':[control(7,'Meetings','radio')]}
        action = self.native([{'choice':'CLICK [7] Meetings'}]).decide_ui(snap,'View Meetings settings')
        self.assertEqual(action.target,7)

    def test_disabled_radio_does_not_become_a_candidate(self):
        choices = MeetingDecider()._choices({'elements':[control(7,'Meetings','AXRadioButton',disabled=True)]})
        self.assertEqual(list(choices),['WAIT'])

    def test_malformed_and_negative_indices_are_excluded(self):
        snap = {'elements':[control(index,'Join now') for index in (None,True,'7',-1)]}
        self.assertEqual(list(MeetingDecider()._choices(snap)),['WAIT'])

    def test_unreadable_observation_is_guarded_without_fake_model_credit(self):
        decider = self.native([])
        decider._decider_lib.decide = lambda *args: self.fail('No model call for a single WAIT option')
        action = decider.decide_ui({'elements':[]},'Open settings')
        self.assertEqual((action.operation,action.mode),('WAIT','guard'))
        self.assertEqual(decider.decision_counts,{'guard':1})
        self.assertEqual(decider.fallbacks,[])

    def test_native_context_includes_goal_and_structured_state_without_table(self):
        calls = []
        decider = self.native([])
        def decide(context, questions):
            calls.append((json.loads(context),questions))
            return [{'choice':'CLICK [7] Join now'}]
        decider._decider_lib.decide = decide
        snap = {'url':'https://example.test/meeting','elements':[control(7,'Join now',value='loaded')]}
        decider.decide(snap,goal='Join with microphone on')
        self.assertEqual(calls[0][0]['elements'],snap['elements'])
        self.assertEqual(calls[0][0]['url'],snap['url'])
        self.assertIn('microphone on',calls[0][0]['goal'])

    def test_unknown_choice_fails_instead_of_becoming_rules_navigation(self):
        decider = self.native([{'choice':'CLICK [99] Delete meeting'}])
        with self.assertRaises(RuntimeError):
            decider.decide_ui({'elements':[control(7,'Meetings','AXRadioButton')]},'View Meetings')
        self.assertEqual(decider.decision_counts,{})
        self.assertEqual(decider.fallbacks,['ValueError'])

    def test_model_ui_navigation_requires_a_model_backend(self):
        with self.assertRaises(ValueError):
            MeetingDecider(backend='rules').decide_ui({'elements':[]},'Open settings')

    def test_optional_model_failure_cannot_pass_a_model_ui_test(self):
        decider = MeetingDecider(backend='systemone')
        with patch.object(decider,'_decide_choices_http',side_effect=TimeoutError):
            with self.assertRaises(RuntimeError):
                decider.decide_ui({'elements':[control(7,'Open Settings')]},'Open settings')
        self.assertEqual(decider.decision_counts,{})

    def test_oversize_candidate_set_fails_explicitly(self):
        with self.assertRaisesRegex(ValueError,'255'):
            MeetingDecider()._choices({'elements':[control(i,str(i)) for i in range(255)]})

    def test_fixture_ids_are_unique_and_gold_controls_are_enabled(self):
        rows = cases()
        self.assertGreaterEqual(len(rows),30)
        self.assertEqual(len({r['id'] for r in rows}),len(rows))
        for row in rows:
            if row['expected']['label']:
                targets = [e for e in row['snapshot']['elements'] if e['text']==row['expected']['label']]
                self.assertEqual(len(targets),1,row['id'])
                self.assertFalse(targets[0]['disabled'],row['id'])

    def test_variant_shuffle_preserves_original_and_relabels_sparse_indices(self):
        original = cases()[0]
        before = json.dumps(original)
        generated = list(variants(original))
        self.assertEqual(json.dumps(original),before)
        self.assertEqual(len({r['id'] for r in generated}),3)
        for row in generated[1:]:
            self.assertTrue(all(e['index']>=200 for e in row['snapshot']['elements']))
            self.assertEqual(row['expected'],original['expected'])

    def test_oracle_rejects_correct_index_with_wrong_action(self):
        row = cases()[0]
        self.assertFalse(evaluate(row,DeciderAction('TYPE_TEXT',2,text='Sarah Guest')))

    def test_oracle_rejects_wrong_control_even_if_click_is_valid(self):
        row = cases()[0]
        self.assertFalse(evaluate(row,DeciderAction('CLICK',10)))

    def test_oracle_requires_exact_guest_name(self):
        row = next(r for r in cases() if r['id']=='guest-name')
        self.assertTrue(evaluate(row,DeciderAction('TYPE_TEXT',2,text='Sarah Guest')))
        self.assertFalse(evaluate(row,DeciderAction('TYPE_TEXT',2,text='Other Guest')))

    def test_oracle_does_not_accept_done_for_wait_or_targeted_wait(self):
        row = next(r for r in cases() if r['id']=='admission-wait')
        self.assertTrue(evaluate(row,DeciderAction('WAIT')))
        self.assertFalse(evaluate(row,DeciderAction('DONE')))
        self.assertFalse(evaluate(row,DeciderAction('WAIT',8)))

    def test_summary_excludes_guard_and_missing_model_calls_from_latency(self):
        result = summary([dict(ok=True,latency_s=100,model_calls=0),dict(ok=False,latency_s=2,model_calls=1),dict(ok=True,latency_s=4,model_calls=1)])
        self.assertEqual(result['checks'],3)
        self.assertEqual(result['passed'],2)
        self.assertEqual(result['model_calls'],2)
        self.assertEqual(result['latency_p50_s'],3)
        self.assertEqual(result['latency_p95_s'],4)

    def test_empty_summary_is_not_model_evidence(self):
        result = summary([])
        self.assertEqual(result['model_calls'],0)
        self.assertIsNone(result['latency_p50_s'])
        self.assertEqual(result['accuracy'],0)
        self.assertIsNone(result['model_accuracy'])

    def test_equivalent_settings_entry_points_still_require_correct_controls(self):
        row = dict(snapshot={'elements':[control(2,'Settings'),control(8,'Delete meeting')]},
                   expected={'operation':'CLICK','label':'Open settings','labels':['Settings','Open settings']})
        self.assertTrue(evaluate(row,DeciderAction('CLICK',2)))
        self.assertFalse(evaluate(row,DeciderAction('CLICK',8)))

    def runner(self, response, tokens=5):
        actual = SimpleNamespace(m=SimpleNamespace(tok=SimpleNamespace(encode=lambda context:[0]*tokens)),
                                 decide=lambda context,questions,**kwargs:response)
        return ObservedRunner(actual,None)

    def test_real_model_probability_shape_must_match_the_options(self):
        runner = self.runner([dict(choice='WAIT',confidence=.9,probs={'WAIT':.9,'UNKNOWN':.1})])
        with self.assertRaises(ValueError):
            runner.decide('context',[{'options':['WAIT','CLICK']}])

    def test_real_model_probabilities_must_be_finite_and_normalized(self):
        for probs in ({'WAIT':float('nan'),'CLICK':.1},{'WAIT':.5,'CLICK':.1},{'WAIT':1.1,'CLICK':-.1}):
            with self.subTest(probs=probs):
                runner = self.runner([dict(choice='WAIT',confidence=.5,probs=probs)])
                with self.assertRaises(ValueError):
                    runner.decide('context',[{'options':['WAIT','CLICK']}])

    def test_real_model_confidence_cannot_be_nonfinite(self):
        runner = self.runner([dict(choice='WAIT',confidence=float('nan'),probs={'WAIT':.9,'CLICK':.1})])
        with self.assertRaises(ValueError):
            runner.decide('context',[{'options':['WAIT','CLICK']}])

    def test_real_model_output_count_must_match_questions(self):
        with self.assertRaises(ValueError):
            self.runner([]).decide('context',[{'options':['WAIT','CLICK']}])

    def test_overlong_context_is_rejected_before_inference_instead_of_truncated(self):
        runner = self.runner([],tokens=4097)
        with self.assertRaises(ValueError):
            runner.decide('context',[{'options':['WAIT','CLICK']}])
        self.assertEqual(runner.calls,0)

    def test_guard_success_cannot_inflate_model_accuracy(self):
        result=summary([dict(ok=True,model_calls=0,latency_s=0),dict(ok=False,model_calls=1,latency_s=1)])
        self.assertEqual(result['accuracy'],.5)
        self.assertEqual(result['model_accuracy'],0)

    def test_navigation_projection_retains_control_identity_and_state(self):
        item=control(7,'Meetings','AXRadioButton',checked=True,disabled=True,value='1')
        projected=navigation_snapshot({'elements':[item]})['elements'][0]
        self.assertEqual(projected,item)

    def test_navigation_projection_omits_prose_but_keeps_text_input(self):
        projected=navigation_snapshot({'elements':[control(2,'Transcript words','AXStaticText'),control(7,'Search','AXTextField',value='query')]})
        self.assertEqual([e['index'] for e in projected['elements']],[7])
        self.assertEqual(projected['elements'][0]['value'],'query')

    def test_navigation_projection_does_not_mutate_the_live_snapshot(self):
        snap={'elements':[dict(control(7,'Settings'),tag='AXButton')]}
        before=json.dumps(snap)
        navigation_snapshot(snap)
        self.assertEqual(json.dumps(snap),before)
