# Code Cleanup Phase 2 Fix Handoff

Phase 2 implementation and behavior are accepted except one missed comment
requirement.

## Fix

In `src/tools/read_loop.rs`, replace:

```rust
// ── Phase 5: private-cursor extraction ────────────────────────────────
```

with a concise present-behavior heading such as:

```rust
// ── Private/shared cursor behavior ────────────────────────────────────
```

No other source or test changes.

Run:

```bash
cargo fmt --all -- --check
cargo test --lib tools::read_loop --locked
```

Stage `src/tools/read_loop.rs` and this handoff. Create a new commit:

`docs: clarify read cursor test heading`

Do not amend, push, merge, open a PR, force-push, or add attribution. Return
commit and check results.
