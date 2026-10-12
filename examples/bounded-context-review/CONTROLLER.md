# Optional native-judgment controller

This experimental caller adapter is opt-in. It does not enable a product default,
invoke a model, or claim improved real-query quality. The existing server remains
the authority for original text, graph membership, access, filters and revision.

`native_review_controller.py` connects the bounded-span adapter to explicit
native judgment bytes. `review_navigation.py` derives legal finite actions only
from returned spans/edges. `review_journal.py` preserves file handoffs and replays
completed calls locally without dispatching them again.

## Decision contract

A judgment separately records `source_relevance`, `answer_coverage`, independent
reasons for both, exact cited substrings, and an independent abstention reason
when stopping without a selection. A relevant source can have partial, conflicting
or unchecked answer coverage. Selection still requires no unresolved identity or
branch ambiguity. The server's actual `branch_choice_required` stop is treated
conservatively as a blocking ambiguity, even when other visible text is useful.
This is not a calibrated confidence score or a guarantee that the answer exists.

The judge can select one source, shortlist at most two and request listed legal
actions, or abstain. It cannot supply a new reference, offset, direction, branch,
cursor, proof, query, filter or scope. The program maps packet-bound short aliases
back to full identities. Display identity group hashes must match the original
candidate proof before labels are shown. Truncated labels remain marked.

One to three explicitly cited **full original spans**, including needed qualifiers
or contradictions, are carried for each shortlisted candidate. Later packets add
new inspected spans and omit unshortlisted candidates. A fresh judge cannot cite
a hidden or omitted old span. If required query/evidence cannot fit, the workflow
fails explicitly; it never silently summarizes, joins, or trims original text.
The judge is responsible for identifying necessary old support; this mechanism
alone cannot prove that its citation choice preserved all semantic information.

## Fixed bounds and complete cost reporting

- Initial judgment packet: 32,768 UTF-8 bytes; later packets: 8,192 bytes each
- At most 3 rounds, 2 shortlisted/inspected candidates, 4 inspection attempts total,
  and 2 attempts per candidate
- Inspection excerpt: at most 512 original UTF-8 bytes within a fixed 2,048-byte
  window and at most 2 graph hops
- A listed unknown-edge probe can reserve two attempts: read current navigation,
  then follow only its unique permitted neighbor. Both count, including failures.
- One per-case ledger counts setup messages, search, optional unassessed gate
  checks, inspections, final reads and failures: at most 32 protocol/tool attempts
  and 262,144 result-text bytes; request plus wire bytes also cap at 1,048,576
- Native output: at most 8,192 canonical UTF-8 bytes per case/round, 24,576 total;
  input is bounded by the packet limits. Native latency is observed, not predicted
  or hard-capped. Batch runners must additionally bound and report contract,
  wrapper, worker-receipt bytes and phase wall time once, rather than per case.

These units are bytes, not tokens. Exact raw result bytes are settled even when
parsing fails. Thrown transport exceptions with unavailable partial costs are
explicitly marked incomplete; zero is not represented as a measured total.
A shared-process batch assigns setup once to a declared case and includes it in
all-in totals. This is stricter than the older experiment that reported setup
separately from per-case caps; any resulting budget stop must be disclosed.

## Handoff and recovery

1. Construct `FileSession(new_directory, Scope.create(...), local_transport)`.
2. Call `perform('setup', ...)` for the actual protocol messages, then
   `perform('discovery', search_arguments)` for compact discovery.
3. Freeze the returned packet and receive a native judgment externally. Nothing
   in this module launches a judge. Pass exact bytes to `session.native(...)`.
4. The controller validates all plans before dispatch and produces the next
   packet or a terminal selection/abstention. Every native file is bound to the
   exact packet and its visible evidence.
5. After a relevant selection, use `perform('read', None)` for the first bounded
   read, or supply the unchanged authorized reader scheduler's arguments. The
   controller checks selected proof/scope, known references, unique returned
   adjacency, cursor binding and repeated requests. A refusal closes the session.
6. `FileSession.restore(...)` replays completed journal records without any tool
   calls, verifies their hashes and reconstructed results, then permits new work.
   Incomplete intents or failures stop explicitly; no automatic retry or repair.

Journals can contain private query/evidence text. Keep them private. They are
local consistency/recovery records, not signed server authorization. The supplied
transport must bind JSON-RPC IDs, record known partial costs and return exact MCP
result bytes through `Reply`. A changed query, graph, identity or filter needs a
new session rather than editing a journal.

## Verification

    python3 -B -m unittest -v test_judgment_contract test_bounded_span_adapter test_native_review_controller test_review_journal test_target_record_ambiguity
    python3 -B smoke_native_controller.py --binary /path/to/existing/recallcard --output new-synthetic-output

The smoke uses invented local messages and scripted judgments. It exercises the
actual search → native-file decision → bounded inspection → local file restore →
partial-answer relevant selection → server-verified read path, including protocol
setup in the ledger. It makes zero model/provider requests or Rust builds. This
test is an integration check, not evidence of real selection/answer improvement.

### Citation adapter compatibility

`recallcard.native-citation-adapter/2` permits multiple exact quotes from the same
visible `E` span inside the existing maximum of three citation entries. Every
quote is validated against its own candidate's visible span before deduplication.
The unchanged server proof and retained context contain that full span once,
in first-cited order. Original native citations remain intact in the journal.
Packet bytes/IDs, selection, ambiguity gates and all budgets are unchanged.

### Known semantic selection limit

The structural gate cannot prove that a topic-related candidate is the exact
record intended by an underspecified query. A later timestamp in a different
conversation does not establish the final reply of the intended conversation.
`test_target_record_ambiguity.py` uses two invented workshop discussions to show
that an explicit `relevant` judgment can currently be admitted despite an
unproven record qualifier; a declared source ambiguity still blocks selection.
This is a documented limitation, not a semantic fix or a new default policy.

For questions such as “the final reply in that discussion,” distinguish the
requested record/ordering qualifier from answer completeness. Preserve source
identity and valid graph ordering, and explicitly report ambiguity or request
bounded evidence when the intended discussion is not established. Do not infer
same-discussion version order from timestamps across different conversations.
A downstream caller remains responsible for this semantic judgment; the current
controller does not silently equate valid hashes with relevance or a target match.
