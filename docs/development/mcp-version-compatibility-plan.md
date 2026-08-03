# MCP Version Compatibility Plan

> **Status: implemented.**
>
> - Phase 1 `refactor: centralize MCP version policy` — `1fc2056a`.
> - Phase 2 `test: index compatibility matrix by MCP version` — `035027d8`;
>   Phase 2 review-fix `docs: correct compatibility test scope` — `ae466564`.
> - Phase 3 `test: add historical MCP client compatibility gate` — `4af2d5ae`;
>   Phase 3 review-fix `test: tighten historical client compatibility checks`
>   — `51602016`.
> - Phase 4 (`docs: define MCP version compatibility policy`) completes the
>   durable policy document, the cross-file drift guards, the historical
>   fixture pin, and full repository/Nix verification. It does not predict
>   its own commit hash; the commit that carries this status line is the
>   Phase 4 commit.

## Status and Goal

This plan follows the rmcp 3 migration. The current server already supports
MCP `2026-07-28` through discovery/stateless requests and MCP `2025-11-25`
through initialize/session requests. Existing typed, raw-wire, stdio,
conformance, and Inspector tests pass locally.

Goal: turn that two-version implementation into an explicit, repeatable
compatibility system. Any future additional MCP version must be added through
one reviewed policy entry and one complete test row; adding a new version
must not silently change behavior for any older advertised version.

User-observable behavior to preserve:

- `2026-07-28` remains preferred and uses `server/discover` plus stateless
  request context.
- `2025-11-25` remains supported and uses `initialize` plus session state.
- Both versions expose the same 25 serial-mcp tools and core resources/prompts.
- Version-specific capabilities, methods, error behavior, and response fields
  remain isolated.
- A real client compiled against historical `rmcp 1.7.0` can still use the
  current server over HTTP and stdio.

## Scope

In scope:

1. Central product-owned protocol policy table.
2. Explicit version-indexed typed and raw-wire test matrix.
3. Real historical rmcp `1.7.0` client fixture.
4. One pinned local/CI compatibility runner.
5. CI and drift guards that require complete coverage for every advertised
   version.
6. Durable support policy, including permanent `2025-11-25` compatibility,
   and contributor documentation.

Out of scope:

- Adding another MCP protocol version now.
- Removing `2025-11-25` support. This version is a permanent compatibility
  requirement for serial-mcp, not a candidate for later deprecation.
- Adding conformance-only methods, fake resources, or fixture endpoints to the
  production server.
- Testing a new client against an old serial-mcp server. serial-mcp ships a
  server; this plan guarantees new-server/old-client compatibility.
- Promising compatibility for versions not explicitly advertised by the
  server.
- Floating npm or Cargo dependencies.
- Replacing official MCP conformance with project-specific tests, or treating
  Inspector as conformance.

## Current Grounding

- `src/server.rs` owns the exact supported slice
  `[V_2026_07_28, V_2025_11_25]`, separate modern/legacy capability views,
  initialize/discover lifecycle gates, and protocol-dependent cache fields.
- `modern_cache_fields()` currently enables cache fields by lexical date
  comparison (`>= 2026-07-28`). That is safe while only the two current
  versions are advertised, but a future advertised version would inherit this
  policy without an explicit review.
- `tests/common/mod.rs` models only `TestProtocol::Modern` and
  `TestProtocol::Legacy`. Those labels become ambiguous after a third version.
- `tests/protocol_compatibility.rs` already has two strong proof layers:
  current rmcp typed clients in both lifecycle modes and exact raw HTTP wire
  assertions against the real binary.
- `tests/stdio_integration.rs` covers both current lifecycle modes over stdio.
- `.github/workflows/ci.yml` runs exact pinned conformance scenario sets for
  both versions and pinned Inspector `2.0.0` for the preferred version.
- `tests/doc_drift.rs` pins external tool versions, scenario lists, expected
  failures, and dual-version documentation.
- Before this migration, `origin/main` used exact resolved `rmcp 1.7.0`
  (`Cargo.lock` checksum
  `0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e`).
  Current typed legacy tests use rmcp 3 compatibility mode, not that historical
  client implementation.

## Industry Research and Applied Decisions

### Docker Engine API

Docker advertises minimum and maximum API versions and negotiates the highest
version shared by client and daemon. Downgrading disables newer features and
adjusts requests/responses for the negotiated API. Docker also publishes an
explicit version matrix and treats removal of an old API floor as a deliberate
deprecation decision, not an incidental SDK update.

Source: <https://docs.docker.com/reference/api/engine/>

Applied decision: serial-mcp keeps an ordered, explicit product-owned list,
selects one exact protocol policy, and never infers support from rmcp's broader
known-version list.

### Kubernetes Version Skew

Kubernetes publishes exact supported skew windows between clients and server
components. Upgrade order and compatibility boundaries are part of the public
contract; unsupported skew is not left to best-effort behavior.

Source: <https://kubernetes.io/releases/version-skew-policy/>

Applied decision: serial-mcp documents its support window as the complete set
of advertised protocol versions. Adding a version never implicitly evicts an
older version, and `2025-11-25` remains permanently supported.

### Git Protocol v2

Git negotiates protocol version, then advertises capabilities. Clients request
only advertised commands/features and ignore unknown capability keys. Git's
agent/version string is informational and must not be used as a proxy for
feature support.

Source: <https://git-scm.com/docs/gitprotocol-v2>

Applied decision: serial-mcp treats protocol version and advertised
capabilities as separate contracts. Features come from the selected policy;
client implementation names or dates do not enable features.

### Protocol Buffers Evolution

Protocol Buffers favors additive changes, preservation of unknown fields, and
permanent reservation of removed identifiers. Old and new implementations are
tested in both serialization directions rather than assuming current generated
code accurately represents old behavior.

Source: <https://protobuf.dev/programming-guides/proto2/#updating>

Applied decision: MCP JSON response evolution stays additive inside a version;
version-specific fields are explicitly included or omitted. Raw-wire tests
remain the contract proof and must inspect absence as well as presence.

### Connect/gRPC Conformance

Connect's conformance suite separates specification conformance from
interoperability and tests servers with reference clients from independent
protocol implementations. This catches defects hidden when client and server
share one implementation.

Source: <https://github.com/connectrpc/conformance>

Applied decision: keep three independent gates: official MCP conformance,
Inspector interoperability, and an actual historical rmcp client. No one gate
substitutes for another.

## Compatibility Contract

### Support policy

- Supported versions are exactly those in the product policy table, ordered
  preferred first.
- New protocol admission is additive. Existing rows remain unchanged unless a
  separate compatibility decision explicitly changes them.
- MCP `2025-11-25` is permanent product compatibility. Its policy row,
  historical-client fixture, raw-wire tests, and conformance matrix must not be
  removed by a future protocol or rmcp dependency update.
- Unknown/unadvertised protocol versions receive no inferred policy and no
  future-only fields.
- Capability use, not client name/version, controls optional behavior.

### Required proof per advertised version

Every policy row must have:

1. Exact negotiation assertion.
2. Correct lifecycle assertion.
3. Exact capability snapshot.
4. Tool/resource/template/prompt surface proof.
5. One hardware-free real tool call.
6. Version-correct response metadata proof, including forbidden-field absence.
7. Unsupported-method and malformed-request proofs.
8. HTTP raw-wire proof.
9. Stdio lifecycle proof.
10. Official conformance scenario set at the exact version.
11. At least one independent implementation smoke: historical client for
    retained legacy versions, Inspector for the preferred version.

## Decided Implementation Shape

### Product policy model

Add `src/mcp_protocol.rs` with crate-private explicit types:

```rust
enum ProtocolLifecycle {
    InitializeSession,
    DiscoverStateless,
}

enum CachePolicy {
    Omit,
    ImmediatePrivate,
}

struct ProtocolPolicy {
    version: ProtocolVersion,
    lifecycle: ProtocolLifecycle,
    cache: CachePolicy,
    resource_subscriptions: bool,
}
```

The module owns an ordered `SUPPORTED_PROTOCOLS` table with exactly two rows.
It exposes lookup by exact `ProtocolVersion`, preferred policy, and ordered
supported versions. No date/range comparison is allowed.

`src/server.rs` must consume this policy for:

- `supported_protocol_versions()` and `discover().supportedVersions`;
- preferred `get_info()` / discovery info;
- legacy initialize admission and returned info;
- capability construction;
- cache-field inclusion for all cacheable response families.

An unknown version maps to no policy. Defensive response shaping omits optional
version-specific fields when no exact policy exists.

### Test protocol model

Replace semantic labels `Modern`/`Legacy` in `tests/common/mod.rs` with explicit
variants:

```text
V2026_07_28
V2025_11_25
```

Use one version-configurable typed client handler where rmcp permits it, with
the lifecycle selected by the explicit case. Keep thin named connection helpers
only when they improve test readability. Test names must contain exact version
or clearly run every case from a table.

Raw-wire assertions remain independent from production helpers. They must not
derive expected capabilities or expected fields from `src/mcp_protocol.rs`,
because that would let implementation and expectation fail together.

### Historical client fixture

Add standalone, non-workspace package:

```text
compat/rmcp-1-client/Cargo.toml
compat/rmcp-1-client/Cargo.lock
compat/rmcp-1-client/src/main.rs
```

Requirements:

- exact dependency `rmcp = "=1.7.0"` with only required client transports;
- `publish = false`;
- own committed lockfile;
- no dependency on serial-mcp library internals;
- HTTP mode accepts server URL;
- stdio mode accepts current serial-mcp binary path and spawns it with an
  isolated temporary profile path;
- asserts negotiated protocol `2025-11-25`, server identity, exact 25-tool
  names, expected resources/templates/prompts, and successful
  `compute_checksum` result (`111` / `6F`);
- bounded execution, clear nonzero failure, clean client/server shutdown;
- prints one small JSON success summary for diagnostics.

This fixture proves historical implementation interoperability. It does not
restore removed serial-mcp-specific streaming tools; exact current 25-tool
surface is the supported product contract.

### Single compatibility runner

Add Linux/CI runner:

```text
scripts/test-mcp-compat.sh
```

It becomes the only owner of external scenario lists and runs:

1. locked serial-mcp binary build;
2. focused Rust protocol, stdio, and subscription tests;
3. locked historical fixture build;
4. historical rmcp 1.7 stdio smoke;
5. isolated loopback HTTP server with bounded discovery readiness probe;
6. historical rmcp 1.7 HTTP smoke;
7. exact legacy and modern official conformance scenario sets;
8. pinned Inspector smoke against preferred version;
9. cleanup through a shell trap;
10. stable reports under `target/conformance-results/`.

Use exact package pins already accepted:

```text
@modelcontextprotocol/conformance@0.2.0-alpha.10
@modelcontextprotocol/inspector@2.0.0
Node 22.19.0 in CI
```

The script must never run `--suite all`, suppress runner exit status, add
product fixture endpoints, or weaken `conformance/expected-failures.yaml`.
Environment overrides may select binary, URL/port, and report directory, but
must not silently change protocol versions or package pins.

`.github/workflows/ci.yml` keeps environment setup, timeout, artifact upload,
and server-log diagnostics, but delegates compatibility execution to this
script. Local and CI behavior therefore share one executable path.

### Drift enforcement

Extend `tests/doc_drift.rs` to verify:

- policy documentation lists exactly both supported versions in preferred
  order;
- compatibility runner has one exact scenario set per supported version;
- npm, Node, and historical rmcp pins remain exact;
- fixture lock resolves rmcp `1.7.0` with the historical checksum;
- `--suite all` remains forbidden;
- expected-failure baseline remains exactly the four accepted IDs;
- README claims match current policy;
- CI invokes the shared runner rather than duplicating scenario loops.

Do not make drift tests parse Rust source syntax as the primary behavior proof.
Unit tests exercise policy values; drift tests guard cross-file release/config
contracts.

## Phased Plan

### Phase 1 — Explicit Protocol Policy

Files:

- add `src/mcp_protocol.rs`;
- update `src/lib.rs`;
- update `src/server.rs`;
- update server unit tests in `src/server.rs` or move policy-focused tests into
  `src/mcp_protocol.rs`.

Work:

1. Add exact two-row policy model.
2. Route supported-version advertisement, lifecycle admission, capability
   view, and cache shaping through exact policy lookup.
3. Remove `modern_cache_fields()` date comparison and modern/legacy policy
   duplication where the new table replaces it.
4. Keep current wire behavior byte-for-byte equivalent where fields/order are
   observable.
5. Add tests proving unknown/future versions inherit no policy and both known
   rows produce exact expected behavior.

Verification:

```bash
cargo fmt --all -- --check
cargo test --locked --lib mcp_protocol
cargo test --locked --lib server::tests
cargo test --locked --test protocol_compatibility
```

Acceptance: both existing versions preserve exact negotiation/capability/cache
behavior; an arbitrary future version receives no modern fields merely because
its date sorts later.

### Phase 2 — Version-Indexed Compatibility Matrix

Files:

- update `tests/common/mod.rs`;
- update `tests/common/spawned.rs`;
- update `tests/protocol_compatibility.rs`;
- update `tests/stdio_integration.rs`;
- update `tests/resource_subscriptions.rs` only for renamed helpers/cases.

Work:

1. Replace `Modern`/`Legacy` identifiers with exact protocol-version cases.
2. Preserve typed and raw-wire proof independence.
3. Make common-surface tests table-driven where one body can honestly exercise
   both versions.
4. Keep lifecycle-specific positive and negative tests separate and explicit.
5. Add a coverage assertion listing exactly every advertised version, so a new
   policy row cannot land without a test case.

Verification:

```bash
cargo fmt --all -- --check
cargo test --locked --test protocol_compatibility
cargo test --locked --test stdio_integration
cargo test --locked --test resource_subscriptions
```

Acceptance: failure output names exact version; adding a third advertised
version creates an obvious missing-case failure rather than silently classifying
it as “modern.”

### Phase 3 — Historical Client and Shared Runner

Files:

- add `compat/rmcp-1-client/Cargo.toml`;
- add `compat/rmcp-1-client/Cargo.lock`;
- add `compat/rmcp-1-client/src/main.rs`;
- add `scripts/test-mcp-compat.sh`;
- update `.github/workflows/ci.yml`.

Work:

1. Build minimal exact rmcp 1.7 client from historical APIs demonstrated by
   `origin/main` tests.
2. Exercise current server over both HTTP and stdio.
3. Move existing conformance scenario loops from CI YAML into shared runner.
4. Keep CI's exact Node/Rust setup, 15-minute job bound, cleanup, and report
   upload.
5. Ensure every failure path preserves server log and nonzero exit.

Verification:

```bash
cargo fmt --manifest-path compat/rmcp-1-client/Cargo.toml --all -- --check
cargo build --locked --manifest-path compat/rmcp-1-client/Cargo.toml
cargo clippy --locked --manifest-path compat/rmcp-1-client/Cargo.toml --all-targets -- -D warnings
bash scripts/test-mcp-compat.sh
```

Acceptance: current binary works with actual rmcp 1.7 over both transports;
shared runner reproduces current official legacy/modern conformance and modern
Inspector results.

### Phase 4 — Policy, Drift Guards, and Full Gate

Files:

- add `docs/development/mcp-version-compatibility-policy.md`;
- update `README.md`;
- update `CHANGELOG.md`;
- update `AGENTS.md`;
- update `tests/doc_drift.rs`;
- update `docs/development/mcp-version-compatibility-plan.md` status.

Work:

1. Publish compatibility contract, version-admission checklist, and permanent
   `2025-11-25` retention rule.
2. Document one local command for complete MCP compatibility testing.
3. Add pre-`2025-11-25` MCP revisions to `FEATURES.md` as a potential future
   feature, explicitly demand-driven and not part of current support.
4. Add drift guards described above, including historical lock checksum.
5. Run all repository and Nix gates after compatibility runner succeeds.

Verification:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --test config_schema_validation
cargo test --locked --test doc_drift
bash scripts/test-mcp-compat.sh
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
nix flake check
```

Acceptance: one documented local runner proves both protocol versions with
current SDK, raw wire, actual old SDK, official conformance, and Inspector;
full repository gate remains green.

## Future MCP Version Admission Checklist

For every new MCP version:

1. Upgrade/pin rmcp only after confirming exact version support.
2. Add one explicit `ProtocolPolicy` row, preferred ordering, lifecycle,
   capability switches, and cache policy.
3. Keep all older rows unchanged by default.
4. Add exact typed test case and raw-wire lifecycle/header/error assertions.
5. Add stdio proof.
6. Add official conformance scenarios with exact `--spec-version`.
7. Run preferred-version Inspector smoke.
8. Retain historical-client gates for every older advertised lifecycle.
9. Update policy docs, README, CHANGELOG, and drift expectations.
10. Run shared compatibility runner and full repository gate before merge.

No version is supported merely because rmcp knows it, its date is newer, or a
current client tolerates it.

## Commit and Review Sequence

Use one reviewed commit per phase:

1. `refactor: centralize MCP version policy`
2. `test: index compatibility matrix by MCP version`
3. `test: add historical MCP client compatibility gate`
4. `docs: define MCP version compatibility policy`

Each phase gets Executor implementation, requested verification, commit, and
Thinker review before next phase. Executor must stop and escalate rather than
weaken tests, alter expected-failure baselines, add fixture behavior to the
server, or invent a new compatibility policy.
