# AGENTS.md — Coding Guidelines for serial-mcp-server

## Build / Test / Lint Commands

```bash
# Full test suite (all 6 layers, ~62 tests)
cargo test

# Run a single test
cargo test --test http_integration list_tools_returns_all_eleven_tools
cargo test --test serial_pty pty_wait_for_matches_real_serial_pattern

# Hardware tests (requires SERIAL_MCP_TEST_PORT env var)
SERIAL_MCP_TEST_PORT=/dev/ttyACM0 cargo test --test hardware_loopback -- --ignored --test-threads=1

# Lint (must pass zero warnings)
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check

# Build all targets including tests
cargo build --all-targets
```

## Code Style

### Imports
Order: `std::*` first, then third-party crates alphabetically, then `crate::*`:
```rust
use std::collections::HashMap;
use std::sync::Arc;

use glob::Pattern;
use rmcp::model::*;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::codec::{self, Encoding};
use crate::error::SerialError;
```

### Formatting
- `cargo fmt` enforced in CI (no trailing commas on multi-line calls, standard Rust style)
- Use `format!("var: {var}")` not `format!("var: {}", var)` (inlined format args)
- Lines wrap naturally at ~100 chars

### Naming
- `snake_case`: functions, variables, fields, modules
- `PascalCase`: types, traits, structs, enums
- `SCREAMING_SNAKE_CASE`: constants, statics
- Tool names: `snake_case` (e.g., `list_ports`, `wait_for`)
- Test names: descriptive `snake_case` (e.g., `pty_wait_for_matches_real_serial_pattern`)

### Types
- Prefer concrete types over generics where possible
- Use `rmcp::Json<T>` for tool responses (not raw strings)
- Use `thiserror::Error` for error enums
- `Result<T>` is the crate alias for `std::result::Result<T, SerialError>`

## Error Handling

**Two-tier model:**
1. **Operational errors** (bad args, IO failure, timeout) → `CallToolResult { is_error: Some(true) }`
2. **Protocol errors** (malformed request) → `McpError` (rmcp handles these)

**SerialError** (from `src/error.rs`):
```rust
pub enum SerialError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Read timeout")]
    ReadTimeout,
    #[error("Invalid baud rate: {0}")]
    InvalidBaudRate(u32),
    #[error("Port {0} is already open")]
    PortAlreadyOpen(String),
    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}
```

**Tool error helper:**
```rust
fn log_tool_err<E: std::fmt::Display>(op: &str, context: &str, err: E) -> String {
    let msg = format!("{op} failed — {context}: {err}");
    error!("{msg}");
    msg
}
```

## Architecture

- **Handler** (`src/handler.rs`): MCP surface — tools, resources, prompts
- **Serial** (`src/serial.rs`): Data plane — `SerialConnection`, `ConnectionManager`, `SerialIo` trait
- **Codec** (`src/codec.rs`): `Encoding` enum (utf8/hex/base64) with encode/decode
- **Error** (`src/error.rs`): Single `SerialError` enum

## Tool Implementation Pattern

```rust
#[tool(description = "...")]
async fn tool_name(
    &self,
    Parameters(args): Parameters<ToolArgs>,
    ctx: RequestContext<RoleServer>,  // if peer access needed
) -> Result<Json<ToolResult>, String> {
    // 1. Parse/validate args
    // 2. Lookup connection if needed
    // 3. Call SerialConnection method
    // 4. Format response
}
```

Long-running tools (`read`, `wait_for`, `send_break`) mark `execution(task_support = "optional")`.

## Key Conventions

- **No unwrap/expect in production code** — use `?` or return errors
- **No `println!`** — use `tracing` (debug! / info! / error!)
- **No `todo!()` or `unimplemented!()`** in committed code
- **Resource notifications**: Fire `notify_resource_list_changed()` on open/close
- **Allowlist check**: In `open` tool, before `ConnectionManager::open()`
- **Tests**: Layered (unit → HTTP integration → PTY → stdio → allowlist → hardware)

## Git Conventions

- **No Co-Authored-By lines** in git commits
- Commit messages follow conventional commits: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`
- Group related changes in single commits (module-level, not phase-level)
- Never commit secrets or credentials

## CI Requirements

All PRs must pass:
1. `cargo fmt --all -- --check` (zero formatting issues)
2. `cargo build --all-targets --locked` (clean build)
3. `cargo test --all-targets --locked` (all tests pass)
4. `cargo clippy --all-targets --locked -- -D warnings` (zero clippy warnings)

The CI workflow is in `.github/workflows/ci.yml`.
