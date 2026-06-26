# Phase 4b Follow-up #2 — Fix `ProtocolPreset` JSON shape (tagged object)

Verification of the Phase 4b follow-up e2e tests surfaced a real defect:
`ProtocolPreset` serializes as a bare string `"at_command"`, but the plan,
the Phase 4b handoff, and ALL tests (unit, proptest struct literals,
native_sim e2e) use the object form `{"type": "at_command"}`. Result: every
tool call that sets `protocol` fails deserialization with
`unknown variant \`type\`, expected \`at_command\``, so the preset is
currently UNUSABLE via JSON. The e2e tests fail before any production logic
runs.

This is a production fix + confirmation that the now-runnable e2e tests
pass. Return a concise summary when done.

## Root cause

`ProtocolPreset` is a unit enum with `#[serde(rename_all = "snake_case")]`
but NO `tag`. Unit enums serialize as a bare string. To produce the
canonical `{"type": "at_command"}` shape (matching `rx_framing`/
`tx_framing`/`rx_parser`, which all use `tag = "type"`), add `tag = "type"`.

The executor of the previous follow-up reported "fmt ✓, build ✓, clippy ✓"
but did NOT run `cargo test --test native_sim_validation -- --ignored`
(the handoff's explicit verification command). Those gates do not execute
`#[ignore]`d tests, so the defect shipped unverified.

## Decision (locked)

Canonical shape is the tagged object: `{"type": "at_command"}`. Add
`#[serde(tag = "type")]` to `ProtocolPreset`.

## In scope

### A. Production fix

`src/framing.rs`, the `ProtocolPreset` enum definition:

Current:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolPreset {
    /// AT-command modem protocol. ...
    AtCommand,
}
```

Change to:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProtocolPreset {
    /// AT-command modem protocol. TX appends `\r`, RX splits on line
    /// endings (auto), RX frames are parsed as AT command responses/URCs.
    AtCommand,
}
```

That's the entire production change — one attribute. `preset_tx_framing`,
`preset_rx_framing`, `preset_rx_parser`, and the override wiring in
`io_ops`/`stream_ops` are all correct and need no change.

### B. Verify the existing tests pass with the new shape

After the fix, the three native_sim e2e tests added in the previous
follow-up SHOULD pass (they already send `{"type": "at_command"}`). Run them
and confirm. If any still fails, root-cause — do NOT modify the tests to
mask a deeper bug unless the failure is a separate test-harness issue
unrelated to the preset shape.

### C. Add a serde roundtrip test

Add one unit test in `src/framing.rs` `mod tests`:

```rust
#[test]
fn protocol_preset_tagged_object_roundtrip() {
    let json = serde_json::json!({ "type": "at_command" });
    let p: ProtocolPreset = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(p, ProtocolPreset::AtCommand);
    let back = serde_json::to_value(p).unwrap();
    assert_eq!(back, json, "must round-trip as tagged object");
    // Also confirm a bare string no longer deserializes (guards against
    // accidentally reverting the tag).
    assert!(
        serde_json::from_value::<ProtocolPreset>(serde_json::json!("at_command")).is_err(),
        "bare string form must be rejected after adding tag"
    );
}
```

This locks the canonical shape and guards against future regression.

### D. Optional: ProtocolPreset schema guard

`ProtocolPreset` is a unit enum with no unsigned fields, so the existing
`check_schema!` uint-format scan does not apply. No new schema regression
entry needed. Skip unless you want defense-in-depth.

## Out of scope

- Changing the bare-string-vs-object decision (locked: tagged object).
- Modifying the e2e tests' JSON shape (they already use the correct
  `{"type": "at_command"}` form).
- Any change to the override wiring or expansion functions.
- Presets other than `at_command`.
- Updating the Phase 4b handoff/recap prose (the fix makes reality match the
  handoff's stated shape; no doc edit needed).

## Verification commands

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --lib framing
cargo test --test native_sim_validation -- --ignored
cargo clippy --all-targets --locked -- -D warnings
```

Specifically confirm the three e2e tests pass:
```bash
cargo test --test native_sim_validation -- --ignored \
    native_write_protocol_preset_appends_cr \
    native_write_explicit_tx_framing_overrides_protocol \
    native_read_explicit_rx_framing_overrides_protocol
```

Full gate after:
```bash
cargo test --locked
```

## Return instructions

When done, return:

- Confirm the one-attribute production fix.
- New serde roundtrip test result (pass, and bare-string rejection holds).
- Results of the three e2e tests — do they now pass? If any still fails,
  root-cause and report (don't paper over).
- Gate results (`fmt`, `build`, `lib framing`, `native_sim_validation --ignored`,
  `clippy`, `cargo test --locked` if run).
- Note that this was a real shipped defect caught by the follow-up e2e tests
  (executor skipped `--ignored` verification on the prior follow-up).