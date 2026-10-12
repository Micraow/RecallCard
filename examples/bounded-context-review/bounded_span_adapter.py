"""Existing candidate-review/1 wire adapter; no full originals or external I/O.

Input bytes must come from the authorized MCP transport, never from a judge.
Unsigned wire hashes check consistency, not issuance or original-source truth.
The server revalidates actual text, source graph, scope and revision on reads.
"""
from __future__ import annotations

import hashlib
import json

from judgment_contract import (
    BudgetLedger, Candidate, ContractError, Evidence, Identity, Scope, Workflow,
    _digest, _integer, _require, _text, digest, encode,
)

TEMPLATE_FIELDS = frozenset({"version", "query_hash", "scopes_hash", "event_filter",
    "as_of", "max_graph_hops", "window_bytes", "excerpt_bytes"})
CANDIDATE_FIELDS = frozenset({"candidate_id", "anchor_ref", "anchor_snapshot",
    "anchor_start_byte", "source_group_key", "graph_generation", "evidence", "packet_hash"})
SPAN_FIELDS = frozenset({"reference", "snapshot", "start_byte", "end_byte", "total_bytes",
    "text", "text_truncated", "role", "occurred_at", "relation"})
ROLES = frozenset({"user", "assistant", "system", "tool"})
FILTER_FIELDS = ("role", "occurred_from", "occurred_until", "on_current_path")


def effective_filter(value):
    if value is None:
        return None
    _require(type(value) is dict and set(value) <= set(FILTER_FIELDS), "unsupported event filter")
    normalized = {key: value.get(key) for key in FILTER_FIELDS}
    return normalized if any(x is not None for x in normalized.values()) else None


def wire_hash(value):
    # Current Rust json!(...).to_string(): sorted object keys, compact UTF-8 JSON.
    return hashlib.sha256(encode(value)).hexdigest()


def wire_packet_hash(template, candidate):
    copy = dict(candidate, packet_hash="")
    return wire_hash([template, copy])


def parse_result(raw):
    _require(type(raw) is bytes, "exact MCP result text bytes required")
    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            _require(key not in result, "duplicate wire JSON field")
            result[key] = value
        return result
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=object_pairs,
                           parse_constant=lambda _: (_ for _ in ()).throw(ContractError("nonfinite JSON")))
    except (ValueError, UnicodeError) as error:
        raise ContractError("invalid MCP result JSON") from error
    _require(type(value) is dict, "MCP result object required")
    return value


class BoundedSpanWorkflow(Workflow):
    """Connect the local judgment contract to already-returned bounded wire spans.

Use the same ledger for discovery, inspection, final reads and errors. Discovery
must have been reserved before dispatch; initialization settles its exact bytes.
This class does not launch tools, invoke judges, schedule traversal, or reread text.
"""

    @classmethod
    def from_discovery(cls, scope: Scope, raw_result: bytes,
                       ledger: BudgetLedger, discovery_ticket):
        ledger.finish_tool(discovery_ticket, raw_result)
        page = parse_result(raw_result)
        _require(page.get("budget_unit") == "utf8_bytes", "unknown discovery budget unit")
        _require(page.get("relevance") == "unassessed", "discovery must remain unassessed")
        template = page.get("review_template")
        _require(type(template) is dict and set(template) == TEMPLATE_FIELDS,
                 "unsupported review template shape")
        _require(template["version"] == "recallcard.candidate-review/1"
                 and template["max_graph_hops"] == 2 and template["window_bytes"] == 2048
                 and template["excerpt_bytes"] == 512, "unsupported review limits/version")
        _require(template["query_hash"] == hashlib.sha256(scope.query.encode()).hexdigest(),
                 "query binding changed")
        _require(template["scopes_hash"] == wire_hash(sorted(set(scope.scopes))), "scope binding changed")
        expected = scope.payload()
        _require(template["event_filter"] == effective_filter(expected["event_filter"])
                 and template["as_of"] == expected["as_of"], "filter/as_of binding changed")
        rows = page.get("results")
        _require(type(rows) is list and bool(rows), "nonempty discovery candidates required")
        obj = cls.__new__(cls)
        obj._wire_template = json.loads(encode(template))
        obj._wire_candidates = {}
        obj._wire_spans = {}
        obj._trusted_spans = {}
        obj.scope_id = scope.scope_id
        candidates = []
        server_ids = set()
        for row in rows:
            _require(type(row) is dict and row.get("confirmation_available") is True,
                     "candidate confirmation unavailable; do not omit silently")
            _require(row.get("context_complete") is False, "discovery completeness changed")
            stops = row.get("context_stop_reasons")
            _require(type(stops) is list and all(type(x) is str for x in stops), "invalid stop reasons")
            wire = obj._decode_candidate(row.get("review_candidate"))
            _require(wire["candidate_id"] not in server_ids, "duplicate/ambiguous server candidate")
            server_ids.add(wire["candidate_id"])
            identity = obj._identity(wire)
            cid = identity.candidate_id(scope.scope_id)
            obj._wire_candidates[cid] = wire
            evidence = tuple(obj._register_span(cid, wire, x) for x in wire["evidence"])
            _require(any(x.reference == wire["anchor_ref"] and x.start_byte == wire["anchor_start_byte"]
                         for x in evidence), "initial anchor evidence/offset missing")
            candidates.append(Candidate(identity, evidence, stop_reasons=tuple(stops)))
        Workflow.__init__(obj, scope, candidates, (), ledger)
        return obj

    @staticmethod
    def _identity(wire):
        return Identity(wire["source_group_key"], wire["anchor_ref"],
                        wire["anchor_snapshot"], wire["graph_generation"])

    def _decode_candidate(self, wire):
        _require(type(wire) is dict and set(wire) == CANDIDATE_FIELDS, "unsupported candidate shape")
        for key in ("anchor_snapshot", "source_group_key", "graph_generation", "packet_hash"):
            _digest(wire[key], key)
        _text(wire["anchor_ref"], "anchor_ref")
        _require(wire["anchor_ref"].startswith("event:"), "event anchor required")
        _integer(wire["anchor_start_byte"], "anchor_start_byte")
        expected = wire_hash(["recallcard.candidate-id/1", wire["source_group_key"],
                              wire["anchor_ref"], wire["anchor_snapshot"]])[:32]
        _require(wire["candidate_id"] == expected, "server candidate ID mismatch")
        _require(wire["packet_hash"] == wire_packet_hash(self._wire_template, wire),
                 "wire packet hash mismatch")
        _require(type(wire["evidence"]) is list and 1 <= len(wire["evidence"]) <= 3,
                 "one to three wire spans required")
        return json.loads(encode(wire))

    def _register_span(self, cid, candidate, span):
        _require(type(span) is dict and set(span) == SPAN_FIELDS, "unsupported evidence shape")
        _text(span["reference"], "reference")
        _require(span["reference"].startswith("event:"), "event evidence required")
        _digest(span["snapshot"], "snapshot")
        _text(span["text"], "text")
        for key in ("start_byte", "end_byte", "total_bytes"):
            _integer(span[key], key)
        start, end, total = (span[x] for x in ("start_byte", "end_byte", "total_bytes"))
        _require(0 <= start < end <= total and end-start <= 512
                 and len(span["text"].encode()) == end-start, "invalid returned UTF-8 range")
        _require(type(span["text_truncated"]) is bool
                 and span["text_truncated"] == (start > 0 or end < total), "truncation mismatch")
        _require(type(span["role"]) is str and type(span["relation"]) is str
                 and span["role"] in ROLES and span["relation"] in {"anchor", "previous", "next"},
                 "unknown role/relation")
        if span["occurred_at"] is not None:
            _text(span["occurred_at"], "occurred_at")
        is_anchor = span["reference"] == candidate["anchor_ref"]
        _require((span["relation"] == "anchor") == is_anchor, "anchor relation mismatch")
        if is_anchor:
            _require(span["snapshot"] == candidate["anchor_snapshot"], "stale anchor snapshot")
        body = {k: span[k] for k in SPAN_FIELDS if k != "text_truncated"}
        body.update(candidate_id=cid, scope_id=self.scope_id)
        evidence = Evidence(**body, evidence_id="evidence:" + digest(body))
        for old in self._trusted_spans.values():
            if old.candidate_id != cid or old.reference != evidence.reference:
                continue
            for key in ("snapshot", "role", "occurred_at", "relation", "total_bytes"):
                _require(getattr(old, key) == getattr(evidence, key), "source revision/metadata changed")
            a, b = max(old.start_byte, start), min(old.end_byte, end)
            if a < b:
                _require(old.text.encode()[a-old.start_byte:b-old.start_byte]
                         == evidence.text.encode()[a-start:b-start], "overlapping original bytes changed")
        self._trusted_spans[evidence.evidence_id] = evidence
        self._wire_spans[evidence.evidence_id] = json.loads(encode(span))
        return evidence

    def _verify_evidence(self, cid, evidence):
        _require(type(evidence) is Evidence and evidence.candidate_id == cid
                 and evidence.scope_id == self.scope_id
                 and self._trusted_spans.get(evidence.evidence_id) == evidence,
                 "missing, altered or foreign returned bounded span")

    def consume_inspection(self, ticket, raw_result: bytes, target_ref: str):
        """Settle exactly one requested tool result; decoder failures fail closed.

        target_ref is copied from the actual dispatched request. The transport
        must correlate its response ID. This method does not authorize an edge.
        """
        cid = ticket.inspect_candidate_id
        _require(self._started.get(cid) == ticket, "unrequested inspection ticket")
        try:
            result = parse_result(raw_result)
            _require(result.get("relevance") == "unresolved"
                     and result.get("deep_read_performed") is False
                     and result.get("results") == [], "inspection escalated or ambiguous result")
            _require(result.get("budget_unit") == "utf8_bytes", "unknown inspection budget unit")
            updated = result.get("updated_candidate")
            if updated is None:
                status = result.get("status")
                _require(status in {"uncertain", "budget_exhausted", "complete"}, "unknown inspection stop")
                reason = result.get("reason", "inspection_" + status)
                _text(reason, "inspection stop reason")
                navigation = result.get("navigation", {})
                stops = navigation.get("stop_reasons", [])
                _require(type(stops) is list and all(type(x) is str for x in stops), "invalid stop reasons")
                self.finish_inspection(ticket, raw_result, None, tuple(dict.fromkeys(stops + [reason])))
                if status == "uncertain":
                    # A server binding/access/revision refusal must not be recast
                    # as a fresh relevance decision on the old proof.
                    self._terminal = True
                return
            _require(result.get("context_complete") is False
                     and result.get("status_scope") == "bounded_inspection_window", "unbounded inspection")
            wire = self._decode_candidate(updated)
            old = self._wire_candidates[cid]
            _require(all(wire[k] == old[k] for k in CANDIDATE_FIELDS - {"evidence", "packet_hash"}),
                     "inspection changed original candidate binding")
            _require(len(wire["evidence"]) == 1 and wire["evidence"][0]["reference"] == target_ref,
                     "inspection target mismatch")
            returned = wire["evidence"][0]
            window = result.get("window")
            _require(type(window) is dict and window.get("max_excerpt_bytes") == 512,
                     "inspection window missing")
            for key in ("start_byte", "end_byte"):
                _integer(window.get(key), "window_" + key)
            begin = old["anchor_start_byte"] if target_ref == old["anchor_ref"] else 0
            _require(window["start_byte"] == begin
                     and begin <= returned["start_byte"] < returned["end_byte"] <= window["end_byte"]
                     <= min(begin + 2048, returned["total_bytes"]), "returned span exceeds fixed window")
            _require(result.get("text_bytes") == returned["end_byte"] - returned["start_byte"],
                     "inspection text byte accounting changed")
            navigation = result.get("navigation")
            _require(type(navigation) is dict and navigation.get("max_graph_hops") == 2,
                     "missing inspection navigation boundary")
            distance = navigation.get("distance_from_anchor")
            _integer(distance, "distance_from_anchor")
            _require(distance <= 2, "inspection radius exceeded")
            stops = navigation.get("stop_reasons")
            _require(type(stops) is list and all(type(x) is str for x in stops), "invalid navigation stops")
            span = self._register_span(cid, wire, wire["evidence"][0])
            self.finish_inspection(ticket, raw_result, span, tuple(stops))
        except Exception:
            if ticket.serial in self.ledger._pending:
                self.ledger.finish_tool(ticket, raw_result)
            self._terminal = True
            raise

    def selected_read_args(self, budget_bytes=8192):
        """Translate explicit relevance to the unchanged server read gate.

        Does not assert completeness, invoke a read, or bypass server revalidation.
        Caller must reserve/settle that read through this same ledger.
        """
        _integer(budget_bytes, "budget_bytes", 1)
        _require(self._judgment is not None and self._judgment.selected_candidate_id is not None,
                 "explicit relevant selection required")
        cid = self._judgment.selected_candidate_id
        assessment = next(x for x in self._judgment.shortlist if x.candidate_id == cid)
        wire = json.loads(encode(self._wire_candidates[cid]))
        wire["evidence"] = [self._wire_spans[x] for x in assessment.support_refs]
        wire["packet_hash"] = wire_packet_hash(self._wire_template, wire)
        anchor = wire["anchor_ref"]
        return {"refs": [anchor], "include_adjacent": True, "context_anchor_ref": anchor,
                "detail": "compact", "budget_bytes": budget_bytes,
                "event_filter": self._wire_template["event_filter"], "as_of": self._wire_template["as_of"],
                "context_review": {"stage": "read", "template": json.loads(encode(self._wire_template)),
                    "candidate": wire, "assessment": {"decision": "relevant",
                        "selected_anchor_ref": anchor, "evidence_indices": list(range(len(wire["evidence"])))}}}
