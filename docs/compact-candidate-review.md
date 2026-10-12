# Compact candidate discovery and bounded inspection

This opt-in workflow preserves original evidence and keeps discovery, inspection,
relevance assessment, and deep reading distinct. Existing `session_discovery`,
`session_candidates`, ordinary `read`, and `context_confirmation` remain available.
No model/API call is made by this interface.

## Discover: text before count

Use `search` with `target="events"`, `detail="session_discovery_compact"`,
`budget_bytes`, and `limit`. Optional `preview_min_bytes` is one of 64, 128, 256,
512 (default 512). Independent semantic/lexical channel deduplication, source
identity, raw scores, and fixed round-robin order are unchanged.

The result contains one `review_template` and each candidate has one
`review_candidate.evidence` array. Original text, speaker, time, snapshot, UTF-8
byte range, total bytes, and `text_truncated` appear together. No generated
summary replaces the originals. Common query/scope/filter/inspection constraints
are shared; candidate-specific identity/revision/graph constraints remain beside
the evidence. Each candidate retains `context_complete=false` and
`context_stop_reasons`, so a filtered, unavailable or ambiguous neighbor is not
silently represented as absent source material. Display source labels may be truncated; the full source-group hash
is used for verification.

The entire serialized JSON, including escaping, counts against the byte budget.
Compact mode preserves the requested excerpt size before maximizing candidate
count. Short original ranges and UTF-8 alignment may yield smaller excerpts. If
one candidate cannot fit, no candidate offset advances: retry with the returned
`recommended_min_budget` and `retry_cursor`, retaining other arguments. This
candidate cursor is not text pagination or graph expansion. A 512-byte setting
is an explicit engineering budget, not an information-theoretic quality claim.
Legacy discovery retains its original count-first fitting behavior.

## Inspect while relevance is unresolved

Copy the common template and one candidate into a read request:

```json
{
  "refs": ["event:<target>"],
  "include_adjacent": true,
  "context_anchor_ref": "event:<original-anchor>",
  "event_filter": null,
  "as_of": null,
  "detail": "compact",
  "budget_bytes": 4096,
  "context_review": {
    "stage": "inspect",
    "template": "<copy review_template object>",
    "candidate": "<copy review_candidate object>",
    "assessment": {
      "decision": "uncertain",
      "selected_anchor_ref": null,
      "evidence_indices": []
    }
  }
}
```

The example's placeholder strings must be replaced by the actual objects/refs;
copy any non-null filter and as_of unchanged. The server rechecks current access,
suppression, original source identity, revision, graph generation, every evidence
span's original bytes, speaker/time and relation. The 128-bit `candidate_id` is
only a compact stable label: full identity and snapshots are always validated.
Its label is stable across excerpt/budget changes but changes with anchor/source
revision. `packet_hash` additionally binds all evidence and the shared template.

Inspection is limited to two graph edges in a monotonic direction from the
original anchor. Every intermediate node must be permitted. Going to an ancestor
never permits crossing into a sibling. Branch ambiguity is assessed before
filtering. Unknown graphs, hidden/filtered intermediates, stale revisions, or
ambiguous identity stop; time order never invents an edge.

Each call returns at most 512 original UTF-8 bytes from one Event, inside a fixed
2048-byte window. For the anchor, the window starts at its original preview
start; other inspected Events start at byte zero. The result's
`updated_candidate.evidence` contains just the newly inspected span, not a second
copy in `results`. It can replace the candidate for a later assessment. An empty
Event returns zero text and no updated candidate, with bounded navigation so the
caller can stop or inspect another permitted Event.

`navigation` exposes only permitted previous/next refs within the same two-hop
radius; there is no unbounded `next_user_ref` shortcut. Inspection always returns
`relevance="unresolved"` and `deep_read_performed=false`, even if a caller supplied
a relevant label. `complete` means only this bounded inspection window ended;
`context_complete=false` and window metadata disclose text outside the window.

An `ri1` cursor continues this finite window. Retain the exact original review,
target, budget, and mode; do not replace it with `updated_candidate` mid-cursor.
It cannot be used for ordinary or assessed deep reading. `offset_bytes` may
locate an original UTF-8 boundary inside the same window, never together with a
cursor. An exhausted budget emits no text and does not advance; retry that
position with the recommended budget. A changed budget needs an explicit offset
rather than the previous budget's cursor.

## Assess, then read

An automatic caller, existing LLM, reranker, or human may compare candidates and
inspect limited additional context. Inspection is optional: this interface does
not prescribe inspecting every candidate or a fixed number of calls. A caller
can inspect only unresolved candidates when the missing context could affect its
decision, while retaining a total workflow budget. No human click is required.
The server does not decide relevance. To deep-read, use `stage="read"`, set `decision="relevant"`,
select the exact original `selected_anchor_ref`, and cite one or more nonempty
`evidence_indices` from the reviewed candidate. Missing/uncertain/irrelevant
assessment returns zero deep-read text. Both old and new confirmation arguments
in one request are rejected.

After checking the complete packet (including unselected evidence), the server
uses the existing confirmed-reader checks. It returns
`relevance="caller_assessed"`, not machine-proven relevance. Keep the same review
for deep-read text cursors and graph steps. The original anchor, access,
source/branch, event_filter, and as_of boundaries remain in force.

## Limits and evaluation

These are unsigned consistency hashes, not server-issued receipts or access
grants. A caller can construct another valid packet from authorized originals;
this interface does not prevent that, prove honest assessment, or enforce a
cumulative quota. A window is fixed for an unchanged packet/cursor, not a global
limit on all future authorized requests. Ordinary legacy reading remains possible.

The orchestrator must predeclare and enforce a total call/byte budget across all
candidate inspections and final reads. Exhaustion or ambiguous evidence must
remain unresolved, not become a fabricated relevant decision. Measure initial
packet bytes, original-text bytes, candidate coverage, selection correctness,
abstention, final original-evidence coverage, calls, total response/wire bytes,
and latency separately. Bytes are not model tokens. Smaller packets alone do
not demonstrate improved semantic accuracy. Seen regression questions are not
new blind validation.
