# Post-0.9 Refinement — Phase 6B Review Fix Handoff

## Goal

Restore `SerialConnection.reconnect_attempts` field privacy after module split.
Current split widened field to `pub(crate)` solely for manager reset.

## Change

- Make `reconnect_attempts` private again in `connection.rs`.
- Add narrow `pub(super) fn reset_reconnect_attempts(&self)` on
  `SerialConnection` that stores zero with existing ordering.
- Replace direct field access in `manager.rs` with method call.
- Remove now-unused atomic `Ordering` import from manager if applicable.
- Add no new test unless existing tests fail; behavior is unchanged and covered
  by reconnect tests.

Verify:

```bash
cargo fmt --all -- --check
cargo test --locked --lib serial
cargo test --locked --test http_integration
cargo clippy --all-targets --locked -- -D warnings
git diff --check
```

Commit handoff + fix with:

```text
refactor: keep reconnect counter encapsulated
```

No push, merge, PR, amend, attribution, or unrelated changes.
