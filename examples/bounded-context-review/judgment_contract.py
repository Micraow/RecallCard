"""Local deterministic evidence/packet contract. No I/O, model, or access grants.

All originals must come through the existing authorized reader. SHA-256 bindings
only detect local inconsistency; they never establish server authorization.
"""
from __future__ import annotations

from dataclasses import asdict, dataclass, field
import hashlib
import json
import re
from typing import Any, Iterable
from types import MappingProxyType

VERSION = "recallcard.local-judgment/1"
POLICY = MappingProxyType({
    "initial_packet_bytes": 32768,
    "subsequent_packet_bytes": 8192,
    "max_rounds_including_initial": 3,
    "max_shortlisted_candidates": 2,
    "max_inspections_total": 4,
    "max_inspections_per_candidate": 2,
    "max_workflow_tool_calls": 32,
    "max_workflow_result_bytes": 262144,
    "max_original_span_bytes": 512,
    "max_retained_spans_per_candidate": 3,
})
# Internal immutable values, deliberately not affected by edits to public POLICY.
_LIMITS = tuple(POLICY.values())
(INITIAL, FOLLOWING, ROUNDS, SHORTLIST, INSPECTIONS, PER_CANDIDATE,
 CALLS, RESULTS, SPAN, SUPPORT) = _LIMITS
RELEVANCE = frozenset({"relevant", "uncertain", "unsupported"})
COVERAGE = frozenset({"not_checked", "partial", "sufficient", "conflicting"})
AMBIGUITIES = frozenset({"source_identity", "branch"})
AMBIGUOUS_STOPS = frozenset({"source_ambiguous", "ambiguous_source_identity",
    "source_identity_ambiguous", "source_identity", "branch_ambiguous",
    "ambiguous_branch", "branch_ambiguity", "branch"})
STOPS = frozenset({"selected", "need_inspection", "source_uncertain",
                   "unsupported", "source_ambiguity", "budget_exhausted"})
INSTRUCTIONS = (
    "Assess source_relevance separately from answer_coverage. A relevant source "
    "may be selected with partial, conflicting, or not_checked answer coverage. "
    "Unresolved source identity or branch ambiguity blocks selection. Cite only "
    "the exact evidence IDs whose full original spans appear in this packet. "
    "Shortlist at most two candidates and explicitly name necessary support "
    "and any candidates requiring inspection. Hashes are local integrity checks, "
    "not authorization, relevance proof, or permission to bypass the MCP read gate."
)


class ContractError(ValueError):
    """Malformed, inconsistent, stale, missing, or foreign local input."""


class BudgetFailure(ContractError):
    """Explicit failure; no required text is silently shortened or omitted."""

    def __init__(self, reason: str, consumed: dict[str, Any], **details: Any):
        self.reason = reason
        self.consumed = consumed
        self.details = details
        super().__init__(f"{reason}: {details}")


def _require(condition: bool, reason: str) -> None:
    if not condition:
        raise ContractError(reason)


def _text(value: Any, name: str, allow_empty: bool = False) -> None:
    _require(type(value) is str and (allow_empty or bool(value)), f"invalid {name}")
    try:
        value.encode("utf-8", errors="strict")
    except UnicodeEncodeError as error:
        raise ContractError(f"invalid UTF-8 in {name}") from error


def _integer(value: Any, name: str, minimum: int = 0) -> None:
    _require(type(value) is int and value >= minimum, f"invalid {name}")


def _digest(value: Any, name: str) -> None:
    _require(type(value) is str and re.fullmatch(r"[0-9a-f]{64}", value) is not None,
             f"invalid full {name} digest")


def encode(value: Any) -> bytes:
    """Exact canonical JSON representation used for all packet byte accounting."""
    try:
        return json.dumps(value, ensure_ascii=False, allow_nan=False,
                          sort_keys=True, separators=(",", ":")).encode("utf-8")
    except (TypeError, ValueError, UnicodeEncodeError) as error:
        raise ContractError("not canonical UTF-8 JSON") from error


def digest(value: Any) -> str:
    return hashlib.sha256(encode(value)).hexdigest()


def _strings(values: Any, name: str) -> None:
    _require(type(values) is tuple, f"{name} must be a tuple")
    for value in values:
        _text(value, name)
    _require(len(set(values)) == len(values), f"duplicate {name}")


@dataclass(frozen=True)
class Scope:
    query: str
    scopes: tuple[str, ...]
    event_filter_json: str = "null"
    as_of: str | None = None

    @classmethod
    def create(cls, query: str, scopes: Iterable[str],
               event_filter: dict[str, Any] | None = None,
               as_of: str | None = None) -> Scope:
        result = cls(query, tuple(scopes), encode(event_filter).decode("utf-8"), as_of)
        result.payload()
        return result

    def payload(self) -> dict[str, Any]:
        _text(self.query, "query")
        _strings(self.scopes, "scopes")
        _require(bool(self.scopes), "scopes required")
        _text(self.event_filter_json, "event filter")
        try:
            value = json.loads(self.event_filter_json)
        except (ValueError, TypeError) as error:
            raise ContractError("invalid event filter JSON") from error
        _require(value is None or type(value) is dict, "event filter must be object or null")
        _require(encode(value).decode("utf-8") == self.event_filter_json,
                 "event filter must be canonical JSON")
        if self.as_of is not None:
            _text(self.as_of, "as_of")
        return {"query": self.query, "scopes": list(self.scopes),
                "event_filter": value, "as_of": self.as_of}

    @property
    def scope_id(self) -> str:
        return "scope:" + digest(self.payload())


@dataclass(frozen=True)
class Identity:
    source_group_key: str
    anchor_ref: str
    anchor_snapshot: str
    graph_generation: str

    def payload(self) -> dict[str, str]:
        for name in ("source_group_key", "anchor_snapshot", "graph_generation"):
            _digest(getattr(self, name), name)
        _text(self.anchor_ref, "anchor_ref")
        _require(self.anchor_ref.startswith("event:") and len(self.anchor_ref) > 6,
                 "original event anchor required")
        return asdict(self)

    def candidate_id(self, scope_id: str) -> str:
        return "candidate:" + digest({"scope_id": scope_id, "identity": self.payload()})


@dataclass(frozen=True)
class Original:
    """Trusted local copy of an authorized original and its graph relation.

    The adapter, not this helper, must establish source/branch/graph/access truth.
    Relations and snapshots are supplied by that adapter, never by a judge.
    """
    scope_id: str
    identity: Identity
    reference: str
    snapshot: str
    role: str
    occurred_at: str | None
    relation: str
    text: str

    @property
    def candidate_id(self) -> str:
        return self.identity.candidate_id(self.scope_id)


@dataclass(frozen=True)
class Evidence:
    candidate_id: str
    scope_id: str
    reference: str
    snapshot: str
    role: str
    occurred_at: str | None
    relation: str
    start_byte: int
    end_byte: int
    total_bytes: int
    text: str
    evidence_id: str

    def body(self) -> dict[str, Any]:
        result = asdict(self)
        result.pop("evidence_id")
        return result

    def payload(self) -> dict[str, Any]:
        return asdict(self)


def evidence_span(original: Original, start_byte: int, end_byte: int) -> Evidence:
    """Extract exactly the requested nonempty UTF-8 range, never truncate it."""
    _integer(start_byte, "start_byte")
    _integer(end_byte, "end_byte")
    _text(original.text, "original text", allow_empty=True)
    raw = original.text.encode("utf-8")
    _require(0 <= start_byte < end_byte <= len(raw), "invalid original byte range")
    _require(end_byte - start_byte <= SPAN, "original span exceeds 512 bytes")
    try:
        text = raw[start_byte:end_byte].decode("utf-8", errors="strict")
    except UnicodeDecodeError as error:
        raise ContractError("range splits UTF-8 codepoint") from error
    body = {"candidate_id": original.candidate_id, "scope_id": original.scope_id,
            "reference": original.reference, "snapshot": original.snapshot,
            "role": original.role, "occurred_at": original.occurred_at,
            "relation": original.relation, "start_byte": start_byte,
            "end_byte": end_byte, "total_bytes": len(raw), "text": text}
    return Evidence(**body, evidence_id="evidence:" + digest(body))


@dataclass(frozen=True)
class Candidate:
    identity: Identity
    evidence: tuple[Evidence, ...]
    source_identity_ambiguous: bool = False
    branch_ambiguous: bool = False
    context_complete: bool = False
    stop_reasons: tuple[str, ...] = ()


@dataclass(frozen=True)
class Assessment:
    candidate_id: str
    source_relevance: str
    answer_coverage: str
    support_refs: tuple[str, ...]
    unresolved: tuple[str, ...] = ()


@dataclass(frozen=True)
class Judgment:
    packet_id: str
    shortlist: tuple[Assessment, ...]
    selected_candidate_id: str | None
    inspect_candidate_ids: tuple[str, ...]
    stop_reason: str

    @classmethod
    def parse(cls, value: dict[str, Any]) -> Judgment:
        """Strict JSON-shaped input: unknown fields and type coercions fail."""
        fields = {"packet_id", "shortlist", "selected_candidate_id",
                  "inspect_candidate_ids", "stop_reason"}
        _require(type(value) is dict and set(value) == fields, "invalid judgment fields")
        _require(type(value["shortlist"]) is list, "shortlist must be array")
        _require(type(value["inspect_candidate_ids"]) is list, "inspect IDs must be array")
        assessments = []
        for item in value["shortlist"]:
            expected = {"candidate_id", "source_relevance", "answer_coverage",
                        "support_refs", "unresolved"}
            _require(type(item) is dict and set(item) == expected, "invalid assessment fields")
            _require(type(item["support_refs"]) is list and type(item["unresolved"]) is list,
                     "support/unresolved must be arrays")
            assessments.append(Assessment(item["candidate_id"], item["source_relevance"],
                                          item["answer_coverage"], tuple(item["support_refs"]),
                                          tuple(item["unresolved"])))
        return cls(value["packet_id"], tuple(assessments), value["selected_candidate_id"],
                   tuple(value["inspect_candidate_ids"]), value["stop_reason"])


@dataclass(frozen=True)
class Ticket:
    serial: int
    result_byte_limit: int
    inspect_candidate_id: str | None = None
    _owner: object = field(default=None, repr=False, compare=False)


@dataclass
class BudgetLedger:
    """In-memory accounting, not a durable quota or server security boundary.

    Reserve before dispatch. Finish with exact raw result bytes, even on upstream
    failure. Calls/inspection attempts are charged at reservation. An oversized
    actual response is still fully counted and seals the ledger in failure.
    """
    tool_calls: int = field(default=0, init=False)
    result_bytes: int = field(default=0, init=False)
    packet_bytes: int = field(default=0, init=False)
    rounds: int = field(default=0, init=False)
    inspections: dict[str, int] = field(default_factory=dict, init=False)
    _pending: dict[int, Ticket] = field(default_factory=dict, init=False, repr=False)
    failure: str | None = field(default=None, init=False)
    _owner: object = field(default_factory=object, init=False, repr=False)

    def snapshot(self) -> dict[str, Any]:
        reserved = sum(t.result_byte_limit for t in self._pending.values())
        count = sum(self.inspections.values())
        return {
            "unit": "utf8_bytes", "tool_calls_consumed": self.tool_calls,
            "tool_calls_remaining": max(0, CALLS - self.tool_calls),
            "result_bytes_consumed": self.result_bytes,
            "result_bytes_reserved": reserved,
            "result_bytes_remaining": max(0, RESULTS - self.result_bytes - reserved),
            "packet_bytes_consumed": self.packet_bytes,
            "packet_bytes_remaining": max(0, INITIAL + FOLLOWING * (ROUNDS - 1) - self.packet_bytes),
            "rounds_consumed": self.rounds, "rounds_remaining": max(0, ROUNDS - self.rounds),
            "inspections_consumed": count, "inspections_remaining": max(0, INSPECTIONS - count),
            "inspections_by_candidate": {
                key: {"consumed": value, "remaining": max(0, PER_CANDIDATE - value)}
                for key, value in sorted(self.inspections.items())},
            "failure": self.failure,
        }

    def _live(self) -> None:
        _require(self.failure is None, f"budget ledger stopped: {self.failure}")

    def _fail(self, reason: str, **details: Any) -> None:
        self.failure = reason
        raise BudgetFailure(reason, self.snapshot(), **details)

    def reserve_tool(self, result_byte_limit: int,
                     inspect_candidate_id: str | None = None) -> Ticket:
        self._live()
        _integer(result_byte_limit, "result_byte_limit", 1)
        if inspect_candidate_id is not None:
            _text(inspect_candidate_id, "inspect candidate ID")
            if sum(self.inspections.values()) >= INSPECTIONS:
                self._fail("inspection_total_budget_exhausted")
            if self.inspections.get(inspect_candidate_id, 0) >= PER_CANDIDATE:
                self._fail("candidate_inspection_budget_exhausted")
        if self.tool_calls >= CALLS:
            self._fail("workflow_call_budget_exhausted")
        remaining = self.snapshot()["result_bytes_remaining"]
        if result_byte_limit > remaining:
            self._fail("workflow_result_budget_exhausted", required_bytes=result_byte_limit,
                       remaining_bytes=remaining)
        self.tool_calls += 1
        ticket = Ticket(self.tool_calls, result_byte_limit, inspect_candidate_id, self._owner)
        self._pending[ticket.serial] = ticket
        if inspect_candidate_id is not None:
            self.inspections[inspect_candidate_id] = self.inspections.get(inspect_candidate_id, 0) + 1
        return ticket

    def finish_tool(self, ticket: Ticket, raw_result: bytes) -> None:
        # Settlement is allowed after a failure so already-dispatched bytes are never hidden.
        _require(type(ticket) is Ticket and ticket._owner is self._owner
                 and self._pending.get(ticket.serial) == ticket,
                 "missing, stale, or foreign tool ticket")
        _require(type(raw_result) is bytes, "raw result must be exact bytes")
        self._pending.pop(ticket.serial)
        self.result_bytes += len(raw_result)
        if len(raw_result) > ticket.result_byte_limit or self.result_bytes > RESULTS:
            self._fail("actual_result_exceeded_reserved_budget", actual_bytes=len(raw_result),
                       reserved_bytes=ticket.result_byte_limit)


class Workflow:
    """One case, at most three fresh judge packets, with explicit evidence carry."""

    def __init__(self, scope: Scope, candidates: Iterable[Candidate],
                 originals: Iterable[Original], ledger: BudgetLedger | None = None):
        _require(type(scope) is Scope, "Scope required")
        self.scope = scope
        self.scope_id = scope.scope_id
        self.ledger = ledger if ledger is not None else BudgetLedger()
        _require(type(self.ledger) is BudgetLedger, "BudgetLedger required")
        self._candidates: dict[str, Candidate] = {}
        self._originals: dict[tuple[str, str], Original] = {}
        self._last: dict[str, Any] | None = None
        self._judgment: Judgment | None = None
        self._new: dict[str, list[Evidence]] = {}
        self._started: dict[str, Ticket] = {}
        self._finished: set[str] = set()
        self._terminal = False
        for candidate in candidates:
            _require(type(candidate) is Candidate, "Candidate required")
            cid = candidate.identity.candidate_id(self.scope_id)
            _require(cid not in self._candidates, "duplicate or ambiguous candidate ID")
            _require(type(candidate.evidence) is tuple, "candidate evidence must be tuple")
            _strings(candidate.stop_reasons, "stop reasons")
            for flag in (candidate.source_identity_ambiguous, candidate.branch_ambiguous,
                         candidate.context_complete):
                _require(type(flag) is bool, "candidate flags must be booleans")
            self._candidates[cid] = candidate
        _require(bool(self._candidates), "candidates required")
        for original in originals:
            self._validate_original(original)
            key = (original.candidate_id, original.reference)
            _require(key not in self._originals, "duplicate or ambiguous original reference")
            self._originals[key] = original
        for cid, candidate in self._candidates.items():
            seen: set[str] = set()
            for evidence in candidate.evidence:
                self._verify_evidence(cid, evidence)
                _require(evidence.evidence_id not in seen, "duplicate initial evidence ID")
                seen.add(evidence.evidence_id)

    def _validate_original(self, original: Original) -> None:
        _require(type(original) is Original, "Original required")
        _require(original.scope_id == self.scope_id, "foreign original scope/filter/as_of")
        cid = original.candidate_id
        _require(cid in self._candidates, "foreign original source/branch/graph identity")
        _text(original.reference, "original reference")
        _require(original.reference.startswith("event:") and len(original.reference) > 6,
                 "original event reference required")
        _digest(original.snapshot, "original snapshot")
        for name in ("role", "relation"):
            _text(getattr(original, name), name)
        if original.occurred_at is not None:
            _text(original.occurred_at, "occurred_at")
        _text(original.text, "original text", allow_empty=True)
        if original.reference == original.identity.anchor_ref:
            _require(original.snapshot == original.identity.anchor_snapshot,
                     "stale anchor snapshot")
            _require(original.relation == "anchor", "anchor relation mismatch")
        else:
            _require(original.relation != "anchor", "foreign anchor relation")

    def _verify_evidence(self, cid: str, evidence: Evidence) -> None:
        _require(type(evidence) is Evidence, "Evidence required")
        _require(evidence.candidate_id == cid and evidence.scope_id == self.scope_id,
                 "foreign evidence source/branch/filter/as_of")
        original = self._originals.get((cid, evidence.reference))
        _require(original is not None, "missing original reference")
        _require(evidence.snapshot == original.snapshot, "stale evidence snapshot")
        expected = evidence_span(original, evidence.start_byte, evidence.end_byte)
        _require(evidence == expected, "changed original text/range/role/identity or invalid evidence ID")

    def _candidate_payload(self, cid: str, evidence: Iterable[Evidence]) -> dict[str, Any]:
        candidate = self._candidates[cid]
        return {"candidate_id": cid, "identity": candidate.identity.payload(),
                "source_identity_ambiguous": candidate.source_identity_ambiguous,
                "branch_ambiguous": candidate.branch_ambiguous,
                "context_complete": candidate.context_complete,
                "stop_reasons": list(candidate.stop_reasons),
                "evidence": [item.payload() for item in evidence]}

    def _emit(self, candidates: list[dict[str, Any]], carried: dict[str, list[str]],
              preceding_stop_reason: str | None) -> dict[str, Any]:
        self.ledger._live()
        _require(not self._terminal, "workflow stopped")
        _require(not self.ledger._pending, "tool results still pending")
        if self.ledger.rounds >= ROUNDS:
            self.ledger._fail("round_budget_exhausted")
        round_number = self.ledger.rounds + 1
        cap = INITIAL if round_number == 1 else FOLLOWING
        payload = {"version": VERSION, "scope_id": self.scope_id, "scope": self.scope.payload(),
                   "round": round_number, "instructions": INSTRUCTIONS,
                   "policy": dict(zip(POLICY.keys(), _LIMITS)),
                   "candidates": candidates, "carried_support_refs": carried,
                   "preceding_stop_reason": preceding_stop_reason, "budgets": {},
                   "packet_id": "packet:" + "0" * 64}
        # Record the pre-emission ledger, avoiding self-referential decimal-width
        # oscillation. Exact post-emission consumption remains in ledger.snapshot().
        state = self.ledger.snapshot()
        state["packet_accounting_point"] = "before_this_packet"
        state["current_packet_limit_bytes"] = cap
        payload["budgets"] = state
        body = {key: value for key, value in payload.items() if key != "packet_id"}
        payload["packet_id"] = "packet:" + digest(body)
        size = len(encode(payload))
        if size > cap:
            self.ledger._fail("required_packet_does_not_fit", required_bytes=size,
                              limit_bytes=cap, round=round_number)
        self.ledger.rounds = round_number
        self.ledger.packet_bytes += size
        # Keep a private decoded copy so a caller cannot mutate validation state.
        self._last = json.loads(encode(payload))
        self._judgment = None
        self._new = {}
        self._started = {}
        self._finished = set()
        return json.loads(encode(payload))

    def initial_packet(self) -> dict[str, Any]:
        _require(self._last is None and self.ledger.rounds == 0, "initial packet already emitted")
        candidates = [self._candidate_payload(cid, candidate.evidence)
                      for cid, candidate in self._candidates.items()]
        return self._emit(candidates, {}, None)

    def accept(self, judgment: Judgment) -> None:
        self.ledger._live()
        _require(not self._terminal and self._last is not None and self._judgment is None,
                 "no unanswered current packet")
        _require(not self.ledger._pending, "tool results still pending")
        _require(type(judgment) is Judgment, "Judgment required")
        _require(judgment.packet_id == self._last["packet_id"], "missing/stale/foreign packet ID")
        _require(type(judgment.shortlist) is tuple and len(judgment.shortlist) <= SHORTLIST,
                 "shortlist exceeds two candidates or is malformed")
        _strings(judgment.inspect_candidate_ids, "inspect candidate IDs")
        _require(len(judgment.inspect_candidate_ids) <= SHORTLIST, "too many inspect candidates")
        _text(judgment.stop_reason, "stop reason")
        _require(judgment.stop_reason in STOPS, "unknown judgment stop reason")
        available = {c["candidate_id"]: c for c in self._last["candidates"]}
        chosen: dict[str, Assessment] = {}
        for assessment in judgment.shortlist:
            _require(type(assessment) is Assessment, "Assessment required")
            _text(assessment.candidate_id, "assessment candidate ID")
            cid = assessment.candidate_id
            _require(cid in available and cid not in chosen, "missing/foreign/duplicate candidate ID")
            _text(assessment.source_relevance, "source relevance")
            _text(assessment.answer_coverage, "answer coverage")
            _require(assessment.source_relevance in RELEVANCE, "invalid source relevance")
            _require(assessment.answer_coverage in COVERAGE, "invalid answer coverage")
            _strings(assessment.support_refs, "support refs")
            _require(1 <= len(assessment.support_refs) <= SUPPORT, "one to three support refs required")
            _strings(assessment.unresolved, "unresolved ambiguities")
            _require(set(assessment.unresolved) <= AMBIGUITIES, "unknown ambiguity")
            evidence = {e["evidence_id"]: e for e in available[cid]["evidence"]}
            _require(set(assessment.support_refs) <= evidence.keys(),
                     "missing/stale/foreign support reference; fresh judge needs original text")
            chosen[cid] = assessment
        _require(set(judgment.inspect_candidate_ids) <= chosen.keys(),
                 "inspection candidate must be explicitly shortlisted")
        _require(all(chosen[cid].source_relevance == "uncertain"
                     for cid in judgment.inspect_candidate_ids),
                 "inspection is limited to uncertain source relevance")
        selected = judgment.selected_candidate_id
        if selected is not None:
            _text(selected, "selected candidate ID")
            _require(selected in chosen, "selected candidate must be shortlisted")
            assessment = chosen[selected]
            candidate = self._candidates[selected]
            _require(assessment.source_relevance == "relevant", "selection requires relevant source")
            _require(not assessment.unresolved and not candidate.source_identity_ambiguous
                     and not candidate.branch_ambiguous
                     and not (set(candidate.stop_reasons) & AMBIGUOUS_STOPS),
                     "unresolved identity/branch ambiguity blocks selection")
            _require(not judgment.inspect_candidate_ids and judgment.stop_reason == "selected",
                     "selection and continuing inspection conflict")
        elif judgment.inspect_candidate_ids:
            _require(judgment.stop_reason == "need_inspection", "inspection stop reason mismatch")
            if self.ledger.rounds >= ROUNDS:
                self.ledger._fail("round_budget_exhausted")
        else:
            _require(judgment.stop_reason not in {"selected", "need_inspection"},
                     "stop reason requires selection or inspection")
        self._judgment = judgment
        self._terminal = selected is not None or not judgment.inspect_candidate_ids

    def begin_inspection(self, candidate_id: str, result_byte_limit: int) -> Ticket:
        _text(candidate_id, "inspect candidate ID")
        _require(not self._terminal and self._judgment is not None,
                 "explicit inspection request required")
        _require(candidate_id in self._judgment.inspect_candidate_ids,
                 "candidate not explicitly requested for inspection")
        _require(candidate_id not in self._started, "candidate already inspected this round")
        ticket = self.ledger.reserve_tool(result_byte_limit, candidate_id)
        self._started[candidate_id] = ticket
        return ticket

    def finish_inspection(self, ticket: Ticket, raw_result: bytes, evidence: Evidence | None,
                          stop_reasons: tuple[str, ...] = (),
                          source_identity_ambiguous: bool = False,
                          branch_ambiguous: bool = False) -> None:
        # Charge the actual response before checking its semantic/provenance fields.
        self.ledger.finish_tool(ticket, raw_result)
        cid = ticket.inspect_candidate_id
        _require(self._started.get(cid) == ticket and cid not in self._finished,
                 "missing, stale, or foreign inspection ticket")
        _strings(stop_reasons, "inspection stop reasons")
        _require(type(source_identity_ambiguous) is bool and type(branch_ambiguous) is bool,
                 "inspection ambiguity flags must be booleans")
        if evidence is not None:
            self._verify_evidence(cid, evidence)
            self._new.setdefault(cid, []).append(evidence)
        else:
            _require(bool(stop_reasons), "empty inspection requires explicit stop reason")
        candidate = self._candidates[cid]
        stops = tuple(dict.fromkeys(candidate.stop_reasons + stop_reasons))
        self._candidates[cid] = Candidate(
            candidate.identity, candidate.evidence,
            candidate.source_identity_ambiguous or source_identity_ambiguous,
            candidate.branch_ambiguous or branch_ambiguous,
            candidate.context_complete, stops)
        self._finished.add(cid)

    def next_packet(self) -> dict[str, Any]:
        _require(not self._terminal and self._judgment is not None and self._last is not None,
                 "continuing explicit judgment required")
        _require(self._finished == set(self._judgment.inspect_candidate_ids),
                 "requested inspections missing or invalid")
        previous = {c["candidate_id"]: c for c in self._last["candidates"]}
        carried: dict[str, list[str]] = {}
        candidates = []
        for assessment in self._judgment.shortlist:
            cid = assessment.candidate_id
            visible = {e["evidence_id"]: Evidence(**e) for e in previous[cid]["evidence"]}
            support = [visible[ref] for ref in assessment.support_refs]
            combined = {e.evidence_id: e for e in support + self._new.get(cid, [])}
            for evidence in combined.values():
                self._verify_evidence(cid, evidence)
            carried[cid] = list(assessment.support_refs)
            candidates.append(self._candidate_payload(cid, combined.values()))
        return self._emit(candidates, carried, self._judgment.stop_reason)
