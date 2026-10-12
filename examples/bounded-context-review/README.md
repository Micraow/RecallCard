# Bounded-span judgment adapter candidate

This is a caller-side candidate, not an enabled production default. It translates
the existing `recallcard.candidate-review/1` response schema into the local
judgment contract, then maps an explicit relevant source selection back to the
unchanged server `context_review` read gate. No new server fields are required
for this tested path. No model, cloud API, or Rust build is performed by this code.

## What is connected

- Discovery template: query/scopes hashes, normalized existing event filter,
  `as_of`, graph radius, finite window, and excerpt limit
- Candidate: full source-group hash, original anchor and snapshot, graph
  generation, anchor offset, existing candidate ID and packet hash
- Evidence: only already-returned text, event reference, snapshot, role/time,
  relation, exact UTF-8 byte range, total size and truncation marker
- Inspection: the actual requested target, unchanged candidate binding,
  bounded-window metadata, navigation distance and stop reasons
- Final read: at most three explicitly cited bounded spans, reindexed into the
  original proof. `answer_coverage=partial` does not erase source relevance.

The adapter uses **no full-original catalog**. It never fetches an event to check
a span. Valid UTF-8, returned byte lengths, overlap consistency and hashes are
checked locally. Only the server can verify that absolute offsets/text actually
match its current originals, that graph edges belong to the permitted branch,
and that scope, suppression, filters and revisions still permit a read. Unsigned
hashes do not establish authenticity. Feed this adapter only the authorized MCP
transport's result text, not a model-generated reconstruction. The transport must
match JSON-RPC response IDs to requests.

No independent branch ID is fabricated: source/anchor/snapshot/graph binding is
retained. The adapter preserves opaque full source identity, not human-readable
display labels. It does not assess relevance or answer correctness. Malformed,
stale, changed-source or radius-invalid inspections are charged then fail closed;
a server uncertainty/refusal stops reuse of the old proof. No automatic retry,
pagination, graph traversal, candidate selection or model invocation is added.

## Budget and use

The inherited fixed limits are 32 KiB initial and 8 KiB later judgment packets,
at most three rounds, two shortlisted candidates, four inspections (two each),
32 workflow tool calls and 262,144 exact result-text bytes per case. Initial and
inspection/final-read calls must share one ledger. Protocol setup and raw wire
bytes remain separately observable; they are not silently represented as model
tokens. Reserve before dispatch; settle exact bytes including failures.

1. Reserve discovery on a `BudgetLedger` before the caller sends `search`.
2. Pass its exact result-text bytes, ticket and expected `Scope` to
   `BoundedSpanWorkflow.from_discovery`.
3. Use the inherited initial packet and explicit judgment methods. No original
   source outside the packet is accessible to a stateless judge.
4. Reserve an explicitly requested inspection with `begin_inspection`. Keep its
   real request/response correlation. Settle using `consume_inspection`, passing
   the target reference copied from that actual request.
5. After a relevant judgment, `selected_read_args` produces the existing server
   request. Reserve and settle that read in the same ledger. The server remains
   authoritative and can refuse the proof.

Nonempty, confirmation-available discovery candidates are currently required;
an empty result or unavailable proof is an explicit unsupported/stop condition,
not a silently omitted candidate. Filters accept only current declared fields;
inactive/default fields are normalized as the existing server does. Timestamps
must already use the server's canonical UTC representation. This small adapter
does not decode JSON-RPC envelopes, choose legal next directions, manage cursors,
or prove a new graph relation from timestamps or message order.

## Verification

Run the standard-library synthetic suite:

    python3 -B -m unittest -v test_judgment_contract test_bounded_span_adapter

An explicit smoke driver can use an existing local binary and an invented vault:

    python3 -B smoke_existing_mcp.py --binary /path/to/recallcard --output new-output-directory

The successful recorded run used the unchanged existing product binary. Search,
bounded inspection and partial-answer source selection passed through real MCP;
the server rejected modified text even after its unsigned proof hash was rebuilt.
The first smoke-driver run omitted the required initialized notification and was
correctly rejected before adapter execution. Its failure is retained separately;
the corrected handshake succeeded in the next run.

The successful smoke run's old `full_original_fetches=0` receipt field means no
extra or unbounded fetches to populate a verification catalog. Its intentional
final deep read is bounded and may return a whole short original within that
budget. Later driver output uses the explicit names `unbounded_original_fetches`
and `extra_reads_for_full_original_verification` to avoid that ambiguity.

No private corpus, query, vector or gold is included. The scripted synthetic
judgments do not measure model selection quality, actual model tokens, latency
improvement, or generalization. The earlier real regression is unchanged.
