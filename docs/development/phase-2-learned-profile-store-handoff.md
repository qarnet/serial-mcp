# Phase 2 Handoff — Shared Learned-Profile Storage Foundation

## Goal

Make existing profiles reliably persistent and process-wide before automatic
profile selection or learning is added. A profile created through the shipped
binary must survive restart, every HTTP MCP session must observe one shared
store, and concurrent writers must not lose each other's updates.

This is Phase 2 of
`docs/development/agent-interface-simplification-plan.md`.

## User-observable behavior being proved

1. Create a profile through MCP, stop the actual server process, start a fresh
   process with the same profile path, and `list_profiles` returns it.
2. HTTP client A creates or deletes a profile and HTTP client B connected to the
   same server immediately observes that result.
3. Concurrent writes from separate clients and separate server processes to the
   same profile file preserve every unrelated profile.
4. Legacy unversioned profile files load. Their next successful mutation writes
   the current schema version without losing settings.
5. Corrupt or unsupported-future files fail startup clearly and are not silently
   treated as empty or overwritten.
6. A failed profile write leaves both the previous on-disk file and current
   in-memory view unchanged.
7. `--profiles-path` selects an isolated persistent store. Without the flag,
   the OS user-config path remains the default.

Tests must prove these behaviors through public MCP calls and real process
lifecycle where specified. Private-field, builder-wiring, `Arc` identity, or
helper-call assertions do not count as phase acceptance.

## In scope

- Replace `SerialHandler`'s handler-local profile vector/path with one
  process-wide `Arc<ProfileStore>`.
- Load the profile file on the production path used by both stdio and HTTP.
- Share one store across all HTTP MCP session handlers.
- Add `--profiles-path <path>` and document it.
- Remove silent current-working-directory fallback when no OS config directory
  exists.
- Serialize profile mutations in-process.
- Use cross-platform advisory file locking and reload-under-lock so separate
  server processes do not lose updates.
- Keep atomic temp-file replacement; sync the temporary file before rename.
- Add a backward-compatible schema version and reject unsupported future
  versions.
- Add profile metadata and bounded prior-revision storage needed by Phase 3.
- Refactor existing `configure(profile)`, `save_profile`, and `delete_profile`
  through the store without changing their MCP argument/result schemas.
- Preserve current `open_profile`, matching, defaults, and explicit profile
  behavior.

## Out of scope

- No automatic profile selection on bare `open`.
- No generated session profile.
- No connection-to-profile binding.
- No automatic persistence from `reconfigure` or connection-mode `configure`.
- No profile match information in `list_ports` yet.
- No rollback MCP tool or profile metadata fields in current MCP results yet.
- No connection recipes, simple facade, boot capture, or capture-to-file.
- No changes to serial connection, ring, framing, parser, or protocol behavior.
- Do not push, merge, open a PR, bump package version, or modify release files.

## Confirmed current defect

`src/server.rs`:

- `SerialHandler::new()` loads the default profile file.
- `SerialHandler::builder().build()` initializes an empty profile vector.

`src/main.rs`:

- Production stdio and HTTP both use `builder().build()`, not `new()`.
- HTTP creates a new handler-local empty vector for every MCP session while
  connections, streams, and budget are process-shared.

Result: normal production startup does not load persisted profiles, and HTTP
profile memory is not shared across sessions.

## Exact architecture

### New `ProfileStore`

Add a focused module, preferably `src/profile_store.rs`, exported by
`src/lib.rs`.

```rust
pub struct ProfileStore {
    path: Option<PathBuf>,
    cache: tokio::sync::RwLock<Vec<Profile>>,
    mutation_lock: tokio::sync::Mutex<()>,
}
```

Required constructors and operations (names may vary slightly, behavior may
not):

```rust
pub fn open(path: PathBuf) -> Result<Self, String>;
pub fn ephemeral() -> Self;
pub async fn list(&self) -> Vec<Profile>;
pub async fn get(&self, name: &str) -> Option<Profile>;
pub async fn upsert(&self, profile: Profile, overwrite: bool) -> Result<bool, String>;
pub async fn update_defaults_preserving_selector(
    &self,
    name: String,
    defaults: ProfileDefaults,
    overwrite: bool,
) -> Result<bool, String>;
pub async fn delete(&self, name: &str) -> Result<(), String>;
pub fn path(&self) -> Option<&Path>;
```

Keep the generic read-modify-write primitive private. Tool code should call
named store operations rather than mutate its cache directly.

### Mutation state transition

For every persistent mutation:

1. Acquire the process-local async mutation mutex.
2. Run blocking file work via `tokio::task::spawn_blocking`; do not block an
   async runtime worker while waiting on another process's file lock.
3. Create/open a sibling lock file named by appending `.lock` to the profile
   file path.
4. Acquire an exclusive advisory lock.
5. Reload and validate the latest profile file from disk while holding the
   lock. Missing file means empty current-version data. Corrupt or unsupported
   data returns an error.
6. Apply the requested operation to a working copy.
7. Serialize current schema, write a `NamedTempFile` in the same directory,
   `sync_all()` the temporary file, then atomically persist over the target.
8. Release the file lock.
9. Only after durable write succeeds, replace the in-memory cache with the full
   resulting profile vector.
10. On any failure, leave cache and previous target file unchanged and return a
    tool error.

Reload-under-lock is mandatory. A lock around a stale in-memory cache still
loses updates from another process.

Use `fs2 = "0.4.3"` for cross-platform advisory locking. Official docs define
`FileExt::lock_exclusive`; it uses `flock(2)` on Unix and `LockFile` on Windows.
Add it as a normal dependency and update `Cargo.lock` through Cargo, not by hand.

### File format and migration

Treat existing unversioned TOML as schema version 1. Current schema is version
2 because metadata/revision records are introduced.

```toml
schema_version = 2

[[profile]]
name = "device-name"
```

Root model:

```rust
struct ProfilesFile {
    #[serde(default = "legacy_schema_version")]
    schema_version: u32,
    #[serde(default)]
    profile: Vec<Profile>,
}
```

Rules:

- Missing file: valid empty v2 store; first mutation creates directories/file.
- Missing `schema_version`: parse as v1 and migrate in memory.
- v1 to v2: preserve selector/defaults; initialize metadata/history defaults.
- `schema_version == 0`: reject.
- `schema_version > 2`: reject with a clear upgrade/newer-version message.
- Invalid TOML or invalid profile fields: reject startup/mutation; never return
  an empty store for an existing invalid file.
- Migration may be persisted on the next successful mutation; startup need not
  rewrite a valid legacy file by itself.

### Metadata and bounded history foundation

Extend `Profile` with serde-defaulted fields so legacy TOML remains readable:

```rust
#[serde(default)]
pub metadata: ProfileMetadata,
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub revisions: Vec<ProfileRevision>,
```

Recommended exact shapes:

```rust
pub struct ProfileMetadata {
    pub generated: bool,
    pub revision: u64,
    pub created_at_ms: Option<u64>,
    pub updated_at_ms: Option<u64>,
    pub last_used_at_ms: Option<u64>,
    pub use_count: u64,
}

pub struct ProfileRevision {
    pub revision: u64,
    pub saved_at_ms: u64,
    pub selector: ProfileSelector,
    pub defaults: ProfileDefaults,
}
```

All unsigned fields on `JsonSchema` types require repository schema helpers.
Add new types to schema guards where appropriate.

Use `MAX_PROFILE_REVISIONS: usize = 5`. On overwrite:

- preserve original `created_at_ms`, generated flag, last-used metadata, and
  use count unless the incoming operation explicitly owns those fields
- append the prior selector/defaults as a revision snapshot
- keep only the newest five prior snapshots
- increment revision (legacy/default revision becomes 1 before first update)
- update `updated_at_ms`

On create: revision 1, generated false for current explicit tools, created and
updated timestamps set, empty history. Phase 3 will create generated profiles
and update last-used fields. No rollback tool is added in this phase.

### Store ownership

`SerialHandler` should hold:

```rust
profile_store: Arc<ProfileStore>
```

Remove handler-local `profiles` and `profiles_path`. Add a builder setter for an
injected `Arc<ProfileStore>`. Builder default may use an ephemeral store for
tests/library construction, but production `main.rs` must construct and inject
a persistent store explicitly.

`SerialHandler::new()`/`Default` may retain compatibility by attempting the
default path and warning/falling back to ephemeral on failure. Production must
not use that fallback: `main.rs` must fail startup on invalid persistent data.

In `run_http`, construct one store before `StreamableHttpService::new`, capture
it in the service factory, and clone the same `Arc` into every handler. Do the
same single injection for stdio.

### Tool integration

- `list_profiles`: call `store.list()` and map current `ProfileSummary`; do not
  expose metadata/history yet.
- `open_profile`: use a cloned list/specific profile from store; do not hold a
  store lock while opening serial hardware.
- `save_profile`: build the same live snapshot and selector as today, then call
  `store.upsert`.
- `configure(profile=...)`: call
  `store.update_defaults_preserving_selector`.
- `configure(connection_id=...)`: unchanged and still non-persistent.
- `delete_profile`: call `store.delete`.

Preserve existing duplicate/overwrite/not-found messages where tests or users
rely on them.

### Path selection

Add `--profiles-path <path>` to:

- `Args`
- `VALUE_TAKING_OPTIONS`
- parsing
- help output and README options
- stdio/HTTP startup wiring

Default remains the platform user config directory plus
`serial-mcp/profiles.toml`. Change default-path resolution to return an error if
`dirs::config_dir()` is unavailable. Do not fall back to `.` or `/tmp`.

An explicit relative filename should work; treat its parent as the current
directory when creating files.

## Behavior-first test plan

### Real process restart

Extend `tests/common/spawned.rs` with a profiles-path-aware constructor and an
async stop/reap operation. Add an HTTP integration test:

1. Start actual binary with temp `--profiles-path`.
2. Create profile A through MCP `configure(profile=...)`.
3. Stop/reap process.
4. Start fresh actual binary with same path.
5. `list_profiles` returns profile A and its configured defaults.

This test catches the exact production builder bug; an in-process builder test
does not.

### Shared HTTP sessions

Using in-process `TestServer` with one shared store and two distinct clients:

1. Client A creates profile A.
2. Client B lists and sees A.
3. Client B deletes A.
4. Client A lists and no longer sees A.

### Concurrent same-process writers

Two distinct HTTP clients concurrently create different profiles with
`tokio::join!`. Both calls succeed; a third list sees both. Restart against the
same file and prove both remain.

### Concurrent server-process writers

Start two actual HTTP server processes with different bind ports but the same
temp profile path. Concurrently create different profiles through each. Stop
both, start a third process, and prove both profiles exist. This proves advisory
locking plus reload-under-lock, not merely one shared `Arc`.

### Legacy migration and future rejection

- Prewrite a valid unversioned current-shape TOML file. Start actual server,
  list legacy profile, create another profile, restart, and prove both remain.
  Confirm resulting file declares schema version 2.
- Prewrite `schema_version = 999`; actual binary startup with explicit path must
  exit nonzero and preserve bytes unchanged.
- Prewrite malformed TOML; actual binary startup must exit nonzero and preserve
  bytes unchanged.

### Failed write preservation

On Unix, use an isolated temp directory and public MCP tools:

1. Create profile A successfully.
2. Make profile directory non-writable while keeping cleanup restoration safe.
3. Attempt to create profile B; call returns tool error.
4. `list_profiles` still shows A and not B.
5. Restore permissions, restart actual server, and prove disk still shows A and
   not B.

Gate this test appropriately for Unix. Do not add a production failure-injection
hook solely for testing.

### Appropriate lower-level tests

Unit/property tests may cover pure parser/migration/revision operations:

- v1 to v2 migration
- future version rejection
- revision cap of five
- overwrite metadata preservation
- generated/default name-independent serialization

Do not count builder field equality, private cache values, lock-call mocks, or
`Arc::ptr_eq` as feature acceptance.

## Test harness changes

- `tests/common/mod.rs`: create one `Arc<ProfileStore>` per TestServer and share
  it across handler factories.
- `tests/common/spawned.rs`: add custom args/profile path and graceful async
  stop/reap support without breaking existing `start()`.
- `tests/stdio_integration.rs` or HTTP integration: test CLI help/value parsing
  for `--profiles-path`, including `--profiles-path --version` value-position
  behavior consistent with existing option parsing rules.

## Expected files

- `Cargo.toml`
- `Cargo.lock`
- `src/lib.rs`
- `src/profiles.rs`
- new `src/profile_store.rs`
- `src/server.rs`
- `src/main.rs`
- `src/tools/port_ops.rs`
- `src/tools/types.rs` only if schema guards/imports require it; no MCP shape
  change expected
- `README.md`
- `AGENTS.md`
- `tests/common/mod.rs`
- `tests/common/spawned.rs`
- `tests/http_integration.rs`
- `tests/stdio_integration.rs`
- relevant schema/property tests
- `docs/development/agent-interface-simplification-plan.md` only if executable
  truth requires a small correction
- this handoff document

Avoid unrelated refactors.

## Repository invariants

- Profile tool failures remain MCP tool errors, not protocol-level errors.
- Open allowlist checks remain before `ConnectionManager::open()`.
- Open/close resource notifications remain unchanged.
- No non-standard unsigned JSON Schema formats.
- No production `unwrap`, `expect`, `println!`, `todo!`, or `unimplemented!`
  beyond documented mutex-poison convention.
- Keep current tool count and all tool titles/output schemas.
- Existing profile TOML must migrate without manual editing.
- Existing explicit profile behavior and tool JSON shapes remain compatible.
- Tests prove public persistence/restart/multi-client behavior.
- No attribution footer in commits.

## Verification

Run focused profile/store/migration tests first, then:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Also run relevant real-binary restart and concurrent-process tests explicitly if
test filters are added. No firmware build or hardware tests are required.

## Commit and return requirements

After implementation and all acceptance behavior passes:

1. Inspect `git status`, `git diff`, and recent log.
2. Stage only intended Phase 2 files.
3. Commit completed work with a concise conventional commit message. Multiple
   focused commits are acceptable; do not amend Phase 1 commits.
4. Do not push, merge, open a PR, force-push, or add attribution.
5. Return:
   - files changed
   - externally observable behavior
   - behavior tests and full commands run/results
   - commit hash(es)/messages
   - blockers and deviations
   - suggested Phase 3 follow-up
