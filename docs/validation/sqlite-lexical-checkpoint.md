# Independent Event lexical channel: implementation checkpoint

2026-10-13. Work in progress; this checkpoint has not passed end-to-end native CI.

The Python worker adds a versioned, rebuildable SQLite posting index alongside the
existing vector cache. It independently retrieves up to 100 lexical Events,
combines these with up to 100 semantic Events using rank fusion, and exposes the
original channel ranks and separate source spans. It does not merely rerank the
old semantic top 100. A source can now enter the candidate window solely through
a complete exact identifier. No calibrated relevance threshold is claimed.

Only an explicit index refresh creates the lexical generation. Existing indexes
remain semantic-only until refreshed; cached vectors can be reused. Posting keys
are hashes, which are not encryption. Original text stays in the Vault. Adjacent
original text is bounded to 256 bytes on each side of a chunk, used only to avoid
indexing truncated identifiers, and is not extra model input or stored plaintext.

Initial checks: 14 synthetic real-SQLite tests passed. The Python regression run
passed 213 tests (one existing skip), excluding two existing HTTP-listener tests;
a test audit hook prohibited socket bind/connect. These are fake numeric encoder
contract tests, not evidence of real model recall quality. Rust source-span and
hash revalidation is being integrated. Cross-entry/native integration, coverage
UI wording, and final independent review remain pending in this checkpoint.

Resource limits are explicit: 128-byte terms, 32 distinct query terms, 4,096
terms per chunk, two million postings per generation, 8,192 postings scanned per
query, 1,024 candidate chunks and 100 lexical Events. Candidate or resource limits
must be reported rather than interpreted as no history. The existing approved
encoder configuration and no-cloud-fallback policy are unchanged. Queries still
need the approved local encoder; this stage is not an offline-encoder fallback.

The fixed real-model baseline remains `ed7fc4`, documented separately. No model
was downloaded or rerun for this checkpoint. No real user conversations or
attachments are included in these sources or synthetic tests.
