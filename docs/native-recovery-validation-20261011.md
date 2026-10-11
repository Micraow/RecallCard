# Native recovery validation — 2026-10-11

**Passed:** the real v0.7 Tauri/WebKitGTK desktop journey, using only synthetic
records and a temporary project. No external AI client was executed.

- Source: `5b6471115b8a51eefc5e9909f948b754158483b5`
- Tree: `b5ce86ada1e0fec3aac7403aeffc17859bb274cb`
- [Successful run](https://github.com/Micraow/RecallCard/actions/runs/38123144625)
- Environment: Ubuntu 22.04, WebKitGTK 4.1, Xvfb, D-Bus and AT-SPI.
- GUI and CLI report the same source commit, version 0.7.0-dev, and `dirty=false`.

## Real workflow checked

1. A native file chooser imports an official-shaped synthetic archive. The
   canonical vault contains exactly three messages, no fabricated Memory, and
   no automatically granted connection.
2. The original-source page displays the imported messages and their roles.
3. Selecting a temporary project previews configuration without writing it or
   granting access. One explicit approval writes the configuration and performs
   a real RecallCard CLI read. It is labelled as a local read, not host approval.
4. Adding an update produces four canonical Events and invalidates the old
   background snapshot. A managed MCP process reads the update from that vault.
5. Restart preserves configuration and distinguishes a local check from an
   actual client request. Connection controls remain usable at 820px width.
6. Revocation removes the background snapshot and rejects the old connection's
   further reads. The four canonical Events remain intact.

All eight recorded journey checkpoints passed. The final receipt confirms four
Events, zero Memories, and presence of both the original statement and update.

## Other checks in the same run

- 628 Rust tests passed across 54 result groups, including the full workspace.
- Core and Tauri formatting and all-target Clippy passed with warnings denied.
- 53 frontend contracts, 20 desktop JavaScript tests, TypeScript and Vite passed.
- Python: 165 passed, one skipped; 70 native-harness unit tests passed.
- Matching release CLI and desktop builds passed.

## Retained failure and limits

[The first run](https://github.com/Micraow/RecallCard/actions/runs/38120317390)
opened the real desktop but failed because its old v0.6 harness waited for an
excerpt no longer present on the v0.7 connection home. Its screenshots and
failure receipt are retained. The successful run uses the existing complete
v0.7 journey and asserts canonical data as well as UI state; it does not merely
relax the old text match.

This validates a synthetic desktop/core/MCP workflow. It does not verify a live
ChatGPT account, execute Codex or Claude Code, exercise a paid model, or establish
complete recovery of every historical source file. Browser-extension integration
and real model-backed desktop memory generation need separate acceptance.
