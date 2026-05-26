# Serial MCP Server — Actionable Audit Plan

**Date:** 2026-05-26
**Audit basis:** Full repo audit at commit `8233e9e` (main).
**Build state at audit:** `cargo fmt --check`, `cargo clippy -D warnings`,
and `cargo test --all-targets` all pass clean (92 active tests + 3 ignored).

This plan supersedes the previous "All Phases Complete (v0.2.2)" PLAN.md.
Items are grouped by risk and ordered for execution. Each item lists:
file/line anchors, the concrete fix, and the test that should accompany it.

---

## Suggested commit grouping

1. **`fix: robustness — pagination panic, subscribe leak, input clamps`**
   — items 1, 2, 5 (+ tests).
2. **`fix: send_break timing precision for sub-250ms durations`** — item 3.
3. **`docs: sync Cargo.toml version, AGENTS.md, PLAN status`** — items 8, 9, 10, 11.
4. **`refactor: remove dead code (SerialPortError, latest_read, encoding_from_str)`**
   — items 13, 14, 15.
5. Remaining items (4, 6, 7, 12, 16–23) can ship opportunistically.

CI must stay green after each commit: `cargo fmt --all -- --check`,
`cargo clippy --all-targets --locked -- -D warnings`, `cargo test --all-targets --locked`.

---

## P0 — Bugs that crash or leak

### 1. `paginate` panics on out-of-range cursor

**File:** `src/server.rs:32-55`

A client-supplied cursor encoding `offset > all.len()` produces an
inverted slice range `all[offset..all.len()]` → panic. With at most 2
static resources today this trivially crashes any session that sends
`cursor = base64("999")`. `offset + page_size` can also overflow `usize`.

**Fix:**

```rust
let offset = cursor
    .as_deref()
    .and_then(|c| base64::engine::general_purpose::STANDARD.decode(c).ok())
    .and_then(|b| String::from_utf8(b).ok())
    .and_then(|s| s.parse::<usize>().ok())
    .unwrap_or(0)
    .min(all.len());                              // clamp offset
let end = offset.saturating_add(page_size).min(all.len());  // saturating add
let items = all[offset..end].to_vec();
```

**Test:** add `paginate_handles_offset_past_end_without_panic` in
`src/server.rs#tests` exercising offsets of `0`, `len()`, `len()+1`,
and `usize::MAX`. Also add an HTTP integration test that sends a
crafted cursor and asserts the server doesn't 500.

---

### 2. `subscribe` task is not cleaned up on `close`

**Files:** `src/tools/stream_ops.rs`, `src/tools/port_ops.rs:53`, `src/server.rs:124`

`close` removes the connection from `ConnectionManager`, but
`SerialHandler::streams` still owns a `StreamHandle` whose `stream_rx`
task holds an `Arc<SerialConnection>`. Consequences:

- The underlying serial port stays open until the stream task exits
  (drop only fires on `unsubscribe` or `SerialHandler` drop).
- The stream task keeps polling the dead fd, burning CPU and spamming
  `error!("RX stream read error…")` logs.
- `connections.list_open()` reports closed while the stream may still
  push notifications — inconsistent client view.

**Fix:** in `SerialHandler::close` (server.rs), after a successful
`port_ops::close`, also remove the entry from `self.streams`. The
existing `StreamHandle::Drop` will then abort the task.

```rust
async fn close(&self, ...) -> Result<Json<CloseResult>, String> {
    let connection_id = args.connection_id.clone();
    let result = port_ops::close(&self.connections, args).await?;
    // Abort any active RX subscription tied to this connection.
    self.streams.lock().await.remove(&connection_id);
    self.notify_resource_changed(&connection_id, &ctx).await;
    Ok(result)
}
```

**Test:** extend `tests/http_integration.rs` with
`subscribe_then_close_stops_streaming_task`: open → subscribe → close
→ assert no further `notifications/message` arrive within 200ms and
the `connections` resource shows zero open connections.

---

### 5. No upper bound on `max_bytes` / `max_chunk_bytes` — memory DoS

**Files:** `src/tools/helpers.rs:61,167,270`, `src/tools/types.rs`

`read`, `wait_for`, `subscribe` all do `vec![0u8; max_bytes]` where
`max_bytes: usize` is unvalidated client input. `max_bytes = usize::MAX`
OOM-kills the server. `subscribe.poll_interval_ms = 0` makes `stream_rx`
a tight CPU loop.

**Fix:** add a single validation helper in `src/tools/helpers.rs`:

```rust
pub const MAX_READ_BYTES: usize = 1024 * 1024;       // 1 MiB
pub const MAX_WAIT_BYTES: usize = 1024 * 1024;       // 1 MiB
pub const MAX_STREAM_CHUNK_BYTES: usize = 64 * 1024; // 64 KiB
pub const MAX_TIMEOUT_MS: u64 = 5 * 60 * 1000;       // 5 min
pub const MIN_POLL_INTERVAL_MS: u64 = 10;
pub const MAX_WRITE_BYTES: usize = 1024 * 1024;      // 1 MiB

pub fn clamp_or_err(name: &str, value: usize, max: usize) -> Result<usize, String> {
    if value > max {
        Err(format!("{name}={value} exceeds maximum {max}"))
    } else { Ok(value) }
}
```

Wire into each tool handler before allocation. Return a tool-level error
(`CallToolResult{is_error}`) rather than truncating silently.

| Tool | Field | Cap |
|---|---|---|
| `read` | `max_bytes` | 1 MiB |
| `wait_for` | `max_bytes` | 1 MiB |
| `subscribe` | `max_chunk_bytes` | 64 KiB |
| `subscribe` | `poll_interval_ms` | min 10ms |
| `read` / `wait_for` / `send_break` | `timeout_ms` / `duration_ms` | 5 min |
| `write` | decoded `data.len()` | 1 MiB |

Also reflect caps in the JSON schemas via `schemars` `range(max = …)`
where straightforward.

**Test:** unit tests asserting `is_error=true` for each over-limit input,
plus one that confirms `subscribe(poll_interval_ms=0)` is rejected.

---

## P1 — Bugs that misbehave but don't crash

### 3. `send_break` overshoots short durations

**File:** `src/tools/control_ops.rs:104-126`

`tokio::time::interval(Duration::from_millis(250))` ticks at t≈0, 250,
500…. The break-release check only runs on tick. With `duration_ms < 250`
the break is held until the next tick at 250ms — a requested 50ms BREAK
becomes ~250ms, significant for some legacy targets.

**Fix:** decouple the deadline from the progress ticker. Sleep to the
deadline, race against cancellation, and emit progress on a separate
interval that doesn't gate release.

```rust
let deadline = start + Duration::from_millis(args.duration_ms);
let mut progress_ticker = tokio::time::interval(Duration::from_millis(250));
progress_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

loop {
    tokio::select! {
        _ = ct.cancelled() => return Err("Cancelled".into()),
        _ = tokio::time::sleep_until(deadline) => break,
        _ = progress_ticker.tick() => {
            // emit progress (omit on first tick to avoid t=0 redundant emit)
            ...
        }
    }
}
```

**Test:** PTY test `send_break_50ms_releases_within_100ms` measuring
elapsed wall-clock and asserting `actual_duration_ms ∈ [40, 100]`.

---

### 4. `BreakResetGuard::drop` spawns on a possibly-dead runtime

**File:** `src/tools/control_ops.rs:60-81`

`tokio::spawn` inside `Drop` panics if no current runtime, which can
happen during shutdown. Separately, `Cell<bool>` makes the future
`!Send` — currently tolerated by rmcp but fragile.

**Fix:**

```rust
use std::sync::atomic::{AtomicBool, Ordering};

struct BreakResetGuard {
    connection: Arc<SerialConnection>,
    disarmed: AtomicBool,
}

impl Drop for BreakResetGuard {
    fn drop(&mut self) {
        if self.disarmed.load(Ordering::Relaxed) { return; }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let connection = Arc::clone(&self.connection);
            handle.spawn(async move {
                let _ = connection.set_break_state(false).await;
            });
        }
    }
}
```

**Test:** existing tests cover the happy path; add a unit test that
constructs and drops the guard with `disarmed=false` inside a
`tokio::runtime::Builder::new_current_thread().build()` and verifies
no panic.

---

### 6. `stream_rx` silently drops bytes on encoding failure

**File:** `src/tools/helpers.rs:276-279`

```rust
let encoded = match codec::encode(encoding, chunk) {
    Ok(s) => s,
    Err(_) => continue,           // bytes lost without trace
};
```

Hits in practice whenever a UTF-8 subscriber joins mid-frame on a binary
stream. Client sees a gap with no signal.

**Fix:** log a warning *and* emit a structured notification so the
client can react:

```rust
Err(e) => {
    warn!("RX encoding error on {}: dropped {} bytes", connection.id(), chunk.len());
    let payload = serde_json::json!({
        "connection_id": connection.id(),
        "encoding_error": true,
        "encoding": encoding.to_string(),
        "bytes_dropped": chunk.len(),
        "reason": e.to_string(),
    });
    let _ = peer.notify_logging_message(LoggingMessageNotificationParam {
        level: LoggingLevel::Warning,
        logger: Some(logger.clone()),
        data: payload,
    }).await;
    continue;
}
```

**Test:** PTY test that opens with `encoding="utf8"`, pushes
`\xFF\xFE` from the device side, and asserts a `Warning`-level
notification with `encoding_error: true` arrives.

---

### 7. `notify_resource_list_changed` fires unconditionally

**File:** `src/server.rs:298-317`

Cheap, but emits on every open/close regardless of subscribers. Spec-
compliant, just noisy. Acceptable to leave as-is; document in a
comment that the `subscribers` map is only consulted for
`resources/updated`, not for `resources/list_changed`.

**Action:** add a clarifying comment. No code change required.

---

## P2 — Documentation drift

### 8. `Cargo.toml` version is `0.2.0`, docs reference `0.2.2`

**File:** `Cargo.toml:3`

CHANGELOG.md has entries for `0.2.1` and `0.2.2`; old PLAN.md was
labeled `v0.2.2`. Bump to `0.2.2` (or `0.2.3` if any P0 items above
ship in this release).

**Action:** edit `Cargo.toml`, regenerate `Cargo.lock`, verify the
description string still matches feature set ("11 tools").

---

### 9. `AGENTS.md` documents a `SerialError` enum that doesn't match the code

**Files:** `AGENTS.md` (Error Handling section), `src/error.rs`

`AGENTS.md` lists: `IoError`, `ReadTimeout`, `InvalidBaudRate`,
`PortAlreadyOpen`, `ConnectionNotFound`, `InvalidArgument`.

Actual `src/error.rs`: `ConnectionFailed`, `ConnectionExists`,
`InvalidConnection`, `InvalidBaudRate`, `ReadTimeout`, `IoError`,
`SerialPortError`.

**Decision needed:** two equally-valid options.

- **Option A (rename code to match docs):** more readable variant names.
  - `ConnectionFailed` → `OpenFailed`
  - `ConnectionExists` → `PortAlreadyOpen`
  - `InvalidConnection` → `ConnectionNotFound`
  - Add `InvalidArgument(String)` (currently inlined as `String` returns).
  - Touch all `matches!(err, SerialError::…)` sites in tests.

- **Option B (update docs to match code):** zero code churn. Replace the
  `AGENTS.md` enum listing with the real variants from `src/error.rs`.

**Recommendation:** Option A — the doc names are clearer
(`ConnectionNotFound` vs `InvalidConnection` removes the "invalid in
what way?" ambiguity), and the type is touched in only ~10 places.

---

### 10. `AGENTS.md` shows wrong format for `log_tool_err`

**File:** `AGENTS.md` (Error Handling section), `src/tools/helpers.rs:396`

Docs claim `format!("{op} failed — {context}: {err}")`. Reality:

```rust
pub fn log_tool_err<E: std::fmt::Display>(op: &str, context: &str, err: E) -> String {
    error!("{op} failed: {err}");
    format!("{context} - {err}")
}
```

**Action:** update `AGENTS.md` snippet to match real signature/output,
or change `log_tool_err` to return the documented format. Recommend
updating the doc (the code's split between log line and user-facing
string is intentional — log keeps `op`, return value keeps `context`).

---

### 11. `PLAN.md` test counts are stale

**Old file:** "Total: 70 tests active, 2 ignored."
**Actual current run:** 92 active + 3 ignored
(46 unit + 22 http + 2 resource_sub + 5 allowlist + 6 pty + 3 stdio +
2 blob + 2 hardware-loopback-ignored).

**Action:** this is fixed by overwriting PLAN.md (you're reading the new
one). Going forward, prefer not to commit a frozen test count — let
CHANGELOG entries reference it instead.

---

### 12. Repo-root dev journals are noisy

**Files:** `STATUS.md`, `PLAN_SCHEMA_FORMAT_FIX.md`, `REVIEW.md`

Read like internal AI scratch pads. Either move under `docs/` or delete.
For a public-facing repo, the project root should be:

```
README.md  CHANGELOG.md  LICENSE  AGENTS.md  Cargo.toml  Cargo.lock  ...
```

**Action:** `git mv STATUS.md PLAN_SCHEMA_FORMAT_FIX.md REVIEW.md docs/`
or delete the obsolete ones (the schema-format fix is already shipped
per CHANGELOG).

---

## P3 — Dead code

### 13. `SerialError::SerialPortError(serialport::Error)` is unused

**File:** `src/error.rs:23`

No constructor anywhere — `build_stream` stringifies via
`ConnectionFailed(format!(...))`. The `#[from]` impl is dead.

**Action:** either remove the variant, or wire `build_stream` to
return `SerialError::SerialPortError(e)` directly and let `Display`
do the formatting. Remove is simplest.

---

### 14. `ConnectionSummary::latest_read` is always `None`

**Files:** `src/serial.rs:474`, `src/serial.rs:461`, `src/server.rs:508`

The field is set to `None` everywhere it's constructed.

**Decision:**

- **Drop the field** (clean): remove from struct, update JSON schema.
- **Populate it** (feature): add a ring buffer to `SerialConnection`
  holding the last N bytes (base64-encoded on read), update on every
  `read()` success. Useful for the `serial://connections` snapshot but
  introduces extra locking.

**Recommendation:** drop it now; reintroduce only when a use case
appears.

---

### 15. `tools::io_ops::encoding_from_str` has no callers

**File:** `src/tools/io_ops.rs:88-90`

Trivial wrapper around `parse_encoding`. Delete.

---

## P4 — Smells / minor

### 16. Resource templates: `/raw` vs JSON detail overlap

**File:** `src/server.rs:425-462`, `src/resources/mod.rs:24`

Both `serial://connections/{id}` and `serial://connections/{id}/raw`
are advertised, the latter is a blob view. Fine to keep, but document
that `/raw` *consumes* bytes from the connection (item 17 below).

---

### 17. `read_resource` for `/raw` consumes bytes (races with `read`/`wait_for`/`subscribe`)

**File:** `src/server.rs:516-532`

`conn.read_latest(256)` blocks for ~100ms and consumes bytes from the
device's read queue. Concurrent tool calls on the same id will see
short reads.

**Options:**

- Document the consumption behavior in the resource description string.
- Add a non-consuming snapshot via an internal ring buffer (ties to
  item 14).

**Recommendation:** documentation-only for now.

---

### 18. `stream_rx` doesn't observe `CancellationToken`

**File:** `src/tools/helpers.rs:262-302`

Stopped only via `unsubscribe`, `Drop`, or peer disconnect. If/when
`tasks/cancel` is wired to subscriptions, plumb a `ct: CancellationToken`
through `subscribe` and `tokio::select!` it against `connection.read(...)`.

**Action:** defer until `tasks/cancel` semantics for subscriptions
are decided.

---

### 19. `SecurityManager::from_env` re-runs per HTTP session

**Files:** `src/bin/http.rs:48-56`, `src/server.rs:79`

`LocalSessionManager` calls the factory closure per new session;
`SerialHandler::with_manager()` re-reads `SERIAL_MCP_ALLOWLIST` each
time, including emitting the `info!("Port allowlist active: …")`
log line per session.

**Fix:** in `src/bin/http.rs`, construct one `SecurityManager` and
clone it into the closure:

```rust
let security = SecurityManager::from_env();
let manager_for_service = Arc::clone(&manager);
let service = StreamableHttpService::new(
    move || Ok(SerialHandler::with_manager_and_security(
        Arc::clone(&manager_for_service),
        security.clone(),
    )),
    ...
);
```

---

### 20. `examples/STM32_demo/Cargo.lock` is modified but uncommitted

**File:** `examples/STM32_demo/Cargo.lock`

`git status` flags this. Either commit the regenerated lockfile or
revert it.

**Action:** decide based on whether the example was intentionally
rebuilt. If not intentional, `git checkout examples/STM32_demo/Cargo.lock`.

---

### 21. `.vscode/` shared settings — audit for personal paths

**File:** `.gitignore:4-6` allowlists `.vscode/extensions.json` and
`.vscode/settings.json`.

**Action:** spot-check both files for absolute paths or local env hints
before any open-source release.

---

### 22. `write` tool has no payload size cap

**File:** `src/tools/io_ops.rs:13-37`

Already partially covered by item 5. Add `MAX_WRITE_BYTES = 1 MiB`
check on the *decoded* byte length (after `codec::decode`), since hex
input is 2x size of bytes and base64 is 4/3x.

---

### 23. `progress_token.clone()` per loop iteration

**Files:** `src/tools/helpers.rs:89,122,209`

`ProgressToken` clones may allocate. Hoist `let token = progress_token.clone();`
outside the loop, or take `&ProgressToken`.

**Action:** micro-optimization; verify with `cargo expand` whether
`ProgressToken::clone` is cheap (it likely wraps an `Arc<str>` or
`String`). Defer unless a profiling pass flags it.

---

## Out of scope (deferred)

- **Cross-process port locking** (advisory flock per port) — was listed
  as Phase 5 in the old PLAN.md, still not implemented, low priority.
- **`serial://connections/{id}/stats` resource** — bytes sent/received
  counters per connection. Also Phase 5 carry-over.
- **`tasks/cancel` for subscriptions** — relates to item 18.

---

## Verification gate for the whole plan

Each commit must keep CI green:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
```

Hardware-gated tests (`SERIAL_MCP_TEST_PORT=…`) are not part of the
gate but should be re-run locally before tagging a release.
