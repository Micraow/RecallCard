"""Independent invented examples only. No corpus, models, network, or gold labels."""
from dataclasses import asdict, replace
import hashlib
import json
import unittest

from judgment_contract import (
    AMBIGUOUS_STOPS, Assessment, BudgetFailure, BudgetLedger, Candidate,
    ContractError, Evidence, Identity, Judgment, Original, POLICY, Scope,
    Workflow, digest, encode, evidence_span,
)


def h(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def toy(count=3, *, flags=None, filter=None, query="Find the synthetic scheduling discussion"):
    scope = Scope.create(query, ["toy-local"], filter, "2026-01-01T00:00:00Z")
    candidates, originals = [], []
    for number in range(count):
        identity = Identity(h(f"source-{number}"), f"event:anchor-{number}",
                            h(f"snapshot-{number}"), h(f"graph-{number}"))
        anchor = Original(scope.scope_id, identity, identity.anchor_ref,
                          identity.anchor_snapshot, "user", "2025-12-01T00:00:00Z",
                          "anchor", f"Toy {number}: choose the meeting day. Alpha. Beta. Gamma.")
        neighbor = Original(scope.scope_id, identity, f"event:next-{number}",
                            h(f"next-snapshot-{number}"), "assistant",
                            "2025-12-01T00:01:00Z", "next", "Friday is only a tentative suggestion.")
        originals += [anchor, neighbor]
        evidence = evidence_span(anchor, 0, len(anchor.text.encode()))
        kwargs = (flags or {}).get(number, {})
        candidates.append(Candidate(identity, (evidence,), **kwargs))
    return scope, candidates, originals


def start(count=3, **kwargs):
    scope, candidates, originals = toy(count, **kwargs)
    workflow = Workflow(scope, candidates, originals)
    return workflow, workflow.initial_packet(), candidates, originals


def assessment(packet, index=0, relevance="uncertain", coverage="not_checked", refs=None,
               unresolved=()):
    candidate = packet["candidates"][index]
    return Assessment(candidate["candidate_id"], relevance, coverage,
                      tuple(refs or [candidate["evidence"][0]["evidence_id"]]), unresolved)


def request(packet, indices=(0,)):
    shortlisted = tuple(assessment(packet, index) for index in indices)
    return Judgment(packet["packet_id"], shortlisted, None,
                    tuple(item.candidate_id for item in shortlisted), "need_inspection")


def inspect(workflow, packet, originals, index=0, **kwargs):
    cid = packet["candidates"][index]["candidate_id"]
    original = next(o for o in originals if o.candidate_id == cid and o.relation == "next")
    span = evidence_span(original, 0, len(original.text.encode()))
    ticket = workflow.begin_inspection(cid, 4096)
    raw = encode({"evidence": span.payload(), "stop_reasons": kwargs.get("stop_reasons", ())})
    workflow.finish_inspection(ticket, raw, span, **kwargs)
    return span


class RelevanceContractTests(unittest.TestCase):
    def test_relevant_partial_selects_without_requiring_complete_answer(self):
        w, packet, _, _ = start()
        a = assessment(packet, relevance="relevant", coverage="partial")
        w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))
        with self.assertRaises(ContractError):
            w.next_packet()

    def test_all_answer_coverages_are_independent_of_relevance(self):
        for coverage in ("not_checked", "partial", "sufficient", "conflicting"):
            with self.subTest(coverage=coverage):
                w, packet, _, _ = start()
                a = assessment(packet, relevance="relevant", coverage=coverage)
                w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))

    def test_uncertain_and_unsupported_cannot_select_even_when_answer_sufficient(self):
        for relevance in ("uncertain", "unsupported"):
            w, packet, _, _ = start()
            a = assessment(packet, relevance=relevance, coverage="sufficient")
            with self.assertRaisesRegex(ContractError, "relevant source"):
                w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))

    def test_explicit_candidate_ambiguities_block(self):
        for flag in ("source_identity_ambiguous", "branch_ambiguous"):
            w, packet, _, _ = start(flags={0: {flag: True}})
            a = assessment(packet, relevance="relevant")
            with self.assertRaisesRegex(ContractError, "ambiguity blocks"):
                w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))

    def test_explicit_judge_ambiguities_block(self):
        for unresolved in (("source_identity",), ("branch",)):
            w, packet, _, _ = start()
            a = assessment(packet, relevance="relevant", unresolved=unresolved)
            with self.assertRaisesRegex(ContractError, "ambiguity blocks"):
                w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))

    def test_explicit_ambiguity_stop_codes_block_even_without_boolean(self):
        for code in AMBIGUOUS_STOPS:
            w, packet, _, _ = start(flags={0: {"stop_reasons": (code,)}})
            a = assessment(packet, relevance="relevant")
            with self.assertRaisesRegex(ContractError, "ambiguity blocks"):
                w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))

    def test_empty_shortlist_allows_explicit_abstention(self):
        w, packet, _, _ = start()
        w.accept(Judgment(packet["packet_id"], (), None, (), "source_uncertain"))

    def test_inspection_requires_uncertain_source_relevance(self):
        for relevance in ("relevant", "unsupported"):
            w, packet, _, _ = start(1)
            a = assessment(packet, relevance=relevance)
            with self.assertRaisesRegex(ContractError, "limited to uncertain"):
                w.accept(Judgment(packet["packet_id"], (a,), None,
                                  (a.candidate_id,), "need_inspection"))

    def test_selection_and_inspection_cannot_be_combined(self):
        w, packet, _, _ = start()
        a = assessment(packet, relevance="relevant")
        with self.assertRaises(ContractError):
            w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id,
                              (a.candidate_id,), "selected"))


class IncrementalPacketTests(unittest.TestCase):
    def test_fresh_packet_has_only_explicit_shortlist_original_support_and_new_text(self):
        w, first, _, originals = start(5)
        w.accept(request(first, (1, 3)))
        returned = [inspect(w, first, originals, index) for index in (1, 3)]
        second = w.next_packet()
        self.assertEqual([c["candidate_id"] for c in second["candidates"]],
                         [first["candidates"][i]["candidate_id"] for i in (1, 3)])
        for current, old_index, new in zip(second["candidates"], (1, 3), returned):
            self.assertEqual(current["evidence"],
                             [first["candidates"][old_index]["evidence"][0], new.payload()])
        self.assertEqual(second["scope"], first["scope"])
        self.assertEqual(second["preceding_stop_reason"], "need_inspection")
        self.assertLessEqual(len(encode(second)), 8192)

    def test_third_packet_omits_unrequested_old_support_but_keeps_exact_new_reference(self):
        w, first, _, originals = start(1)
        w.accept(request(first))
        added = inspect(w, first, originals)
        second = w.next_packet()
        a = assessment(second, refs=[added.evidence_id])
        w.accept(Judgment(second["packet_id"], (a,), None, (a.candidate_id,), "need_inspection"))
        inspect(w, second, originals)
        third = w.next_packet()
        self.assertEqual(third["candidates"][0]["evidence"], [added.payload()])
        forgotten = first["candidates"][0]["evidence"][0]["evidence_id"]
        a = assessment(third, relevance="relevant", refs=[forgotten])
        with self.assertRaisesRegex(ContractError, "fresh judge needs original text"):
            w.accept(Judgment(third["packet_id"], (a,), a.candidate_id, (), "selected"))

    def test_mutating_returned_packet_cannot_change_validation_state(self):
        w, packet, _, _ = start()
        good = assessment(packet, relevance="relevant")
        packet["candidates"][0]["evidence"][0]["text"] = "forged"
        packet["candidates"][0]["evidence"][0]["evidence_id"] = "forged"
        with self.assertRaises(ContractError):
            w.accept(Judgment(packet["packet_id"], (replace(good, support_refs=("forged",)),),
                              good.candidate_id, (), "selected"))
        w.accept(Judgment(packet["packet_id"], (good,), good.candidate_id, (), "selected"))

    def test_stale_packet_judgment_is_rejected(self):
        w, first, _, originals = start(1)
        prior = request(first)
        w.accept(prior)
        inspect(w, first, originals)
        w.next_packet()
        with self.assertRaisesRegex(ContractError, "stale/foreign packet ID"):
            w.accept(prior)

    def test_unrequested_inspection_and_missing_results_are_rejected(self):
        w, first, _, _ = start(2)
        with self.assertRaises(ContractError):
            w.begin_inspection(first["candidates"][0]["candidate_id"], 1000)
        w.accept(request(first))
        with self.assertRaises(ContractError):
            w.begin_inspection(first["candidates"][1]["candidate_id"], 1000)
        with self.assertRaises(ContractError):
            w.next_packet()
        self.assertEqual(w.ledger.tool_calls, 0)

    def test_empty_inspection_preserves_stop_reason_and_support(self):
        w, first, _, _ = start(1)
        w.accept(request(first))
        cid = first["candidates"][0]["candidate_id"]
        ticket = w.begin_inspection(cid, 100)
        w.finish_inspection(ticket, b'{}', None, ("filtered_neighbor",))
        second = w.next_packet()
        self.assertEqual(second["candidates"][0]["evidence"], first["candidates"][0]["evidence"])
        self.assertEqual(second["candidates"][0]["stop_reasons"], ["filtered_neighbor"])

    def test_inspection_cannot_clear_prior_ambiguity_or_stop_reasons(self):
        w, first, _, originals = start(1, flags={0: {"branch_ambiguous": True,
                                                     "stop_reasons": ("filtered_neighbor",)}})
        w.accept(request(first))
        inspect(w, first, originals, branch_ambiguous=False, stop_reasons=("window_end",))
        second = w.next_packet()
        self.assertTrue(second["candidates"][0]["branch_ambiguous"])
        self.assertEqual(second["candidates"][0]["stop_reasons"], ["filtered_neighbor", "window_end"])

    def test_filter_as_of_scope_and_branch_are_preserved_and_hash_bound(self):
        scope, candidates, originals = toy(1, filter={"role": "user", "nested": {"x": [1, 2]}})
        w = Workflow(scope, candidates, originals)
        first = w.initial_packet()
        w.accept(request(first))
        inspect(w, first, originals)
        second = w.next_packet()
        self.assertEqual(first["scope"], second["scope"])
        self.assertEqual(first["candidates"][0]["identity"], second["candidates"][0]["identity"])
        for changed in (replace(scope, as_of="2026-02-01T00:00:00Z"),
                        Scope.create(scope.query, scope.scopes, {"role": "assistant"}, scope.as_of),
                        replace(scope, scopes=("other",))):
            with self.assertRaisesRegex(ContractError, "foreign original"):
                Workflow(changed, candidates, originals)


class IntegrityTests(unittest.TestCase):
    def test_exact_multibyte_range_and_role(self):
        scope, candidates, originals = toy(1)
        original = replace(originals[0], text="A中🙂Z")
        span = evidence_span(original, 1, 8)
        self.assertEqual(span.text, "中🙂")
        self.assertEqual(span.end_byte - span.start_byte, 7)
        self.assertEqual(span.role, "user")
        Workflow(scope, [replace(candidates[0], evidence=(span,))], [original])

    def test_split_utf8_negative_boolean_empty_and_oversize_ranges_rejected(self):
        _, _, originals = toy(1)
        original = replace(originals[0], text="中" * 200)
        for start_byte, end_byte in ((1, 3), (0, 2), (-1, 3), (True, 3), (0, 0), (0, 513), (0, 999)):
            with self.subTest(range=(start_byte, end_byte)), self.assertRaises(ContractError):
                evidence_span(original, start_byte, end_byte)

    def test_changed_text_role_time_relation_range_total_or_id_rejected(self):
        scope, candidates, originals = toy(1)
        good = candidates[0].evidence[0]
        changes = [dict(text="invented"), dict(role="assistant"), dict(occurred_at=None),
                   dict(relation="previous"), dict(start_byte=1), dict(total_bytes=999),
                   dict(evidence_id="evidence:" + "0" * 64)]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(ContractError):
                Workflow(scope, [replace(candidates[0], evidence=(replace(good, **change),))], originals)

    def test_rehashing_forged_text_cannot_create_original_evidence(self):
        scope, candidates, originals = toy(1)
        forged = replace(candidates[0].evidence[0], text="made up exact length irrelevant")
        forged = replace(forged, evidence_id="evidence:" + digest(forged.body()))
        with self.assertRaisesRegex(ContractError, "changed original text"):
            Workflow(scope, [replace(candidates[0], evidence=(forged,))], originals)

    def test_stale_snapshot_or_graph_source_branch_identity_rejected(self):
        scope, candidates, originals = toy(1)
        good = candidates[0].evidence[0]
        with self.assertRaisesRegex(ContractError, "stale evidence snapshot"):
            Workflow(scope, [replace(candidates[0], evidence=(replace(good, snapshot=h("stale")),))], originals)
        for field in ("source_group_key", "anchor_snapshot", "graph_generation", "anchor_ref"):
            identity = replace(candidates[0].identity, **{field: ("event:other-branch" if field == "anchor_ref" else h("different"))})
            with self.subTest(field=field), self.assertRaises(ContractError):
                Workflow(scope, [replace(candidates[0], identity=identity)], originals)

    def test_missing_original_and_duplicate_original_candidate_rejected(self):
        scope, candidates, originals = toy(1)
        for cs, os in ((candidates, []), (candidates, originals + [originals[0]]),
                       (candidates + candidates, originals)):
            with self.assertRaises(ContractError):
                Workflow(scope, cs, os)

    def test_cross_source_support_reference_rejected(self):
        w, packet, _, _ = start(2)
        foreign = packet["candidates"][1]["evidence"][0]["evidence_id"]
        a = assessment(packet, relevance="relevant", refs=[foreign])
        with self.assertRaisesRegex(ContractError, "foreign support"):
            w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))

    def test_cross_source_inspection_response_counts_bytes_but_cannot_enter_packet(self):
        w, packet, candidates, _ = start(2)
        w.accept(request(packet))
        ticket = w.begin_inspection(packet["candidates"][0]["candidate_id"], 100)
        with self.assertRaisesRegex(ContractError, "foreign evidence"):
            w.finish_inspection(ticket, b'bad result', candidates[1].evidence[0])
        self.assertEqual(w.ledger.result_bytes, 10)
        with self.assertRaisesRegex(ContractError, "missing or invalid"):
            w.next_packet()

    def test_short_digest_and_invalid_unicode_rejected(self):
        scope, candidates, originals = toy(1)
        with self.assertRaises(ContractError):
            replace(candidates[0].identity, source_group_key="1234").payload()
        with self.assertRaises(ContractError):
            Scope.create("bad\ud800", ["toy"])

    def test_packet_id_is_hash_of_all_other_fields(self):
        _, packet, _, _ = start()
        body = {k: v for k, v in packet.items() if k != "packet_id"}
        self.assertEqual(packet["packet_id"], "packet:" + digest(body))


class SchemaTests(unittest.TestCase):
    def test_strict_parse_round_trip(self):
        _, packet, _, _ = start()
        value = json.loads(encode(asdict(request(packet))))
        self.assertEqual(Judgment.parse(value), request(packet))

    def test_unknown_missing_wrong_types_and_duplicate_refs_rejected(self):
        w, packet, _, _ = start()
        value = json.loads(encode(asdict(request(packet))))
        for broken in ({**value, "confidence": 0.99}, {k: v for k, v in value.items() if k != "packet_id"},
                       {**value, "shortlist": "not an array"}, {**value, "inspect_candidate_ids": "bad"}):
            with self.assertRaises(ContractError):
                Judgment.parse(broken)
        a = assessment(packet)
        malformed = [replace(a, support_refs=a.support_refs * 2), replace(a, support_refs=()),
                     replace(a, source_relevance="maybe"), replace(a, answer_coverage="complete"),
                     replace(a, unresolved=("uncertain-answer",))]
        for broken in malformed:
            with self.assertRaises(ContractError):
                w.accept(Judgment(packet["packet_id"], (broken,), None, (), "source_uncertain"))

    def test_more_than_two_candidates_rejected(self):
        w, packet, _, _ = start()
        with self.assertRaises(ContractError):
            w.accept(request(packet, (0, 1, 2)))

    def test_more_than_three_support_spans_rejected_without_truncation(self):
        scope, candidates, originals = toy(1)
        pieces = tuple(evidence_span(originals[0], i, i + 1) for i in range(4))
        w = Workflow(scope, [replace(candidates[0], evidence=pieces)], originals)
        packet = w.initial_packet()
        a = assessment(packet, refs=[e.evidence_id for e in pieces])
        with self.assertRaisesRegex(ContractError, "three support refs"):
            w.accept(Judgment(packet["packet_id"], (a,), None, (), "source_uncertain"))

    def test_duplicate_candidate_and_inspect_ids_rejected(self):
        w, packet, _, _ = start()
        judgment = request(packet)
        for broken in (replace(judgment, shortlist=judgment.shortlist * 2),
                       replace(judgment, inspect_candidate_ids=judgment.inspect_candidate_ids * 2)):
            with self.assertRaises(ContractError):
                w.accept(broken)


class BudgetTests(unittest.TestCase):
    def test_policy_values_are_frozen(self):
        self.assertEqual(dict(POLICY), {
            "initial_packet_bytes": 32768, "subsequent_packet_bytes": 8192,
            "max_rounds_including_initial": 3, "max_shortlisted_candidates": 2,
            "max_inspections_total": 4, "max_inspections_per_candidate": 2,
            "max_workflow_tool_calls": 32, "max_workflow_result_bytes": 262144,
            "max_original_span_bytes": 512, "max_retained_spans_per_candidate": 3})
        with self.assertRaises(TypeError):
            POLICY["initial_packet_bytes"] = 999999

    def test_packet_budget_accounts_full_json_utf8_and_accounting_fields(self):
        w, packet, _, _ = start(query='查找\\"🙂' * 10)
        size = len(encode(packet))
        self.assertEqual(packet["budgets"]["packet_accounting_point"], "before_this_packet")
        self.assertEqual(w.ledger.packet_bytes, size)
        self.assertEqual(packet["budgets"]["packet_bytes_consumed"], 0)
        self.assertEqual(packet["budgets"]["current_packet_limit_bytes"], 32768)
        self.assertEqual(w.ledger.snapshot()["packet_bytes_remaining"], 49152 - size)
        self.assertGreater(size, len(encode(packet).decode()))

    def test_decimal_width_boundaries_do_not_cause_byte_accounting_oscillation(self):
        for count in range(20050, 20080):
            with self.subTest(count=count):
                w, packet, _, _ = start(1, query="q" * count)
                self.assertEqual(w.ledger.packet_bytes, len(encode(packet)))
                self.assertLess(len(encode(packet)), 32768)

    def test_initial_packet_exact_byte_limit_and_one_byte_over(self):
        _, baseline, _, _ = start(1, query="q")
        count = 32768 - len(encode(baseline)) + 1
        w, exact, _, _ = start(1, query="q" * count)
        self.assertEqual(len(encode(exact)), 32768)
        self.assertEqual(w.ledger.packet_bytes, 32768)
        scope, candidates, originals = toy(1, query="q" * (count + 1))
        w = Workflow(scope, candidates, originals)
        with self.assertRaises(BudgetFailure) as caught:
            w.initial_packet()
        self.assertEqual(caught.exception.details["required_bytes"], 32769)

    def test_initial_oversize_fails_explicitly_without_emission_or_truncation(self):
        scope, candidates, originals = toy(1, query="中" * 12000)
        w = Workflow(scope, candidates, originals)
        with self.assertRaises(BudgetFailure) as caught:
            w.initial_packet()
        self.assertEqual(caught.exception.reason, "required_packet_does_not_fit")
        self.assertGreater(caught.exception.details["required_bytes"], 32768)
        self.assertEqual(w.ledger.rounds, 0)
        self.assertEqual(w.ledger.packet_bytes, 0)
        self.assertEqual(scope.query, "中" * 12000)

    def test_following_oversize_fails_instead_of_dropping_required_query_or_support(self):
        w, first, _, originals = start(1, query="q" * 7000)
        w.accept(request(first))
        inspect(w, first, originals)
        with self.assertRaises(BudgetFailure) as caught:
            w.next_packet()
        self.assertEqual(caught.exception.details["limit_bytes"], 8192)
        self.assertEqual(w.ledger.rounds, 1)
        self.assertGreater(w.ledger.result_bytes, 0)

    def test_three_rounds_two_inspections_per_candidate_four_total(self):
        w, packet, _, originals = start(2)
        sizes = [len(encode(packet))]
        for _ in range(2):
            w.accept(request(packet, (0, 1)))
            for index in (0, 1):
                inspect(w, packet, originals, index)
            packet = w.next_packet()
            sizes.append(len(encode(packet)))
        state = w.ledger.snapshot()
        self.assertEqual(state["rounds_consumed"], 3)
        self.assertEqual(state["rounds_remaining"], 0)
        self.assertEqual(state["inspections_consumed"], 4)
        self.assertEqual(state["inspections_remaining"], 0)
        self.assertEqual(state["packet_bytes_consumed"], sum(sizes))
        with self.assertRaises(BudgetFailure):
            w.accept(request(packet))

    def test_exactly_32_tool_calls_allowed_33rd_blocked_before_dispatch(self):
        ledger = BudgetLedger()
        for _ in range(32):
            ledger.finish_tool(ledger.reserve_tool(1), b'x')
        self.assertEqual(ledger.snapshot()["tool_calls_remaining"], 0)
        with self.assertRaises(BudgetFailure):
            ledger.reserve_tool(1)
        self.assertEqual(ledger.tool_calls, 32)

    def test_result_budget_exact_limit_and_no_extra_reservation(self):
        ledger = BudgetLedger()
        ticket = ledger.reserve_tool(262144)
        self.assertEqual(ledger.snapshot()["result_bytes_reserved"], 262144)
        ledger.finish_tool(ticket, b'x' * 262144)
        self.assertEqual(ledger.result_bytes, 262144)
        self.assertEqual(ledger.snapshot()["result_bytes_remaining"], 0)
        with self.assertRaises(BudgetFailure):
            ledger.reserve_tool(1)

    def test_actual_oversize_is_counted_and_failure_is_terminal(self):
        ledger = BudgetLedger()
        ticket = ledger.reserve_tool(2)
        with self.assertRaises(BudgetFailure):
            ledger.finish_tool(ticket, "中文".encode())
        self.assertEqual(ledger.result_bytes, 6)
        with self.assertRaises(ContractError):
            ledger.reserve_tool(1)

    def test_pending_result_reservations_count_against_remaining(self):
        ledger = BudgetLedger()
        ledger.reserve_tool(200000)
        with self.assertRaises(BudgetFailure):
            ledger.reserve_tool(62145)
        self.assertEqual(ledger.tool_calls, 1)
        self.assertEqual(ledger.snapshot()["result_bytes_reserved"], 200000)

    def test_stale_forged_and_foreign_tickets_rejected(self):
        left, right = BudgetLedger(), BudgetLedger()
        left_ticket, right_ticket = left.reserve_tool(100), right.reserve_tool(100)
        with self.assertRaises(ContractError):
            left.finish_tool(right_ticket, b'{}')
        with self.assertRaises(ContractError):
            left.finish_tool(replace(left_ticket, result_byte_limit=101), b'{}')
        left.finish_tool(left_ticket, b'{}')
        with self.assertRaises(ContractError):
            left.finish_tool(left_ticket, b'{}')

    def test_per_candidate_inspection_limit(self):
        ledger = BudgetLedger()
        for _ in range(2):
            ledger.finish_tool(ledger.reserve_tool(1, "candidate:a"), b'x')
        with self.assertRaises(BudgetFailure):
            ledger.reserve_tool(1, "candidate:a")
        self.assertEqual(ledger.tool_calls, 2)

    def test_total_inspection_limit(self):
        ledger = BudgetLedger()
        for name in ("a", "a", "b", "b"):
            ledger.finish_tool(ledger.reserve_tool(1, name), b'x')
        with self.assertRaises(BudgetFailure):
            ledger.reserve_tool(1, "c")
        self.assertEqual(sum(ledger.inspections.values()), 4)

    def test_pending_generic_tool_blocks_packet_until_exact_result_accounted(self):
        scope, candidates, originals = toy(1)
        ledger = BudgetLedger()
        ticket = ledger.reserve_tool(100)
        w = Workflow(scope, candidates, originals, ledger)
        with self.assertRaisesRegex(ContractError, "still pending"):
            w.initial_packet()
        ledger.finish_tool(ticket, b'{"toy_discovery":true}')
        packet = w.initial_packet()
        self.assertEqual(packet["budgets"]["tool_calls_consumed"], 1)
        self.assertEqual(packet["budgets"]["result_bytes_consumed"], 22)

    def test_judgment_waits_until_all_dispatched_results_are_settled(self):
        w, packet, _, _ = start(1)
        ticket = w.ledger.reserve_tool(1)
        a = assessment(packet, relevance="relevant")
        with self.assertRaisesRegex(ContractError, "still pending"):
            w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))
        w.ledger.finish_tool(ticket, b'x')
        w.accept(Judgment(packet["packet_id"], (a,), a.candidate_id, (), "selected"))

    def test_same_ledger_ticket_cannot_substitute_workflow_inspection_ticket(self):
        w, packet, candidates, _ = start(1)
        w.accept(request(packet))
        cid = packet["candidates"][0]["candidate_id"]
        actual = w.begin_inspection(cid, 100)
        substitute = w.ledger.reserve_tool(100, cid)
        with self.assertRaisesRegex(ContractError, "foreign inspection ticket"):
            w.finish_inspection(substitute, b'{}', candidates[0].evidence[0])
        w.ledger.finish_tool(actual, b'{}')
        with self.assertRaisesRegex(ContractError, "missing or invalid"):
            w.next_packet()

    def test_result_budget_counts_raw_serialization_not_text_only(self):
        ledger = BudgetLedger()
        raw = b'{"text":"\\u4e2d","metadata":"unchanged"}'
        ledger.finish_tool(ledger.reserve_tool(len(raw)), raw)
        self.assertEqual(ledger.result_bytes, len(raw))
        self.assertNotEqual(ledger.result_bytes, len("中".encode()))


if __name__ == "__main__":
    unittest.main()
