# Post-0.9 Refinement — Phase 5 Review Fix Handoff

## Goal

Fix two matcher-review mismatches in `26058d3`:

1. bounded literal context currently saves all requested/available pre-match
   bytes; it must cap pre-match context at
   `min(requested_context, max_buffered_bytes)`;
2. framed subscribe literal context disappeared because frame-local unbounded
   `Matcher::push` does not save shaped context after raw `accumulated` removal.

## Exact changes

### Bounded context cap

In `Matcher::push_bounded`, shape saved literal context with:

```text
effective_context = min(configured_context, max_buffered_bytes)
```

Payload may additionally contain full matched literal. Add unit test where
configured context exceeds small max and assert saved data length/content equals
small max pre-context + match. This prevents requested context from bypassing
connection memory/result bound.

### Frame-local context preservation

Make unbounded `Matcher::push` save literal context on `Found` before returning,
using full configured context bounded naturally by bytes present in current
frame. Generalize saved-context accessor/state names/docs so they are not
bounded-only. Each push clears stale saved context; `reset_window` still clears
window, base, and saved context. Avoid duplicate shaping in `push_bounded`: use
an internal helper or overwrite with bounded-cap shape after `push`.

`rx_consume` must continue using unbounded push/reset; no pattern may span
frames. Final framed subscribe stop notification with context must now contain
context from matching frame and frame-relative match index, not no data and not
bytes from earlier frames.

Add public HTTP or PTY test with at least two frames where match occurs in second
frame. Assert:

- matching frame index is second frame;
- final stop `data` is requested pre-context + literal from second frame only;
- final relative `match_index` equals actual returned pre-context count;
- encoding remains correct;
- no cross-frame bytes appear.

## Scope limits

No policy constants, wire shapes, cursor behavior, regex/glob behavior, module
movement, or unrelated docs changes. Update AGENTS only if wording currently
claims saved context is raw-only/bounded-only.

## Verification

```bash
cargo fmt --all -- --check
cargo test --locked --lib match_config
cargo test --locked --test http_integration
cargo test --locked --test serial_pty
cargo clippy --all-targets --locked -- -D warnings
git diff --check
```

Commit handoff and fix in scoped commit(s), with implementation commit message:

```text
fix: preserve bounded and frame-local match context
```

Do not push, merge, open PR, amend, or add attribution. Return files, tests,
commit hashes, blockers, and deviations.
