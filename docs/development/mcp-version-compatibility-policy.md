# MCP version compatibility policy

This document defines which MCP protocol versions serial-mcp supports, how each
version behaves, how compatibility is proven, and how a future version may be
admitted. The executable contract is `src/mcp_protocol.rs`. The verification
gate is `scripts/test-mcp-compat.sh`.

## Supported versions

| Protocol version | Status | Lifecycle | Resource subscriptions | SEP-2549 cache fields |
|---|---|---|---|---|
| `2026-07-28` | Preferred | `server/discover` + stateless per-request `_meta` | Enabled (`resources.subscribe: true`) | `ttlMs: 0` / `cacheScope: "private"` on every cacheable family |
| `2025-11-25` | Permanent | `initialize` + session state | Disabled | Omitted (never sent to legacy peers) |

`2026-07-28` is the preferred version. It uses discovery-based stateless
requests, modern capability advertisement, and immediate-private cache fields.
`2025-11-25` remains supported permanently through the initialize and session
lifecycle, with subscriptions disabled and no cache fields on the wire.

## Single source of truth

A protocol version is supported ONLY when it has an exact row in the
product-owned policy table in `src/mcp_protocol.rs`
(`SUPPORTED_PROTOCOLS`). Support is never inferred from any of these facts:

- rmcp's `ProtocolVersion::KNOWN_VERSIONS` list;
- a version's date sorting after or before a supported version;
- a current client tolerating the version;
- the version being present in the pinned historical client's SDK.

Unknown, custom, and future protocol versions receive no policy, no
capability view, and no version-specific response fields.

## Compatibility directions

- Required in the product gate: the current server works with old clients.
  A real historical client compiled against exact `rmcp 1.7.0` proves this for
  the permanent legacy version over both HTTP and stdio. Current-SDK typed and
  raw-wire tests provide additional proof.
- Outside the product gate: new clients against historical serial-mcp server
  binaries. serial-mcp ships a server, not a client library. An old binary
  cannot regress from current source, so this direction is not part of the
  compatibility contract.

## Required proof layers

The proof layers are independent. No layer substitutes for another. Policy
unit tests, typed current-SDK cases, raw wire, stdio, and official conformance
apply to EVERY advertised version. The actual historical client applies to
retained older lifecycle and version rows, including permanent `2025-11-25`.
The Inspector smoke applies to the preferred version, `2026-07-28`.

1. Policy unit tests (`src/mcp_protocol.rs`) check exact rows, preferred order,
   and the absence of policy for unknown or future versions.
2. Typed current-SDK cases (`tests/protocol_compatibility.rs`) use real rmcp
   clients negotiated through the exact lifecycle. They assert the surface,
   capabilities, and cache-field presence or absence.
3. Raw wire (`tests/protocol_compatibility.rs`) uses hand-built JSON-RPC POSTs
   against the spawned binary. It asserts exact HTTP status, JSON-RPC error
   codes, headers, and response fields. Production helpers are independent of
   these expectations, so implementation and expectation cannot fail together.
4. Stdio (`tests/stdio_integration.rs`) covers both lifecycles over the spawned
   binary's stdin/stdout transport.
5. The actual historical client (`compat/rmcp-1-client/`, exact `rmcp 1.7.0`)
   is a real pre-migration client over HTTP and stdio. It proves historical
   interoperability rather than current-SDK compatibility mode.
6. Official conformance uses the pinned
   `@modelcontextprotocol/conformance@0.2.0-alpha.10` at the exact
   `--spec-version` for each version. It is installed from the committed
   `compat/mcp-validation/` lockfile and invoked as the local
   `node_modules/.bin/conformance` binary (never npx).
7. Inspector interoperability uses the pinned
   `@modelcontextprotocol/inspector@2.0.0` smoke against the preferred version.
   This is interoperability, not conformance, and uses the local
   `node_modules/.bin/mcp-inspector` binary.

## Permanent `2025-11-25` retention

MCP `2025-11-25` compatibility is a permanent product requirement. A future
protocol or rmcp dependency update must not remove or weaken its policy row,
historical-client fixture, raw-wire tests, conformance scenario set, or drift
guards. Adding a new version is strictly additive. It never mutates,
deprecates, or evicts the `2025-11-25` row.

## Future-version admission checklist

For every new MCP protocol version to be supported:

1. Upgrade or pin rmcp only after confirming exact version support.
2. Add one explicit `ProtocolPolicy` row with preferred ordering, lifecycle,
   capability switches, and cache policy.
3. Keep all older rows unchanged by default.
4. Add an exact typed test case and raw-wire lifecycle, header, and error
   assertions.
5. Add a stdio proof.
6. Add official conformance scenarios with the exact `--spec-version`.
7. Run the preferred-version Inspector smoke.
8. Retain the historical-client gates for every older advertised lifecycle.
9. Update policy docs, README, CHANGELOG, and drift expectations.
10. Run the shared compatibility runner and the full repository gate before
    merge.

No version is supported merely because rmcp knows it, its date is newer, or a
current client tolerates it. Pre-`2025-11-25` revisions are not supported.
They are tracked only as a demand-driven feature idea in the product backlog
(`docs/BACKLOG.md`).

## Local verification

The one complete local and CI MCP version gate is:

```bash
bash scripts/test-mcp-compat.sh
```

It runs, in order: the locked serial-mcp build; the lockfile-pinned MCP
validation tooling install (`npm ci --ignore-scripts` in
`compat/mcp-validation/`, with lifecycle scripts disabled, no npx, and no dynamic
package resolution); the focused Rust protocol, stdio, and subscription
tests; the locked historical `rmcp 1.7.0` fixture build; the fixture over
stdio; an isolated loopback HTTP server with a bounded `server/discover`
readiness probe; the fixture over HTTP; the exact legacy (`2025-11-25`) and
modern (`2026-07-28`) official conformance scenario sets via the local
`node_modules/.bin/conformance` binary; and the pinned Inspector smoke via
the local `node_modules/.bin/mcp-inspector` binary. Environment overrides
select paths and port only. Protocol versions, package pins, and scenario sets
are fixed in the script and the committed npm lockfile.

### Exact pins and report location

- Conformance package: `@modelcontextprotocol/conformance@0.2.0-alpha.10`
  (exact direct version, with no `^`, `~`, tags, or ranges).
- Inspector package: `@modelcontextprotocol/inspector@2.0.0`.
- Both live in the committed npm project `compat/mcp-validation/` (private
  `package.json`, committed `package-lock.json` with exact versions and
  `sha512-` integrity for every locked package, including transitives). Install
  the tree ONLY from that lockfile with `npm ci --ignore-scripts`. Invoke the
  local `node_modules/.bin` binaries directly. Validation never uses `npx` and
  never runs package lifecycle scripts, which is preinstall supply-chain
  hardening. `node_modules/` is gitignored. Only lockfile semantics are
  committed.
- Node `22.19.0` and Rust `1.97.1` in CI.
- Historical client: `rmcp = "=1.7.0"` with `default-features = false` and
  only the required client/transport features, resolved in the fixture's
  committed `Cargo.lock` (checksum
  `0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e`).
- Reports land under `target/conformance-results/` with one
  `<scenario>-2025-11-25` / `<scenario>-2026-07-28` directory per scenario.

### Expected failures and forbidden practices

`conformance/expected-failures.yaml` baselines exactly the four documented
fixture-dependent checks, all under `server-stateless:sep-2575-*`. A baseline
entry that starts passing fails the run as stale. Any other failure is an
unexpected regression. The runner never runs `--suite all`, never suppresses a
runner exit status (`set -euo pipefail` plus GNU `timeout` per scenario), and
never adds fixture endpoints, fake resources, or conformance-only methods to
the production server.

## Industry rationale

### Docker Engine API

Docker Engine API negotiates the highest shared version, disables newer
features on downgrade, publishes an explicit version matrix, and treats
removal of an old API floor as a deliberate deprecation decision rather than an
incidental SDK update. serial-mcp keeps an ordered, explicit product-owned list
and never infers support from rmcp's broader known-version list.
(<https://docs.docker.com/reference/api/engine/>)

### Kubernetes version skew

Kubernetes version skew publishes exact supported skew windows as part of the
public contract. Unsupported skew is not left to best-effort behavior.
serial-mcp documents its support window as exactly the advertised set, and
adding a version never implicitly evicts an older one.
(<https://kubernetes.io/releases/version-skew-policy/>)

### Git protocol v2

Git protocol v2 negotiates the protocol version and then advertises
capabilities. Clients request only advertised commands, ignore unknown
capability keys, and treat agent and version strings as informational.
serial-mcp treats protocol version and advertised capabilities as separate
contracts.
(<https://git-scm.com/docs/gitprotocol-v2>)

### Protocol Buffers evolution

Protocol Buffers evolution favors additive changes, preservation of unknown
fields, and permanent reservation of removed identifiers. It tests old and new
implementations in both directions. serial-mcp keeps MCP JSON response
evolution additive inside a version and proves field absence as well as
presence on the raw wire.
(<https://protobuf.dev/programming-guides/proto2/#updating>)

### Connect/gRPC conformance

Connect/gRPC conformance separates specification conformance from
interoperability and tests servers with independent client implementations.
serial-mcp keeps three independent gates: official MCP conformance, Inspector
interoperability, and an actual historical rmcp client.
(<https://github.com/connectrpc/conformance>)
