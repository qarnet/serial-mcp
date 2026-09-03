# Documentation

The root [README](../README.md) is the human-scannable entry point. This index
points at the focused guides that hold the detailed contracts.

## Using serial-mcp

| Guide | What it covers |
|---|---|
| [Agent Configuration](agent-config.md) | Wiring the server into Claude Code, Claude Desktop, Cursor, VS Code, Zed, opencode, Codex, Hermes, and HTTP/remote setups; port names by platform; troubleshooting. |
| [RX and Reading](rx-and-reading.md) | The always-on RX ring, shared cursor, tagged `from` forms, timeouts/silence/match, ring wrap and `bytes_lost`, lossless hex fallback, the hardware flow-control caveat, `capture_boot`, and resource subscriptions. |
| [Device Profiles](device-profiles.md) | Automatic profile sessions: `profile_matches` outcomes, identity rules, generated/reused selection, overlay precedence, write-through learning, partial failures, revision CAS, rollback, deletion guard, `open_profile`, `save_profile`. |
| [Persistent Capture](persistent-capture.md) | The complete `export_log` contract: enabling with `--capture-dir`, quota options, the portable filename rules, atomic snapshots, the advisory lock, failure semantics, and the trust boundary. |
| [Protocol Guide](protocols.md) | Framing and parsers, the seven presets, field precedence, and checksum/error behavior. |
| [Protocol References](protocols/references.md) | Normative spec citations for the implemented protocols. |

## Developing serial-mcp

| Doc | What it covers |
|---|---|
| [Product Backlog](BACKLOG.md) | Planned and in-progress work; each entry links to its plan document under [development/plans/](development/plans/). |
| [Development Notes](development/README.md) | Index of durable development documentation: compatibility policy, protocol matrix, evaluator reports, decision records, plans. |
| [MCP Version Compatibility Policy](development/mcp-version-compatibility-policy.md) | Supported protocol versions, permanent legacy retention, admission checklist, proof layers. |
| [Architecture Decision Records](adr/README.md) | Proposed and accepted architecture decisions, including test-foundation changes. |
| [CHANGELOG](../CHANGELOG.md) | Release history. |
| [AGENTS.md](../AGENTS.md) | Contributor guidelines and implementation invariants. |
