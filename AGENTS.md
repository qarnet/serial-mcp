# AGENTS.md — serial-mcp

## Build / Test / Lint

```bash
cargo test                                            # all non-ignored tests
cargo test --lib <test_name>                          # single unit test
cargo test --test <file_stem> <test_name>             # single integration test
cargo build --all-targets                             # build everything
cargo clippy --all-targets --locked -- -D warnings    # lint (CI-equivalent)
cargo fmt --all -- --check                            # format check

# Schema validation (example configs vs vendored schemas)
cargo test --test config_schema_validation

# Schema drift check (requires network, fetches upstream schemas)
cargo test --locked --test config_schema_validation -- --ignored

# Hardware tests (requires loopback device on serial port)
SERIAL_MCP_TEST_PORT=/dev/ttyACM0 cargo test --test hardware_loopback -- --ignored

# Fuzz (requires nightly + cargo-fuzz; see fuzz/run.sh)
./fuzz/run.sh [seconds_per_target]
```

CI also runs `nix flake check` via the `nix-flake` job.

## Prerequisites

- Rust 1.88+ (pinned in `rust-toolchain.toml`) with clippy, rustfmt, rust-src, rust-analyzer
- `libudev-dev` + `pkg-config` on Linux for `serialport` crate
- Nix + direnv for dev shell (`use flake` in `.envrc`) — optional but recommended
- CI sets `RUSTFLAGS="-D warnings"` — all warnings are errors

## Architecture

```
src/
  main.rs            entrypoint (CLI arg parsing, transport selection)
  lib.rs             re-exports: SerialHandler, Result, SerialError
  server.rs          MCP surface — 12 tools, 3 resources, 2 prompts, pagination
  serial.rs          SerialConnection, ConnectionManager, port config types
  codec.rs           Encoding enum (utf8/text/hex/base64) encode/decode
  error.rs           SerialError enum + Result<T> alias
  security.rs        allowlist matching ( glob patterns)
  limits.rs          MAX_READ_BYTES, MAX_WRITE_BYTES, MAX_TIMEOUT_MS, etc.
  match_config.rs    pattern-matching config for read/subscribe match option
  rx_session.rs      RX session manager (shared RX buffer across read/subscribe)
  rx_metadata.rs     RX metadata tracking
  tx_session.rs      TX session manager
  buffer_budget.rs   buffer reservation for concurrent RX consumers
  stop_controller.rs cooperative cancellation for long-running tools
  schema_helpers.rs  custom schemars overrides (avoid non-standard "uint" format)
  prompts/           diagnose_port, interactive_terminal
  resources/         serial://ports, serial://connections, serial://connections/{id}
  tools/             port_ops (list_ports, list_connections, open, close)
                     io_ops (read, write, flush)
                     control_ops (set_dtr_rts, set_flow_control, send_break)
                     stream_ops (subscribe, unsubscribe)
                     types (arg/response structs), helpers
```

`build.rs` injects `GIT_HASH` / `GIT_HASH_AVAILABLE` env vars at compile time.

### Tool implementation pattern

```rust
#[tool(description = "...")]
async fn tool_name(&self, Parameters(args): Parameters<ToolArgs>) -> Result<Json<ToolResult>, String> {
    // 1. Validate args
    // 2. Lookup connection via self.connections
    // 3. Call SerialConnection / session method
    // 4. Return Json<response>
}
```

Long-running tools (`read`, `subscribe`) mark `execution(task_support = "optional")`.

## Error Handling

Two-tier model:
1. **Operational errors** (bad args, IO, timeout) → `CallToolResult { is_error: Some(true) }`
2. **Protocol errors** (malformed request) → `McpError` (rmcp handles these)

`SerialError` (`src/error.rs`): `OpenFailed`, `PortAlreadyOpen{port,connection_id,name}`, `PortAlreadyOpening`, `ConnectionNotFound`, `ConnectionClosed`, `InvalidBaudRate`, `ReadTimeout`, `IoError`.  
`Result<T>` = `std::result::Result<T, SerialError>`.

## Code Style

- **Imports**: `std::*` → third-party alphabetically → `crate::*`
- **Format**: `cargo fmt`; inline format args (`{var}` not `{}, var}`)
- **Naming**: snake_case (fns/vars), PascalCase (types), SCREAMING_SNAKE (consts), tool names snake_case
- **Types**: concrete over generic; `rmcp::Json<T>` for tool responses; `thiserror::Error` for enums

## Key Conventions

- No `unwrap`/`expect` in production code — use `?` or return errors
- No `println!` — use `tracing` (debug! / info! / error!)
- No `todo!()` / `unimplemented!()` in committed code
- Resource notifications: fire `notify_resource_list_changed()` on open/close
- Allowlist check in `open` tool before `ConnectionManager::open()`
- Tool output schemas must not use non-standard `"format":"uint"` (see `schema_helpers.rs`)
- All 12 tools must have `output_schema` and `title` (verified by `verify_all_tool_schemas` test)

## Test Layers

1. Unit tests (`cargo test --lib`) — schema validation, codec, internal logic
2. HTTP integration (`--test http_integration`) — in-process MCP client via axum + loopback
3. PTY (`--test serial_pty`) — pseudo-terminal loopback
4. stdio integration (`--test stdio_integration`) — child-process stdio transport
5. Allowlist (`--test allowlist`) — security/glob matching
6. Proptest (`--test proptest`) — property-based (codec, match_config)
7. Config schema validation (`--test config_schema_validation`) — vendored + upstream schemas
8. Hardware (`--test hardware_loopback`, `--test xiao_ble_validation`) — require physical device + `SERIAL_MCP_TEST_PORT`

## Git Conventions

- No Co-Authored-By lines in commits
- Conventional commits: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`
- Group related changes per module, not per phase
- Never commit secrets

## CI (`nix flake check` + `.github/workflows/ci.yml`)

1. `cargo fmt --all -- --check`
2. `cargo build --all-targets --locked`
3. `cargo test --locked`
4. `cargo clippy --all-targets --locked -- -D warnings`
5. Config schema validation test

## Firmware (`firmware/`)

NCS/Zephyr test firmware for XIAO BLE nRF52840, used by `tests/xiao_ble_validation.rs`.

### Critical truths

- Transport is **physical `uart0`**, not USB CDC-ACM.
- Host port `/dev/ttyACM0` is the **PicoProbe USB serial bridge** — not XIAO native USB.
- XIAO TX/RX pins talk through PicoProbe at `115200 8N1`.
- Flash with `pyocd` at **`0x0`**. If linker shows `0x27000`, the board `pm_static.yml` won.
- `firmware/pm_static.yml` overrides board default. Do not remove it.

### Do not drift

- Do **not** add USB CDC-ACM (`zephyr_cdc_acm_uart`).
- Do **not** wait for DTR, re-enable `CONFIG_CONSOLE` or `CONFIG_UART_CONSOLE`.
- Do **not** use `west flash` — use `pyocd`.
- Do **not** use `CONFIG_BUILD_OUTPUT_UF2=y` or `CONFIG_USE_DT_CODE_PARTITION=y`.

### Build and flash

```bash
# Build (pristine)
nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
  west build -b xiao_ble /path/to/serial-mcp/firmware --pristine

# Verify link origin is 0x0, then flash
pyocd flash -t nrf52840 ~/ncs/v3.3.0/nrf/build/firmware/zephyr/zephyr.hex
```

### Architecture

```
src/
  main.c        super loop, command dispatch
  uart_drv.c/h  physical uart0 IRQ RX + ringbuf TX
  command.c/h   all commands, spam timer, app state
```

### Command reference

| Command | Response |
|---------|----------|
| `ping` | `pong\r\n` |
| `info` | `board=XIAO_BLE_nRF52840 build=0.1.0 <date> <time>\r\n` |
| `spam <N> hex [delay=<ms>]` | `spam start count=N delay=N\r\n` then hex payload |
| `spam stop` | `Spam stopped: N bytes sent\r\n` |
| `rxbuf status/clear` | inspect or clear partial line buffer |
| `trace on/off` | emit `RX[n]=0xXX` per byte |
| `slow on [<us>]` | sleep before command dispatch |

Tests match on exact string `Spam complete`. Run with `--test-threads=1` (one port, one owner).

### Known pitfalls

- **Silent on `/dev/ttyACM0`** — USB CDC path still active; check `uart_drv.c` binds `uart0`.
- **Linker at `0x27000`** — `firmware/pm_static.yml` missing or not picked up.
- **`>` prompt or echoed commands** — `CONFIG_CONSOLE` or shell got re-enabled.
- **Random hex instead of `pong`** — prior spam flood still draining; `spam stop` clears TX ring.

### Recovery checklist

1. Confirm `prj.conf` has `CONFIG_BUILD_OUTPUT_UF2=n` and `CONFIG_USE_DT_CODE_PARTITION=n`
2. Confirm `firmware/pm_static.yml` sets `address: 0x0`
3. Build with `nrfutil sdk-manager toolchain launch --ncs-version v3.3.0`
4. Verify linker origin is `0x0`
5. Flash with `pyocd flash -t nrf52840 .../zephyr.hex`
6. Test `ping` on `/dev/ttyACM0`
7. Run `cargo test --test xiao_ble_validation -- --ignored --test-threads=1`

Full detail in `firmware/AGENTS.md`.

## Fuzz (`fuzz/`)

Targets: `tool_call_json`, `codec_roundtrip`, `clamp_bounds`. Run via `fuzz/run.sh`. Requires nightly toolchain + `cargo-fuzz`.

## Schemas (`schemas/`)

Vendored JSON schemas for agent config formats (Claude Code, Codex, opencode). Used by `config_schema_validation` test. Schema drift checked daily via `.github/workflows/schema-drift.yml`.
