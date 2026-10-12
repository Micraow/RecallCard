# Offline event-vector cache reuse

The CLI and direct/managed stdio MCP can explicitly use existing raw-event
vectors. This is separate from the Memory semantic index. No encoder, Python
worker, provider request, credential or background job is started by this path.
The source cache remains read-only during import and search.

This is an exact local scan with bounded input size, not an ANN service. It is
appropriate for validating and reusing a finite exported cache; it does not
claim interactive latency at arbitrary corpus sizes. Desktop/native IPC setup
does not yet configure this index. `mcp --ipc-endpoint` therefore rejects an
event-index override rather than bypassing the server's configuration.

## Import source vectors

First import the original events with the normal authorized importer. Preserve
the export's real parent/node/branch metadata. Text-only fixtures do not acquire
relationships merely by supplying vectors. Do not create graph edges from
timestamps or array positions.

Create an explicit `recallcard.event-vector-manifest/1` JSON file:

```json
{
  "schema": "recallcard.event-vector-manifest/1",
  "space": {
    "schema": "private-vector-cache-v1",
    "endpoint": "https://embedding.invalid/v1/embeddings",
    "model": "example-model",
    "dimensions": 3
  },
  "chunks": [
    {
      "ref": "event:<canonical-event-id>",
      "start_byte": 0,
      "end_byte": 12,
      "text_sha256": "<sha256-of-exact-UTF-8-substring>"
    }
  ]
}
```

These are placeholders, not usable vectors. Each reference must resolve to an
authorized, unsuppressed event. Ranges are half-open UTF-8 byte offsets and must
start/end on character boundaries. The exact source substring is hashed; no
normalization, trimming or quote concatenation occurs. Empty/whitespace-only
chunks are rejected. Duplicate `(ref, start_byte, end_byte)` entries are rejected.

The cache consists of `metadata.json` containing the identical `space` object
and `<text_sha256>.bin` files. `RCVEC001` is:

- 8-byte ASCII magic `RCVEC001`
- 32-byte SHA-256 of the space object encoded with Python
  `json.dumps(space, sort_keys=True)` (including default separator spaces)
- 32-byte SHA-256 of the exact source/query UTF-8 text
- `dimensions` finite little-endian float64 values with L2 norm within `1e-6`
  of one

Endpoint, model, dimensions and schema all participate in the space binding.
The endpoint is an identity string, never contacted. Unknown JSON fields are
rejected. Paths must be absolute, ordinary local files without symlink ancestors.

```sh
recallcard --vault /absolute/private/vault event-vector-import \
  --scope personal \
  --manifest /absolute/private/event-vector-manifest.json \
  --cache-dir /absolute/private/source-cache \
  --output /absolute/private/event-vector-index.json
```

Every source range, hash and unique vector is validated before a private index
is committed atomically. Existing output is never overwritten. Import streams a
separate, regenerable contiguous vector pack alongside the index; the original
cache is read-only. The index is the commit point and binds each packed record's
offset and complete binary SHA-256, the pack size/hash, and chunk-to-event ranges.
A failed validation leaves no published index or temporary pack. Copy/move the
index and its referenced pack together and update explicit paths as needed.

Search opens the pack once rather than opening thousands of source files for
every query. It reads only authorized/filtered records, verifies their complete
binary hash plus model/text header and norm, and retains scalar scores only.
No full vector corpus or cross-request score cache is retained. Source revisions,
suppression, changed index or authorization still take effect on the next call.
Older explicitly imported indexes without a pack remain readable. The
manifest/index is limited to 32 MiB and 100,000 chunks; dimensions are
1–16,384 and total unique vector input is limited to 512 MiB. Source text and
cache identity are checked again at search time. Packed source vectors are a
verified snapshot of the import, independent of later edits to individual source
cache bins; the query vector is still read from the chosen query/source cache.
Changed index metadata requires
a restart; stale source bindings cause visible fallback rather than stale text.

## Search through the actual MCP

```sh
recallcard --vault /absolute/private/vault mcp --scope personal \
  --event-vector-index /absolute/private/event-vector-index.json
```

For a managed connection also supply its existing `--connection-id`. Original
connection authorization, scope intersection, revocation checks and observation
remain in effect. Startup configuration cannot be changed by MCP search args.

Call `search` with `target: "events"` and optionally
`detail: "session_context"`. The same flags are available on CLI `search`.
Each event uses its best matching chunk; repeated chunks do not multiply its
score. Up to 256 event candidates are fused with the existing lexical ranking
using the existing equal-weight reciprocal-rank fusion (`k=60`). This deliberately
preserves the baseline ranking policy: storage optimization does not change it.
RRF discards cosine-score margins, so agreement with a weak lexical match can
outrank a stronger semantic-only match. Do not assume a correct offline vector
rank remains the production top result; evaluate the complete MCP path.
Role/time/current-path filters and current authorization/suppression apply
before vectors are loaded. Visible result sources are revalidated after scoring.

The actual winning source chunk is returned, even if it is deep inside a long
message. Its full `matched_chunk_range` is distinguished from the shorter
returned `text_range`. Bounded session context starts from that hit and follows
the verified graph in both directions. See [context coverage and pagination](bounded-session-context.md); a completed selected window is not a completed
conversation or proof that all relevant evidence has been found.

`coverage.event_vectors` reports `available`, the model/dimensions, eligible
chunk/event counts, candidate cap/truncation and `network_requests: 0`. If a
vector is unavailable or invalid, it reports `unavailable` with an error and
the existing lexical search remains usable. `timing_ms` separates cache checks,
source loading/validation, dot products, vault snapshots, final ranking and graph
window construction; these measurements are not an SLA. This indicator remains present in
compact/no-match responses. It must not be presented as successful semantic
retrieval when it says unavailable.

The result JSON obeys `budget_bytes`, including metadata. This is neither a
tokenizer count nor a whole-MCP-request allowance. MCP framing and surrounding
client context are additional bytes; clients needing a token ceiling must use
their actual model tokenizer. Cursors must be replayed with the same search
mode, filters, access scope, limit and budget.

## Explicit event ranking policies

`--event-vector-ranking hybrid|semantic_only|semantic_first` is fixed by the
operator at CLI/MCP startup, not supplied by an untrusted MCP caller. The default
remains `hybrid` so existing configurations do not silently change ranking.
Both other policies require each search to specify `target: "events"`; they do
not reinterpret the separate Memory/navigation search channels.

- `hybrid`: existing equal-weight lexical/vector RRF with `k=60`.
- `semantic_only`: rank the up-to-256 eligible vector candidates by cosine
  similarity. No lexical-only extras are appended. This is a clear diagnostic
  comparison, not a claim that vector similarity proves relevance or truth.
- `semantic_first`: retain the same vector order, then append lexical-only
  candidates. Lexical overlap cannot add a vote that overtakes a vector hit.
  Returned scores are priority ranks, not cosine values or comparable BM25 units.

For example, start the explicit semantic-primary reading path with:

```sh
recallcard --vault /absolute/private/vault mcp --scope personal \
  --event-vector-index /absolute/private/event-vector-index.json \
  --event-vector-ranking semantic_first
```

An operator can compare `semantic_only` against `hybrid` without re-embedding or
changing the source pack. `coverage.event_vectors.ranking_policy` and
`score_kind` identify the selected contract. If vectors are missing/invalid,
all policies retain visible existing lexical fallback rather than claiming
semantic success; the reported score kind identifies fallback. Cursors bind the
policy even when two policies happen to produce identical rankings.

Rare identifiers may deserve literal priority. A caller must explicitly request
the existing `lexical_match: "identifier_prefix"` override with `semantic_first`
or `hybrid`: exact identifier hits then precede semantic candidates. Strict
`semantic_only` rejects that contradictory override. No query topic, question ID,
gold label or model-specific threshold changes the policy automatically.

None of these policies detects negation, resolves conflicting speakers or
validates an answer. Independent synthetic tests show that semantically similar
opposite instructions can still rank highly: the original negation and role are
preserved, and explicit role/scope filters must be respected. Known-question
regressions can compare policies, but unseen sessions/questions are required for
claims about generalization. Production MCP evidence coverage and generated
answer correctness must be measured separately.

## New queries require new query vectors

An imported source corpus does **not** provide arbitrary-query semantic search.
The exact query UTF-8 hash must already exist in the cache. A cache containing
24 previously evaluated queries supports those exact queries, not all future
questions. Missing vectors produce
`event_query_not_cached_no_network_attempted` and visible lexical fallback.
No provider is contacted to fill the gap.

To supply a query vector obtained separately, save the exact query in a UTF-8
file (including any intentional trailing newline), and a JSON file:

```json
{
  "schema": "recallcard.query-vector/1",
  "space": {
    "schema": "private-vector-cache-v1",
    "endpoint": "https://embedding.invalid/v1/embeddings",
    "model": "example-model",
    "dimensions": 3
  },
  "query_sha256": "<sha256-of-exact-query-file-bytes>",
  "vector": [1.0, 0.0, 0.0]
}
```

Use a separate, already-created private query-cache directory:

```sh
recallcard --vault /absolute/private/vault event-vector-query-import \
  --query-file /absolute/private/query.txt \
  --vector-file /absolute/private/query-vector.json \
  --cache-dir /absolute/private/query-cache
recallcard --vault /absolute/private/vault mcp --scope personal \
  --event-vector-index /absolute/private/event-vector-index.json \
  --event-query-cache /absolute/private/query-cache
```

The import validates the exact query hash, dimensions, norm and space, and
writes only to the explicitly selected destination without overwriting existing
vectors. The optional query cache must match the source index's space. Search
checks it first, then the source cache. This interface permits future provider
integration but does not implement or authorize that integration.

## Verification boundaries

Public integration tests use independent artificial 3-D vectors and toy text.
They exercise CLI import, real stdio MCP, source graph expansion, UTF-8 ranges,
filters/suppression, managed-connection revocation, corrupt/stale bindings,
symlink rejection, query import and explicit missing-query fallback. They do not
measure embedding quality. Evaluating previously known fixture questions must
be labeled known-source evaluation, never blind validation. Keep real chats,
source exports, vector caches, query files and detailed traces outside public
repositories; publish only approved aggregate results and methods.


### Explicit query encoding profiles

Legacy query caches retain their existing query-cache-then-source-cache lookup.
An explicitly selected versioned profile directory instead only reads its own
profile-bound v2 entries and never falls back to legacy vectors. See
[offline query encoding profiles](query-encoding-profiles.md).
For independent bounded lexical/semantic candidate discovery before caller
assessment and source reading, see [session discovery](session-discovery.md).
