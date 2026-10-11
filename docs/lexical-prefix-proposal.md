# Explicit identifier-prefix matching

The default matching contract stays unchanged. Current Latin/code terms require
word boundaries; Chinese lexical queries do not translate English method names.
Prefix matching is not a solution to cross-language semantic retrieval.

The implementation exposes `lexical_match: "identifier_prefix"` as an
explicit CLI/MCP option, with `exact` remaining the default. Boundaries:

1. Expand only ASCII alphabetic identifier prefixes of at least three characters.
   Chinese phrases, numbers and code/path punctuation keep their existing rules.
   No implicit wildcard, substring, edit-distance, translation or special-name list.
2. Require a left identifier boundary. Only the right boundary may be relaxed.
   An explicit `Nov` could match `NovaEngine`, but not `SuperNovaEngine`.
3. Apply authorization, suppression, state, role, session and time filters before
   expansion. Prefix mode must not widen any permission or evidence boundary.
4. Rank exact whole-query matches before prefix-expanded matches. Keep existing
   evidence weighting within a tier and deterministic reference tie-breaking.
   Deduplicate a result that matches both tiers.
5. Bound expansion work and reported candidates. If truncated, report it; never
   silently reinterpret truncation as no match. Include mode in pagination/snapshot
   identity and reject a cursor reused under a different matching mode.
6. Report the active matching mode in coverage. Preserve source offsets and return
   actual matched text so a caller can inspect a candidate before using it.

Regression coverage includes tests for default exact behavior, explicit
prefix opt-in, minimum length, left-boundary rejection, punctuation/numeric and
Chinese behavior, case normalization, exact-first sorting, stable ties, duplicate
removal, pagination/cursor mismatch, bounded expansion, hidden/revoked sources,
role/session/time filters and byte budgets. Use artificial unrelated identifiers.

Retain prior retrieval failures as the original baseline.
Known-answer replays are regressions only. Any further cold-start claim needs new
held-out conversations/questions and the same tool and evidence-depth budget.

CLI: `recallcard search Nova --lexical-match identifier_prefix`. MCP search uses `lexical_match: "identifier_prefix"`. Existing callers omit it and retain `exact`. Expanded lexical candidates are capped at 256, with `prefix_candidates_truncated` in coverage; exact-mode matches are retained ahead of them. This option adds no network calls or embedding configuration.
