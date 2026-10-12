# Bounded discovery before source reading

`search(target="events", detail="session_discovery")` is an explicit new workflow.
Existing `session_candidates`, `session_context`, ranking policies and ordinary
`read` remain available. Discovery does not call an embedding service, reranker,
or language model.

## Independent channels, not a confidence score

1. Apply the existing current authorization, suppression, source revision,
   role/path/time and `as_of` filters.
2. Keep the lexical ranking independently from the event-vector ranking.
3. Deduplicate each channel by the full source identity
   `(scope, session, platform, account_namespace, conversation_id)`.
4. Interleave distinct source sessions in fixed semantic/lexical rounds,
   skipping already returned identities. The first channel introducing a
   session chooses its preview anchor.

This is deterministic discovery order, not reranked relevance. There are no
question-specific weights, minimum cosine, or confidence thresholds. The
response retains each available channel's source-session rank, raw score and
anchor reference. Cosine and the existing lexical score have different units;
they are not added or compared as probabilities. A session's two channel
anchors can differ; only the explicitly selected primary anchor defines the
preview path. Other anchors are provenance, not implicitly combined evidence.

Only in this new mode, all validated eligible event scores (bounded by the
existing 100,000-chunk index limit) reach source-session grouping before each
independent channel is capped at 256 distinct source sessions. A long session
therefore cannot consume all 256 semantic event slots. Coverage reports the
eligible session counts and each channel's truncation. Other search modes retain
their old 256-event semantic cap. This release does not claim full-corpus output
coverage, a new semantic ranking model, or automatic relevance judgment. Missing query vectors remain a visible lexical-only result,
with no network attempt. Query encoding profile and legacy-cache status are
reported in vector coverage; see [query profiles](query-encoding-profiles.md).

## Context for a caller or reranker

Each candidate contains at most three separate original-text slices:

- the primary hit anchor
- its verified previous message, if permitted
- its verified unique next message, if permitted

Every slice is at most 512 UTF-8 bytes and includes its source reference,
snapshot, role, occurrence time, relation, exact byte range and truncation flag.
An excerpt is never a synthesized conversation or a whole-session summary.
Filtered, hidden, revised, ambiguous, unknown and sibling-branch relationships
are not filled in from timestamps. Graph stop reasons remain explicit.

The entire serialized JSON response obeys `budget_bytes` (512–32768). The
implementation reduces each preview's slice budget before reducing candidate
count, so one long first hit does not consume all peers' space. A lexical
preview is re-centered on the actual original match at every smaller budget;
its offsets and evidence hashes are updated together. A malformed source graph
only makes that candidate's confirmation unavailable, not other sessions.
A budget that cannot carry a usable candidate returns `budget_exhausted`, a
recommended minimum, and no progress cursor. Use its `retry_cursor` together
with `recommended_min_budget` while keeping the other search arguments: this
retry token binds the new budget at the same candidate offset, so a larger
later candidate need not force a restart or repeat earlier sessions. `next_cursor` lists more candidates, not text
tails or additional neighbors. It binds the query, source graph, authorized
scope, filters, encoding configuration, independent channel ranks, mode, limit
and byte budget.

The caller can be the currently used LLM, a local reranker, another explicitly
configured assessor, or a person. A dedicated reranking model is optional and
is not included here. Compare whether the limited evidence concerns the
question's specific subject, speaker and requested answer/correction relation;
allow all candidates to remain uncertain. Do not select by the largest score
or simply because a generic short followup sounds like the question.

## Explicit caller assessment and guarded reading

A discovery item's `read_args` carries `context_confirmation`, containing:

- `binding`: version, query hash, primary anchor and snapshot, full source-group
  hash, authorized-scopes hash, filter/as-of constraints, source-graph generation
  and up to three original evidence spans
- `binding_hash`: a deterministic consistency checksum over that binding
- `assessment`: initially `decision="uncertain"`, no selected anchor and no
  evidence indices

After comparing the bounded evidence, an automatic caller may change only
`assessment` to, for example:

```json
{
  "decision": "relevant",
  "selected_anchor_ref": "<copy candidate anchor_ref>",
  "evidence_indices": [0, 1]
}
```

The selected nonempty indices must refer to the actual returned evidence array.
No natural-language rationale or score substitutes for those bindings. A caller
that cannot establish relevance should retain `uncertain` or use `irrelevant`,
then stop or reformulate rather than deeply traverse the first session.

With this optional field, `read` requires one Event and checks selection,
current source authorization/suppression/revision, original anchor, graph,
scopes, filters, as-of, evidence snapshots, UTF-8 boundaries and text hashes.
It checks the requested Event remains on the original anchor's verifiable
monotonic path. A missing assessment, invalid selection or stale binding returns
`status="uncertain"`, `results=[]`, and `deep_read_performed=false`; malformed
requests still receive ordinary parameter errors.

Success reports `relevance="caller_assessed"` and
`evidence_binding="verified"`. These mean **source consistency was checked**.
They do not prove semantic relevance, that a caller read or understood the
packet, or that a checksum was signed/issued by the server. This field grants
no permission. Keep the same confirmation and original context anchor on every
text page and graph step; the read cursor also binds the confirmation.

Ordinary reads omitting this field retain their old behavior. This is an
opt-in caller workflow, not a global gate that cannot be bypassed by using the
legacy API. An evaluation or integration claiming to use it must actually
follow discovery → explicit assessment → confirmed read, rather than silently
falling back to unconfirmed top1 traversal.

## Evaluation boundaries

The tests use independent synthetic source graphs and 3-D vectors. They verify
channel fairness, exact source/offset preservation, default abstention,
automatic caller plumbing, scope/branch/filter/revision checks, byte budgets,
cursor behavior and real MCP stdio interaction. They do not establish a
reranker's relevance accuracy or an answer-quality improvement.

Count all discovery, assessment-input and subsequent read output bytes and
calls. UTF-8 bytes are not tokenizer tokens; the consuming model's actual
input/history/output and tokenizer are needed for token/cost measurement.
