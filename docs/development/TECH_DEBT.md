# Technical Debt — serial-mcp

Tracked cleanups that are safe but not yet done. None of these change tool
behavior or block a release. Each item notes effort and whether it is a good
candidate for the current PR vs. a later subminor.

> Origin: code review of PR #27 (frame pipeline v0.7.0). The SLIP O(n²) item
> lives in [FEATURES.md](FEATURES.md#slip-decoder-performance--drop-on-byte-draining)
> (near-term performance), not here, because it needs a design choice rather
> than a mechanical cleanup.

---

## 1. Dead code: `FrameDecoder::drain_consumed()`

- **Where:** `src/framing.rs` (`FrameDecoder::drain_consumed`).
- **What:** A documented no-op kept "for API compatibility." It has **zero
  callers** anywhere in `src/` or `tests/` — the buffer is drained inside
  `push()` as frames are extracted.
- **Why it's debt:** Dead public method; misleads readers into thinking callers
  must drain manually. `clippy` does not flag it because it is `pub`.
- **Fix:** Delete the method and its doc comment.
- **Effort:** Trivial. **Risk:** None (no callers).
- **Candidate for this PR:** Yes — zero-risk deletion.

## 2. Vestigial offset pre-reservation in `FrameDecoder::new()`

- **Where:** `src/framing.rs`, `FrameDecoder::new` — the `offset` binding from
  `LengthPrefixed::initial_offset` and the `buf.reserve(skip)` it feeds.
- **What:** The actual byte-skipping is handled at decode time by
  `remaining_offset`. The `offset`/`reserve` only pre-allocates capacity for a
  prefix that is *drained away*, never stored — so the reservation is pointless.
- **Why it's debt:** Vestigial code that reads as if it matters for
  correctness; the second return value of the match arm exists solely to feed a
  no-op `reserve`.
- **Fix:** Drop the `offset` plumbing and the `buf.reserve(skip)` block; build
  `buf` as a plain `Vec::new()`.
- **Effort:** Small. **Risk:** None (capacity hint only).
- **Candidate for this PR:** Yes, alongside #1.

## 3. Duplicate subsequence search

- **Where:** `find_subsequence` (`src/framing.rs`) and `find_subslice`
  (`src/tools/helpers.rs`).
- **What:** Two byte-for-byte identical functions: same empty-needle guard,
  same length check, same `windows().position()` body.
- **Why it's debt:** Two implementations of one primitive drift independently;
  each carries its own tests for the same logic.
- **Fix:** Promote one into a shared location (e.g. a small `util` module or an
  existing shared module) and call it from both. Keep one set of unit tests.
- **Effort:** Small. **Risk:** Low (pure function; covered by existing tests on
  both sides).
- **Candidate for this PR:** Optional. Reasonable to fold in, but touches two
  modules — fine to defer to a subminor if keeping this PR focused.

## 4. `.expect()` in `read_length_prefix` violates the no-unwrap convention

- **Where:** `src/framing.rs`, `read_length_prefix` (the `prefix_size == 2` and
  `prefix_size == 4` arms calling `try_into().expect(...)`).
- **What:** AGENTS.md states production code uses no `unwrap`/`expect` (the
  accepted exception is mutex-poison `.expect("poisoned")`). These `try_into`
  expects are provably safe — the caller guarantees `buf.len() >= total_needed`
  before calling — but they are not in the accepted-exception category.
- **Why it's debt:** Convention violation; a future refactor of the caller's
  length check could turn a "can't happen" into a panic.
- **Fix:** Use a non-panicking form, e.g.
  `u16::from_be_bytes(bytes[..2].try_into().unwrap_or([0; 2]))`, or restructure
  to read fixed-size arrays directly. The existing `_ => 0` safe-fallback arm
  shows the intended non-panicking spirit.
- **Effort:** Trivial. **Risk:** None.
- **Candidate for this PR:** Optional; cheap enough to include.

---

### Suggested grouping

- **Fold into this PR (zero risk):** #1, #2 — pure deletions in one file.
- **Either now or next subminor:** #3, #4 — small, low risk, but cross-module
  (#3) / convention-only (#4); fine to batch into a dedicated cleanup commit.
