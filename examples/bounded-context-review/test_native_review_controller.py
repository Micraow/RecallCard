"""Synthetic controller integration: no private corpus, model or provider calls."""
import copy
import unittest
from bounded_span_adapter import wire_packet_hash
from judgment_contract import ContractError, Scope, encode
from native_review_controller import Controller, Reply
from test_bounded_span_adapter import fixture, h, inspection


def arguments(scope):
    return dict(query=scope.query, target='events', detail='session_discovery_compact',
        limit=10, budget_bytes=32768, event_filter=scope.payload()['event_filter'], as_of=scope.as_of)


class LocalToy:
    def __init__(self, page):
        self.page = page
        self.controller = None
        self.requests = []
        self.mutate = None
        self.error = None

    def __call__(self, name, args):
        self.requests.append((name, copy.deepcopy(args)))
        if name == 'search':
            value = copy.deepcopy(self.page)
        elif name in ('initialize', 'tools/list', 'notifications/initialized'):
            return Reply(b'{}' if name != 'notifications/initialized' else b'', len(encode(args)), 32, 1)
        elif args['context_review']['stage'] == 'read':
            text = 'The invented bounded final original.'
            value = {'relevance':'caller_assessed','evidence_binding':'verified','results': [{'ref': args['refs'][0], 'text': text,
                'text_range':dict(start_byte=0,end_byte=len(text.encode()),total_bytes=len(text.encode())),
                'metadata':{'adjacent':{'previous_ref':None,'next_refs':[],'constraints':{'context_anchor_ref':args['context_anchor_ref']}}}}], 'next_cursor': None}
        else:
            controller = self.controller
            cid = next(cid for cid, wire in controller.workflow._wire_candidates.items()
                       if wire['candidate_id'] == args['context_review']['candidate']['candidate_id'])
            value = inspection(controller.workflow, cid)
            span = value['updated_candidate']['evidence'][0]
            span['start_byte'] = args['offset_bytes']
            span['end_byte'] = args['offset_bytes']+len(span['text'].encode())
            value['updated_candidate']['packet_hash'] = wire_packet_hash(controller.workflow._wire_template, value['updated_candidate'])
        if self.mutate:
            self.mutate(value, args)
        raw = encode(value)
        return Reply(raw, len(encode(args)), len(raw)+80, 2, self.error)


def start(count=3, scope=None):
    scope, page = fixture(scope, count)
    c = Controller(scope); toy = LocalToy(page); toy.controller = c
    packet = c.discover(arguments(scope), toy)
    return c, toy, packet


def assessment(row, relevant=False, support=None):
    support = support or [row['context'][0]]
    return dict(candidate=row['candidate'], source_relevance='relevant' if relevant else 'uncertain',
        answer_coverage='partial', support=[dict(evidence=x['evidence'], quote=x['text'][:20]) for x in support],
        unresolved=[], source_reason='Visible subject link.' if relevant else 'Subject link is not established.',
        coverage_reason='The final answer is not fully visible.')


def decision(packet, action='inspect', candidates=None):
    rows = candidates or packet['candidates'][:1]
    return dict(packet_id=packet['packet_id'], assessments=[assessment(row, action == 'select') for row in rows],
        selected=rows[0]['candidate'] if action == 'select' else None,
        inspect=[dict(candidate=row['candidate'], action_id=next(x['action_id'] for x in row['legal_actions'] if x['action'] == 'continue'),
                      missing_evidence_reason='Need the next original subject link.') for row in rows] if action == 'inspect' else [],
        stop_reason={'select':'selected', 'inspect':'need_inspection', 'abstain':'source_uncertain'}[action],
        abstention_reason='No bounded subject link.' if action == 'abstain' else None)


class ControllerTests(unittest.TestCase):
    def test_search_native_inspect_native_partial_select_original_read(self):
        c, toy, p = start(); first = p['candidates'][0]['context'][0]
        p2 = c.accept_native(encode(decision(p)), toy, 11)
        self.assertEqual(len(p2['candidates']), 1)
        retained = p2['candidates'][0]['context']
        self.assertTrue(retained[0]['retained'])
        self.assertEqual(retained[0]['text'], first['text'])
        self.assertFalse(retained[-1]['retained'])
        c.accept_native(encode(decision(p2, 'select')), toy, 7)
        raw = c.read_selected(toy)
        self.assertIn(b'invented', raw)
        self.assertEqual([x[1]['context_review']['stage'] for x in toy.requests[1:]], ['inspect','read'])
        summary = c.costs.summary()
        self.assertEqual(summary['native_rounds'], 2)
        self.assertEqual(summary['native_elapsed_ms'], 18)
        self.assertEqual(summary['protocol_and_tool_calls'], 3)
        self.assertEqual(summary['result_bytes'], c.costs.ledger.result_bytes)
        self.assertEqual(c.workflow._originals, {})

    def test_setup_failure_and_native_bytes_share_ledger(self):
        scope, page = fixture(count=1); c = Controller(scope); toy = LocalToy(page); toy.controller = c
        for method in ('initialize','notifications/initialized','tools/list'):
            c.setup(toy, method, {})
        p = c.discover(arguments(scope), toy)
        raw = encode(decision(p, 'abstain')); c.accept_native(raw, toy, 3)
        cost = c.costs.summary()
        self.assertEqual(cost['setup_calls'], 3)
        self.assertEqual(cost['ledger']['tool_calls_consumed'], 4)
        self.assertEqual(cost['native_output_bytes'], len(raw))
        self.assertEqual(cost['native_input_bytes'], len(encode(p)))
        self.assertIsNone(cost['tokens'])

    def test_partial_conflicting_not_checked_answer_can_select_supported_source(self):
        for coverage in ('partial','conflicting','not_checked','sufficient'):
            with self.subTest(coverage=coverage):
                c, toy, p = start(); d = decision(p, 'select'); d['assessments'][0]['answer_coverage'] = coverage
                c.accept_native(encode(d), toy); self.assertIsNotNone(c.final['selected'])

    def test_source_uncertainty_never_masquerades_as_relevance(self):
        c, toy, p = start(); d = decision(p, 'select'); d['assessments'][0]['source_relevance'] = 'uncertain'
        with self.assertRaises(ContractError): c.accept_native(encode(d), toy)
        self.assertEqual(len(toy.requests), 1)
        with self.assertRaises(ContractError): c.read_selected(toy)

    def test_explicit_source_and_coverage_reasons_are_independent_required_fields(self):
        for key in ('source_reason','coverage_reason'):
            c, toy, p = start(); d = decision(p, 'select'); d['assessments'][0].pop(key)
            with self.assertRaises(ContractError): c.accept_native(encode(d), toy)

    def test_abstention_requires_independent_reason_and_cannot_deep_read(self):
        c, toy, p = start(); d = decision(p, 'abstain'); d['abstention_reason'] = ''
        with self.assertRaises(ContractError): c.accept_native(encode(d), toy)
        c, toy, p = start(); c.accept_native(encode(decision(p, 'abstain')), toy)
        with self.assertRaises(ContractError): c.read_selected(toy)

    def test_identity_or_branch_ambiguity_blocks_selection(self):
        for ambiguity in ('source_identity','branch'):
            c, toy, p = start(); d = decision(p, 'select'); d['assessments'][0]['unresolved'] = [ambiguity]
            with self.assertRaises(ContractError): c.accept_native(encode(d), toy)

    def test_stale_packet_unknown_alias_missing_quote_and_direction_fail_before_read(self):
        mutations = [lambda d: d.update(packet_id='packet:'+h('other')),
            lambda d: d['assessments'][0].update(candidate='C99'),
            lambda d: d['assessments'][0]['support'][0].update(evidence='E99'),
            lambda d: d['assessments'][0]['support'][0].update(quote='forged exact original'),
            lambda d: d['inspect'][0].update(action_id='A999')]
        for mutation in mutations:
            c, toy, p = start(); d = decision(p); mutation(d)
            with self.assertRaises(ContractError): c.accept_native(encode(d), toy)
            self.assertEqual(len(toy.requests), 1)
            self.assertEqual(c.costs.summary()['native_rounds'], 1)

    def test_missing_previous_support_never_resolves_from_hidden_catalog(self):
        c, toy, p = start(); p2 = c.accept_native(encode(decision(p)), toy)
        d = decision(p2, 'select'); d['assessments'][0]['support'][0]['evidence'] = 'E99'
        with self.assertRaises(ContractError): c.accept_native(encode(d), toy)

    def test_two_candidates_four_inspections_three_rounds(self):
        c, toy, p = start(); p2 = c.accept_native(encode(decision(p, candidates=p['candidates'][:2])), toy)
        p3 = c.accept_native(encode(decision(p2, candidates=p2['candidates'])), toy)
        self.assertEqual(sum(c.costs.ledger.inspections.values()), 4)
        self.assertTrue(all(not row['legal_actions'] for row in p3['candidates']))
        c.accept_native(encode(decision(p3, 'abstain')), toy)
        self.assertEqual(c.costs.summary()['native_rounds'], 3)
        self.assertEqual(len(toy.requests), 5)

    def test_dropped_candidate_cannot_reenter_next_round(self):
        c, toy, p = start(); dropped = p['candidates'][1]
        p2 = c.accept_native(encode(decision(p)), toy); d = decision(p2, 'select')
        d['selected'] = dropped['candidate']; d['assessments'] = [assessment(dropped, True)]
        with self.assertRaises(ContractError): c.accept_native(encode(d), toy)

    def test_scope_and_asof_survive_every_read(self):
        scope = Scope.create('beacon', ['personal'], {'role':'user'}, '2026-02-03T00:00:00Z')
        c, toy, p = start(scope=scope); p2 = c.accept_native(encode(decision(p)), toy)
        c.accept_native(encode(decision(p2, 'select')), toy); c.read_selected(toy)
        for _, args in toy.requests[1:]:
            self.assertEqual(args['event_filter']['role'], 'user')
            self.assertEqual(args['as_of'], scope.as_of)

    def test_incorrect_offset_and_graph_revision_charged_then_closed(self):
        for mode in ('offset','revision','distance','sibling'):
            c, toy, p = start()
            def mutate(value, args):
                wire = value['updated_candidate']
                if mode == 'offset':
                    wire['evidence'][0]['start_byte'] += 1; wire['evidence'][0]['end_byte'] += 1
                if mode == 'revision': wire['graph_generation'] = h('new revision')
                if mode == 'distance': value['navigation']['distance_from_anchor'] = 2
                if mode == 'sibling': value['navigation']['next_refs'] = ['event:one','event:two']
                wire['packet_hash'] = wire_packet_hash(c.workflow._wire_template, wire)
            toy.mutate = mutate
            with self.assertRaises(ContractError): c.accept_native(encode(decision(p)), toy)
            self.assertTrue(c.failed)
            self.assertEqual(c.costs.ledger.tool_calls, 2)
            self.assertEqual(c.costs.ledger.result_bytes, c.costs.summary()['result_bytes'])
            with self.assertRaises(ContractError): c.packet()

    def test_server_refusal_stops_without_promoting_old_proof(self):
        c, toy, p = start()
        def refuse(value, args):
            value.clear(); value.update(status='uncertain', reason='source_ambiguous', results=[],
                relevance='unresolved', deep_read_performed=False, budget_unit='utf8_bytes')
        toy.mutate = refuse
        self.assertIsNone(c.accept_native(encode(decision(p)), toy))
        self.assertEqual(c.final['stop_reason'], 'server_refusal')
        with self.assertRaises(ContractError): c.read_selected(toy)

    def test_transport_error_and_oversized_native_output_preserve_costs(self):
        c, toy, p = start(); toy.error = 'synthetic_transport_failure'
        with self.assertRaises(ContractError): c.accept_native(encode(decision(p)), toy)
        self.assertEqual(c.costs.summary()['failed_calls'], 1)
        self.assertEqual(c.costs.summary()['protocol_and_tool_calls'], 2)
        c, toy, p = start()
        with self.assertRaises(ContractError): c.accept_native(b'x'*8193, toy)
        self.assertEqual(c.costs.summary()['native_output_bytes'], 8193)
        self.assertEqual(len(toy.requests), 1)

    def test_duplicate_json_unknown_fields_fail_closed(self):
        c, toy, p = start()
        with self.assertRaises(ContractError): c.accept_native(b'{"selected":null,"selected":null}', toy)
        c, toy, p = start(); d = decision(p, 'select'); d['full_original'] = 'invented bypass'
        with self.assertRaises(ContractError): c.accept_native(encode(d), toy)

    def test_model_cannot_modify_the_emitted_packet(self):
        c, toy, p = start(); p['candidates'][0]['context'][0]['text'] = 'mutated external copy'
        d = decision(p, 'select')
        with self.assertRaises(ContractError): c.accept_native(encode(d), toy)

    def test_discovery_scope_mismatch_before_any_dispatch(self):
        scope, page = fixture(); c = Controller(scope); toy = LocalToy(page)
        args = arguments(scope); args['query'] = 'other'
        with self.assertRaises(ContractError): c.discover(args, toy)
        self.assertEqual(toy.requests, [])



class NavigationAndReadTests(unittest.TestCase):
    def test_probe_follow_unique_edge_costs_two_and_preserves_monotonic_relation(self):
        c, toy, p = start(count=1)
        def edge(value, args):
            wire = value['updated_candidate']; span = wire['evidence'][0]
            if args['refs'][0] == wire['anchor_ref']:
                value['navigation']['next_refs'] = ['event:toy-neighbor']
            else:
                span.update(reference='event:toy-neighbor', snapshot=h('neighbor-snapshot'), relation='next',
                            start_byte=0, end_byte=len(span['text'].encode()))
                value['navigation'].update(distance_from_anchor=1, previous_ref=wire['anchor_ref'], next_refs=[])
            wire['packet_hash'] = wire_packet_hash(c.workflow._wire_template, wire)
        toy.mutate = edge
        d = decision(p); row = p['candidates'][0]
        d['inspect'][0]['action_id'] = next(x['action_id'] for x in row['legal_actions'] if x['action']=='check_next')
        p2 = c.accept_native(encode(d), toy)
        self.assertEqual(len(toy.requests), 3)
        self.assertEqual(sum(c.costs.ledger.inspections.values()), 2)
        self.assertEqual(p2['candidates'][0]['context'][-1]['relation'], 'next')
        self.assertTrue(all(not x['legal_actions'] for x in p2['candidates']))
        self.assertEqual(len(p2['candidates'][0]['context']), 3)

    def test_probe_without_unique_neighbor_stops_at_one_charged_call(self):
        c, toy, p = start(count=1); d = decision(p)
        d['inspect'][0]['action_id'] = next(x['action_id'] for x in p['candidates'][0]['legal_actions'] if x['action']=='check_next')
        p2 = c.accept_native(encode(d), toy)
        self.assertEqual(len(toy.requests), 2)
        self.assertEqual(sum(c.costs.ledger.inspections.values()), 1)
        self.assertEqual(len(p2['candidates'][0]['context']), 2)

    def test_deep_refusal_prevents_old_proof_retry(self):
        c, toy, p = start(); c.accept_native(encode(decision(p,'select')), toy)
        def reject(value, args):
            value.clear(); value.update(status='uncertain', results=[], relevance='unresolved', deep_read_performed=False)
        toy.mutate = reject
        c.read_selected(toy)
        self.assertTrue(c.failed)
        with self.assertRaises(ContractError): c.read_selected(toy)
        self.assertEqual(len(toy.requests), 2)

    def test_deep_cursor_binding_cannot_switch_scope_target_or_proof(self):
        c, toy, p = start(); c.accept_native(encode(decision(p,'select')), toy)
        toy.mutate = lambda value,args: value.update(next_cursor='toy-fixed-cursor')
        c.read_selected(toy)
        args = c.workflow.selected_read_args(); args['cursor'] = 'toy-fixed-cursor'
        changed = copy.deepcopy(args); changed['event_filter'] = {'role':'assistant'}
        with self.assertRaises(ContractError): c.read_request(changed, toy)
        changed = copy.deepcopy(args); changed['refs'] = ['event:unobserved']
        with self.assertRaises(ContractError): c.read_request(changed, toy)
        c.read_request(args, toy)
        with self.assertRaises(ContractError): c.read_request(args, toy)
        self.assertEqual(len(toy.requests), 3)

    def test_ten_long_originals_fit_without_truncating_text(self):
        scope, page = fixture(count=10)
        for row in page['results']:
            wire = row['review_candidate']; span = wire['evidence'][0]
            span['text'] = 'α'*256; span['end_byte'] = 512
            wire['packet_hash'] = wire_packet_hash(page['review_template'], wire)
        c = Controller(scope); toy = LocalToy(page); toy.controller = c
        p = c.discover(arguments(scope), toy)
        self.assertEqual(sum(len(row['context'][0]['text'].encode()) for row in p['candidates']), 5120)
        self.assertLessEqual(len(encode(p)), 32768)
        self.assertTrue(all(row['context'][0]['text']=='α'*256 for row in p['candidates']))

    def test_setup_is_charged_again_after_resume_before_inspection(self):
        c, toy, p = start(); c.setup(toy, 'initialize', {})
        p2 = c.accept_native(encode(decision(p)), toy)
        self.assertEqual(c.costs.summary()['setup_calls'], 1)
        self.assertEqual(c.costs.ledger.tool_calls, 3)
        self.assertEqual(p2['budget_before_packet']['setup_calls'], 1)

class ReviewFindingsTests(unittest.TestCase):
    def test_actual_branch_stop_blocks_discovery_and_inspected_selection(self):
        for stage in ('discovery','inspection'):
            scope, page = fixture(count=1)
            if stage == 'discovery': page['results'][0]['context_stop_reasons'] = ['branch_choice_required']
            c = Controller(scope); toy = LocalToy(page); toy.controller = c
            p = c.discover(arguments(scope), toy)
            if stage == 'inspection':
                toy.mutate = lambda value,args: value['navigation'].update(stop_reasons=['branch_choice_required'])
                p = c.accept_native(encode(decision(p)), toy)
            self.assertTrue(p['candidates'][0]['branch_ambiguous'])
            with self.assertRaises(ContractError): c.accept_native(encode(decision(p,'select')), toy)

    def test_display_identity_group_must_equal_bound_original_group(self):
        for mode in ('mismatch','unknown','bad_type','valid'):
            scope, page = fixture(count=1); row = page['results'][0]
            display = {'platform':'toy','account_namespace':'synthetic-owner','conversation_id':'toy-session',
                'group_key': row['review_candidate']['source_group_key'], 'truncated':False}
            if mode == 'mismatch': display['group_key'] = h('another group')
            if mode == 'unknown': display['branch'] = 'invented'
            if mode == 'bad_type': display['truncated'] = 0
            row['source_identity'] = display; c = Controller(scope); toy = LocalToy(page); toy.controller = c
            if mode == 'valid':
                p = c.discover(arguments(scope), toy)
                self.assertEqual(p['candidates'][0]['source_identity']['account_namespace'], 'synthetic-owner')
            else:
                with self.assertRaises(ContractError): c.discover(arguments(scope), toy)
                self.assertEqual(c.costs.ledger.tool_calls, 1)
                self.assertTrue(c.failed)

    def test_thrown_transport_does_not_claim_zero_is_exact_measured_cost(self):
        c, toy, p = start()
        def broken(name,args): raise OSError('synthetic error')
        with self.assertRaises(ContractError): c.accept_native(encode(decision(p)), broken)
        self.assertFalse(c.costs.summary()['transport_costs_complete'])
        self.assertEqual(c.costs.summary()['failed_calls'], 1)

class BudgetAndProtocolTests(unittest.TestCase):
    def test_later_packet_budget_failure_does_not_silently_truncate_required_query_or_evidence(self):
        scope = Scope.create('beacon '+'x'*9000,['personal'])
        c,toy,p=start(count=1,scope=scope)
        with self.assertRaisesRegex(ContractError,'required_packet_does_not_fit'):
            c.accept_native(encode(decision(p)),toy)
        self.assertEqual(c.costs.ledger.tool_calls,2)
        self.assertEqual(c.costs.ledger.rounds,1)
        self.assertEqual(c.workflow._trusted_spans[next(iter(c.workflow._trusted_spans))].text,p['candidates'][0]['context'][0]['text'])

    def test_mandatory_unassessed_probe_is_one_time_charged_and_does_not_select(self):
        c,toy,p=start(count=1)
        def check(value,args):
            value.clear();value.update(status='uncertain',results=[],relevance='unresolved',deep_read_performed=False)
        toy.mutate=check
        c.check_unassessed_gate(toy)
        self.assertEqual(c.costs.ledger.tool_calls,2)
        self.assertIsNone(c.final)
        with self.assertRaises(ContractError):c.check_unassessed_gate(toy)
        self.assertEqual(len(toy.requests),2)

    def test_unassessed_probe_that_accidentally_reads_fails_closed(self):
        c,toy,p=start(count=1)
        with self.assertRaises(ContractError):c.check_unassessed_gate(toy)
        self.assertTrue(c.failed)
        self.assertEqual(c.costs.ledger.tool_calls,2)

class CitationAdapterTests(unittest.TestCase):
    def test_two_distinct_quotes_from_same_span_produce_one_proof_span(self):
        c,toy,p=start(count=1);d=decision(p,'select');span=p['candidates'][0]['context'][0]
        d['assessments'][0]['support']=[dict(evidence=span['evidence'],quote='beacon'),dict(evidence=span['evidence'],quote='source clue α.')]
        raw=encode(d);c.accept_native(raw,toy)
        self.assertEqual(len(c.workflow._judgment.shortlist[0].support_refs),1)
        self.assertEqual(c.judgments[0],d)
        proof=c.workflow.selected_read_args()['context_review']['candidate']
        self.assertEqual(len(proof['evidence']),1)
        self.assertEqual(proof['evidence'][0]['text'],span['text'])
        self.assertEqual(c.costs.summary()['native_output_bytes'],len(raw))

    def test_same_span_quotes_retained_as_one_complete_original_next_round(self):
        c,toy,p=start(count=1);d=decision(p);span=p['candidates'][0]['context'][0]
        d['assessments'][0]['support']=[dict(evidence=span['evidence'],quote='beacon'),dict(evidence=span['evidence'],quote='source clue α.')]
        p2=c.accept_native(encode(d),toy)
        retained=[x for x in p2['candidates'][0]['context'] if x['retained']]
        self.assertEqual(len(retained),1)
        self.assertEqual(retained[0]['text'],span['text'])

    def test_duplicate_reference_does_not_skip_validation_of_second_quote(self):
        for quote in ('forged later quote','', 'α'*101):
            c,toy,p=start(count=1);d=decision(p,'select');span=p['candidates'][0]['context'][0]
            d['assessments'][0]['support'].append(dict(evidence=span['evidence'],quote=quote))
            with self.assertRaises(ContractError):c.accept_native(encode(d),toy)
            self.assertEqual(len(toy.requests),1)

    def test_cross_candidate_quote_is_not_imported_by_alias_reuse(self):
        c,toy,p=start(count=2);d=decision(p,'select')
        other=p['candidates'][1]['context'][0]
        d['assessments'][0]['support'].append(dict(evidence=p['candidates'][0]['context'][0]['evidence'],quote=other['text']))
        with self.assertRaises(ContractError):c.accept_native(encode(d),toy)
        self.assertEqual(len(toy.requests),1)

    def test_cross_evidence_quote_and_forged_alias_fail(self):
        for mode in ('quote','alias'):
            c,toy,p=start(count=1);p2=c.accept_native(encode(decision(p)),toy);d=decision(p2,'select')
            old,new=p2['candidates'][0]['context']
            d['assessments'][0]['support']=[dict(evidence=old['evidence'],quote='beacon'),
                dict(evidence=old['evidence'] if mode=='quote' else 'E99',quote=new['text'])]
            with self.assertRaises(ContractError):c.accept_native(encode(d),toy)
            self.assertEqual(len(toy.requests),2)

    def test_three_quotes_two_spans_keep_first_seen_unique_order(self):
        c,toy,p=start(count=1);p2=c.accept_native(encode(decision(p)),toy);d=decision(p2,'select')
        old,new=p2['candidates'][0]['context']
        d['assessments'][0]['support']=[dict(evidence=new['evidence'],quote=new['text']),
            dict(evidence=old['evidence'],quote='beacon'),dict(evidence=new['evidence'],quote='bounded clue')]
        c.accept_native(encode(d),toy)
        proof=c.workflow.selected_read_args()['context_review']['candidate']['evidence']
        self.assertEqual([x['text'] for x in proof],[new['text'],old['text']])

    def test_multiple_quotes_do_not_raise_raw_citation_count_limit(self):
        c,toy,p=start(count=1);d=decision(p,'select');citation=d['assessments'][0]['support'][0]
        d['assessments'][0]['support']=[copy.deepcopy(citation) for _ in range(4)]
        with self.assertRaises(ContractError):c.accept_native(encode(d),toy)


if __name__ == '__main__': unittest.main()
