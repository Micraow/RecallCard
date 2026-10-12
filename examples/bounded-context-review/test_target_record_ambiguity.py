"""Invented two-discussion counterexample: admission is not semantic correctness.

These scripted judgments demonstrate the present contract limitation. They do
not implement a selector, change thresholds, or reproduce private question text.
"""
import unittest
from bounded_span_adapter import wire_packet_hash
from judgment_contract import ContractError, Scope, encode
from native_review_controller import Controller
from test_bounded_span_adapter import fixture
from test_native_review_controller import LocalToy, arguments, decision


def paired_discussions(query='Which edits did the final reply in that workshop discussion accept? Quote them.'):
    scope=Scope.create(query,['personal'])
    _,page=fixture(scope,count=2)
    for index,row in enumerate(page['results']):
        wire=row['review_candidate'];span=wire['evidence'][0]
        text=('An invented workshop review: the blue label was accepted.' if index==0
              else 'A separate workshop review: the round label was accepted.')
        span.update(text=text,end_byte=len(text.encode()),occurred_at='2025-01-01T01:00:00Z' if index==0 else '2025-01-02T01:00:00Z')
        wire['packet_hash']=wire_packet_hash(page['review_template'],wire)
        row['source_identity']=dict(platform='toy',account_namespace='synthetic-owner',
            conversation_id='workshop-alpha' if index==0 else 'workshop-beta',group_key=wire['source_group_key'],truncated=False)
    c=Controller(scope);toy=LocalToy(page);toy.controller=c
    packet=c.discover(arguments(scope),toy)
    later=next(row for row in packet['candidates'] if row['source_identity']['conversation_id']=='workshop-beta')
    choice=decision(packet,'select',candidates=[later])
    choice['assessments'][0]['source_reason']='It is topic-related and later than the other visible discussion.'
    choice['assessments'][0]['coverage_reason']='The bounded snippet does not establish which discussion was intended or its final reply.'
    return c,toy,packet,choice


class TargetRecordAmbiguityTests(unittest.TestCase):
    def test_current_contract_can_admit_topical_source_without_proving_target_record(self):
        c,toy,packet,choice=paired_discussions()
        c.accept_native(encode(choice),toy)
        # Current structural gate accepts explicit relevance. This is NOT a test
        # that the selected discussion actually answers the underspecified query.
        self.assertEqual(c.final['selected'],choice['selected'])
        self.assertFalse(all(row['context_complete'] for row in packet['candidates']))
        self.assertEqual(len(toy.requests),1)

    def test_partial_answer_does_not_override_unresolved_source_identity(self):
        c,toy,_,choice=paired_discussions()
        self.assertEqual(choice['assessments'][0]['answer_coverage'],'partial')
        choice['assessments'][0]['unresolved']=['source_identity']
        with self.assertRaises(ContractError):c.accept_native(encode(choice),toy)
        self.assertEqual(len(toy.requests),1)

    def test_no_visible_target_coverage_can_abstain_without_claiming_global_absence(self):
        c,toy,_,choice=paired_discussions('Find the final correction in the telescope calibration discussion.')
        choice['assessments'][0].update(source_relevance='unsupported',answer_coverage='not_checked',
            source_reason='Only workshop label discussions are visible.',coverage_reason='Target discussion is not established by this bounded page.')
        choice.update(selected=None,inspect=[],stop_reason='unsupported',
            abstention_reason='No visible candidate establishes the target discussion; no claim about the entire archive.')
        c.accept_native(encode(choice),toy)
        self.assertIsNone(c.final['selected'])
        with self.assertRaises(ContractError):c.read_selected(toy)
        self.assertEqual(len(toy.requests),1)


if __name__=='__main__':unittest.main()
