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
8. Protocol emulator (`--test protocol_emulator`, `--test protocol_emulator_binary`) — emulated serial device
9. TX session (`--test tx_session`) — write serialisation tests
10. Resource subscriptions (`--test resource_subscriptions`) — MCP resource change notifications
11. Hardware (`--test hardware_loopback`, `--test xiao_ble_validation`) — require physical device + `SERIAL_MCP_TEST_PORT`
12. Planned: `native_sim_validation`, `bootloader_touch_emulated` — see `firmware/UNIFIED_FIRMWARE_PLAN.md`

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

NCS/Zephyr v3.3.0 test firmware. One source tree, two build targets:

| Target | UART | USB CDC-ACM | Output |
|--------|------|-------------|--------|
| **native_sim** | PTY (`/dev/pts/N`) | opt-in via USB/IP | `zephyr.exe` (Linux 32-bit) |
| **xiao_ble** | physical `uart0` via PicoProbe | opt-in via native USB-C | `zephyr.hex` at `0x27000` |

Full architecture, command reference, build commands, and recovery checklist in
`firmware/AGENTS.md`. High-level summary below.

### Critical truths

- Command channel uses `DT_CHOSEN(zephyr_console)` = `&uart0` on both targets.
- xiao_ble: Adafruit UF2 bootloader + SoftDevice s140 at `0x0`, app at `0x27000`.
- xiao_ble: `/dev/ttyACM0` is PicoProbe bridge, NOT native USB.
- native_sim: compiles 32-bit (`-m32`), needs multilib gcc. Blocked on NixOS.
- USB CDC-ACM is **opt-in**: `boards/<board>_usb.conf` + matching overlay.
- USB CDC-ACM enables 1200-baud touch → bootloader entry: native_sim `exit(42)`,
  xiao_ble writes GPREGRET + NVIC reset.

### Build

```bash
# native_sim (no USB — Tier 1)
west build -b native_sim firmware/

# xiao_ble (no USB — Tier 3, requires Nordic ARM toolchain)
nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
  west build -b xiao_ble firmware/ --pristine

# Flash xiao_ble app (at 0x27000)
pyocd flash -t nrf52840 --base-address 0x27000 .../zephyr.hex
```

### Key config files

| File | Role |
|------|------|
| `prj.conf` | Shared: `CONSOLE=n`, `UART_CONSOLE=n`, no USB |
| `boards/native_sim.conf` | `CONSOLE=y` (overrides prj), `UART_NATIVE_PTY_0_ON_OWN_PTY=y` |
| `boards/xiao_ble.conf` | `CONFIG_BUILD_OUTPUT_UF2=y`, `USE_DT_CODE_PARTITION=y` |
| `pm_static.yml` | App at `0x27000` (xiao_ble only; native_sim ignores) |
| `boards/<board>_usb.conf` | Opt-in: USB device-next + CDC-ACM |

### Tests

```bash
SERIAL_MCP_TEST_PORT=/dev/ttyACM0 cargo test --test xiao_ble_validation -- --ignored --test-threads=1
```

`--test-threads=1` is mandatory — parallel tests fight over the same serial port.

native_sim tests (`tests/native_sim_validation.rs`, `tests/bootloader_touch_emulated.rs`)
not yet written. See `firmware/UNIFIED_FIRMWARE_PLAN.md`.

## Fuzz (`fuzz/`)

Targets: `tool_call_json`, `codec_roundtrip`, `clamp_bounds`. Run via `fuzz/run.sh`. Requires nightly toolchain + `cargo-fuzz`.

## Schemas (`schemas/`)

Vendored JSON schemas for agent config formats (Claude Code, Codex, opencode). Used by `config_schema_validation` test. Schema drift checked daily via `.github/workflows/schema-drift.yml`.
