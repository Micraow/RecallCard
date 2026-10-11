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
For evidence outside this window, use returned original references with an
explicit `read`/adjacent request, applying the desired filters to that read's
result. Such a read is a separate decision, not automatic widening.

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
