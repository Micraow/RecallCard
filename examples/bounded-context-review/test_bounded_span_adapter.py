"""Invented candidate-review/1 wire records; no private fixtures or source text."""
import copy
import hashlib
import unittest
from dataclasses import replace

from judgment_contract import Assessment, BudgetFailure, BudgetLedger, ContractError, Judgment, Scope, encode
from bounded_span_adapter import BoundedSpanWorkflow, effective_filter, wire_hash, wire_packet_hash


def h(text):
    return hashlib.sha256(text.encode()).hexdigest()


def fixture(scope=None, count=3):
    scope = scope or Scope.create("beacon", ["personal"])
    t = {"version":"recallcard.candidate-review/1", "query_hash":h(scope.query),
         "scopes_hash":wire_hash(sorted(set(scope.scopes))),
         "event_filter":effective_filter(scope.payload()["event_filter"]), "as_of":scope.as_of,
         "max_graph_hops":2, "window_bytes":2048, "excerpt_bytes":512}
    rows = []
    for i in range(count):
        text = f"beacon {i}: source clue α."
        c = {"candidate_id":"", "anchor_ref":f"event:toy-{i}", "anchor_snapshot":h(f"snapshot-{i}"),
             "anchor_start_byte":0, "source_group_key":h(f"group-{i}"), "graph_generation":h(f"graph-{i}"),
             "evidence":[], "packet_hash":""}
        c["candidate_id"] = wire_hash(["recallcard.candidate-id/1",c["source_group_key"],c["anchor_ref"],c["anchor_snapshot"]])[:32]
        c["evidence"] = [{"reference":c["anchor_ref"],"snapshot":c["anchor_snapshot"],"start_byte":0,
            "end_byte":len(text.encode()),"total_bytes":10000,"text":text,"text_truncated":True,
            "role":"user","occurred_at":"2026-01-02T00:00:00Z","relation":"anchor"}]
        c["packet_hash"] = wire_packet_hash(t,c)
        rows.append({"review_candidate":c,"confirmation_available":True,"context_complete":False,
                     "context_stop_reasons":["request_filter_boundary"]})
    return scope, {"budget_unit":"utf8_bytes","relevance":"unassessed","results":rows,"review_template":t}


def start(page=None, scope=None):
    if page is None:
        scope, page = fixture(scope)
    ledger = BudgetLedger()
    raw = encode(page)
    ticket = ledger.reserve_tool(32768)
    return BoundedSpanWorkflow.from_discovery(scope, raw, ledger, ticket)


def judgment(packet, relevant=False):
    candidate = packet["candidates"][0]
    cid = candidate["candidate_id"]
    support = (candidate["evidence"][0]["evidence_id"],)
    return Judgment(packet["packet_id"], (Assessment(cid,"relevant" if relevant else "uncertain",
        "partial",support),), cid if relevant else None, () if relevant else (cid,),
        "selected" if relevant else "need_inspection")


def inspection(workflow, cid, text="Additional bounded clue β."):
    c = copy.deepcopy(workflow._wire_candidates[cid]); old = c["evidence"][0]
    start_byte = old["end_byte"]
    span = dict(old, text=text,start_byte=start_byte,end_byte=start_byte+len(text.encode()))
    c["evidence"] = [span]; c["packet_hash"] = wire_packet_hash(workflow._wire_template,c)
    return {"status":"partial","status_scope":"bounded_inspection_window","results":[],
        "updated_candidate":c,"relevance":"unresolved","deep_read_performed":False,
        "budget_unit":"utf8_bytes","text_bytes":len(text.encode()),"context_complete":False,
        "navigation":{"previous_ref":None,"next_refs":[],"stop_reasons":["request_filter_boundary"],
                      "distance_from_anchor":0,"max_graph_hops":2},
        "window":{"start_byte":0,"end_byte":2048,"max_excerpt_bytes":512,
                  "more_original_text_outside_window":True},"next_cursor":"opaque-server-cursor"}


class BoundedAdapterTests(unittest.TestCase):
    def test_initial_consumes_only_bounded_text_with_no_full_originals(self):
        workflow = start(); packet = workflow.initial_packet()
        self.assertEqual(workflow._originals,{})
        self.assertEqual(len(packet["candidates"]),3)
        self.assertEqual(workflow.ledger.tool_calls,1)
        self.assertLess(sum(len(x.text.encode()) for x in workflow._trusted_spans.values()),100)
        self.assertTrue(all(x.total_bytes==10000 for x in workflow._trusted_spans.values()))

    def test_partial_answer_selects_with_unchanged_server_gate(self):
        workflow = start(); packet=workflow.initial_packet(); workflow.accept(judgment(packet,True))
        args=workflow.selected_read_args(); proof=args["context_review"]
        self.assertEqual(proof["assessment"]["decision"],"relevant")
        self.assertEqual(proof["assessment"]["evidence_indices"],[0])
        self.assertEqual(proof["candidate"]["packet_hash"],wire_packet_hash(proof["template"],proof["candidate"]))
        self.assertNotIn("answer_coverage",proof["assessment"])
        proof["template"]["query_hash"]="changed"
        self.assertEqual(workflow.selected_read_args()["context_review"]["template"]["query_hash"],h("beacon"))

    def test_increment_keeps_only_explicit_shortlist_and_visible_originals(self):
        workflow=start(); p=workflow.initial_packet(); j=judgment(p);workflow.accept(j)
        ticket=workflow.begin_inspection(j.inspect_candidate_ids[0],4096)
        response=inspection(workflow,j.inspect_candidate_ids[0]); raw=encode(response)
        workflow.consume_inspection(ticket,raw,response["updated_candidate"]["anchor_ref"])
        p2=workflow.next_packet()
        self.assertEqual(len(p2["candidates"]),1)
        self.assertEqual(len(p2["candidates"][0]["evidence"]),2)
        self.assertLessEqual(len(encode(p2)),8192)
        self.assertIn("request_filter_boundary",p2["candidates"][0]["stop_reasons"])
        self.assertEqual(workflow.ledger.tool_calls,2)

    def test_filter_asof_and_sorted_scopes_preserved(self):
        scope=Scope.create("beacon",["work","personal"],{"role":"user"},"2026-01-03T00:00:00Z")
        workflow=start(scope=scope);p=workflow.initial_packet();workflow.accept(judgment(p,True))
        args=workflow.selected_read_args()
        self.assertEqual(args["event_filter"],effective_filter({"role":"user"}))
        self.assertEqual(args["as_of"],scope.as_of)

    def test_empty_filter_normalizes_only_declared_defaults(self):
        self.assertIsNone(effective_filter({}))
        with self.assertRaises(ContractError):effective_filter({"unknown":None})

    def test_missing_stale_and_changed_discovery_bindings_reject(self):
        for mutation in (lambda p:p["review_template"].update(query_hash=h("other")),
                         lambda p:p["review_template"].update(scopes_hash=h("other")),
                         lambda p:p["review_template"].update(as_of="2025-01-01T00:00:00Z"),
                         lambda p:p["results"][0]["review_candidate"].update(packet_hash=h("wrong")),
                         lambda p:p["results"][0]["review_candidate"].pop("graph_generation"),
                         lambda p:p["results"][0].update(confirmation_available=False)):
            with self.subTest(mutation=mutation):
                scope,page=fixture();mutation(page)
                with self.assertRaises(ContractError):start(page,scope)

    def test_duplicate_candidate_and_unknown_branch_field_reject(self):
        scope,page=fixture();page["results"].append(copy.deepcopy(page["results"][0]))
        with self.assertRaises(ContractError):start(page,scope)
        scope,page=fixture();page["results"][0]["review_candidate"]["branch_id"]="invented"
        with self.assertRaises(ContractError):start(page,scope)

    def test_bad_utf8_size_role_truncation_or_range_reject(self):
        for key,value in (("end_byte",1),("start_byte",-1),("role","developer"),
                          ("text_truncated",False),("total_bytes",1)):
            with self.subTest(key=key):
                scope,page=fixture();c=page["results"][0]["review_candidate"];c["evidence"][0][key]=value
                c["packet_hash"]=wire_packet_hash(page["review_template"],c)
                with self.assertRaises(ContractError):start(page,scope)

    def test_inspection_binding_target_and_radius_fail_closed_and_charge(self):
        for mode in ("graph","target","radius","window","bytes"):
            with self.subTest(mode=mode):
                w=start();p=w.initial_packet();j=judgment(p);w.accept(j);cid=j.inspect_candidate_ids[0]
                ticket=w.begin_inspection(cid,4096);response=inspection(w,cid);wire=response["updated_candidate"]
                if mode=="graph":wire["graph_generation"]=h("changed");wire["packet_hash"]=wire_packet_hash(w._wire_template,wire)
                if mode=="radius":response["navigation"]["distance_from_anchor"]=3
                if mode=="window":response["window"]["end_byte"]=1
                if mode=="bytes":response["text_bytes"]=1
                raw=encode(response);before=w.ledger.result_bytes
                target="event:other" if mode=="target" else wire["anchor_ref"]
                with self.assertRaises(ContractError):w.consume_inspection(ticket,raw,target)
                self.assertEqual(w.ledger.result_bytes,before+len(raw))
                self.assertTrue(w._terminal)

    def test_overlap_change_cannot_be_legitimized_by_new_unsigned_hash(self):
        w=start();p=w.initial_packet();j=judgment(p);w.accept(j);cid=j.inspect_candidate_ids[0]
        ticket=w.begin_inspection(cid,4096);response=inspection(w,cid);c=response["updated_candidate"]
        span=copy.deepcopy(w._wire_candidates[cid]["evidence"][0]);span["text"]=span["text"].replace("beacon","forged")
        c["evidence"]=[span];c["packet_hash"]=wire_packet_hash(w._wire_template,c);response["text_bytes"]=len(span["text"].encode())
        with self.assertRaises(ContractError):w.consume_inspection(ticket,encode(response),c["anchor_ref"])

    def test_server_refusal_stops_old_proof_reassessment(self):
        w=start();p=w.initial_packet();j=judgment(p);w.accept(j);cid=j.inspect_candidate_ids[0]
        ticket=w.begin_inspection(cid,4096)
        raw=encode({"status":"uncertain","reason":"source_ambiguous","results":[],"next_cursor":None,
                    "relevance":"unresolved","deep_read_performed":False,"budget_unit":"utf8_bytes"})
        w.consume_inspection(ticket,raw,"event:toy-0")
        self.assertIn("source_ambiguous",w._candidates[cid].stop_reasons)
        with self.assertRaises(ContractError):w.next_packet()
        with self.assertRaises(ContractError):w.selected_read_args()

    def test_changed_local_evidence_rejected_without_original_catalog(self):
        w=start();e=next(iter(w._trusted_spans.values()))
        altered=replace(e,text="changed",evidence_id=e.evidence_id)
        with self.assertRaises(ContractError):w._verify_evidence(e.candidate_id,altered)

    def test_raw_result_budget_and_duplicate_json_are_not_hidden(self):
        scope,page=fixture();raw=encode(page);ledger=BudgetLedger();ticket=ledger.reserve_tool(10)
        with self.assertRaises(BudgetFailure):BoundedSpanWorkflow.from_discovery(scope,raw,ledger,ticket)
        self.assertEqual(ledger.result_bytes,len(raw))
        ledger=BudgetLedger();ticket=ledger.reserve_tool(32768);raw=b'{"results":[],"results":[]}'
        with self.assertRaises(ContractError):BoundedSpanWorkflow.from_discovery(scope,raw,ledger,ticket)
        self.assertEqual(ledger.result_bytes,len(raw))


if __name__=="__main__":unittest.main()
