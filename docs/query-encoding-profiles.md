# Offline query encoding profiles

RecallCard can bind explicitly imported query vectors to their declared input
construction. This is an offline file contract: no encoding, provider request,
source re-embedding, or global model-space change occurs. The original
`Space`, source `RCVEC001` cache and `recallcard.query-vector/1` remain valid.
Vectors and all examples below are artificial 3-D plumbing examples, not model
quality evidence.

## Version 2 input

Supply the exact UTF-8 query file and this JSON to the existing
`event-vector-query-import --query-file … --vector-file … --cache-dir …` command:

```json
{
  "schema": "recallcard.query-vector/2",
  "space": {
    "schema": "private-vector-cache-v1",
    "endpoint": "https://encoding-toy.invalid/v1/embeddings",
    "model": "original-profile-toy",
    "dimensions": 3
  },
  "query_sha256": "<SHA256 of exact query UTF-8 bytes>",
  "encoded_input_sha256": "<SHA256 of exact constructed input UTF-8 bytes>",
  "encoding_profile": {
    "kind": "instruct_query/1",
    "instruction": "Retrieve original passages relevant to this question."
  },
  "vector": [1.0, 0.0, 0.0]
}
```

The two supported profiles have immutable construction rules:

- `{"kind":"raw/1"}`: encoded input is the exact query string.
- `{"kind":"instruct_query/1","instruction":"…"}`: encoded input is exactly
  `Instruct: ` + instruction + LF (`0x0A`) + `Query: ` + query. The labels use the
  shown ASCII case and one ASCII space after each colon. There is no implicit
  trailing newline, whitespace trimming, Unicode normalization, or escaping.
  Newlines intentionally present in the query are retained. The instruction
  must contain non-whitespace content, have at most 4096 UTF-8 bytes and contain
  no Unicode control characters. Leading/trailing instruction spaces, if
  supplied, are preserved. The entire query file has the existing 4096-byte
  limit and must contain non-whitespace content.

Both hashes, vector dimension/unit norm, and the complete source space
(schema, endpoint, model, dimensions) are checked before import. Search
reconstructs the input from its actual query and checks the same bindings again.
The importer cannot establish that a separately supplied vector was actually
produced from that input or attest to provider-side preprocessing. Coverage
therefore reports
`declared_input_hashes_validated_provider_preprocessing_unverified`.

## One explicit profile per query-cache directory

Create a fresh private directory (or one containing only matching
`metadata.json`). Import writes, without overwriting existing files:

- `metadata.json`: unchanged source `Space` format
- `profile.json`: `recallcard.query-cache-profile/1`, full `space`, and the exact
  `encoding_profile` object
- `<query_sha256>.query-v2.json`: the complete validated v2 entry

A retained zero-byte `.query-import.lock` serializes cooperating imports across
profile, metadata and entry validation/publication. Its guard explicitly unlocks
on drop, including when a concurrent process spawn temporarily inherited a file
handle. A concurrent writer receives
`event_query_import_busy` rather than an automatic retry. The declaration and
space are rechecked before entry publication. Each file is published atomically
with no-clobber and fsync. The profile is
published before any entry, and a complete entry is a single atomic JSON file;
there is no separate vector/sidecar pair that could be mismatched by a partial
write. An interrupted initial import may leave a declared but empty cache,
which can accept a later import of that same profile. A profile cannot be
silently replaced. Existing legacy `.bin` directories cannot be relabeled by
the importer. A v2 entry without its declaration is rejected, including after
restarting the reader. Invalid declarations and changed declarations fail
closed; changing a selected declaration requires restarting the reader, and
entries must still match it.

Choose the directory explicitly with `--event-query-cache`. A selected
profiled cache only reads v2 entries in that directory. Missing queries never
fall back to the source cache or any legacy `.bin` in the selected directory.
Any such legacy `.bin` files are isolated and ignored. Use a different
query-cache directory to keep a second profile (including a different
instruction) for the same query. Source index and cache files are never
rewritten by these operations unless the operator explicitly chooses them as
an import destination; even then, importing v2 refuses a populated legacy
cache. Do not select existing source caches as query import destinations.

## Legacy compatibility and coverage

Version 1 imports retain the same `.bin` bytes and do not create a profile.
An explicitly selected cache without a profile retains the original behavior:
try its legacy query `.bin`, then try the original source cache. Without an
explicit query-cache selection, queries also use the original source cache.
These results are identified as
`legacy_raw_provider_preprocessing_unverified`, never as verified raw or
instructed encoding. A version 1 import into a profiled directory is rejected.

Successful search coverage includes `query_encoding_profile`,
`query_encoding_profile_sha256`, `query_input_binding`, `query_sha256`,
`encoded_input_sha256`, and `query_cache_selection`. Legacy encoded-input and
profile hashes are null because they were never established. The profile hash
covers its version, complete source space and construction profile. Query-cache
selection and profile also bind search cursors even if the ranking is identical.
The profile is an operator-selected cache property, never a per-query request to
change the global space or contact a model service.
