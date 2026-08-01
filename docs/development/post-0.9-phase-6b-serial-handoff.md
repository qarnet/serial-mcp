# Post-0.9 Refinement — Phase 6B Serial Split Handoff

## Role and delivery constraint

Perform second mechanical Phase 6 split on current branch. No behavior, API,
schema, test-set, or wire change. Commit split before returning. No push, merge,
PR, amend, or attribution.

## Goal

Replace `src/serial.rs` with focused `src/serial/` tree while preserving every
flat `crate::serial::*` / `serial_mcp::serial::*` path and exact lifecycle,
configuration, I/O, profile-binding, line-control, reconnect, status, test
support, and schema behavior.

## Target tree

```text
src/serial/
  mod.rs
  config.rs
  connection.rs
  manager.rs
  port_info.rs
  test_support.rs
```

## Exact ownership

### `config.rs`

Move serial-line and lifecycle data/config declarations:

- `MAX_BAUD_RATE`;
- `DataBits`, `StopBits`, `Parity`, `FlowControl` plus serialport conversions,
  `FromStr`, and to-string helpers;
- `ConnectionConfig` and default functions;
- `ConnectionState`, `ReconnectPolicy`, `is_fatal_disconnect`;
- `ActiveProfileBinding`;
- `FlushTarget`;
- `ConnectionSummary`, `ConnectionStatus`;
- config-only tests.

Import `PortInfo` from sibling module for fields; do not move port discovery.

### `port_info.rs`

Move `PortTransport`, `PortInfo`, `PortProvider`, `SystemPortProvider`, OS
enumeration/conversion and private display/hardware-ID helpers. Keep standalone;
no config dependency.

### `connection.rs`

Move:

- `SerialIo` trait and `SerialStream` implementation;
- `SerialConnection` struct, Debug impl, all methods and test-only state setter;
- stream construction, baud validation, serialport error conversion;
- connection/I/O/close/disconnect-state tests.

Import config and port-info siblings explicitly. Keep private helpers private;
do not widen them merely for tests—place tests in this module.

### `manager.rs`

Move `ConnectionManager`, private registry state, duplicate-port lookup, all
manager methods, and manager-only tests. Depend on config + connection siblings.

### `test_support.rs`

Move existing public test-support module contents unchanged: loopback I/O,
queued-TX I/O/handle/state, factory functions, trait implementations. Preserve
existing `crate::serial::test_support::*` paths and current compilation cfg
(do not newly hide it behind `cfg(test)` because integration tests consume it).

### `mod.rs`

Declare submodules; keep implementation modules private except
`pub mod test_support`. Explicitly flat-re-export all formerly public symbols,
including at minimum:

```text
ActiveProfileBinding, ConnectionConfig, ConnectionManager, ConnectionState,
ConnectionStatus, ConnectionSummary, DataBits, FlowControl, FlushTarget,
Parity, PortInfo, PortProvider, PortTransport, ReconnectPolicy,
SerialConnection, SerialIo, StopBits, SystemPortProvider, MAX_BAUD_RATE,
is_fatal_disconnect
```

Re-export existing `pub(crate)` to-string helpers at `crate::serial::*` because
tool helpers import them there. Do not add new external API.

Keep cross-cutting `serial::schema` regression module in `mod.rs`; update its
header path to `src/serial/mod.rs`. Preserve every `check_schema!` entry and
test exactly once.

## Test placement and exactness

- Config parsing/round-trip/error tests → `config.rs`.
- Baud/private stream and connection I/O/lifecycle tests → `connection.rs`.
- Manager duplicate/close/get tests → `manager.rs`.
- Schema tests → `mod.rs`.
- Preserve every pre-split `#[test]` exactly once. Compare names/count before and
  after; no duplicate or omission is acceptable.

## Path/docs changes

Update active references:

- `AGENTS.md`: lifecycle → `src/serial/`; schema → `src/serial/mod.rs`;
  provider → `src/serial/port_info.rs`; line-control lock →
  `src/serial/connection.rs`.
- `src/tools/mod.rs` schema guard comment → `src/serial/mod.rs`.
- FEATURES combined serial/helpers debt → helper-only split debt; do not remove
  helper work before Phase 6C.
- Do not change historical CHANGELOG references.

## Mechanical constraints

- No algorithm/error/field/default/visibility/public path/serde/schema/comment
  rewrite except imports, module docs, and active path comments.
- Delete `src/serial.rs`; no duplicate source.
- Consumer files and integration imports should remain unchanged via flat
  re-exports unless compiler proves a private internal path must change.
- No unrelated cleanup or formatting churn.

## Verification

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked --lib
cargo test --locked serial::schema
cargo test --locked --test http_integration
cargo test --locked --test serial_pty
cargo test --locked --test tx_session
cargo test --locked --test proptest
cargo test --locked --test allowlist
cargo test --locked --test doc_drift
cargo clippy --all-targets --locked -- -D warnings
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --output-dir target/phase-6b-agent-eval
git diff --check
```

Compare evaluator byte-for-byte to Phase 6A: 27 tools and identical catalog.
Do not commit evaluator output. Compare pre/post serial test names and counts,
inspect status/diff/log, and confirm active `src/serial.rs` references are gone
except historical CHANGELOG text.

## Commit and recap

Commit scoped split and this handoff as:

```text
refactor: split serial connection internals
```

Return ownership map, re-export/visibility decisions, exact test-set comparison,
files, commands/results, evaluator comparison, commit, blockers, deviations, and
Phase 6C follow-up.
