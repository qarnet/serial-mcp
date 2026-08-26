# Agent configuration

`serial-mcp` must be on your `PATH`. A Cargo installation normally makes it
available as `serial-mcp`.

Configuration formats differ between clients. For clients with a published JSON
schema, this repository provides an example in [`example-configs/`](../example-configs/).
A Rust integration test validates that example. For clients without a published
schema, use the official documentation. If a configuration stops working, check
those docs. Schemas can change.

## Port names by platform

| Platform | Example ports | Notes |
|---|---|---|
| Linux | `/dev/ttyACM0`, `/dev/ttyUSB0` | Add user to `dialout` group: `sudo usermod -aG dialout $USER` |
| macOS | `/dev/tty.usbmodem1101`, `/dev/tty.usbserial-*` | Grant serial permission on first use |
| Windows | `COM3`, `COM4` | No extra setup needed |

## Claude Code and Claude Desktop

Claude Code reads `.mcp.json` in a project. It reads `~/.claude.json` globally.
Claude Desktop reads one of these files:

- Linux: `~/.config/claude-desktop/claude_desktop_config.json`
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

See the [Claude MCP documentation](https://code.claude.com/docs/en/mcp).
The published schema is `https://json.schemastore.org/claude-code-settings.json`.
Use [`example-configs/claude_code.json`](../example-configs/claude_code.json) as
an example.

## Cursor

Cursor reads `.cursor/mcp.json` in a project. It reads `~/.cursor/mcp.json`
globally. See the [Cursor MCP documentation](https://cursor.com/docs/mcp).
No published schema exists, so this repository does not provide an example.

## VS Code (Copilot)

VS Code reads `.vscode/mcp.json` in the workspace. See the [VS Code MCP
configuration documentation](https://code.visualstudio.com/docs/agents/reference/mcp-configuration).
No published schema exists. VS Code provides built-in IntelliSense instead.
Use `"servers"` as the top-level key, not `"mcpServers"`.

## Zed

Zed reads `~/.config/zed/settings.json` under `"context_servers"`. See the [Zed
MCP documentation](https://zed.dev/docs/ai/mcp). No published schema exists.
This repository does not provide an example. Use `"context_servers"` as the
top-level key. Do not add a `type` field. Zed infers the transport from
`command` or `url`.

## opencode

opencode reads `opencode.json` or `opencode.jsonc` in a project. It reads
`~/.config/opencode/opencode.json` globally. See the
[opencode configuration documentation](https://opencode.ai/config.json).
The schema is `https://opencode.ai/config.json`.
Use [`example-configs/opencode.json`](../example-configs/opencode.json) as an
example. Use `"mcp"` as the top-level key, not `"mcpServers"`.

## OpenAI Codex

Codex reads `~/.codex/config.json` globally. It reads `.codex/config.json` in a
project. See the [Codex documentation](https://developers.openai.com/codex).
The schema is `https://developers.openai.com/codex/config-schema.json`.
Use [`example-configs/codex.json`](../example-configs/codex.json) as an example.
Use `"mcp_servers"` as the top-level key. Do not add a `type` field. Codex
infers the transport from `command` or `url`.

## Hermes Agent

Hermes reads `~/.hermes/config.yaml` or `.hermes.yaml` in a project. See the
[Hermes MCP feature documentation](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp/).
No published schema exists. No example is provided because Hermes uses YAML
rather than JSON. The [MCP feature docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp/)
describe the YAML format.

## HTTP transport (remote and headless)

Start the server with `--transport=http` on the target machine:

```bash
serial-mcp --transport=http
# Set a custom bind address
serial-mcp --transport=http --bind=0.0.0.0:8000
```

Use this configuration with any client that supports streamable HTTP:

```json
{
  "mcpServers": {
    "serial": {
      "type": "streamable-http",
      "url": "http://127.0.0.1:8000/mcp"
    }
  }
}
```

## Run from source without installing

```json
{
  "mcp": {
    "serial": {
      "type": "local",
      "command": [
        "cargo", "run", "--quiet",
        "--manifest-path", "/path/to/serial-mcp/Cargo.toml",
        "--bin", "serial-mcp", "--",
        "--allowlist=/dev/ttyACM*"
      ]
    }
  }
}
```

## Troubleshooting

- `Failed to open port` or `Unable to acquire exclusive lock on serial port`:
  another program already owns the device. Close tools such as `picocom`,
  `screen`, or `minicom`. Close any serial monitor or other `serial-mcp` instance
  too.
- `Connection busy: ... already owns RX`: a receive-side operation, `read`, is
  already active on that connection. Each connection has one shared RX pump.
  Only one `read` can be active at a time. Finish the current operation before
  starting another. See [RX and reading](rx-and-reading.md) for the RX model.

## Schema validation

The config examples are validated against their published JSON schemas by a Rust
integration test. The schemas are:

```
claude_code_settings = "https://json.schemastore.org/claude-code-settings.json"
opencode_config     = "https://opencode.ai/config.json"
codex_config        = "https://developers.openai.com/codex/config-schema.json"
```

Run the test locally with:

```bash
cargo test --locked --test config_schema_validation
```

Vendored schemas live in [`schemas/`](../schemas/). To refresh them from the
latest upstream versions, run `./scripts/update-config-schemas.sh`.

A scheduled [GitHub Actions workflow](../.github/workflows/schema-drift.yml)
checks daily for upstream schema changes.

Each client also validates configuration at runtime:

| Tool | How to validate |
|---|---|
| Claude Code CLI / Desktop | Run `claude mcp list` to see connection status |
| Cursor | Settings → MCP. A green dot means connected |
| VS Code | Open the Command Palette and run `MCP: List Servers` |
| Zed | Open AI → MCP Servers |
| opencode | Validation runs on startup; check `~/.local/share/opencode/opencode.log` |
| Hermes Agent | Run `hermes mcp list` to see connected servers |

If a configuration fails, use that client's documentation link to verify the
current JSON shape. Schemas can change between versions.
