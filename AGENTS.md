# AGENTS.md — serial-mcp

## Fast truth

- Root server: `src/main.rs` selects stdio vs HTTP transport, parses CLI limits, and mounts HTTP at `/mcp`.
- MCP surface lives in `src/server.rs`; tool handlers are split under `src/tools/`, prompts under `src/prompts/`, resources under `src/resources/`.
- Connection lifecycle is in `src/serial.rs`; shared RX/TX coordination is in `src/rx_session.rs`, `src/tx_session.rs`, and `src/stop_controller.rs`.
- `build.rs` injects `GIT_HASH` / `GIT_HASH_AVAILABLE`.

## Commands worth using

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# focused runs
cargo test --lib <test_name>
cargo test --test <file_stem> <test_name>
cargo test --test serial_pty
cargo test --test http_integration
cargo test --test stdio_integration
cargo test --test config_schema_validation

# networked schema drift check
cargo test --locked --test config_schema_validation -- --ignored

# native_sim tests (needs firmware built first — see Firmware section)
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1
```

- CI runs exactly: fmt -> build -> test -> clippy, plus `cargo test --locked --test config_schema_validation` on Ubuntu.
- CI and schema workflows set `RUSTFLAGS="-D warnings"`. Treat warnings as errors locally too.
- `nix flake check` is part of CI. On Nix, prefer `nix develop` before changing firmware or release workflow bits.

## Invariants easy to break

- Tool failures should usually become MCP tool results with `is_error: Some(true)`, not protocol-level `McpError`. Keep malformed-request errors separate from operational errors.
- All tool outputs need `output_schema` and `title`; `verify_all_tool_schemas` enforces this.
- Do not emit non-standard schema `"format": "uint"`; use helpers in `src/schema_helpers.rs`.
- `open` must enforce allowlist checks before `ConnectionManager::open()`.
- Open/close changes must notify resource subscribers via `notify_resource_list_changed()`.
- Production code convention here: no `unwrap`/`expect`, no `println!`, no committed `todo!()` / `unimplemented!()`.

## Test map

- `cargo test --lib` covers core logic.
- `tests/http_integration.rs` exercises real MCP HTTP transport in-process.
- `tests/serial_pty.rs` is real PTY serial I/O on Unix.
- `tests/stdio_integration.rs` spawns binary over stdin/stdout.
- `tests/protocol_emulator*.rs` are protocol hardening tests.
- `tests/config_schema_validation.rs` validates generated schemas against vendored examples; ignored case fetches upstream schemas.
- `tests/native_sim_validation.rs` — Tier 1: native_sim firmware over PTY. 11 tests, < 2s, pure software. Env: `SERIAL_MCP_NATIVE_SIM_BIN` (default `build/firmware/zephyr/zephyr.exe`).
- `tests/native_sim_connection_lifecycle.rs` — software-only lifecycle: named connection, `set_flow_control`, close-while-read, reopen. Run with `--test-threads=1`.
- `tests/bootloader_touch_emulated.rs` — Tier 2: 1200-baud USB/IP touch → exit(42). Needs `vhci_hcd` + NOPASSWD sudoers (or udev rule). Env: `SERIAL_MCP_NATIVE_SIM_USB_BIN`, `USBIP_NATIVE_SIM_ATTACH_CMD` / `USBIP_NATIVE_SIM_DETACH_CMD`. Must use `--test-threads=1`.
- There are no hardware-required tests in this repo. All test coverage is runnable on a normal Linux host.

## Firmware / NCS

- Read `firmware/AGENTS.md` before touching Zephyr code; root file only keeps top-level gotchas.
- `nix develop` now auto-loads Nordic toolchain env via `nrfutil sdk-manager toolchain env --ncs-version v3.3.0 --as-script sh`, sets `ZEPHYR_BASE`, and exposes firmware helpers on `PATH`.
- Use helpers instead of retyping wrappers:

```bash
fw-build-native
fw-build-native-usb
fw-run-native
fw-run-native-usb-attached
```

- `native_sim` is a 32-bit host build (`-m32`). Repo flake now supplies multilib GCC; do not reintroduce "NixOS unsupported" guidance.
- The XIAO BLE nRF52840 target was removed. The test firmware now targets `native_sim` only.
- Do not switch firmware command channel away from `DT_CHOSEN(zephyr_console)`.
- native_sim tests need firmware built first: `fw-build-native` for Tier 1, `fw-build-native-usb` for Tier 2. Tier 2 also needs `vhci_hcd` kernel module + NOPASSWD sudoers for `usbip-native-sim-attach` (or a vhci_hcd udev rule).

## Release workflow

- Release job derives tag from `Cargo.toml` version (`v<version>`), tags `main` automatically after CI success, uploads binaries, then publishes crate. Bumping package version has release consequences.
- Release artifacts are built for: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.

## Repo workflow

- Conventional commits used here: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`.
- Never add attribution footers or co-author lines.
