# Private local recall trials

The manual gateway supports two explicit, mutually exclusive data modes:
`--synthetic-only` and `--private-local-only`. The latter is for specifically
user-authorized, sanitized private material. It is a local file queue, not a
network provider. It rejects other destinations; a queue cannot silently switch
modes. Raw requests, replies, corpus files, source identifiers and fingerprints
must stay outside public source and evidence directories.

The model operator reads each actual request and supplies a response. The
production DreamClient validates extraction/consolidation results. Production
Rust review and explicit approval control writes; actual read-only MCP performs
recall. A manually authored successful response is never substituted for a
real tool return.

A bounded recall run can specify `--max-rounds 6 --max-tool-calls 4`. Excess tool
calls are rejected before execution, including a batch that would exceed the
remaining budget. A budget stop is incomplete retrieval, not evidence that the
library contains no matching material.

## Evaluation boundaries

- Separate a known-source end-to-end workflow check from a cold-start retrieval
  check. An operator who participated in extraction is not blind to those sources.
- Fix selection rules and hold the answer table away from the responding operator.
  Run unrelated questions in independent MCP/model contexts. Score only after
  final answers and retain both failures and appropriate abstentions.
- Keep partial exports, missing images, empty fragments and unknown branch
  relationships explicit. A later timestamp alone does not establish a branch.
- Track organized coverage separately from imported original conversations.
  An absent directory entry does not imply absent original dialogue.
- Preserve actual failures. Do not turn a known-answer replay into a new blind
  success or replace a deep evidence question with a shallower one to claim savings.

## Cost measurement

Measure exact serialized UTF-8 request bytes, tool-return bytes, tool counts,
new message bytes, replayed history bytes and repeated schema bytes separately.
The optional bytes/4 figure is an uncalibrated proxy, not tokenizer or billed
usage. Stable component hashes identify reusable content, not actual provider
cache hits. JSON minification cannot save more if the payload is already compact.

## Publication boundary

Only the generic harness, artificial unit tests and this method description are
public deliverables. Private dialogue, excerpts, names, identifiers, corpus/source
fingerprints, runtime settings, keys and local private paths are excluded. This
page contains no user conversation examples or individual test answer keys.
