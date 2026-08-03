# MCP Version Compatibility — Phase 2 Handoff

## Goal

Replace ambiguous modern/legacy test identities with explicit MCP protocol
version cases and make compatibility coverage visibly complete for every
advertised version. Preserve all current wire behavior and test depth.

Implement this phase, run verification, inspect diff/status/log, and commit the
complete phase. Do not push.

## Worktree and Starting Point

Work only in:

```text
/home/thomas-workstation/repos/serial-mcp-pr50-analysis
```

Starting detached HEAD: `1fc2056a` (`refactor: centralize MCP version policy`).
Worktree must start clean. Read repository `AGENTS.md` and
`docs/development/mcp-version-compatibility-plan.md` first.

## Grounding

- `tests/common/mod.rs` currently has `TestProtocol::{Modern,Legacy}`, a
  no-state `TestClientHandler`, separate `LegacyClientHandler`, and duplicate
  connect helpers whose return types differ.
- `tests/common/spawned.rs` mirrors modern/legacy helper names.
- `tests/protocol_compatibility.rs` contains independent typed and raw-wire
  layers. Preserve that independence: raw expected values must not derive from
  production `src/mcp_protocol.rs`.
- `tests/stdio_integration.rs` duplicates modern/legacy client setup.
- `tests/resource_subscriptions.rs` uses modern/legacy connection helpers but
  its behavior is intentionally version-specific.
- Phase 1's production policy remains crate-private. Integration coverage can
  compare its independent test-case list to `server/discover` wire output; do
  not expose production internals for tests.
- rmcp `ClientHandler::get_info()` can return a `ClientInfo` carrying an exact
  protocol version. One cloneable handler containing the explicit test case can
  serve both lifecycle modes and give common helpers one return type.

## Decided Test Model

### Explicit cases

Keep the public test enum name `TestProtocol`, but replace variants with:

```rust
pub enum TestProtocol {
    V2026_07_28,
    V2025_11_25,
}
```

Add:

```rust
pub const ALL: [Self; 2] = [Self::V2026_07_28, Self::V2025_11_25];
pub fn version(self) -> ProtocolVersion;
pub fn lifecycle(self) -> ClientLifecycleMode;
```

`version()` returns exact rmcp constants. `lifecycle()` maps
`V2026_07_28` to discovery with only that preferred version and
`V2025_11_25` to initialize.

### One versioned handler

Replace `LegacyClientHandler` with:

```rust
#[derive(Clone)]
pub struct VersionedClientHandler {
    protocol: TestProtocol,
}
```

Provide a constructor. Its `ClientHandler::get_info()` returns default client
info tagged with `self.protocol.version()` for both cases.

Keep `TestClientHandler` and generic `connect_client`/`connect_to_url` only for
existing tests that intentionally exercise rmcp defaults. Do not silently
change those generic helpers.

Add primary explicit helpers with one return type:

```text
connect_protocol_client(server, protocol)
connect_protocol_to_url(url, protocol)
```

Thin exact-version convenience helpers are allowed where they keep resource
subscription tests readable, but names must be version-explicit:

```text
connect_2026_07_28_client
connect_2025_11_25_client
connect_2026_07_28_to_url
connect_2025_11_25_to_url
```

Remove `connect_modern_*`, `connect_legacy_*`, and `LegacyClientHandler` after
all call sites migrate.

`tests/common/spawned.rs` receives matching exact-version names and the common
`VersionedClientHandler` return type. Remove semantic modern/legacy helper names.

## `tests/protocol_compatibility.rs`

Preserve both proof layers and all existing assertions. This phase is test
architecture, not coverage reduction.

### Typed layer

- Replace `typed_modern` / `typed_legacy` with one
  `typed_protocol(TestProtocol, ...)` helper using the common versioned handler.
- Consolidate genuinely identical common-surface tests into a loop over
  `TestProtocol::ALL` when doing so improves failure output and does not obscure
  lifecycle-specific behavior.
- Keep version-specific capability/cache/unsupported-method tests explicit.
- Every test name that targets one protocol must include `2026_07_28` or
  `2025_11_25`; do not use modern/legacy as sole identity.
- Assertion messages should include the exact version/case.

Required typed proof for each case remains:

1. negotiated version;
2. exact 25-tool surface;
3. resources/templates/prompts;
4. successful `compute_checksum`;
5. readable `serial://ports`;
6. correct capabilities;
7. correct cache field presence or absence.

### Raw-wire layer

Rename version-bearing helpers and tests to exact-version names, for example:

```text
modern_meta                 -> meta_2026_07_28
modern_capabilities_json    -> capabilities_2026_07_28_json
common_capabilities_json    -> capabilities_2025_11_25_json
raw_legacy_session          -> raw_2025_11_25_session
```

Semantic terms such as “stateless”, “session”, “discovery”, or “initialize”
remain useful descriptions. Avoid “modern”/“legacy” as version keys because
they become ambiguous after a future revision.

Keep exact raw headers/status/error/result/cache assertions independent from
typed rmcp deserialization.

### Coverage lock

Add one public-boundary test that compares:

- independent expected versions from `TestProtocol::ALL`, in order; and
- exact `supportedVersions` returned by raw `server/discover`.

The test must fail on missing, extra, or reordered versions. Do not import or
expose `src/mcp_protocol.rs` internals. This ensures a future production policy
row requires an explicit test case.

## `tests/stdio_integration.rs`

- Replace duplicate modern/legacy setup with one
  `start_stdio_protocol_client(TestProtocol)` returning
  `RunningService<RoleClient, VersionedClientHandler>` and the temp directory.
- Keep generic default-lifecycle setup/tests unchanged unless a mechanical type
  adjustment is required.
- Rename exact lifecycle tests/helpers to version-bearing names.
- Preserve `2026-07-28` subscription-listener cancellation coverage and both
  versions' tool/checksum assertions.

## `tests/resource_subscriptions.rs`

Migrate helper names to exact versions only. Preserve behavior and test scope:

- subscription-positive paths use `connect_2026_07_28_client`;
- rejection/capability-negative paths use `connect_2025_11_25_client`;
- descriptive variable names may say stateless/session, but version identity in
  helper/test names must be exact.

Do not refactor event-hub, watcher, session, or subscription implementation.

## Documentation in This Phase

Add this handoff document to the phase commit. Update only comments/docstrings
in touched test files as needed. Durable README/AGENTS/CHANGELOG/FEATURES and
drift updates remain Phase 4.

## Out of Scope

- Production protocol policy changes.
- New protocol versions or pre-`2025-11-25` support.
- Historical rmcp 1.7 fixture.
- Conformance runner/CI changes.
- Test deletion or assertion weakening.
- Product tool/resource/prompt behavior.
- Expected-failure baseline changes.

## Verification

Run:

```bash
cargo fmt --all -- --check
cargo test --locked --test protocol_compatibility
cargo test --locked --test stdio_integration
cargo test --locked --test resource_subscriptions
cargo clippy --all-targets --locked -- -D warnings
```

Also search changed tests and common helpers to confirm no obsolete symbols
remain:

```text
TestProtocol::Modern
TestProtocol::Legacy
LegacyClientHandler
connect_modern
connect_legacy
typed_modern
typed_legacy
```

Ordinary prose may still use “modern” or “legacy” when explaining lifecycle
semantics; symbol/test identity must use exact versions.

## Commit and Recap

Stage only Phase 2 files and this handoff. Commit:

```text
test: index compatibility matrix by MCP version
```

No attribution footer. Do not push, merge, amend, or open a PR.

Return recap with files, test architecture, coverage preservation, commands and
results, commit hash/message, git status, deviations, blockers, and follow-up.

Stop and escalate before committing if two materially different attempts fail,
rmcp types prevent one versioned handler without unsafe/type erasure, coverage
would need weakening, or scope must expand. Preserve evidence and partial state.
