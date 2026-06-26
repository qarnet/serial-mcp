# Frame Pipeline Hardening Plan

Phased plan for the PR #27 CI fix, platform test separation, technical debt,
testing gaps, and the near-term SLIP decoder performance item.

## Phase 0 — Immediate CI platform-gate fix

**Goal:** unblock Windows CI with the real platform-scoped fix, not a
dead-code suppression.

**Work:**

- Gate native-sim helpers that are only used by Unix-only tests with
  `#[cfg(unix)]`:
  - `open_with`
  - `write_preset`
  - `extract_trace_bytes`

**Reason:** the callers are all inside `#[cfg(unix)]` tests. On Windows the
callers are not compiled, so ungated helpers become dead code and fail under
`-D warnings`. Gating helpers to the same platform as their callers keeps the
compile graph honest and avoids masking the issue with `#[allow(dead_code)]`.

**Verification:**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
```

## Phase 0b — Split native-sim validation tests by platform

**Goal:** prevent future Unix-only helper/test drift from breaking Windows CI.

**Chosen option:** wrapper + platform modules.

Target layout:

```text
tests/native_sim_validation.rs
tests/native_sim_validation/
  unix.rs
  shared.rs      # only if truly shared test helpers are needed
  windows.rs     # add later only when real Windows-specific tests exist
```

Top-level wrapper:

```rust
#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;
```

If there are no Windows-specific native-sim validation tests yet, omit
`windows.rs` and its module declaration until it is needed.

**Rules:**

- Unix-only helpers live in `tests/native_sim_validation/unix.rs`.
- Cross-platform helpers move to `tests/native_sim_validation/shared.rs` only if
  they are genuinely used by multiple platform modules.
- Top-level `tests/native_sim_validation.rs` stays as module plumbing only; no
  helper implementations there.
- Avoid `#[allow(dead_code)]` as the platform-separation mechanism.

**Reason:** Cargo only auto-discovers top-level `tests/*.rs` integration test
targets. The wrapper keeps one stable test target while physically separating
Unix, Windows, and shared code.

**Verification:**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --test native_sim_validation -- --ignored
```

The ignored native-sim test run still requires the firmware binary as usual.

## Phase 1a — Local framing cleanup

**Goal:** remove low-risk technical debt in `src/framing.rs` before broader
refactors.

**Work from `TECH_DEBT.md`:**

1. Delete dead public method `FrameDecoder::drain_consumed()`.
2. Remove vestigial `offset` pre-reservation in `FrameDecoder::new()`:
   - drop the `offset` binding from `LengthPrefixed::initial_offset`
   - drop the `buf.reserve(skip)` capacity hint
   - initialize `buf` directly with `Vec::new()`
3. Remove `.expect()` from `read_length_prefix` and keep the function
   non-panicking.

**Reason:** these are mechanical, low-risk cleanups that reduce misleading code
and align with the repository convention against production `unwrap`/`expect`.

**Verification:**

```bash
cargo test --lib --locked
cargo clippy --all-targets --locked -- -D warnings
```

## Phase 1b — Shared subsequence search cleanup

**Goal:** remove duplicate byte-subsequence search logic.

**Work from `TECH_DEBT.md`:**

- Replace duplicate implementations:
  - `find_subsequence` in `src/framing.rs`
  - `find_subslice` in `src/tools/helpers.rs`
- Promote one implementation into a shared module.
- Call the shared helper from both sites.
- Keep one focused test set for the shared primitive and remove duplicate tests
  where appropriate.

**Reason:** two byte-for-byte implementations of the same primitive can drift.
This cleanup is small but crosses modules, so it is separated from Phase 1a for
review clarity.

**Verification:**

```bash
cargo test --lib --locked
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
```

## Phase 2 — Precedence resolution hardening

**Goal:** eliminate near-duplicate four-layer precedence ladders and unit-test
the resolution semantics directly.

**Work from `TESTING_GAPS.md`:**

The current precedence rule is:

```text
explicit call field > call-time protocol preset > connection default > connection protocol preset
```

It is hand-written in three areas:

- `tx_framing` resolution in `io_ops::write`
- `rx_framing` / `rx_parser` resolution in `io_ops::read`
- `rx_framing` / `rx_parser` resolution in `stream_ops::subscribe`

Implement shared, testable resolution helper(s), then route all three call sites
through them.

**Required tests:**

- Explicit call field beats call-time protocol preset.
- Call-time protocol preset beats connection default.
- Connection default beats connection protocol preset.
- Mixed gap-fill case: explicit `rx_framing` plus connection protocol default
  still pulls parser from the connection protocol preset when no higher parser
  source exists.
- Write/read/subscribe call sites agree with the shared helper semantics.

**Reason:** this is the highest-value testing gap. Refactoring the ladder into a
shared helper fixes both duplication and coverage at once.

**Verification:**

```bash
cargo test --lib --locked
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
```

## Phase 3 — TX length-prefixed `u16` overflow coverage

**Goal:** cover the remaining TX length-prefixed overflow branch.

**Work from `TESTING_GAPS.md`:**

- Add a unit test for `TxFramingMode::LengthPrefixed` with `prefix_size = 2`.
- Use payload length `65536`.
- Assert the error reports that the payload exceeds maximum `65535`.

**Reason:** `prefix_size = 1` overflow is covered; `prefix_size = 2` is a
distinct branch with its own error path.

**Verification:**

```bash
cargo test --lib tx_length_prefixed --locked
cargo clippy --all-targets --locked -- -D warnings
```

## Phase 4 — SLIP decoder performance: make byte draining `O(n)`

**Goal:** fix the near-term feature/performance issue in `FEATURES.md`.

Current SLIP decode consumes input with `buf_outer.remove(0)` one byte at a
time. `Vec::remove(0)` shifts all remaining bytes, making a large decode path
`O(n²)`. RX chunks can be 4096 bytes, so this can become expensive for large or
cross-chunk SLIP frames.

**Chosen implementation option:** cursor + single drain.

**Work:**

- Replace per-byte `remove(0)` with a local read cursor.
- Read from `buf_outer[read_pos]` while advancing the cursor.
- Preserve the existing `SlipState` behavior, including malformed escape and
  resync semantics.
- Drain consumed input once with `buf_outer.drain(..read_pos)` before returning.

**Options considered:**

1. **Cursor + single drain — chosen.** Smallest change, preserves the current
   state machine, and gets `O(n)` behavior with low regression risk.
2. **Use `VecDeque<u8>`.** Makes front removal cheap, but changes buffer type and
   is awkward because other decoders rely on slice-friendly `Vec` behavior.
3. **Slice-scan between SLIP END markers.** Most consistent with other decoder
   modes, but larger refactor with more edge cases around cross-chunk escapes,
   malformed escape handling, and resync.

**Tests:**

- Existing SLIP unit and integration tests must pass unchanged.
- Add a regression-style unit test for a large SLIP payload if useful, focused on
  correctness and state behavior rather than wall-clock timing.
- Do not add a flaky performance timing assertion.

**Verification:**

```bash
cargo test --lib slip --locked
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
```

## Phase 5 — Deferred peer-disconnect matching-frame coverage

**Goal:** keep the known gap explicit while avoiding a flaky test.

**Deferred item from `TESTING_GAPS.md`:**

`SubscribeFrameSink::on_frame` has a documented legacy quirk: if notification
emit fails for the matching frame, it still records the match path and records a
notification drop. This differs from the non-matching path, which returns
`PeerDisconnected`.

**What the future test must prove:**

- Matching-frame emit failure records the notification drop.
- Matching-frame emit failure preserves the documented "matched despite failed
  emit" behavior.
- Non-matching-frame emit failure still returns `PeerDisconnected`.

**Why deferred:** current harnesses cannot deterministically force a peer
disconnect at the exact notification emit point. A sleep/timing-based test would
be flaky and would not reliably prove the intended behavior.

**Required prerequisite:** a failure-injection notification harness that can fail
on a chosen emit call deterministically.

**Current action:** keep the gap documented in `TESTING_GAPS.md`; do not add a
timing-based test.

## Overall recommended execution order

1. Phase 0 — CI platform-gate fix.
2. Phase 0b — native-sim platform module split.
3. Phase 1a — local framing cleanup.
4. Phase 1b — shared subsequence helper cleanup.
5. Phase 2 — precedence resolution extraction and unit tests.
6. Phase 3 — TX length-prefixed `u16` overflow test.
7. Phase 4 — SLIP decoder `O(n)` fix using cursor + single drain.
8. Phase 5 — keep peer-disconnect matching-frame coverage deferred until a
   deterministic failure-injection harness exists.

## Final verification gate

Run the repository's standard gate after all non-deferred phases:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

If native-sim firmware assets are available, also run:

```bash
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```
