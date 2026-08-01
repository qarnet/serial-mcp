# Post-0.9 Refinement — Phase 6A Framing Split Handoff

## Role and delivery constraint

Perform first mechanical Phase 6 split on existing branch
`refactor/post-0.9-refinement`. No behavior, API, schema, or wire change. Commit
this split before returning. Do not push, merge, open PR, amend, or add
attribution.

## Goal

Replace monolithic `src/framing.rs` with focused `src/framing/` module tree while
preserving every existing `crate::framing::*` and `serial_mcp::framing::*` path,
all tests, parser/framing behavior, schemas, and tool catalog bytes.

## Target tree and exact ownership

```text
src/framing/
  mod.rs
  config.rs
  codecs.rs
  decoder.rs
  parsers/
    mod.rs
```

### `config.rs`

Move configuration/data declarations and preset expansion only:

- `RxFramingConfig`, `RxFramingMode`, `LineEnding`;
- `ProtocolPreset`, `preset_tx_framing`, `preset_rx_framing`,
  `preset_rx_parser`;
- `TxFramingConfig`, `TxFramingMode` enum declaration, `TxLineEnding`,
  `Endianness`;
- `ParserConfig`, `ParserType`;
- related defaults, serde/schemars impls, and configuration/preset tests.

Do not place `TxFramingMode::encode` implementation here.

### `codecs.rs`

Move:

- `impl TxFramingMode::encode`;
- SLIP constants/stuffing;
- COBS stuffing;
- length-prefix/delimiter/blank-frame byte helpers;
- `hex_upper`;
- codec/TX tests.

Use sibling config types. Keep helpers no more visible than required
(`pub(crate)` when decoder needs them).

### `decoder.rs`

Move:

- `Frame`, `ParsedFrame`, `FrameDecodeError`, `PushOutcome`;
- `FrameDecoder` and all internal state enums;
- `FrameParser` trait;
- frame emission, line/delimiter/length/start-end, SLIP, COBS decoding;
- decoder/framing-error/partial/checksum/skip-empty tests.

Import byte primitives from codecs, config types from config, and parser builder
from parsers.

### `parsers/mod.rs`

Move parser factory and implementations:

- `build_parser`;
- AT command, JSON lines, shell prompt, raw, NMEA, and Modbus ASCII parsers;
- parser-only newline/prefix/checksum helpers;
- parser tests.

It may depend on `decoder::{FrameDecodeError, FrameParser, ParsedFrame}` and
config parser types. Rust sibling-module name resolution permits decoder/parser
cross-reference; do not expose new public API to avoid it.

### `mod.rs`

Declare submodules and explicitly re-export all formerly public framing symbols
at flat original path. At minimum preserve:

```text
Endianness, Frame, FrameDecodeError, FrameDecoder, LineEnding,
ParsedFrame, ParserConfig, ParserType, ProtocolPreset, PushOutcome,
RxFramingConfig, RxFramingMode, TxFramingConfig, TxFramingMode,
TxLineEnding, preset_rx_framing, preset_rx_parser, preset_tx_framing
```

Keep internal helpers private or `pub(crate)`; do not turn submodules into new
external API unless Rust visibility requires it. Prefer private `mod` plus flat
re-exports over `pub mod` where external callers do not need nested paths.

## Mechanical constraints

- Move code; do not rewrite algorithms, errors, comments, constants, serde
  shapes, schemas, tests, or assertions except imports/path comments.
- Preserve all test coverage. Distribute tests beside owning implementation;
  add explicit imports where old single-file scope supplied names implicitly.
- Delete `src/framing.rs`; never leave duplicate module sources.
- Update active path references:
  - `tests/doc_drift.rs` must read `src/framing/config.rs` for
    `ProtocolPreset` and update diagnostics/comments;
  - `AGENTS.md`, `docs/development/protocol-matrix.md`, and active native_sim
    test comments must point at new tree/file;
  - remove completed framing-split debt section from FEATURES;
  - do not rewrite historical CHANGELOG references.
- No unrelated formatting or cleanup.

## Verification

Run after movement and before commit:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked --lib
cargo test --locked --test proptest
cargo test --locked --test doc_drift
cargo test --locked --test http_integration
cargo test --locked --test serial_pty
cargo clippy --all-targets --locked -- -D warnings
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --output-dir target/phase-6a-agent-eval
git diff --check
```

Compare evaluator to `target/phase-5-agent-eval`: exactly 27 tools and
byte-identical descriptions/input/output schemas. Do not commit evaluator output.
Inspect all `src/framing.rs` references: only historical CHANGELOG text may
remain.

## Commit and recap

Stage only this split, path-reference updates, debt removal, and this handoff.
Commit:

```text
refactor: split framing internals into focused modules
```

Return moved symbol map, visibility/re-export decisions, files changed, all
commands/results, evaluator comparison, commit hash, blockers, deviations, and
Phase 6B follow-up.
