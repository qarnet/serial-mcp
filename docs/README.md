# Documentation

The root [README](../README.md) is the main entry point. This index links to
the guides with detailed contracts.

## Using serial-mcp

| Guide | What it covers |
|---|---|
| [Agent configuration](agent-config.md) | Client setup for Claude Code, Claude Desktop, Cursor, VS Code, Zed, opencode, Codex, and Hermes. HTTP and remote use. Port names by platform. Troubleshooting. |
| [RX and reading](rx-and-reading.md) | Always-on RX ring and shared cursor. Tagged `from` forms. Timeouts, silence, and matching. Ring wrap and `bytes_lost`. Lossless hex fallback, flow control, `capture_boot`, and resource subscriptions. |
| [Device profiles](device-profiles.md) | Profile sessions and `profile_matches` outcomes. Identity rules. Generated and reused selection. Overlay precedence and write-through learning. Partial failures, revision CAS, rollback, deletion guards, `open_profile`, and `save_profile`. |
| [Persistent capture](persistent-capture.md) | The `export_log` contract and `--capture-dir` setup. Quota options and portable filenames. Atomic snapshots and the advisory lock. Failure semantics and the trust boundary. |
| [Protocol guide](protocols.md) | Framing and parsers. The seven presets. Field precedence. Checksum and error behavior. |
| [Protocol references](protocols/references.md) | Normative references for implemented protocols |

## Developing serial-mcp

| Doc | What it covers |
|---|---|
| [Development notes](development/README.md) | Index of active development documentation. It includes the roadmap, protocol matrix, compatibility policy, and evaluator reports. |
| [Roadmap](development/FEATURES.md) | Active roadmap. It also tracks technical debt. |
| [MCP version compatibility policy](development/mcp-version-compatibility-policy.md) | Supported protocol versions and permanent legacy retention. Admission checklist and proof layers. |
| [CHANGELOG](../CHANGELOG.md) | Release history |
| [AGENTS.md](../AGENTS.md) | Contributor guidelines and implementation invariants |
