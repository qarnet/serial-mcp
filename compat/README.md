# Compatibility fixtures and dependency policy

This directory contains two different kinds of compatibility assets. They have
different dependency-update rules and must not be treated as one dependency
tree.

## Directory roles

### `mcp-validation/`

This is the current validation-tool installation used by
[`scripts/test-mcp-compat.sh`](../scripts/test-mcp-compat.sh). It contains exact
direct pins for:

- `@modelcontextprotocol/conformance@0.2.0-alpha.10`
- `@modelcontextprotocol/inspector@2.0.0`

The committed npm lockfile pins the complete transitive graph. Installation
uses `npm ci --ignore-scripts`; validation invokes only local lockfile-installed
binaries and never uses `npx`.

Security updates to transitive packages are allowed here when all of these
conditions hold:

1. The two direct MCP validation-tool pins stay unchanged unless a deliberate
   conformance/Inspector upgrade is being performed.
2. The resulting `package-lock.json` remains complete and reproducible.
3. Lifecycle scripts remain disabled.
4. `bash scripts/test-mcp-compat.sh` passes in full.

### `rmcp-1-client/`

This is a historical interoperability fixture, not a current dependency
installation. It intentionally compiles against exact `rmcp 1.7.0` with
`default-features = false` and only these features:

- `client`
- `transport-child-process`
- `transport-streamable-http-client-reqwest`

Its committed `Cargo.lock` pins the original rmcp package checksum:

```text
0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e
```

Do not update this fixture's rmcp version to resolve an ordinary dependency
alert. A newer rmcp would no longer prove that the current server interoperates
with the actual pre-migration client. Any change to that exact pin or checksum
is a product compatibility decision, not routine dependency maintenance.

## Prior compatibility-directory dependency updates

The dependency updates made before the September 2026 rmcp alerts all affected
`mcp-validation/`, not `rmcp-1-client/`:

| Date | Commit | Change |
|---|---|---|
| 2026-08-17 | `e3ac9dbe` | `nanoid` 3.3.17 to 3.3.18 |
| 2026-09-08 | `7c933363` | `fast-uri` 3.1.5 to 3.1.6 and `qs` 6.15.3 to 6.16.0 through npm overrides |
| 2026-09-15 | `0e160d3a` | `hono` 4.13.0 to 4.13.8 in the npm lockfile |

These updates did not weaken the historical-client guarantee:

- the direct conformance and Inspector pins remained unchanged;
- `compat/rmcp-1-client/Cargo.lock` was not modified after the fixture was
  introduced on 2026-08-03;
- the merged pull requests passed the full CI compatibility job;
- a fresh `bash scripts/test-mcp-compat.sh` run on 2026-09-21 passed every
  gate, including both rmcp 1.7.0 transport smokes, both protocol-version
  conformance sets, and the Inspector smoke;
- `npm audit --package-lock-only --ignore-scripts` reported zero npm
  vulnerabilities during the assessment.

## September 2026 rmcp alert assessment

Three Dependabot alerts point only to
`compat/rmcp-1-client/Cargo.lock`. Production serial-mcp resolves rmcp 3.1.4,
which is newer than every first patched version below.

| Advisory | Severity | First patched | Fixture assessment |
|---|---:|---:|---|
| [GHSA-33f5-2c5q-wgwj](https://github.com/modelcontextprotocol/rust-sdk/security/advisories/GHSA-33f5-2c5q-wgwj) / CVE-2026-63127 | High | 2.0.0 | OAuth metadata validation flaw is unreachable because the fixture does not enable rmcp's `auth` feature or use OAuth. |
| [GHSA-9pj6-vhgr-3mwh](https://github.com/modelcontextprotocol/rust-sdk/security/advisories/GHSA-9pj6-vhgr-3mwh) / CVE-2026-63128 | High | 2.0.0 | Streamable HTTP server session leak is unreachable because the fixture enables no rmcp server feature and acts only as a client. |
| [GHSA-9g45-5xwm-f3wc](https://github.com/modelcontextprotocol/rust-sdk/security/advisories/GHSA-9g45-5xwm-f3wc) / CVE-2026-64684 | Medium | 2.1.0 | Custom-header redirect leak is unreachable in the fixture: it constructs the HTTP transport with `StreamableHttpClientTransport::from_uri`, supplies no credentials, and leaves `custom_headers` empty. |

Recommended alert disposition is **Vulnerable code is not used**, with an
individual comment recording the feature/path evidence above. This README
records the assessment only; it does not dismiss the alerts.

Reassess immediately if the historical fixture gains any of these behaviors:

- OAuth or the rmcp `auth` feature;
- an rmcp HTTP-server feature;
- custom authentication headers or other secrets;
- connections to untrusted remote endpoints instead of the bounded local
  compatibility server.

## Retaining `2025-11-25` compatibility

There are two separate decisions:

1. retaining the exact rmcp 1.7.0 historical-client fixture;
2. retaining the server's `2025-11-25` initialize/session lifecycle.

The fixture should remain for as long as the protocol lifecycle remains. It is
the independent proof that compatibility works with a real historical SDK,
rather than only with current-SDK compatibility modes.

Current repository policy makes `2025-11-25` permanent. The normative contract
is
[`docs/reference/mcp-version-compatibility-policy.md`](../docs/reference/mcp-version-compatibility-policy.md),
and the executable policy row is in
[`src/mcp_protocol.rs`](../src/mcp_protocol.rs).

The MCP specification permits a modern-only server, but doing so is breaking:
a legacy client cannot fall forward to the `2026-07-28` discovery/stateless
lifecycle. The official compatibility matrix explicitly says that a legacy
client against a modern-only server fails. Current ecosystem evidence also
argues against removal:

- `2026-07-28` became final only on 2026-07-28;
- the official TypeScript SDK v2 still defaults `Client.connect()` to the 2025
  initialize handshake unless modern negotiation is explicitly selected;
- the official Inspector still defaults its protocol-era setting to legacy;
- current SDKs deliberately provide dual-era server modes.

Sources:

- [MCP versioning and compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
- [MCP 2026-07-28 changes](https://modelcontextprotocol.io/specification/2026-07-28/changelog)
- [TypeScript SDK 2026-07-28 migration](https://ts.sdk.modelcontextprotocol.io/v2/migration/support-2026-07-28)
- [Inspector protocol eras](https://modelcontextprotocol.io/docs/2026-07-28/tools/inspector/protocol-eras)

### Earliest review and removal criteria

Do not remove compatibility now. An earliest sensible **review** date is
2027-07-28, one year after the modern revision became final. This is a product
safety floor, not an automatic removal date and not a specification mandate:
MCP's formal twelve-month deprecation minimum governs deprecated features, not
support for an older protocol revision.

Removal should happen only when all of these conditions are met:

1. Every client documented in
   [`docs/guides/agent-configuration.md`](../docs/guides/agent-configuration.md)
   has been tested successfully against the modern lifecycle on each applicable
   transport.
2. Major SDKs and Inspector no longer default to the legacy lifecycle, or the
   repository has an explicit user migration path for every remaining legacy
   default.
3. A deprecation notice and migration instructions have shipped for a
   meaningful release window.
4. Known users and issue history provide no evidence of required legacy-only
   clients during that window.
5. Removal is identified as a breaking product release rather than arriving
   through a routine rmcp or dependency update.
6. Product policy is deliberately changed from permanent retention before any
   implementation or test removal begins.

If removal is approved, remove the lifecycle and its proof together: the
`2025-11-25` policy row, initialize/session handling, legacy capability and
wire-shaping paths, historical rmcp fixture, raw-wire and typed tests, stdio
tests, official legacy conformance scenarios, drift guards, and associated
documentation. Do not remove only the fixture while continuing to claim legacy
support.

## Verification

Run the complete compatibility gate after any dependency or policy change in
this directory:

```bash
bash scripts/test-mcp-compat.sh
```

For feature-level inspection of the historical fixture:

```bash
cargo tree --locked -e features -i rmcp@1.7.0 \
  --manifest-path compat/rmcp-1-client/Cargo.toml
```

Expected feature graph contains the client and two transport features listed
above, with no `auth` or server feature.

## Scope of this record

This document records current evidence and recommended policy. It does not
approve or perform a dependency bump, Dependabot dismissal, compatibility
deprecation, or protocol-support removal.
