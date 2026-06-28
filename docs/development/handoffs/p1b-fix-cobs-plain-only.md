# Handoff — Phase 1b fix: Restrict COBS to plain (0x00) only

Follow-up to commit `f85c1ab` (P1b). The shipped COBS implementation has a
**correctness bug** in the configurable-delimiter path (delimiter `0x7E`): the
decoder inserts the delimiter byte as the "phantom zero" instead of `0x00`,
breaking round-trip on payloads containing `0x00` bytes. The plain-COBS
(`0x00`) path is correct and fully tested.

**Decision: ship plain COBS only.** Drop the configurable delimiter, make
`Cobs` a unit variant (like `Slip`), and remove the broken `0x7E` path. The
configurable delimiter was a late scope expansion; the plan/matrix always
specified plain COBS (`0x00`). PPP-COBS (`0x7E`) is a different protocol variant
(two-step zero-elimination + 7E-substitution per the IETF draft) that deserves
its own careful implementation in a later PR. Shipping a broken `0x7E` path
into 1.0 is worse than deferring it.

## Phase goal

Repair P1b by removing the configurable-delimiter surface, leaving a correct,
plain-COBS-only implementation that matches the plan/matrix. No behavior change
for the `0x00` path (already working); the `0x7E` path is removed entirely.

## In-scope changes — `src/framing.rs`

Convert `Cobs` from a struct variant carrying `delimiter: u8` to a **unit
variant** on both `RxFramingMode` and `TxFramingMode`, hardcoding `0x00`.

1. **`RxFramingMode::Cobs`** (currently framing.rs:112-121): replace the struct
   variant
   ```rust
   Cobs {
       /// Frame delimiter byte. Default: 0x00 (plain COBS).
       #[serde(default = "default_cobs_delim")]
       #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
       delimiter: u8,
   },
   ```
   with the unit variant
   ```rust
   /// COBS (Consistent Overhead Byte Stuffing) framing. Byte-stuffed payloads
   /// delimited by 0x00 (plain COBS, Cheshire/Baker). The delimiter never
   /// appears inside an encoded block. The PPP-COBS draft variant (0x7E) is
   /// not supported; it is tracked for a future PR.
   Cobs,
   ```
   Update the doc comment to drop the `0x7E` mention and the "configurable"
   language.

2. **`TxFramingMode::Cobs`** (currently framing.rs:305-313): same conversion —
   unit variant, doc comment updated to drop `0x7E`/configurable language.

3. **`default_cobs_delim`** (framing.rs:150): delete this function entirely.
   No longer referenced once the variants are unit.

4. **`TxFramingMode::Cobs` encode arm** (framing.rs:425-429): replace
   ```rust
   TxFramingMode::Cobs { delimiter } => {
       let mut framed = vec![*delimiter];
       framed.extend_from_slice(&cobs_stuff(payload, *delimiter));
       framed.push(*delimiter);
       Ok(framed)
   }
   ```
   with
   ```rust
   TxFramingMode::Cobs => {
       let mut framed = vec![0x00];
       framed.extend_from_slice(&cobs_stuff(payload));
       framed.push(0x00);
       Ok(framed)
   }
   ```
   Keep the SLIP-parity `[0x00][stuffed][0x00]` frame format (leading + trailing
   0x00). This is consistent with SLIP's `END [stuffed] END` and the existing
   passing tests; do NOT change to the minimal `[stuffed][0x00]` format (that
   would be a separate design decision outside this fix's scope).

5. **`cobs_stuff`** (framing.rs:1601): change signature from
   `fn cobs_stuff(payload: &[u8], delimiter: u8) -> Vec<u8>` to
   `fn cobs_stuff(payload: &[u8]) -> Vec<u8>`. Replace all internal `delimiter`
   references with the literal `0x00`. The encoder eliminates `0x00` bytes from
   the payload (the COBS "zero") and emits code bytes counting runs to the next
   `0x00`. After this change:
   - `if b == delimiter` → `if b == 0x00`
   - `skip_delim_code(code, delimiter)` calls → just `code` (no remapping needed
     for `0x00` — `skip_delim_code` returns `code` unchanged when
     `delimiter == 0x00`, so it is a no-op; remove the calls).
   - The `need_split` branch: for `0x00`, `delimiter == 0x00` is always true, so
     the `code == 0xFF` split condition applies; simplify to the `0x00` branch
     only (drop the `else` arm and `skip_delim_code(code, delimiter) == 0xFF`
     condition).
   - Remove the `0x7E`/`skip_delim_code`-related comments.

6. **`skip_delim_code` / `unskip_delim_code`** (framing.rs:1644-1668): delete
   both functions entirely. They only existed for the non-`0x00` delimiter
   remapping. Grep to confirm no remaining callers before deleting.

7. **`DecoderMode::Cobs`** (framing.rs:551-554): change from
   ```rust
   Cobs {
       state: CobsState,
       delimiter: u8,
   },
   ```
   to
   ```rust
   Cobs {
       state: CobsState,
   }
   ```
   (drop the `delimiter` field — `0x00` is hardcoded everywhere it is used).

8. **`cobs_decode`** (framing.rs ~705+): change to hardcode `0x00`. Change the
   destructure
   ```rust
   let (state, delimiter) = match mode {
       DecoderMode::Cobs { state, delimiter } => (state, *delimiter),
       ...
   ```
   to
   ```rust
   let state = match mode {
       DecoderMode::Cobs { state } => state,
       ...
   ```
   Replace every `delimiter` reference in the function body with the literal
   `0x00`:
   - `buf_outer.iter().position(|&b| b == delimiter)` → `... b == 0x00`
   - `if b == delimiter` → `if b == 0x00`
   - `buf.push(delimiter)` → `buf.push(0x00)` (the phantom zero — this is the
     **core fix**; with `0x00` the phantom was already correct, but the
     hardcoded form makes the invariant explicit and removes the `0x7E` bug
     surface).
   - `if next == delimiter` → `if next == 0x00`
   Remove the `delimiter` local. Update the doc comment (framing.rs ~709) to
   drop "and the configured `delimiter`".

9. **`FrameDecoder::new` Cobs arm** (framing.rs ~950): change
   ```rust
   RxFramingMode::Cobs { delimiter } => DecoderMode::Cobs {
       state: CobsState::BeforeFirstDelim,
       delimiter: *delimiter,
   },
   ```
   to
   ```rust
   RxFramingMode::Cobs => DecoderMode::Cobs {
       state: CobsState::BeforeFirstDelim,
   },
   ```

10. **`flush_partial` COBS branches** (framing.rs ~1042+): update the
    `if let DecoderMode::Cobs { state: ..., delimiter: _ }` patterns to drop the
    `delimiter: _` field (the `DecoderMode::Cobs` variant no longer has it).
    There are 3 COBS branches in `flush_partial` (`InBlock`, `PendingPhantom`);
    update all three patterns.

11. **`ProtocolPreset::Cobs` preset mappings** (framing.rs ~191, ~219):
    `preset_tx_framing` / `preset_rx_framing` currently produce
    `Cobs { delimiter: 0x00 }`. Change to produce the unit `Cobs`:
    ```rust
    ProtocolPreset::Cobs => TxFramingConfig {
        mode: TxFramingMode::Cobs,
    },
    ```
    and
    ```rust
    ProtocolPreset::Cobs => RxFramingConfig {
        mode: RxFramingMode::Cobs,
        max_frames: None,
        include_terminators: false,
    },
    ```
    (the `preset_rx_parser` arm producing `Raw` is unchanged — it does not
    reference `delimiter`).

12. **`ProtocolPreset::Cobs` variant doc comment** (framing.rs ~169): update
    from
    "COBS (Consistent Overhead Byte Stuffing) framing with delimiter 0x00,
    raw payload (no parser)."
    to
    "COBS (Consistent Overhead Byte Stuffing, plain 0x00-delimited) framing,
    raw payload (no parser)."

## In-scope changes — tests in `src/framing.rs`

13. **`cobs_rx_config` helper** (framing.rs ~3383): change signature from
    `fn cobs_rx_config(delimiter: u8) -> RxFramingConfig` to
    `fn cobs_rx_config() -> RxFramingConfig`, hardcoding `RxFramingMode::Cobs`
    (unit). Update ALL call sites in the COBS tests:
    `cobs_rx_config(0x00)` → `cobs_rx_config()`. Remove the `0x7E` call sites
    entirely (the tests using `cobs_rx_config(0x7E)` are deleted — see below).

14. **Delete the `0x7E`-specific tests**:
    - `rx_cobs_custom_delimiter_0x7e` (framing.rs ~3531): delete entirely.
    - Any `0x7E` portions of `rx_cobs_invalid_code_surfaces_framing_error` and
      `rx_cobs_resyncs_after_invalid_code` (framing.rs ~3501-3522): these
      tests have a `0x00` portion AND a `0x7E` portion. Delete the `0x7E`
      portion only; keep the `0x00` portion. Read the full test bodies to see
      the split. The `0x7E` portion constructs `cobs_rx_config(0x7E)` and feeds
      a code byte `0x00` expecting a decode error — with the hardcoded
      `0x00`-only path, that test no longer makes sense (a `0x00` byte IS the
      delimiter, not a "bogus code"). Remove those `0x7E` assertions.

15. **Update all remaining COBS tests** to use the unit `Cobs` variant:
    - `tx_cobs_encodes_block_then_delimiter` (framing.rs ~3402):
      `TxFramingMode::Cobs { delimiter: 0x00 }` → `TxFramingMode::Cobs`.
    - `tx_cobs_stuffs_payload_with_delimiter_byte` (framing.rs ~3418): same.
    - `rx_cobs_skips_to_first_delimiter` (~3427): `cobs_rx_config(0x00)` →
      `cobs_rx_config()`, `TxFramingMode::Cobs { delimiter: 0x00 }` →
      `TxFramingMode::Cobs`.
    - `rx_cobs_decodes_basic_frame` (~3439): same pattern.
    - `rx_cobs_decodes_delimiter_byte_in_payload` (~3450): same.
    - `roundtrip_cobs_arbitrary_binary` (~3462): same.
    - `roundtrip_cobs_empty_payload` (~3475): same.
    - `roundtrip_cobs_max_overhead` (~3488): same.
    - `roundtrip_cobs_255_ones` (~3602): same.
    - `cobs_preset_equivalent_to_bare_cobs_framing` (~3585): the hand-built
      configs must use unit `Cobs` too:
      ```rust
      let bare_tx = TxFramingConfig { mode: TxFramingMode::Cobs };
      let bare_rx = RxFramingConfig {
          mode: RxFramingMode::Cobs,
          max_frames: None,
          include_terminators: false,
      };
      ```
    - `preset_tx_framing_cobs_returns_cobs_0x00` (~3553): the assertion
      `matches!(cfg.mode, TxFramingMode::Cobs { delimiter: 0x00 })` becomes
      `matches!(cfg.mode, TxFramingMode::Cobs)`. Consider renaming the test to
      `preset_tx_framing_cobs_returns_cobs` for accuracy (the `0x00` is now
      implicit). Same for `preset_rx_framing_cobs_returns_cobs_0x00` →
      `preset_rx_framing_cobs_returns_cobs`.

16. **`cobs_stuff_preserves_payload_without_delimiter`** (framing.rs ~3391):
    the call `cobs_stuff(&[0x00, 0x41, 0x00], 0x00)` becomes
    `cobs_stuff(&[0x00, 0x41, 0x00])`. Update the assertion: the encoded block
    must not contain `0x00`.

## In-scope changes — `src/serial.rs` `check_schema!` list

17. **Remove the `RxFramingConfig` / `RxFramingMode` `check_schema!` entries**
    that P1b added (serial.rs, the two lines added in commit `f85c1ab`):
    `check_schema!(rx_framing_config_has_no_uint_formats, RxFramingConfig);`
    and `check_schema!(rx_framing_mode_has_no_uint_formats, RxFramingMode);`.
    Rationale: with `Cobs` now a unit variant, `RxFramingMode` and
    `RxFramingConfig` have **no unsigned integer fields** (the only u8 field
    anywhere in RX framing was `LengthPrefixed.prefix_size`, which already had
    its own guard via the existing TX entries — wait, verify: grep for `u8` /
    `u16` / `u32` / `usize` fields on `RxFramingConfig` / `RxFramingMode` after
    the unit-variant conversion). If ANY unsigned field remains on
    `RxFramingConfig` / `RxFramingMode`, KEEP the corresponding guard. If they
    are truly uint-free after the conversion, removing the guards is correct
    (they would pass trivially but are dead weight). Verify by grepping
    `prefix_size: u8` (RxFramingMode::LengthPrefixed, framing.rs ~82) — that
    field still exists and still has `#[schemars(schema_with = "uint_schema")]`,
    so the `RxFramingConfig` guard IS still meaningful (it catches regressions
    on that field). **Keep both RX guards.** Do NOT remove them. (The
    `RxFramingMode` guard is defensive — `LengthPrefixed.prefix_size` lives on
    the enum; keep it for symmetry with `TxFramingMode`.) Summary: leave the
    `check_schema!` RX entries that P1b added IN PLACE. This is a no-op change
    for this fix — the guards remain. The note here is to prevent the executor
    from removing them thinking the u8 field is gone; `LengthPrefixed`'s u8
    field is unrelated to COBS and stays.

## In-scope changes — `tests/proptest.rs`

18. **`cobs_roundtrip_arbitrary_payload`** (added in commit `f85c1ab`): update
    `TxFramingMode::Cobs { delimiter: delim }` → `TxFramingMode::Cobs` and
    `RxFramingMode::Cobs { delimiter: delim }` → `RxFramingMode::Cobs`. Remove
    the `let delim = 0x00u8;` line and the comment about the `0x7E` variant
    (the comment is now stale). The proptest now covers plain COBS only, which
    is the correct, tested path. Keep the bounded `0..=512` byte strategy.

## In-scope changes — `fuzz/fuzz_targets/codec_roundtrip.rs`

19. **COBS fuzz block** (added in commit `f85c1ab`): the `for &delim in
    &[0x00u8, 0x7E]` loop must drop `0x7E`. Replace the loop with a single
    plain-COBS round-trip:
    ```rust
    // COBS roundtrip (plain COBS, delimiter 0x00)
    use serial_mcp::framing;
    {
        let mode = framing::TxFramingMode::Cobs;
        if let Ok(framed) = mode.encode(data) {
            let cfg = framing::RxFramingConfig {
                mode: framing::RxFramingMode::Cobs,
                ..Default::default()
            };
            if let Ok(mut dec) = framing::FrameDecoder::new(&cfg, None) {
                if let Ok(frames) = dec.push(&framed) {
                    assert!(!frames.is_empty(), "COBS decode produced no frames");
                    let mut reconstructed = Vec::new();
                    for f in &frames {
                        reconstructed.extend_from_slice(&f.data);
                    }
                    assert_eq!(reconstructed, data, "COBS roundtrip mismatch");
                }
            }
        }
    }
    ```
    The `use serial_mcp::framing;` can stay at the top of the block (it was
    inside the loop in the original; hoist it out of the loop since there is no
    loop anymore).

## In-scope changes — `CHANGELOG.md`

20. **Update the COBS subsection** under `[Unreleased]` (added in commit
    `f85c1ab`). Reword to reflect that COBS is plain (`0x00`-delimited) only,
    and that the PPP-COBS draft variant (`0x7E`) is deferred to a future PR.
    Drop any mention of a "configurable delimiter." The Highlights table cell
    can stay as-is (it just says "COBS framing"). Add a sub-note if the
    existing text mentions configurability.

## Out-of-scope changes

- Do NOT change the `[0x00][stuffed][0x00]` frame format to the minimal
  `[stuffed][0x00]`. The leading-`0x00` SLIP-parity format works and is
  tested; changing it is a separate design decision.
- Do NOT re-implement PPP-COBS (`0x7E`) in this PR. It is deferred.
- Do NOT add a `FEATURES.md` entry for the deferred PPP-COBS — that is a
  separate doc decision. (If you want, add a one-line "Deferred" note under the
  existing COBS plan entry, but do NOT create a new FEATURES.md item; the
  plan/matrix already describe plain COBS as the target.)
- Do NOT touch `checksums.rs` — it is correct and unaffected.
- Do NOT touch `ProtocolPreset::Cobs`'s unit-variant status (it was already
  unit in P1b; only the framing-mode variants carried `delimiter`).
- Do NOT touch `precedence.rs` — the `cobs_preset_explicit_framing_wins_over_preset`
  test uses `Some(ProtocolPreset::Cobs)` and `preset_tx_framing(Cobs)`; with
  the unit variant, the explicit `Line { ending: Crlf }` still wins. Verify
  the test still compiles (it should — `ProtocolPreset::Cobs` was already
  unit; only the preset-mapping return values change shape, and the test
  compares against the explicit value, not the preset output shape).
- Do NOT bump `Cargo.toml` version.
- Do NOT touch `config_schema_validation.rs`.
- Do NOT touch `stream_ops.rs` / `stop_controller.rs` / `rx_metadata.rs` — the
  `CobsInvalidCode` error path is unchanged.

## Relevant files and current behavior

- `src/framing.rs` — all changes above. The `Cobs` struct variants become unit;
  `cobs_stuff` / `cobs_decode` hardcode `0x00`; `skip_delim_code` /
  `unskip_delim_code` are deleted; `default_cobs_delim` is deleted;
  `DecoderMode::Cobs` loses its `delimiter` field; tests drop the `0x7E` path.
- `src/serial.rs` — the `check_schema!` RX entries from P1b STAY (they guard
  `LengthPrefixed.prefix_size`, which is unrelated to COBS). No change.
- `tests/proptest.rs` — the COBS proptest drops the `delim` variable.
- `fuzz/fuzz_targets/codec_roundtrip.rs` — the `0x7E` loop iteration is
  removed; the fuzz block becomes plain-COBS-only.
- `CHANGELOG.md` — the COBS subsection is reworded.

## Expected API / UX shape after the fix

```jsonc
// write — COBS-framed payload (plain 0x00)
{ "connection_id": "c1", "data": "hi", "protocol": {"type": "cobs"} }
// write — bare COBS framing (unit variant, no delimiter field)
{ "connection_id": "c1", "data": "hi", "tx_framing": {"type": "cobs"} }

// read — COBS framing, raw frames
{ "connection_id": "c1", "protocol": {"type": "cobs"}, "timeout_ms": 1000 }
// read — bare COBS framing
{ "connection_id": "c1", "rx_framing": {"type": "cobs"}, "timeout_ms": 1000 }
```

The `0x7E` PPP-COBS variant is no longer selectable. A caller who previously
set `delimiter: 126` will get a serde "unknown field" / variant-mismatch error
(because `Cobs` is now unit). This is the intended breaking-fix for an
unreleased, buggy feature.

## Verification that the fix is correct

The orchestrator verified the bug with a stress test (300 zeros, 254 ones +
10 zeros) that FAILED on the `0x7E` path in commit `f85c1ab`. After this fix,
those payloads round-trip correctly on the `0x00` path (they already did
before; the fix removes the broken `0x7E` path that could never have worked).
The executor should add a regression test to lock in the `0x00`-long-zero-run
correctness (see Test plan).

## Test plan

21. **Add a regression test** `roundtrip_cobs_long_zero_run_300` in
    `src/framing.rs` test module (mirror `roundtrip_cobs_max_overhead`):
    ```rust
    #[test]
    fn roundtrip_cobs_long_zero_run_300() {
        // 300 zero bytes: exercises multiple COBS code blocks and the
        // phantom-zero reinsertion. Regression guard for the 0x7E bug class
        // (the bug inserted the delimiter as the phantom; with 0x00 the
        // phantom is 0x00 and this round-trips correctly).
        let payload = vec![0u8; 300];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }
    ```
    This is the test that would have caught the `0x7E` bug if the proptest had
    covered it; it locks in the `0x00` correctness.

22. **Verify all existing COBS tests still pass** after the unit-variant
    conversion: `cargo test --lib cobs`. The test names change slightly
    (`preset_tx_framing_cobs_returns_cobs_0x00` →
    `preset_tx_framing_cobs_returns_cobs`); update the names and assert the
    unit-variant `matches!`.

### Repo gate (must pass before returning)

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Additionally, the executor should run a one-off adversarial check (NOT
committed) to confirm `0x00` round-trips on long zero-runs and all-byte-value
payloads — the `roundtrip_cobs_long_zero_run_300` test covers the former; for
the latter, the existing `roundtrip_cobs_arbitrary_binary` (with `[0x00, 0xFF,
0x41, 0x00, 0x00, 0xFF, 0x7E]`) already exercises `0x7E` as a *payload byte*
(which is fine — `0x7E` in the payload is just a non-zero data byte for the
`0x00`-delimiter encoder; it is NOT a delimiter).

## Constraints and invariants (from AGENTS.md)

- Production code: **no `unwrap`/`expect`, no `println!`, no committed
  `todo!()`/`unimplemented!()`**. Tests may use `.unwrap()` / `.expect("...")`.
- **Schema invariant**: with `Cobs` now a unit variant, `RxFramingMode` and
  `TxFramingMode` still carry `LengthPrefixed.prefix_size: u8` (with
  `uint_schema`), so the `check_schema!` guards for `RxFramingConfig` /
  `RxFramingMode` / `TxFramingConfig` / `TxFramingMode` all remain meaningful.
  Do NOT remove any of the four entries.
- Additive schema change: converting `Cobs` from struct-variant to
  unit-variant is a **breaking** schema change (the `delimiter` field
  disappears). This is acceptable because the `delimiter` field shipped in an
  UNRELEASED commit (`f85c1ab`, under `[Unreleased]` in CHANGELOG) and was
  buggy. Document the breaking fix in CHANGELOG.
- Do not add AI/tool attribution to the commit message.
- Conventional commits: this is a `fix:` (correcting a buggy unreleased
  feature). Suggested message below.
- No `Co-Authored-By` / `Generated with` footers.
- Do not push, merge, open PRs, amend, or force-push.

## Verification commands

Run after implementation, before returning the recap:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# focused runs
cargo test --lib cobs
cargo test --lib checksums
cargo test --lib protocol_preset_cobs
cargo test --lib cobs_preset_explicit_framing_wins_over_preset
cargo test --test proptest cobs_roundtrip
cargo build --manifest-path fuzz/Cargo.toml   # fuzz target compiles
```

Confirm `rg -n "delimiter|0x7E|0x7e|skip_delim_code|unskip_delim_code|default_cobs_delim" src/framing.rs`
returns no COBS-related hits (the `Delimiter` framing mode has its own
unrelated `delimiter` field — those hits are fine; filter them out mentally).
The COBS code should have zero `delimiter` / `0x7E` references after the fix.

## Return to orchestrator

Before returning, commit the scoped work and report:

- Files changed (with the one-line summary of each).
- Behavior changed (COBS restricted to plain 0x00; 0x7E path removed; unit
  variant; regression test added).
- Confirmation that `rg -n "delimiter|0x7E|skip_delim_code|default_cobs_delim"
  src/framing.rs` shows no COBS-related hits (only the unrelated `Delimiter`
  framing mode's `delimiter` field may appear).
- Test commands run and results (paste the gate output tail, or summarise pass
  counts).
- Commit hash and full commit message.
- Blockers, if any.
- Deviations from this handoff, if any, with rationale.

Use conventional commit message, e.g.:

```
fix: restrict cobs to plain 0x00 delimiter, drop broken 0x7E path

The configurable-delimiter COBS path (commit f85c1ab) had a correctness bug:
the decoder inserted the delimiter byte as the "phantom zero" instead of 0x00,
breaking round-trip on 0x00-containing payloads when delimiter was 0x7E. The
plain-COBS (0x00) path was already correct; the 0x7E PPP-COBS variant requires
the draft's two-step zero-elimination + 7E-substitution and is deferred to a
future PR.

- framing.rs: Cobs RX/TX variants become unit (drop delimiter: u8 field);
  cobs_stuff/cobs_decode hardcode 0x00; delete skip_delim_code/unskip_delim_code
  and default_cobs_delim; DecoderMode::Cobs drops delimiter field
- framing.rs: delete rx_cobs_custom_delimiter_0x7e test; drop 0x7E portions of
  the invalid-code/resync tests; add roundtrip_cobs_long_zero_run_300
- tests/proptest.rs: drop the delim variable (plain COBS only)
- fuzz/fuzz_targets/codec_roundtrip.rs: drop the 0x7E loop iteration
- CHANGELOG.md: reword COBS subsection to plain-only, note 0x7E deferred

Breaking fix for an unreleased feature. Schema-breaking (Cobs struct variant
→ unit variant) but the delimiter field never shipped in a release.
```