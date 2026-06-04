# Agent Configuration

Note: `serial-mcp` must be on your `PATH`. If installed via `cargo install`, it should already be available as `serial-mcp`.

Config schemas vary by tool. Each section links to the official schema reference and a ready-to-use example config in [`example-configs/`](../example-configs/). If a config stops working, check the linked docs — schemas can change.

## Port names by platform

| Platform | Example ports | Notes |
|---|---|---|
| Linux | `/dev/ttyACM0`, `/dev/ttyUSB0` | Add user to `dialout` group: `sudo usermod -aG dialout $USER` |
| macOS | `/dev/tty.usbmodem1101`, `/dev/tty.usbserial-*` | Grant serial permission on first use |
| Windows | `COM3`, `COM4` | No extra setup needed |

## Claude Code CLI

**File:** `.mcp.json` (project) or `~/.claude.json` (global)
**Schema:** [code.claude.com/docs/en/mcp](https://code.claude.com/docs/en/mcp)
**Example:** [`example-configs/claude_code.json`](../example-configs/claude_code.json)

## Claude Desktop

**File:**
- Linux: `~/.config/claude-desktop/claude_desktop_config.json`
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

**Schema:** [code.claude.com/docs/en/mcp](https://code.claude.com/docs/en/mcp)
**Example:** [`example-configs/claude_desktop.json`](../example-configs/claude_desktop.json)

## Cursor

**File:** `.cursor/mcp.json` (project) or `~/.cursor/mcp.json` (global)
**Schema:** [cursor.com/docs/mcp](https://cursor.com/docs/mcp)
**Example:** [`example-configs/cursor.json`](../example-configs/cursor.json)

## VS Code (Copilot)

**File:** `.vscode/mcp.json` in your workspace
**Schema:** [code.visualstudio.com/docs/agents/reference/mcp-configuration](https://code.visualstudio.com/docs/agents/reference/mcp-configuration)
**Example:** [`example-configs/vscode.json`](../example-configs/vscode.json)

## Zed

**File:** `~/.config/zed/settings.json` under `"context_servers"`
**Schema:** [zed.dev/docs/ai/mcp](https://zed.dev/docs/ai/mcp)
**Example:** [`example-configs/zed.json`](../example-configs/zed.json)

## opencode

**File:** `opencode.json` / `opencode.jsonc` (project) or `~/.config/opencode/opencode.json`
**Schema:** [opencode.ai/config.json](https://opencode.ai/config.json)
**Example:** [`example-configs/opencode.json`](../example-configs/opencode.json)

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

[`example-configs/opencode.json`](../example-configs/opencode.json) — set `command` to:

```json
["cargo", "run", "--quiet", "--manifest-path", "/path/to/serial-mcp/Cargo.toml", "--bin", "serial-mcp", "--", "--allowlist=/dev/ttyACM*"]
```

## Troubleshooting

- `Failed to open port` or `Unable to acquire exclusive lock on serial port`: another program already owns the device. Close tools like `picocom`, `screen`, `minicom`, serial monitors, or another `serial-mcp` instance.
- `Connection busy: ... already owns RX`: one receive-side MCP operation is already active on that connection. Finish or unsubscribe the current `read`, `read_line`, `wait_for`, or `subscribe` operation before starting another.

## Schema validation

Each tool validates config differently:

| Tool | How to validate |
|---|---|
| Claude Code CLI / Desktop | Run `claude mcp list` — shows connection status for each server |
| Cursor | Open Cursor Settings → MCP — green dot means connected |
| VS Code | Command Palette → `MCP: List Servers` — shows status |
| Zed | Open Zed → AI → MCP Servers — lists servers and their status |
| opencode | opencode validates on startup; check `~/.local/share/opencode/opencode.log` for errors |

If a config doesn't work, click the schema link for that tool to verify the current JSON shape — schemas can change between versions.
