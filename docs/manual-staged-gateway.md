# Reproduce the default staged memory workflow

This synthetic-only developer test uses the real Rust MemoryRuntime, PythonMemoryProvider,
Python DreamClient and verified loopback HTTPS. A human reads each actual request and
writes its response; no success response is supplied by the harness. No paid model,
external Agent or real account is needed. It is separate from desktop UI acceptance.

Requirements: Rust, Python 3.10+, OpenSSL, and a working loopback TCP environment.
Run from the repository root:

```sh
cargo build -p recallcard --example manual_staged_runtime
python3 scripts/manual_staged_gateway.py --synthetic-only \
  --binary target/debug/examples/manual_staged_runtime \
  --queue /tmp/recallcard-staged-check/queue \
  --output /tmp/recallcard-staged-check/transport.json
```

Use a new output directory. The first request appears under `queue/requests/`.
Read the complete request before writing a Chat Completions response envelope.
Submit your actual response with:

```sh
python3 scripts/manual_model_gateway.py reply \
  --queue /tmp/recallcard-staged-check/queue \
  --id REQUEST_ID --body /tmp/operator-response.json
```

Repeat for the actual second request. The first stage extracts; the runtime retrieves
candidates and re-exports the final job before consolidation. With no existing Memory,
the final job can retain the same hash and `extract` operation. That does not prove
old-memory consolidation.

The gateway exits after both stages reach review. It does not approve or publish.
Inspect `runtime-evidence/review.json` and its result hash. Only after approving the
synthetic proposal, execute (replace both placeholders with the actual values):

```sh
RECALLCARD_STATE_DIR=/tmp/recallcard-staged-check/state \
  target/debug/examples/manual_staged_runtime approve \
  /tmp/recallcard-staged-check/runtime RESULT_HASH JOB_ID
```

A second invocation with a new output/queue and paired `--runtime-root` and
`--state-dir` reuses that synthetic vault and submits the built-in user correction.
Read the new Extract and Consolidate requests rather than replaying a canned answer.
The continuation fixture expects exactly one existing Memory before its correction.

To test cancellation while a request is waiting, use the harness `revoke ROOT`
with the same RECALLCARD_STATE_DIR. A failed/cancelled job and no publication are the
expected result, not a successful gateway receipt.

The TLS certificate and key are temporary. Only the test child trusts its CA;
certificate verification stays enabled and system trust is unchanged. Do not use
real credentials or personal data. Raw queues contain model protocol messages and
must remain local; publish only reviewed, redacted evidence and content hashes.

## Verified on 2026-10-11

- Empty vault: two actual HTTPS requests; zero Memories before review; explicit
  approval creates one Memory; re-approval is idempotent.
- User correction: real old candidate and its inherited original Event are included
  in the final job. Explicit approval supersedes the old Memory and creates one
  active replacement with both user sources.
- The live directory leaf contains only the replacement and its current description.
  A real MCP process rejects the old revision instead of returning stale content.
- Revoking consent while the first HTTPS request waits cancels the production worker;
  the fresh synthetic vault retains zero Memories.
- Retained initial failures exposed two repaired protocol mismatches: staged review
  must allow auto_apply=false, and consolidation must accept unchanged inherited
  candidate evidence without permitting unrelated or changed source Events.

Production fixes: public commit `f4fad613792f7046cb0f99642f409dd268223629`.
Python regression: 169 passed, one skipped. Rust all-target Clippy and formatting
passed; 14 manual-gateway unit tests passed. The earlier complete desktop/core CI
counts and scope remain documented separately in the native validation report.

## Normal desktop setup and autonomous packaged service

A second verified path uses `manual_service_runtime` and gateway `--service-mode`.
Build the example, then place that test executable beside the actual candidate package
CLI and its Python resources. The example calls the same `DesktopSession` model operation
as Tauri settings, with a synthetic session-only credential. It does not call `tick`.
After the independent service publishes its program identity, the example captures a
new synthetic Event; the service discovers it and sends both actual HTTPS requests.
The operator reads and answers them through the same queue commands above.

The gateway supplies a child-only Python launcher for its temporary CA through the
process PATH. The production CLI and Python modules are unchanged, certificate
verification stays enabled, and no trust or credentials are installed system-wide.
Do not use this harness with personal data or real credentials.

Verified with candidate `66e9b7f`: zero Memories before review; normal stop clears
session credentials and preserves the review; approving while paused is rejected;
normal setup can resume the service, and explicit approval of the same reviewed hash
publishes once. A duplicate approval and later service polls leave one completed job,
one Memory and two reserved model calls. Directory and original-source reads succeed.
The gateway has already closed during cached-result approval, so no fresh model answer
is required. The service is explicitly stopped at the end.

Harness actions are `start`, `resume`, `approve`, and `stop`. Resume requires explicitly
providing the synthetic session credential again. Use the same absolute root and
RECALLCARD_STATE_DIR for all actions. This acceptance covers real shared settings/service
APIs and the shipped CLI, not mouse interaction with the model settings/review widgets.
