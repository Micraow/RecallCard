# Bounded SQLite vector index (experimental v2 primitive)

Status: an opt-in storage/query building block. It is **not wired into the CLI,
MCP, desktop, or default search path**. Existing v1 JSON limits and network
approval rules are unchanged. Storage tests do not establish embedding or
memory-organization quality.

## Trusted boundary

`recallcard_worker.bounded_index.BoundedIndex` never loads a model, makes network
requests, or crawls a Vault. Its database path and space configuration must come
from trusted startup configuration, not a model/web search request.

The caller must retain the canonical Vault read lock while supplying and using
`SourceState` values. Omit hidden, deleted, suppressed and unauthorized sources.
For a Memory, include its current revision in the ref and all required backing
Event hashes. The query checks every backing Event against the current allowed
snapshot. Source role, occurrence time, scope and session identity are preserved.
Navigation hints must already be checked against the real provider branch graph;
never infer a parent/next/correction link merely from timestamp order.

Returned source refs, hashes and UTF-8 ranges require final host validation against
canonical text before reading or sending text to a caller. This module stores no
source text and cannot perform that final byte-boundary check itself.

## Spaces and generations

Sign the complete embedding space configuration, including provider, exact model
revision/artifact hashes, dimensions, numeric precision, pooling/normalization,
query/document prefixes, tokenizer and chunk/preprocessing version. Local and
cloud providers must never share a signature just because dimensions match.

- `begin(sources, expected_chunks)` creates or resumes a deterministic building
  generation. Changed source state or changed space produces a new generation.
- Run model inference **outside** database operations. `append` accepts already
  encoded chunks, at most 64 per transaction.
- Each committed batch has a cursor and content receipt. Replaying identical
  batches is idempotent; conflicting retries fail.
- `publish` verifies completion and atomically switches the published pointer.
  A failed/partial build leaves the previous published generation readable.
- Queries use a database read transaction, so a concurrent publish cannot mix
  pages from different generations.
- Current source/hash/scope filtering still applies to old generations. A new
  source can be absent from the index; changed/hidden/deleted source rows must not
  be used. Coverage reports partial rather than pretending to cover new records.

The first primitive does not automatically reuse embeddings between generations,
remove historical generations, schedule builds, or export canonical source text.
Incremental same-space input-hash reuse and explicitly bounded retention are
integration work. Do not silently delete user history to reclaim index space.

## Bounded ranking and optional diversity

Vectors are normalized little-endian float32 BLOBs. Queries fetch at most 256 rows
at a time, filter authorization and source identity **before decoding vectors**,
and keep a bounded top-k heap. No complete JSON vector matrix is sent over stdio.

The default backend uses the Python standard library. `backend="numpy"` explicitly
selects the optional locally installed NumPy backend; no installation or download
is attempted automatically. It materializes only an authorized page, uses float64
accumulation, and validates finite/unit-normalized vectors. Tests compare its
ranking with the reference implementation.

Diversity caps are optional candidates, not an established product default:
`max_chunks_per_source` and `per_session_limit` default to `None`. A long message
may contain several distant required facts. Capping it at one chunk can lose
those facts; later user corrections can also consume a session quota.

`include_candidate_pool=True` retains up to 100 pre-diversity candidates and marks
pool truncation. `target_refs` permits a scoped, source-specific deeper search.
Returned `previous`, `next`, `corrects` and `branch_choices` links come from the
current canonical snapshot and are filtered again for scope/source visibility.
The eventual transport must still enforce its response-byte budget; the internal
candidate pool is not meant to be copied wholesale into a model prompt.

`kind="memory"` and `kind="event"` permit separate retrieval layers. Their context
allocation and real relevance need workload evaluation; the store does not label
an assistant recommendation as a user decision or resolve conflicting branches.

## Resource bounds

The constructor enforces a disk-page cap (default 512 MiB). Vector size prediction
can reject obviously oversized builds early; SQLite also enforces the page cap
when metadata/row overhead exhaust it. Disk-full tests confirm batch rollback and
preservation of the old published generation.

Other bounds include 64-row writes, 256-row reads, 100 results/raw candidates,
8192 dimensions, a 32 MiB canonical source manifest and 16 links per source. These
are v2-specific bounds, not removal of the older protocol's safety limits.

## Verification

Run the focused synthetic tests:

    PYTHONPATH=python python3 -m unittest discover -s python/tests -p test_bounded_index.py -v

For the optional NumPy path, run that command with a Python environment where
NumPy is installed from the normal official registry. The NumPy test is explicitly
skipped otherwise. All fixtures are synthetic; none contain exported chats.

Covered cases include checkpoint resume/retry conflict, publish rollback,
disk-full rollback, current source changes, Memory backing-source revocation,
space changes, exact full-sort equivalence, filtering before vector decoding,
and the distant-facts/later-correction quota counterexample.
