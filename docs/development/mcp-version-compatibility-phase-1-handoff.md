# MCP Version Compatibility — Phase 1 Handoff

## Goal

Centralize serial-mcp's two supported MCP protocol contracts in one exact,
product-owned policy table. Preserve current `2026-07-28` and `2025-11-25`
wire behavior while preventing an unknown future version from inheriting
features merely because its date sorts after `2026-07-28`.

Implement this phase, run its verification, inspect diff/status/log, and commit
the complete phase. Do not push.

## Worktree and Starting Point

Work only in:

```text
/home/thomas-workstation/repos/serial-mcp-pr50-analysis
```

Starting HEAD is detached at `d24befb5`. This is intentional. Existing
untracked planning documents are part of this phase and must be committed with
the implementation:

```text
docs/development/mcp-version-compatibility-plan.md
```

Do not touch `/home/thomas-workstation/repos/serial-mcp`; it has unrelated user
work.

## Grounding

- Full plan: `docs/development/mcp-version-compatibility-plan.md`.
- Current version/lifecycle implementation: `src/server.rs`, especially
  `modern_cache_fields`, `SUPPORTED_PROTOCOL_VERSIONS`, capability/info
  helpers, and `ServerHandler::{get_info,initialize,supported_protocol_versions,discover}`.
- Current rmcp `ProtocolVersion` is `Clone` but not `Copy`; its known constants
  are defined in the pinned rmcp 3.0.1 source. Clone versions when producing
  owned vectors/info.
- `RequestContext::protocol_version()` returns `Option<ProtocolVersion>`.
- Current public behavior is proved by `tests/protocol_compatibility.rs`; do
  not change that test architecture in Phase 1.
- Current exact supported order is `2026-07-28`, then `2025-11-25`.
- `2025-11-25` is permanent product compatibility.
- Production convention: no `unwrap`, `expect`, `println!`, `todo!`, or
  `unimplemented!`.

## In Scope

### 1. Add `src/mcp_protocol.rs`

Add crate-private explicit policy types:

```rust
pub(crate) enum ProtocolLifecycle {
    InitializeSession,
    DiscoverStateless,
}

pub(crate) enum CachePolicy {
    Omit,
    ImmediatePrivate,
}

pub(crate) struct ProtocolPolicy {
    pub(crate) version: ProtocolVersion,
    pub(crate) lifecycle: ProtocolLifecycle,
    pub(crate) cache: CachePolicy,
    pub(crate) resource_subscriptions: bool,
}
```

Derive only traits useful to implementation/tests (`Debug`, `Clone`,
`PartialEq`, `Eq` as appropriate). Do not derive `Copy` around
`ProtocolVersion`.

Own one ordered static/const table with exactly these rows:

| Version | Lifecycle | Cache | Resource subscriptions |
|---|---|---|---|
| `V_2026_07_28` | `DiscoverStateless` | `ImmediatePrivate` | `true` |
| `V_2025_11_25` | `InitializeSession` | `Omit` | `false` |

Expose crate-private helpers:

- exact lookup by `&ProtocolVersion` returning `Option<&'static ProtocolPolicy>`;
- preferred policy (first row, without fallible indexing or production
  `unwrap`/`expect`);
- ordered supported-version `Vec<ProtocolVersion>` cloned from policy rows;
- a policy/cache query suitable for `Option<ProtocolVersion>` response
  contexts, if that keeps `server.rs` call sites small.

Do not retain a separate hand-maintained version array. Policy table is the
single production source for advertised versions.

Exact-match rule: `V_2026_07_28` receives modern cache fields; arbitrary custom
or older known versions that have no policy receive no policy and no cache
fields. No lexical/date/range comparison.

Add `pub(crate) mod mcp_protocol;` in `src/lib.rs`.

### 2. Route `src/server.rs` through policy

Remove `SUPPORTED_PROTOCOL_VERSIONS` and the date-based
`modern_cache_fields()` implementation.

Keep response helper names clear; rename `modern_read_result` to a
policy-neutral name if needed. Every cacheable response family must use exact
policy lookup:

- `tools/list`;
- `resources/list`;
- `resources/templates/list`;
- every `resources/read` result;
- `prompts/list`.

Build capabilities from `ProtocolPolicy.resource_subscriptions`:

- both rows retain tools/resources/prompts/completions;
- only `2026-07-28` enables resources subscribe;
- no logging, list-changed flags, tasks, or unrelated capabilities.

Build `ServerInfo` from a selected policy rather than separate hard-coded
version helpers. Thin `legacy_initialize_info` / `modern_discovery_info`
wrappers may remain only if they delegate to policy lookup and improve clarity;
they must not duplicate version/feature decisions.

Lifecycle behavior:

- `get_info()` returns preferred-policy (`2026-07-28`) info because rmcp uses
  it for modern subscription-filter intersection.
- `supported_protocol_versions()` returns `Cow::Owned` from the policy table in
  preferred order. Do not add another borrowed static version list.
- `discover()` returns the same ordered supported versions and preferred info.
- `initialize()` performs exact policy lookup and admits only
  `InitializeSession`; all other versions return existing method-not-found
  before peer bookkeeping.
- Accepted legacy initialize still calls
  `context.peer.set_peer_info(request.clone())` before returning info.

Do not attempt to override rmcp's own transport-level version semantics. This
phase centralizes serial-mcp-owned policy only.

### 3. Unit proofs

Add focused tests, preferably in `src/mcp_protocol.rs`, proving:

1. Exact ordered rows are `2026-07-28`, `2025-11-25`.
2. Preferred row is `2026-07-28` discovery/stateless with immediate-private
   cache and subscriptions enabled.
3. `2025-11-25` is initialize/session with omitted cache and subscriptions
   disabled.
4. Known unsupported older versions (`2025-06-18`, `2025-03-26`,
   `2024-11-05`) return no policy.
5. A deserialized custom future version such as `2099-01-01` returns no policy
   and does not enable cache fields.

Update existing `src/server.rs` tests to assert behavior through policy. Remove
the old test expectation that arbitrary future dates enable modern cache
fields. Preserve capability-view and lifecycle assertions.

Public-boundary regression proof remains:

```bash
cargo test --locked --test protocol_compatibility
```

It must prove user-observable old/new behavior remained unchanged.

## Out of Scope

- Renaming `TestProtocol::Modern` / `Legacy`; Phase 2 owns that.
- Historical rmcp 1.7 fixture; Phase 3 owns that.
- Shared conformance runner or CI changes; Phase 3 owns that.
- README, FEATURES, CHANGELOG, AGENTS, or drift changes; Phase 4 owns those.
- Supporting MCP versions older than `2025-11-25`; this becomes a potential
  feature only in Phase 4 documentation.
- Adding a future MCP version.
- Any tool/resource/prompt/schema or serial behavior change.
- Any change to conformance expected failures.

## Verification

Run from worktree root:

```bash
cargo fmt --all -- --check
cargo test --locked --lib mcp_protocol
cargo test --locked --lib server::tests
cargo test --locked --test protocol_compatibility
cargo clippy --all-targets --locked -- -D warnings
```

If a command filter discovers zero tests, fix test/module naming or run the
smallest correct broader command; report exact replacement.

## Commit and Recap

Stage only Phase 1 files plus the two compatibility planning documents. Commit:

```text
refactor: centralize MCP version policy
```

No attribution footer. Do not push, merge, amend, or open a PR.

Return one recap containing:

1. Files changed.
2. Policy/API shape implemented.
3. User-observable behavior preserved.
4. Exact verification commands and results.
5. Commit hash/message.
6. Current git status.
7. Deviations, blockers, or suggested follow-up.

Stop and escalate before committing if two materially different attempts fail,
rmcp behavior contradicts this handoff, policy requires architecture invention,
tests would need weakening, or scope must expand. Preserve evidence and current
worktree state when escalating.
