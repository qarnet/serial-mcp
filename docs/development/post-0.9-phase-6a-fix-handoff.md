# Post-0.9 Refinement — Phase 6A Review Fix Handoff

## Goal

Remove one accidental duplicate test introduced by framing split. Pre-split
`src/framing.rs` had 202 `#[test]` functions; split tree has 203 because
`line_ending_default_is_auto` appears in both `config.rs` and `decoder.rs`.

## Change

Keep exactly one copy beside configuration tests in `src/framing/config.rs` and
remove duplicate from `src/framing/decoder.rs`. Do not alter test body, production
code, imports, visibility, docs, or any other test.

Verify:

```bash
rg -c '#\[test\]' src/framing/*.rs src/framing/parsers/mod.rs
cargo fmt --all -- --check
cargo test --locked --lib framing
cargo clippy --all-targets --locked -- -D warnings
git diff --check
```

Total across split framing files must equal 202 and test name must occur once.
Commit handoff + fix with implementation commit:

```text
test: remove duplicate framing split test
```

Do not push, merge, open PR, amend, or add attribution.
