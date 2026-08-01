# Code Cleanup Phase 5A Handoff

## Goal

Consolidate exact duplicate test helpers while preserving cross-platform test
compilation and Linux native_sim runtime behavior. This subphase does not touch
`TestServer` constructors; those are Phase 5B.

Implement Phase 5A only. Commit completed work before returning.

## In scope

- `tests/common/mod.rs`
- `tests/common/binaries.rs`
- `tests/common/firmware.rs`
- `tests/common/spawned.rs`
- `tests/native_sim_validation/unix.rs`
- `tests/native_sim_connection_lifecycle.rs`
- `tests/http_integration.rs`
- `tests/stdio_integration.rs`
- `tests/blob_resources.rs`
- This handoff document

## Out of scope

- Do not alter `tests/native_sim_validation.rs` platform dispatch or
  `tests/native_sim_validation/windows.rs` stub.
- Do not add/remove target `cfg` attributes.
- Do not change ignored-test attributes, test names, assertions, timeouts,
  serial parameters, PTY discovery text, or CI workflows.
- Do not consolidate suite-specific native_sim MCP helpers (`open_pty`,
  `write_cmd`, `sync_boot`, etc.) in this subphase.
- Do not add async client cleanup wrappers/macros.
- Do not change `TestServer` constructors yet.
- Do not change production code.

## Platform grounding

1. `tests/native_sim_validation.rs` always includes `mod common`, then selects
   `unix.rs` or `windows.rs` with target `cfg`.
2. `tests/native_sim_connection_lifecycle.rs` is not target-gated. Cargo builds
   it on Windows/macOS/Linux during `--all-targets`; tests remain ignored unless
   explicitly requested.
3. Shared `NativeSimFirmware` must therefore be defined and compile on every
   platform. Do **not** put `#[cfg(unix)]` on the type, imports, or methods.
4. Current harness uses only cross-platform Tokio process/I/O APIs. PTY text is
   parsed as a string; actual runtime remains Linux-only in CI.
5. Local environment lacks `rustup`, so Windows cross-compilation is not
   available. `cargo build/test/clippy --all-targets` is local gate; GitHub's
   Windows runner is final compile authority.

## Exact implementation shape

### 1. Shared workspace root

Move duplicate `workspace_root()` implementation into `tests/common/mod.rs`:

```rust
pub fn workspace_root() -> &'static std::path::PathBuf
```

Use one function-local `OnceLock<PathBuf>` and preserve current
`CARGO_MANIFEST_DIR` debug assertion.

Update `binaries.rs` and `firmware.rs` to call `super::workspace_root()`. Remove
only now-unused imports/statics. Keep their independent build-once state.

### 2. Shared native_sim process harness

Move `NativeSimFirmware` into `tests/common/firmware.rs` as a public test helper:

```rust
pub struct NativeSimFirmware { ... }

impl NativeSimFirmware {
    pub async fn spawn() -> anyhow::Result<Self>;
    pub fn pty_path(&self) -> &str;
    pub fn try_exit_code(&mut self) -> Option<i32>;
}
```

Preserve exactly:

- `ensure_plain_firmware_built()` resolution.
- `stdout(Stdio::piped())`, `stderr(Stdio::null())`, `kill_on_drop(true)`.
- Five-second discovery deadline with 500ms read polls.
- Search for `uart connected to pseudotty:` then `/dev/pts/` suffix.
- Error when path not printed in five seconds.
- Background stdout drain task.
- Best-effort `child.start_kill()` in `Drop`.

Keep fields private. Keep drain handle alive on struct. Use no Unix-only APIs.
Update `firmware.rs` module docs because spawning is no longer caller-owned.

Delete duplicate harness structs, `zephyr_bin`, `Drop`, and now-unused process/
reader imports from both suites. Import shared type. Keep `Duration` in each
suite when other tests still use it. Convert lifecycle direct field reads to
`fw.pty_path()`.

### 3. Shared notification collector

Delete duplicate `NotificationCollector` implementation from
`tests/common/spawned.rs`.

Make `spawn_client` return the parent `super::NotificationCollector` type and
delegate connection setup to:

```rust
super::connect_to_url(server.url.as_str()).await
```

Remove only unused handler/transport imports. Keep `spawn_client` API so HTTP
test call sites remain unchanged.

### 4. Shared explicit tool list

Move identical `EXPECTED_TOOLS` constant from HTTP and stdio integration files
to `tests/common/mod.rs` as:

```rust
pub const EXPECTED_TOOLS: &[&str] = &[...];
```

Keep list explicit and byte/order-identical. Do not derive it from production
`tool_catalog`; transport tests must retain independent expected values.
Update both suites to use `common::EXPECTED_TOOLS` or import it.

### 5. Remove duplicate stdio build wrappers

Delete both local `build_stdio_server()` functions. At their existing call
sites, call `common::binaries::ensure_serial_mcp_built().expect(...)` directly,
retaining each suite's useful failure context. Remove imports made redundant.

Do not introduce another wrapper with same one-line body.

## Verification

Run all compile gates before runtime suites:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Then run affected suites:

```bash
cargo test --test http_integration --locked -- --test-threads=1
cargo test --test stdio_integration --locked -- --test-threads=1
cargo test --test blob_resources --locked -- --test-threads=1
cargo test --test native_sim_validation -- --ignored --test-threads=1
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

If native_sim artifact cannot be built/run locally, report exact blocker and
leave phase open; do not claim runtime verification. Current repo normally
auto-builds missing firmware through `fw-build-native`.

## Acceptance criteria

- One workspace-root function.
- One `NativeSimFirmware` implementation.
- Shared harness remains compiled by lifecycle target on all platforms.
- One `NotificationCollector` implementation.
- One explicit expected-tool list.
- No duplicate stdio build wrapper.
- Test names, counts, platform gates, and runtime behavior unchanged.
- All requested checks pass with no warnings, including both native_sim suites.
- Diff contains no production or unrelated cleanup.
- Working tree clean after commit.

## Commit and recap

Before returning:

1. Inspect status, diff, and recent log.
2. Stage only intended test-helper/suite files and this handoff.
3. Commit with suggested message:

   `refactor: consolidate cross-platform test helpers`

4. Do not push, merge, open PR, amend, force-push, or add attribution.
5. Return files, platform behavior, tests/results, commit hash/message,
   deviations, blockers, and follow-up concerns.
