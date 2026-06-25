# Phase 1 Recap — RX framing rename + TX framing

**Date:** 2026-06-26
**Source plan:** `docs/development/PHASE1_RX_TX_FRAMING_HANDOFF.md`
**Status:** Complete

---

## Summary

Renamed and flattened the RX framing option on `read` and `subscribe` (from
`framing` / `FramingConfig` to `rx_framing` / `RxFramingConfig` with flattened
`RxFramingMode`) and added a symmetric TX framing option (`tx_framing` /
`TxFramingConfig`) on `write`. Added line-ending controls on both sides
(`auto`/`lf`/`cr`/`crlf` for RX, `lf`/`cr`/`crlf` for TX). All four framing
modes (line, delimiter, length-prefixed, start/end) now carry through RX decode
and TX encode symmetrically.

---

## Files changed

### `src/framing.rs` — core rewrite (largest change)

- Renamed `FramingConfig` → `RxFramingConfig`, `FramingMode` → `RxFramingMode`.
- Flattened the old `mode` wrapper: `RxFramingMode` is now `#[serde(flatten)]`
  inside `RxFramingConfig`, so the JSON shape goes from
  `{"mode":{"type":"line"}}` to `{"type":"line"}`.
- Added `LineEnding` enum (`Auto`, `Lf`, `Cr`, `Crlf`) and `ending` field on
  `RxFramingMode::Line`. Default is `Auto` (preserves original behavior).
- Extracted line matching into four private methods:
  `match_line_auto`, `match_line_lf`, `match_line_cr`, `match_line_crlf`.
- Added `TxFramingConfig` struct and `TxFramingMode` enum with `encode()` method
  that turns a decoded payload into framed bytes. Modes: `Line` (with
  `TxLineEnding::{Lf,Cr,Crlf}`), `Delimiter`, `LengthPrefixed`,
  `StartEnd`.
- Added `TxLineEnding` enum (`Lf`, `Cr`, `Crlf` — no `Auto`).
- Added 24 new unit tests: 6 for new line-ending modes, 14 for TX framing
  encode (per-mode unit + edge cases), 6 round-trip tests, 2 JSON
  deserialization tests.

### `src/tools/types.rs` — request/response shapes

- Renamed `framing` → `rx_framing` on `ReadArgs` and `SubscribeArgs`, updating
  types from `Option<FramingConfig>` to `Option<RxFramingConfig>`.
- Added `tx_framing: Option<TxFramingConfig>` to `WriteArgs`.
- Added `decoded_bytes: usize` (with `uint_schema` annotation) to `WriteResult`.
- Updated doc comments to say `rx_framing` instead of `framing`.

### `src/tools/io_ops.rs` — write path

- Wired TX framing into `write()`: after codec decode, if `tx_framing` is
  present, call `TxFramingMode::encode()` and send the framed bytes instead of
  the raw payload. Clamp both decoded and framed lengths against
  `MAX_WRITE_BYTES`.
- Populate `decoded_bytes` in `WriteResult`.

### `src/tools/helpers.rs` — read path

- Changed `read_bytes_via_session` parameter type from
  `Option<FramingConfig>` to `Option<RxFramingConfig>`.
- Updated all test helpers and test cases constructing framing configs to use
  `RxFramingConfig` / `RxFramingMode` / `LineEnding::Auto`.

### `src/tools/stream_ops.rs` — subscribe path

- Changed `stream_rx_via_session` parameter type from
  `Option<FramingConfig>` to `Option<RxFramingConfig>`.
- Updated `subscribe()` to pass `args.rx_framing`.

### `src/tools/rx_consume.rs` — shared RX consumption

- Updated test imports and `line_decoder()` helper to use `RxFramingConfig` /
  `RxFramingMode::Line { ending: LineEnding::Auto }`.

### `src/server.rs` — tool descriptions

- Updated `write` description to mention `tx_framing` and `decoded_bytes`.
- Updated `read` description to say `rx_framing`.
- Updated `subscribe` description to say `rx_framing`.

### `src/serial.rs` — schema regression list

- Added `check_schema!` entries for `TxFramingConfig` and `TxFramingMode`.

### `src/tools/mod.rs` — schema assertion

- Added `framing_fields_renamed_in_tool_schemas` test asserting that
  write/read/subscribe input schemas contain `rx_framing` / `tx_framing` and
  do NOT contain the old `framing` field.

### Test files

- `tests/proptest.rs`: renamed `FramingConfig` → `RxFramingConfig`,
  `FramingMode` → `RxFramingMode`. Updated `WriteArgs` and `WriteResult`
  constructions for new fields. Renamed `framing_config_roundtrip_all_modes`
  → `rx_framing_config_roundtrip_all_modes`.
- `tests/serial_pty.rs`: updated JSON payloads from
  `"framing": {"mode":{"type":"line"}}` to `"rx_framing":{"type":"line"}`.
- `tests/native_sim_validation.rs`: updated all 10 framing JSON payloads
  to the new flattened `rx_framing` shape, including nested `parser` and
  `max_frames` fields.

---

## Final API shape

### RX framing (read / subscribe)

```json
{
  "connection_id": "abc",
  "rx_framing": {
    "type": "line",
    "ending": "auto",
    "parser": { "type": "at_command" },
    "max_frames": 10,
    "include_terminators": false
  }
}
```

Rust types: `RxFramingConfig` (mode flattened via `#[serde(flatten)]`),
`RxFramingMode` (tagged `"type"`), `LineEnding` (default `Auto`).

### TX framing (write)

```json
{
  "connection_id": "abc",
  "data": "AT+CGMI",
  "encoding": "utf8",
  "tx_framing": {
    "type": "line",
    "ending": "cr"
  }
}
```

Rust types: `TxFramingConfig`, `TxFramingMode` (tagged `"type"`),
`TxLineEnding` (no `Auto`). Write result now includes `decoded_bytes`
(payload length before framing) and `bytes_written` (framed bytes sent).

---

## Tests added / updated

- **New unit tests (25):** 6 line-ending mode tests, 14 TX framing encode
  tests, 6 round-trip tests, 2 deserialize tests, 1 schema assertion test,
  1 proptest for populated `tx_framing` roundtrip. All in `src/framing.rs`,
  `src/tools/mod.rs`, and `tests/proptest.rs`.
- **Updated tests:** All existing framing tests ported to `RxFramingConfig` /
  `RxFramingMode` shape. 2 PTY tests, 10 native_sim tests updated for new
  JSON shape. Proptest framing roundtrip test renamed and ported.
- **Preserved:** All 339 lib tests, 41 HTTP integration, 15 PTY, 53 proptest
  pass unchanged post-rename.

---

## Gate results

| Command | Result |
|---------|--------|
| `cargo fmt --all -- --check` | PASS |
| `cargo build --all-targets --locked` | PASS (no warnings) |
| `cargo test --locked` | PASS (338 lib + all integration suites) |
| `cargo clippy --all-targets --locked -- -D warnings` | PASS |

---

## Scope decisions

- **Serde flatten works cleanly:** `#[serde(flatten)]` on `RxFramingMode` inside
  `RxFramingConfig` produced the expected JSON shape with no schemars
  workarounds needed. The `type` discriminator and variant fields promote to
  the parent level correctly.
- **Line ending default:** Existing `{"type":"line"}` JSON payloads continue
  to deserialize to `ending: Auto` via `#[serde(default)]`, preserving backward
  compatibility within the new field name.
- **TX `auto` rejected:** `TxLineEnding` has no `Auto` variant. Agents must
  specify `lf`, `cr`, or `crlf` explicitly. An `auto` value in JSON would
  fail deserialization with a clear serde error.
- **TX length overflow:** `prefix_size=1` rejects payloads >255 bytes;
  `prefix_size=2` rejects >65535. All errors surface as tool result errors,
  not protocol errors.

---

## Out of scope (deferred to Phase 2+)

- SLIP, COBS framing modes.
- Protocol presets (`at_command`, `json_lines`, `slip_json`).
- Moving `parser` out of `rx_framing` into a separate `rx_parser` field.
- AT-command TX builder, JSON serialization helper for TX.
- Profile defaults for framing.
- Adaptive bare-CR `auto` mode for RX line.
- TX escaping / byte-stuffing in `start_end`.
