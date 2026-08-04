# MCP Version Compatibility Policy

Durable product contract for which MCP protocol versions serial-mcp supports,
how each version behaves, how compatibility is proven, and how a future
version may be admitted. This is the policy document; the executable contract
is `src/mcp_protocol.rs` and the verification gate is
`scripts/test-mcp-compat.sh`.

## Supported versions

| Protocol version | Status | Lifecycle | Resource subscriptions | SEP-2549 cache fields |
|---|---|---|---|---|
| `2026-07-28` | Preferred | `server/discover` + stateless per-request `_meta` | Enabled (`resources.subscribe: true`) | `ttlMs: 0` / `cacheScope: "private"` on every cacheable family |
| `2025-11-25` | Permanent | `initialize` + session state | Disabled | Omitted (never sent to legacy peers) |

`2026-07-28` is the preferred version: discovery-based, stateless requests,
modern capability advertisement, and immediate-private cache fields. The
`2025-11-25` version remains supported permanently through the initialize /
session lifecycle with subscriptions disabled and no cache fields on the
wire.

## Single source of truth

A protocol version is supported ONLY when it has an exact row in the
product-owned policy table in `src/mcp_protocol.rs`
(`SUPPORTED_PROTOCOLS`). Support is never inferred from:

- rmcp's `ProtocolVersion::KNOWN_VERSIONS` list;
- a version's date sorting after (or before) a supported version;
- a current client tolerating the version;
- the presence of the version in the pinned historical client's SDK.

Unknown, custom, and future protocol versions receive no policy, no
capability view, and no version-specific response fields.

## Compatibility directions

- **Required (in product gate):** the current server works with old clients.
  This is proven for the permanent legacy version by a real historical client
  compiled against exact `rmcp 1.7.0`, over both HTTP and stdio, in addition
  to current-SDK typed and raw-wire tests.
- **Out of product gate:** new clients against historical serial-mcp server
  binaries. serial-mcp ships a server, not a client library; an old binary
  cannot regress from current source, so this direction is not part of the
  compatibility contract.

## Required proof layers

The proof layers are independent — no layer substitutes for another. Policy
unit tests, typed current-SDK cases, raw wire, stdio, and official
conformance apply to EVERY advertised version. The actual historical client
applies to retained older lifecycle/version rows (permanent `2025-11-25`),
and the Inspector smoke applies to the preferred version (`2026-07-28`):

1. **Policy unit tests** (`src/mcp_protocol.rs`) — exact rows, preferred
   order, no policy for unknown/future versions.
2. **Typed current-SDK cases** (`tests/protocol_compatibility.rs`) — real
   rmcp clients negotiated through the exact lifecycle, asserting surface,
   capabilities, and cache-field presence/absence.
3. **Raw wire** (`tests/protocol_compatibility.rs`) — hand-built JSON-RPC
   POSTs against the spawned binary asserting exact HTTP status, JSON-RPC
   error codes, headers, and response fields, independent of production
   helpers so implementation and expectation cannot fail together.
4. **Stdio** (`tests/stdio_integration.rs`) — both lifecycles over the
   spawned binary's stdin/stdout transport.
5. **Actual historical client** (`compat/rmcp-1-client/`, exact `rmcp 1.7.0`)
   — a real pre-migration client implementation over HTTP and stdio, proving
   historical interoperability rather than current-SDK compatibility mode.
6. **Official conformance** — pinned
   `@modelcontextprotocol/conformance@0.2.0-alpha.10` at the exact
   `--spec-version` for each version, installed from the committed
   `compat/mcp-validation/` lockfile and invoked as the local
   `node_modules/.bin/conformance` binary (never npx).
7. **Inspector interoperability** — pinned
   `@modelcontextprotocol/inspector@2.0.0` smoke against the preferred
   version (interoperability, not conformance), via the local
   `node_modules/.bin/mcp-inspector` binary.

## Permanent `2025-11-25` retention

MCP `2025-11-25` compatibility is a permanent product requirement. Its policy
row, the historical-client fixture, the raw-wire tests, the conformance
scenario set, and the drift guards must not be removed or weakened by a future
protocol or rmcp dependency update. Adding a new version is strictly additive:
it never mutates, deprecates, or evicts the `2025-11-25` row.

## Future-version admission checklist

For every new MCP protocol version to be supported:

1. Upgrade/pin rmcp only after confirming exact version support.
2. Add one explicit `ProtocolPolicy` row: preferred ordering, lifecycle,
   capability switches, and cache policy.
3. Keep all older rows unchanged by default.
4. Add an exact typed test case and raw-wire lifecycle/header/error
   assertions.
5. Add a stdio proof.
6. Add official conformance scenarios with the exact `--spec-version`.
7. Run the preferred-version Inspector smoke.
8. Retain the historical-client gates for every older advertised lifecycle.
9. Update policy docs, README, CHANGELOG, and drift expectations.
10. Run the shared compatibility runner and the full repository gate before
    merge.

No version is supported merely because rmcp knows it, its date is newer, or a
current client tolerates it. Pre-`2025-11-25` revisions are not supported and
are tracked only as a demand-driven feature idea in `FEATURES.md`.

## Local verification

The one complete local/CI MCP version gate:

```bash
bash scripts/test-mcp-compat.sh
```

It runs, in order: the locked serial-mcp build; the lockfile-pinned MCP
validation tooling install (`npm ci --ignore-scripts` in
`compat/mcp-validation/` — lifecycle scripts disabled, no npx, no dynamic
package resolution); the focused Rust protocol, stdio, and subscription
tests; the locked historical `rmcp 1.7.0` fixture build; the fixture over
stdio; an isolated loopback HTTP server with a bounded `server/discover`
readiness probe; the fixture over HTTP; the exact legacy (`2025-11-25`) and
modern (`2026-07-28`) official conformance scenario sets via the local
`node_modules/.bin/conformance` binary; and the pinned Inspector smoke via
the local `node_modules/.bin/mcp-inspector` binary. Environment overrides
select paths/port only; protocol versions, package pins, and scenario sets
are fixed in the script and the committed npm lockfile.

### Exact pins and report location

- Conformance package: `@modelcontextprotocol/conformance@0.2.0-alpha.10`
  (exact direct version, no `^`/`~`/tags/ranges).
- Inspector package: `@modelcontextprotocol/inspector@2.0.0`.
- Both live in the committed npm project `compat/mcp-validation/` (private
  `package.json`, committed `package-lock.json` with exact versions plus
  `sha512-` integrity for every locked package, including transitives). The
  tree is installed ONLY from that lockfile with
  `npm ci --ignore-scripts` and the local `node_modules/.bin` binaries are
  invoked directly — validation never uses `npx` and never runs package
  lifecycle scripts (preinstall supply-chain hardening). `node_modules/` is
  gitignored; only lockfile semantics are committed.
- Node `22.19.0` and Rust `1.97.1` in CI.
- Historical client: `rmcp = "=1.7.0"` with `default-features = false` and
  only the required client/transport features, resolved in the fixture's
  committed `Cargo.lock` (checksum
  `0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e`).
- Reports land under `target/conformance-results/` with one
  `<scenario>-2025-11-25` / `<scenario>-2026-07-28` directory per scenario.

### Expected failures and forbidden practices

`conformance/expected-failures.yaml` baselines exactly the four documented
fixture-dependent checks (all under `server-stateless:sep-2575-*`). A
baseline entry that starts passing fails the run as stale; any other failure
is an unexpected regression. The runner never runs `--suite all`, never
suppresses a runner exit status (`set -euo pipefail` + GNU `timeout` per
scenario), and never adds fixture endpoints, fake resources, or
conformance-only methods to the production server.

## Industry rationale

- **Docker Engine API** negotiates the highest shared version, disables newer
  features on downgrade, publishes an explicit version matrix, and treats
  removal of an old API floor as a deliberate deprecation decision — not an
  incidental SDK update. serial-mcp keeps an ordered, explicit product-owned
  list and never infers support from rmcp's broader known-version list.
  (<https://docs.docker.com/reference/api/engine/>)
- **Kubernetes version skew** publishes exact supported skew windows as part
  of the public contract; unsupported skew is not left to best-effort
  behavior. serial-mcp documents its support window as exactly the advertised
  set, and adding a version never implicitly evicts an older one.
  (<https://kubernetes.io/releases/version-skew-policy/>)
- **Git protocol v2** negotiates protocol version, then advertises
  capabilities; clients request only advertised commands and ignore unknown
  capability keys, and agent/version strings are informational. serial-mcp
  treats protocol version and advertised capabilities as separate contracts.
  (<https://git-scm.com/docs/gitprotocol-v2>)
- **Protocol Buffers evolution** favors additive changes, preservation of
  unknown fields, and permanent reservation of removed identifiers, testing
  old and new implementations in both directions. serial-mcp keeps MCP JSON
  response evolution additive inside a version and proves field absence as
  well as presence on the raw wire.
  (<https://protobuf.dev/programming-guides/proto2/#updating>)
- **Connect/gRPC conformance** separates specification conformance from
  interoperability and tests servers with independent client implementations.
  serial-mcp keeps three independent gates: official MCP conformance,
  Inspector interoperability, and an actual historical rmcp client.
  (<https://github.com/connectrpc/conformance>)
