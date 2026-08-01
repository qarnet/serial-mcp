# Phase 6 Handoff — Safe Persistent Capture Foundation

## Goal

Remove unrestricted filesystem writes from `export_log` and establish the
containment, symlink, quota, atomicity, and lifecycle policy required before any
future continuous raw capture feature. Do not add continuous capture tools in
this phase.

Current `export_log` accepts an arbitrary path and calls `std::fs::write` on the
async runtime with no containment, no-clobber, lock, quota, or behavior tests.
Phase 6 replaces that unsafe boundary.

## Public behavior

1. Persistent export is disabled unless server starts with an explicit absolute
   dedicated capture directory.
2. `export_log.path` remains the wire field for compatibility but accepts one
   portable `.jsonl` filename only—not an arbitrary path.
3. Export never overwrites an existing file or follows a destination symlink.
4. Complete JSONL snapshot is committed atomically within configured root.
5. Per-file, total-byte, and file-count quotas are enforced from a fresh scan
   under process-local and advisory cross-process locks.
6. Success returns exact event/byte counts, canonical final path, and quota
   usage. Failure creates no final file and changes no existing capture.
7. No new capture tool, background writer, or path-bearing API is added.

## In scope

- Process-wide `CaptureStore`, disabled by default.
- CLI capture root and quota options.
- Flat portable filename validation.
- Root containment and symlink policy.
- Bounded consistent JSONL serialization.
- Atomic no-clobber same-root write.
- Cross-process quota locking.
- Public MCP behavior and startup validation tests.
- Future continuous-capture lifecycle design record.

## Non-scope

- No `start_capture`/`stop_capture` tools.
- No file rotation or retention deletion.
- No caller-created subdirectories.
- No overwrite flag.
- No arbitrary absolute path compatibility.
- No hostile-root capability security against an operator replacing trusted
  root ancestors while server runs; portable std/tempfile/fs2 APIs cannot
  fully provide directory-handle-relative guarantees.
- No package version bump.
- No broad log-capacity redesign unless required by focused export safety.

## CLI and defaults

Add:

```text
--capture-dir <absolute-existing-directory>
--capture-max-file-bytes <N>   default 16777216 (16 MiB)
--capture-max-total-bytes <N>  default 268435456 (256 MiB)
--capture-max-files <N>        default 256
```

Rules:

- without `--capture-dir`, persistence is disabled
- quota options explicitly supplied without capture dir are startup errors
- root must be absolute, exist, be a directory, and not itself be a symlink
- canonicalize once at startup
- all limits > 0; per-file <= total
- add every option to `VALUE_TAKING_OPTIONS` and help/drift tests
- no fallback to cwd, OS config, or temp directory

Production creates one `Arc<CaptureStore>` and shares it through stdio/HTTP
handlers. Library/builder default is disabled.

## CaptureStore

Add `src/capture_store.rs`:

```rust
pub struct CaptureLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_files: usize,
}

pub struct CaptureStore { /* root, limits, async mutex */ }

impl CaptureStore {
    pub fn disabled() -> Self;
    pub fn open(root: PathBuf, limits: CaptureLimits) -> Result<Self, String>;
    pub fn is_enabled(&self) -> bool;
    pub fn max_file_bytes(&self) -> u64;
    pub async fn write_new(
        &self,
        requested_name: String,
        bytes: Vec<u8>,
    ) -> Result<CaptureWriteResult, String>;
}
```

`CaptureStore` is process-wide like ProfileStore. Add it to
`SerialHandlerOptions`/builder/handler; inject same Arc into every HTTP session.

## Filename contract

Keep MCP field name `path`, but update docs/schema description: it is a
filename relative to configured capture directory.

Accepted filename:

- ASCII
- 1–120 characters before/including suffix as implementation documents
- starts alphanumeric
- remaining characters only alphanumeric, `.`, `_`, `-`
- ends `.jsonl` case-sensitively
- no `/` or `\`
- no `.`/`..`, absolute/root/prefix components, or nested path
- no internal `.serial-mcp-` prefix
- reject Windows reserved stems case-insensitively (`CON`, `PRN`, `AUX`, `NUL`,
  `COM1`–`COM9`, `LPT1`–`LPT9`), including names with extension

Flat namespace intentionally removes intermediate symlink traversal and keeps
quota scan bounded/simple. Absolute paths—even direct children—are rejected.

## Root, symlink, and trust policy

At startup:

- use `symlink_metadata` on configured path; reject direct symlink
- require directory
- canonicalize and store canonical root
- inspect lock path; reject it if symlink/non-regular
- open/create `.serial-mcp-captures.lock` and prove advisory lock works

At each transaction under lock:

- scan direct children only
- committed managed files are valid portable `.jsonl` regular files
- destination existing as regular file, symlink, directory, or special entry is
  a no-clobber error
- reject symlink entries with managed `.jsonl` names; never follow them
- ignore lock file and recognized same-process temp prefix for quota accounting
- do not delete unknown/orphan files automatically

Advisory locking protects cooperating serial-mcp processes. Configured root and
ancestors remain operator-controlled trust boundary. State this explicitly.

## Locking, quota, and commit transaction

Use existing `fs2` + `tempfile`; add no dependency.

`write_new`:

1. reject disabled policy and invalid filename before connection/file work
2. reject bytes larger than max-file quota
3. acquire process-local async mutex
4. run blocking work in `spawn_blocking`
5. acquire exclusive advisory root lock
6. fresh scan committed managed files; use checked arithmetic
7. reject existing destination
8. enforce `count + 1 <= max_files`
9. enforce `total + bytes <= max_total_bytes`
10. create `tempfile::Builder` temp in same root with reserved internal prefix
11. write all bytes, `sync_all`
12. `persist_noclobber` destination
13. sync root directory on Unix; document portable Windows crash-durability
    limitation
14. return final canonical path and post-commit usage

Zero-byte exports consume one file slot. No final file exists before successful
commit. A temp file may survive process crash; this phase never silently treats
it as committed or deletes arbitrary files.

## Bounded JSONL snapshot

Add to `LogBuffer`:

```rust
pub struct JsonlSnapshot {
    pub bytes: Vec<u8>,
    pub events: usize,
}

pub fn jsonl_snapshot(&self, max_bytes: u64) -> Result<JsonlSnapshot, String>;
```

Requirements:

- lock log once for consistent point-in-time snapshot
- serialize one event per line with trailing newline
- check size before extending output; checked arithmetic
- fail when exact snapshot exceeds max-file quota
- do not clone full event deque then build second full String
- serialization errors and quota errors leave no file

Serialize in blocking context. `CaptureStore` provides file limit; export tool
coordinates snapshot and write without blocking Tokio worker threads.

## Tool behavior and result

`export_log` first checks store enabled/path validity, then connection, then
captures bounded snapshot and commits. Disabled error:

```text
Persistent capture is disabled; start serial-mcp with --capture-dir <absolute-directory>
```

Keep existing fields and add:

```rust
pub struct ExportLogResult {
    pub connection_id: String,
    pub path: String, // canonical absolute final file
    pub events_written: usize,
    pub bytes_written: u64,
    pub files_used: usize,
    pub total_bytes_used: u64,
}
```

Annotate every unsigned field. Success means complete durable no-clobber commit;
failure is tool error. Tool count remains 27. Update description to explain
configured root, filename-only path, no overwrite, and quotas.

## Behavior-first tests

### Public MCP

Use in-memory connection/log plus real HTTP MCP boundary and injected
CaptureStore:

1. Disabled export errors before path write and creates nothing.
2. Enabled export writes valid JSONL; events and exact byte counts match
   `get_log` snapshot.
3. Empty/disabled log commits zero-byte file and consumes one file slot.
4. Traversal, nested path, absolute path, slash/backslash, bad suffix,
   overlength/internal/reserved Windows names all fail without files.
5. Existing regular target remains byte-identical; no overwrite.
6. Existing symlink target is rejected and outside target untouched (Unix).
7. Concurrent same-name exports yield exactly one success.
8. Per-file quota failure creates no final/temp committed file.
9. Total-byte quota persists across exports and fresh CaptureStore instances.
10. File-count quota includes prior committed files.
11. Independent stores/process-local mutexes sharing root cannot exceed total or
    count quota because advisory lock serializes scan+commit.
12. Result path is canonical root child; usage fields reflect commit.
13. Connection remains usable after export failure.
14. Snapshot consistency: events added after snapshot are not partially mixed
    into committed file.

### Startup/CLI

- help documents all options
- value-taking `--capture-dir --version` is not mistaken for version flag
- relative, missing, non-directory, direct symlink roots reject startup
- zero/invalid quota relation and quota-without-root reject startup
- valid root starts in stdio and HTTP modes

### Unit/property

- portable filename validator
- quota checked arithmetic/boundaries
- managed-file scanner classification
- no-clobber commit
- cross-store lock behavior
- bounded JSONL exact-limit and one-byte-over cases

## Existing behavior migration

- Arbitrary path writes are intentionally removed for security.
- Existing callers must configure server and pass filename only.
- Existing destination overwrite is intentionally removed.
- Tool name and `path` field remain.
- Result additions are additive.
- Document migration in README and CHANGELOG under unreleased/current version;
  do not bump package version.

## Future continuous-capture lifecycle design

Create `docs/development/safe-continuous-capture-design.md`; do not implement.
Must specify:

- process-wide registry and stable capture IDs
- states: starting/running/stopping/stopped/failed
- initially one active raw capture per connection
- start/stop/status/list tools; stop idempotent
- private RX cursor, atomic live-edge mark, shared cursor untouched
- raw bytes exact; framing/parser excluded initially
- bounded queue/backpressure; explicit gaps, never silent loss
- stop reasons: match/timeout/silence/cancelled/quota/io/ring overrun/
  connection closed/server shutdown
- disconnect stops initially; reconnect continuation deferred
- segment rotation before per-file quota; same lock/quotas/no-clobber finalization
- internal partial-file names count toward quota and have explicit orphan policy
- cancellation before start response leaves no orphan task
- connection close/server shutdown bounded stop+finalize lifecycle
- no profile learning, no payloads in traces/errors
- status offsets/bytes observed/written/lost/segments/quota/error
- no automatic deletion until deterministic retention policy exists
- tests for cancellation, disconnect, wrap, disk failure, quota, rotation,
  restart orphan handling, concurrent clients

Conclude whether future feature is still justified. Phase 4/5 evidence supports
bounded in-memory boot capture, not yet continuous disk capture; default
recommendation should remain “do not implement until concrete task evidence.”

## Evaluator/docs

- Keep Phase 4 baseline historical.
- Run current evaluator against baseline and record export schema/catalog delta
  in Phase 6 decision record.
- Update README, server instructions/tool description, `--help`, CHANGELOG,
  AGENTS.md, doc drift tests.
- No tool-count change.

## Expected files

- new `src/capture_store.rs`
- `src/lib.rs`, `src/limits.rs`, `src/log_buffer.rs`
- `src/main.rs`, `src/server.rs`
- `src/tools/port_ops.rs`, `src/tools/types.rs`
- `src/serial.rs` schema guards
- test common builders/spawned helper
- HTTP/stdio/CLI tests
- README, CHANGELOG, AGENTS.md
- Phase 6 decision and future lifecycle docs
- evaluator report note if generated
- this handoff

## Verification

```bash
cargo test --lib capture_store
cargo test --lib log_buffer
cargo test --test http_integration export_log --locked
cargo test --test stdio_integration --locked
cargo test --manifest-path xtask/Cargo.toml
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
git diff --check
```

## Commit requirements

Inspect status/diff/log, stage only Phase 6 files, commit conventional message,
return behavior/tests/catalog delta/hash/deviations. Do not amend, push, merge,
open PR, add attribution, or add continuous-capture tools.
