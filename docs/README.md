# Documentation

The root [README](../README.md) is the human-scannable entry point. This
index points at the focused documents that hold the detailed contracts.

Directories are purpose-named: what a document is for determines where it
lives, and where it lives tells you its lifecycle.

## Guides — task-oriented, shipped behavior

How do I accomplish a particular task with serial-mcp?

| Guide | What it covers |
|---|---|
| [Agent Configuration](guides/agent-configuration.md) | Wiring the server into Claude Code, Claude Desktop, Cursor, VS Code, Zed, opencode, Codex, Hermes, and HTTP/remote setups; port names by platform; troubleshooting. |
| [RX and Reading](guides/rx-and-reading.md) | The always-on RX ring, shared cursor, tagged `from` forms, timeouts/silence/match, ring wrap and `bytes_lost`, lossless hex fallback, the hardware flow-control caveat, `capture_boot`, and resource subscriptions. |
| [Device Profiles](guides/device-profiles.md) | Automatic profile sessions: `profile_matches` outcomes, identity rules, generated/reused selection, overlay precedence, write-through learning, partial failures, revision CAS, rollback, deletion guard, `open_profile`, `save_profile`. |
| [Persistent Capture](guides/persistent-capture.md) | The complete `export_log` contract: enabling with `--capture-dir`, quota options, the portable filename rules, atomic snapshots, the advisory lock, failure semantics, and the trust boundary. |
| [Protocol Guide](guides/protocols.md) | Framing and parsers, the seven presets, field precedence, and checksum/error behavior. |

## Reference — normative contracts

What exactly is supported, guaranteed, configured, or standardized?

| Document | What it covers |
|---|---|
| [MCP Version Compatibility Policy](reference/mcp-version-compatibility-policy.md) | Supported protocol versions, permanent legacy retention, admission checklist, proof layers. |
| [Protocol Matrix](reference/protocol-matrix.md) | Support/status matrix for every protocol with a cited spec. |
| [Protocol Specifications](reference/protocol-specifications.md) | Normative spec citations for the implemented protocols. |

## Product — intent and lifecycle

What is planned, in progress, accepted, or dropped?

| Document | What it covers |
|---|---|
| [Product contract](product/README.md) | Backlog.md configuration, field vocabulary, lifecycle states, Definitions of Ready/Review/Done, ownership boundaries. |
| [Backlog](product/backlog/) | One Markdown file per item, managed with the `backlog` CLI (dev-shell). Active items live in `tasks/`; `completed/` and `archive/` retain product history. `backlog board` is the view; there is no Markdown table duplicate. |

## Design — active technical designs

Documents that are substantial enough to need standalone reasoning:
cross-boundary work, trade-off analysis, new subsystems. Transient by
design; durable information moves to guides/reference/ADRs when work ships.

| Design | Backlog item |
|---|---|
| [Server-runtime ownership](design/PB-001-server-runtime-ownership.md) | PB-001 |
| [Continuous capture](design/PB-025-continuous-capture.md) | PB-025 |

## ADRs — durable architecture decisions

[adr/](adr/) holds architecture decision records that outlive their
implementation. Superseded decisions are marked, not erased.

## Reports — point-in-time evidence

| Report | What it covers |
|---|---|
| [Agent Interface Evaluation](reports/agent-interface-evaluation.md) | Current evaluator report: 25-tool catalog, accepted/rejected interface decisions with thresholds, limitations. |
| [Agent Interface Baseline](reports/agent-interface-baseline.json) | Committed historical baseline (26 tools, 258964 bytes) for `xtask agent-eval --baseline` diffs. |
| [Windows Serial E2E Investigation](reports/windows-serial-e2e-investigation.md) | Decision record: Windows serial E2E deferred; needs a pre-provisioned signed-driver runner or an approved design. |

## Maintenance — how the repository itself is kept

| Document | What it covers |
|---|---|
| [Documentation Hygiene marker](maintenance/documentation-hygiene.md) | Managed by the OpenCode `documentation-hygiene` skill; records the last full-repository audit. Do not edit manually. |

## Also

- [CHANGELOG](../CHANGELOG.md) — release history.
- [AGENTS.md](../AGENTS.md) — contributor guidelines and implementation
  invariants.