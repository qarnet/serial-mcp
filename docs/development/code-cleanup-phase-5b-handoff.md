# Code Cleanup Phase 5B Handoff

## Goal

Replace specialized `TestServer::start_with_*` constructor combinations with
one test-only builder while keeping simple server startup concise. Preserve
every injected dependency, default, profile-store lifetime, and test behavior.

Implement Phase 5B only. Commit completed work before returning.

## In scope

- `tests/common/mod.rs`
- Integration test call sites using specialized `TestServer::start_with_*`
  methods, currently primarily:
  - `tests/http_integration.rs`
  - `tests/allowlist.rs`
  - `tests/serial_pty.rs`
- Other integration test files only if search proves they call a removed
  specialized method
- This handoff document

## Out of scope

- Keep `TestServer::start()` and `TestServer::start_with(manager)` public test
  shortcuts with current behavior and call sites.
- Do not change `connect_client`, notification collectors, spawned server,
  native_sim harness, test cleanup, or production code.
- Do not change any test name, assertion, timeout, platform gate, serial setup,
  profile fixture, security pattern, capture root, or concurrency.
- Do not expose test builder outside integration-test support.
- Do not add generic maps, dynamic downcasts, or closure-based injection.
- Do not change CI workflows.

## Grounding and defaults

`tests/common/mod.rs::TestServer` currently provides:

- `start()`
- `start_with(manager)`
- `start_with_and_security(manager, security)`
- `start_with_capture_store(manager, capture_store)`
- `start_with_provider(manager, provider)`
- `start_with_profiles_path(manager, path)`
- `start_with_profiles_path_and_security(manager, path, security)`
- `start_with_provider_and_profiles_path(manager, provider, path)`

All specialized forms converge on `start_inner` with five inputs.

Preserve defaults:

- Fresh `ConnectionManager` for `start()`.
- Caller manager for builder/`start_with`.
- Empty allowlist security manager.
- Ephemeral profile store when no path/store supplied.
- `SystemPortProvider` when no provider supplied.
- Disabled `CaptureStore` when none supplied.
- One shared `Arc<ProfileStore>` cloned into all session handlers.
- Existing profile path is opened/validated before server starts and the
  resulting store remains alive for server lifetime.

## Exact implementation shape

### 1. Add test-only builder

Add beside `TestServer`:

```rust
pub struct TestServerBuilder {
    manager: Arc<ConnectionManager>,
    security: SecurityManager,
    profile_store: Option<Arc<ProfileStore>>,
    provider: Option<Arc<dyn PortProvider>>,
    capture_store: Option<Arc<CaptureStore>>,
}
```

Use full existing type paths/imports as appropriate.

Expose:

```rust
impl TestServer {
    pub fn builder(manager: Arc<ConnectionManager>) -> TestServerBuilder;
}

impl TestServerBuilder {
    pub fn security(self, security: SecurityManager) -> Self;
    pub fn profiles_path(self, path: PathBuf) -> Self;
    pub fn port_provider(self, provider: Arc<dyn PortProvider>) -> Self;
    pub fn capture_store(self, store: Arc<CaptureStore>) -> Self;
    pub async fn start(self) -> TestServer;
}
```

Names may vary only for clearer consistency. Do not make fields public.

`profiles_path` opens `ProfileStore` immediately with the same
`expect("open profiles store for test server")` behavior as current helpers and
stores its `Arc`. This keeps legacy/restart fixture loading identical.

Builder `start` contains current `start_inner` server construction or delegates
to one private function. There must remain one real construction path.

### 2. Keep simple shortcuts

Implement:

```rust
pub async fn start() -> Self {
    Self::builder(Arc::new(ConnectionManager::new())).start().await
}

pub async fn start_with(manager: Arc<ConnectionManager>) -> Self {
    Self::builder(manager).start().await
}
```

Remove all six specialized combination methods after call-site migration:

- `start_with_and_security`
- `start_with_capture_store`
- `start_with_provider`
- `start_with_profiles_path`
- `start_with_profiles_path_and_security`
- `start_with_provider_and_profiles_path`

### 3. Migrate specialized call sites mechanically

Examples:

```rust
TestServer::builder(manager)
    .security(security)
    .start()
    .await
```

```rust
TestServer::builder(manager)
    .profiles_path(path)
    .port_provider(provider)
    .start()
    .await
```

```rust
TestServer::builder(manager)
    .capture_store(store)
    .start()
    .await
```

Keep argument evaluation/ownership equivalent. Do not merge tests or rewrite
surrounding setup.

### 4. Comments

Document builder defaults once. Remove constructor-specific comments that only
repeat method signatures. Preserve comments explaining profile fixture loading,
one process-wide store, provider sharing, capture-store behavior, and user
config isolation.

## Platform safety

No platform-specific code should change. Still run all-target build/test/clippy
because `tests/common/mod.rs` is compiled into many independent integration test
crates on Linux, macOS, and Windows CI.

## Verification

First prove no removed calls remain:

```bash
rg 'TestServer::start_with_(and_security|capture_store|provider|profiles_path|profiles_path_and_security|provider_and_profiles_path)' tests
```

Expected: no matches. Then run:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test http_integration --locked -- --test-threads=1
cargo test --test allowlist --locked -- --test-threads=1
cargo test --test serial_pty --locked -- --test-threads=1
```

Use `rg` only for this explicit verification if available; otherwise use
repository search tooling and report result.

## Acceptance criteria

- One real in-process TestServer construction path.
- Simple `start`/`start_with` remain concise.
- Specialized dependency combinations use named builder methods.
- All removed methods have zero call sites.
- Defaults and dependency lifetimes unchanged.
- No platform/test behavior changes.
- Requested checks pass with no warnings.
- Diff contains no unrelated cleanup.
- Working tree clean after commit.

## Commit and recap

Before returning:

1. Inspect status, diff, and recent log.
2. Stage only intended test support/call sites and this handoff.
3. Commit with suggested message:

   `refactor: simplify test server setup`

4. Do not push, merge, open PR, amend, force-push, or add attribution.
5. Return files, defaults/platform behavior, tests/results, commit hash/message,
   deviations, blockers, and follow-up concerns.
