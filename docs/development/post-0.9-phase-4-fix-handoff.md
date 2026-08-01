# Post-0.9 Refinement — Phase 4 Review Fix Handoff

## Goal

Fix one review mismatch in commit `9067068`: partial-frame subscribe bytes are
added to `total_returned` unconditionally, even when encode+hex fails or sending
the partial notification fails. Returned-byte accounting must count only bytes
successfully emitted to client, matching raw chunk and non-matching frame paths.

## Scope

- `src/tools/stream_ops.rs`: move partial-frame `total_returned` increment into
  successful `notify_logging_message` branch.
- Keep decoder `frames_emitted` semantics unchanged: decoder produced partial
  frame even if transport could not deliver notification.
- Keep current warn/drop accounting on encode failure and peer-send failure.
- Add or strengthen focused test assertions for successful partial fallback:
  stop `bytes_returned` includes exact partial raw length and `truncated` remains
  false when all observed partial bytes were emitted.
- Commit this fix handoff and implementation separately.

## Out of scope

- No other Phase 4 behavior, matcher, schema, docs, or refactor changes.
- Do not alter legacy matching-frame failed-emit quirk documented in code.

## Verification

Run:

```bash
cargo fmt --all -- --check
cargo test --locked --test http_integration subscribe_partial_binary_falls_back_to_hex
cargo test --locked --test http_integration
cargo clippy --all-targets --locked -- -D warnings
git diff --check
```

Inspect status/diff/log. Commit only intended fix files with:

```text
fix: count only emitted partial frame bytes
```

Do not push, amend, merge, open a PR, or add attribution. Return exact files,
tests, commit hash, blockers, and deviations.
