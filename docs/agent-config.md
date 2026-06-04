# Agent Configuration

Note: `serial-mcp` must be on your `PATH`. If installed via `cargo install`, it should already be available as `serial-mcp`.

Config schemas vary by tool. Where a published JSON schema exists, we use it for validation via [`lint-examples.sh`](../lint-examples.sh). Where none exists, we link the official docs and note the limitation. If a config stops working, check the linked docs — schemas can change.

## Port names by platform

| Platform | Example ports | Notes |
|---|---|---|
| Linux | `/dev/ttyACM0`, `/dev/ttyUSB0` | Add user to `dialout` group: `sudo usermod -aG dialout $USER` |
| macOS | `/dev/tty.usbmodem1101`, `/dev/tty.usbserial-*` | Grant serial permission on first use |
| Windows | `COM3`, `COM4` | No extra setup needed |

---

## Claude Code CLI

**File:** `.mcp.json` (project) or `~/.claude.json` (global)
**Docs:** [code.claude.com/docs/en/mcp](https://code.claude.com/docs/en/mcp)
**Schema:** `https://json.schemastore.org/claude-code-settings.json`
**Example:** [`example-configs/claude_code.json`](../example-configs/claude_code.json)

## Claude Desktop

**File:**
- Linux: `~/.config/claude-desktop/claude_desktop_config.json`
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

**Docs:** [code.claude.com/docs/en/mcp](https://code.claude.com/docs/en/mcp)
**Schema:** `https://json.schemastore.org/claude-code-settings.json`
**Example:** [`example-configs/claude_desktop.json`](../example-configs/claude_desktop.json)

## Cursor

**File:** `.cursor/mcp.json` (project) or `~/.cursor/mcp.json` (global)
**Docs:** [cursor.com/docs/mcp](https://cursor.com/docs/mcp)
**Schema:** none published for MCP config
**Example:** [`example-configs/cursor.json`](../example-configs/cursor.json)

## VS Code (Copilot)

**File:** `.vscode/mcp.json` in your workspace
**Docs:** [code.visualstudio.com/docs/agents/reference/mcp-configuration](https://code.visualstudio.com/docs/agents/reference/mcp-configuration)
**Schema:** none published; VS Code has built-in IntelliSense
**Example:** [`example-configs/vscode.json`](../example-configs/vscode.json)
**Note:** Uses `"servers"` as the top-level key, not `"mcpServers"`.

## Zed

**File:** `~/.config/zed/settings.json` under `"context_servers"`
**Docs:** [zed.dev/docs/ai/mcp](https://zed.dev/docs/ai/mcp)
**Schema:** none published
**Example:** [`example-configs/zed.json`](../example-configs/zed.json)
**Note:** Uses `"context_servers"` as the top-level key. No `type` field — inferred from `command` vs `url`.

## opencode

**File:** `opencode.json` / `opencode.jsonc` (project) or `~/.config/opencode/opencode.json`
**Docs:** [opencode.ai/config.json](https://opencode.ai/config.json) (the schema is the docs)
**Schema:** `https://opencode.ai/config.json`
**Example:** [`example-configs/opencode.json`](../example-configs/opencode.json)
**Note:** Uses `"mcp"` as the top-level key, not `"mcpServers"`.

## Hermes Agent

**File:** `~/.hermes/config.yaml` or project `.hermes.yaml`
**Docs:** [hermes-agent.nousresearch.com/docs/user-guide/features/mcp](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp/)
**Schema:** none published
**Example:** not provided — Hermes uses YAML, not JSON. See the [MCP feature docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp/) for the YAML config format.

---

## HTTP transport (remote / headless)

Start the server with `--transport=http` on the target machine:

```bash
serial-mcp --transport=http
# custom bind address:
serial-mcp --transport=http --bind=0.0.0.0:8000
```

Agent config (any client that supports streamable HTTP):

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

## Dev one-liner (no install, cargo run from source)

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

- `Failed to open port` or `Unable to acquire exclusive lock on serial port`: another program already owns the device. Close tools like `picocom`, `screen`, `minicom`, serial monitors, or another `serial-mcp` instance.
- `Connection busy: ... already owns RX`: one receive-side MCP operation is already active on that connection. Finish or unsubscribe the current `read`, `read_line`, `wait_for`, or `subscribe` operation before starting another.

## Schema validation

A [lint script](../lint-examples.sh) validates the example configs against their published JSON schemas. Schemas used:

```
claude_code_settings = "https://json.schemastore.org/claude-code-settings.json"
opencode_config     = "https://opencode.ai/config.json"
```

Run locally: `./lint-examples.sh` — requires `cargo install jsonschema-cli`.

Each tool also validates config at runtime:

| Tool | How to validate |
|---|---|
| Claude Code CLI / Desktop | Run `claude mcp list` — shows connection status |
| Cursor | Settings → MCP — green dot means connected |
| VS Code | Command Palette → `MCP: List Servers` |
| Zed | AI → MCP Servers |
| opencode | Validates on startup; check `~/.local/share/opencode/opencode.log` |
| Hermes Agent | Run `hermes mcp list` — shows connected servers |

If a config doesn't work, click the docs link for that tool to verify the current JSON shape — schemas can change between versions.
