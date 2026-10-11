# Bounded, bidirectional event context

`search` now accepts `detail: "session_context"` with `target: "events"`.
The existing default search/read contracts stay unchanged. This is an explicit
source-reading mode, not a new embedding model or automatic full-chat injection.

Example MCP arguments using an artificial keyword:

```json
{"query":"beacon","target":"events","detail":"session_context","budget_bytes":8192}
```

Use the returned `next_cursor` with the same query, filters, detail, limit and
byte budget. The CLI supports the same `search --detail session_context` mode.
`limit` limits session groups per page, not the number of original messages.
The output is still one of the existing four read-only MCP tools.

## Select a candidate before expanding

For an ambiguous question, first use the explicit `detail: "session_candidates"`
mode with `target: "events"`, for example:

```json
{"query":"beacon","target":"events","detail":"session_candidates","limit":3,"budget_bytes":8192}
```

This lists one original anchor excerpt per distinct scope/session/official-source
identity before any graph expansion. Excerpts share the byte budget and are at
most 512 UTF-8 bytes each; a giant first response cannot consume the whole page
with its text tails. The default `limit` still applies to candidate groups per
page. `next_cursor` lists further groups, never tails or neighboring messages.
Exact text ranges, role/time, source identity and the original anchor snapshot
remain available. A reused session label is not an identity boundary. Oversized
source labels are explicitly marked truncated; an oversized `session_ref` is
returned as null with a display-only `session_label`, never as a fabricated
exact reference. The original `anchor_ref`/`read_args` remain usable, and the
source-group hash keeps display-label collisions distinct.

Every candidate is marked `relevance: "unassessed"`. `score_kind` describes the
existing ordering signal (cosine, priority rank, RRF or existing ranking score);
none is a calibrated probability that a session contains the answer. This mode
adds no score threshold, gold-trained rule or automatic abstention classifier.
Candidate availability does not establish relevance. If the short excerpts do
not connect to the requested subject, stop deep reading and ask for a candidate
selection or a discriminating detail rather than expanding the first result.

After selecting a candidate, use its `read_args` with the existing `read` tool.
That request preserves the original anchor, Event filters and `as_of`, and
starts at the displayed excerpt's UTF-8 offset rather than a long message's
unrelated prefix. The offset belongs to that Event only: remove it (and any
text cursor) when following a different neighboring reference. Its 8192-byte
budget is per call; callers must still enforce their total budget.
Relevance can remain unresolved even after reading, and an empty filtered or
lexical result does not prove that the user has no relevant history.

`coverage.eligible_event_count` counts currently authorized Events surviving the
explicit filters; `retrieved_event_candidates` counts candidates under the
selected retrieval mode. `indexed_eligible_event_count` is available when the
local vector scan completed. These counts do not include excluded scopes.
The vector candidate cap is still disclosed by `coverage.event_vectors`; the
candidate list is not exhaustive conversation discovery. Missing query vectors
remain explicitly reported lexical fallback. `status: "complete"` only ends
this ranked candidate list, not relevance assessment or full-source reading.
`budget_exhausted` requires a larger same-request budget and a fresh cursor.

## What is selected

- Rank candidates as before, then group by scope, session and official source
  identity (platform, account namespace, conversation). A reused session name
  cannot connect unrelated sources or permissions. A group's rank is its best
  candidate score, not a bonus for having many messages.
- Select at most two matching anchors per group. Walk up to three verified
  graph edges in each direction, with at most twelve messages per group.
- Expand backwards and forwards separately. Walking to a parent never turns
  into a walk down a sibling branch. Multiple unknown forward branches require
  an explicit choice. A recorded current path may select its unique next node.
- Authorization, suppression, source revisions, role, time and path filters all
  apply. Traversal stops at a request-filtered neighbor. Hidden or unauthorized
  references, text and counts are not disclosed, nor bridged across.
- Unknown relationships are not inferred from timestamps, array position or
  message IDs. Exported graph node IDs are distinguished from message IDs.
  Recorded omitted export nodes remain an explicit `source_export_gap`.

This is a deterministic bounded neighborhood, not a semantic topic-boundary
classifier. Distant topic changes inside a large session can remain outside the
window. Independent matching anchors can belong to different branches: each
result retains its own `anchor_ref`, relation and known current-path marker;
session grouping does not assert that they form one linear conversation.

## Byte budget, source fidelity and continuation

The complete returned result JSON must fit `budget_bytes` (512–32768 UTF-8
bytes). It is not a character count, tokenizer count or provider billing limit.
MCP framing, tool definitions and replayed conversation history are additional
overhead. A token-bounded client must count with its actual model tokenizer.

Every selected message receives an initial excerpt before long-message tails
are returned round-robin. Anchor excerpts start at a verified vector hit when available; otherwise they
are centered on actual query text.
Thus a long assistant response does not consume all available bytes before a
short user correction. Small budgets may still require multiple pages.

Each piece carries the original `ref`, `role`, occurrence time, snapshot and
exact UTF-8 `text_range`. Text is a contiguous, unmodified projection: no joining
of separate quotes and no inserted ellipses. Initial excerpts can be followed
by earlier ranges from the same message; order pieces by `start_byte` to
reassemble it. The snapshot matches a normal compact `read` of that event.

`text_truncated` describes one piece. `truncated`/`next_cursor` describe pages
remaining in the selected window. `status: "complete"` and
`status_scope: "selected_context_window"` mean only that those selected pieces
have been returned; they never assert that the entire conversation is complete.
Check each session's `window_limited` and `limit_reasons`, and the global
`coverage.completeness: "bounded_visible_source_graph_only"`.

Limits explicitly include anchor/message/hop caps, request-filter boundaries,
unknown parents, unavailable source graphs, ambiguous identities and branch
choices. An absent neighbor is not proof that the full source had no neighbor.
For evidence outside this window, use `sessions[].continuation_candidates`
and `coverage.continuation` for explicit additional `read` calls. A search cursor
only pages the already selected window; it never grows that window. A read
cursor only pages the currently requested event's text.

The MCP read/sources Event contract now accepts the same `event_filter` and
`as_of`, plus `context_anchor_ref`. Copy the original result's `anchor_ref` into
`context_anchor_ref`, keep it unchanged for every subsequent read, and copy the
search filters. For example, with artificial references:

```json
{"refs":["event:toy-boundary"],"context_anchor_ref":"event:toy-anchor","include_adjacent":true,"detail":"compact","event_filter":{"on_current_path":true},"budget_bytes":8192}
```

Follow `metadata.adjacent.previous_ref` or its unique `next_refs` with a new read
call, keeping those constraints. `constraints.direction_from_anchor` prevents
a backward step from turning into a sibling branch; a forward step cannot
turn backward. The original anchor and every intermediate node must still be
current, authorized, unsuppressed and request-filter-compatible in the same
source/account/scope/session graph. A filtered intermediary is a stop, including
for `next_user_ref`; it is not skipped. Branch ambiguity is determined before
request filtering, so hiding one branch does not make another branch unique.

`constraints.stop_reasons` and `may_continue` distinguish an allowed next step
from a filter, source-gap or branch boundary. An absent link is not proof of a
complete original export. Unknown or parallel branches require a separate
explicit choice; keeping the anchor never silently rebases onto that branch.
These options accept one Event only, not Memory/view/batch reads. Constrained
reads return exact text projections rather than a raw record carrying unfiltered
relationship metadata. Their text cursor binds filters, anchor, scope, current
source graph, detail and byte budget. Unconstrained legacy read behavior remains
available; do not use it as a continuation of a constrained search.

This is an explicit MCP navigation contract; the CLI's legacy `read` arguments
are unchanged. Each graph step and each long-event text page is another tool
call. Clients must cap total calls and cumulative response bytes, retain text
ranges already read, and stop or ask before exceeding their total budget.
A per-call 8192-byte limit is not an 8192-byte limit for the entire workflow.
No tokenizer count is implied, and ranking errors cannot be repaired by merely
reading more of a wrongly selected session.

`budget_exhausted` is distinct from `no_matches`. Increase to at least the
recommended budget and restart without a cursor. Cursors bind the source graph,
ranking, filters, access scopes, mode, limit and byte budget; changed data or
permissions require a new search. Empty text still advances the cursor.

## Important production and evaluation boundaries

Raw-event vectors can now be supplied through an explicitly selected offline
index; see [existing vector cache import](offline-event-vectors.md). The existing
memory semantic index remains separate. Without that event index, raw-event
anchors remain lexical. With it, only queries whose vectors are already cached
(or explicitly imported into a separate query cache) use event-vector ranking.
Missing query vectors produce visible lexical fallback, never an automatic
provider request. A standalone embedding score is not a production MCP result.

DeepSeek and Qwen official imports validate and retain parent/child relationships
and nearest visible parents; Qwen also retains current-path markers. ChatGPT's
all-branches import retains its source graph, while the historical current-only
projection lacks this metadata. Partial/text-only fixtures that discard graph
metadata cannot exercise bidirectional source traversal. Restore relationships
from an authorized original export and re-import/re-export them; do not invent
edges from timestamps or a stored serialization order.

Public tests contain independent artificial conversations only. They check
source boundaries, both directions, giant responses, UTF-8 reconstruction,
real stdio MCP invocation and pagination. Passing them does not establish model
answer quality, blind retrieval accuracy, provider token usage, desktop IPC or
support for a private corpus whose graph metadata is absent.
