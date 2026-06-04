# Agent Configuration

Note: `serial-mcp` must be on your `PATH`. If installed via `cargo install`, it should already be available as `serial-mcp`.

Config schemas vary by tool. Each section links to the official schema reference. If a config stops working, check the linked docs — schemas can change.

## Port names by platform

| Platform | Example ports | Notes |
|---|---|---|
| Linux | `/dev/ttyACM0`, `/dev/ttyUSB0` | Add user to `dialout` group: `sudo usermod -aG dialout $USER` |
| macOS | `/dev/tty.usbmodem1101`, `/dev/tty.usbserial-*` | Grant serial permission on first use |
| Windows | `COM3`, `COM4` | No extra setup needed |

## Claude Code CLI

`.mcp.json` (project) or `~/.claude.json` (global). [Schema →](https://code.claude.com/docs/en/mcp)

```json
{
  "mcpServers": {
    "serial": {
      "type": "stdio",
      "command": "serial-mcp",
      "args": ["--allowlist=/dev/ttyACM*,/dev/ttyUSB*"]
    }
  }
}
```

<details>
<summary>Windows</summary>

```json
{
  "mcpServers": {
    "serial": {
      "type": "stdio",
      "command": "C:\\Users\\<user>\\.cargo\\bin\\serial-mcp.exe",
      "args": ["--allowlist=COM3,COM4"]
    }
  }
}
```

</details>

## Claude Desktop

[Same schema →](https://code.claude.com/docs/en/mcp). Config file location:
- **Linux:** `~/.config/claude-desktop/claude_desktop_config.json`
- **macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows:** `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "serial": {
      "type": "stdio",
      "command": "serial-mcp",
      "args": ["--allowlist=/dev/ttyACM0"]
    }
  }
}
```

<details>
<summary>macOS / Windows</summary>

macOS:
```json
{
  "mcpServers": {
    "serial": {
      "type": "stdio",
      "command": "serial-mcp",
      "args": ["--allowlist=/dev/tty.usbmodem*,/dev/tty.usbserial-*"]
    }
  }
}
```

Windows:
```json
{
  "mcpServers": {
    "serial": {
      "type": "stdio",
      "command": "C:\\Users\\<user>\\.cargo\\bin\\serial-mcp.exe",
      "args": ["--allowlist=COM3,COM4"]
    }
  }
}
```

</details>

## Cursor

[Schema →](https://cursor.com/docs/mcp). `.cursor/mcp.json` (project) or `~/.cursor/mcp.json` (global):

```json
{
  "mcpServers": {
    "serial": {
      "type": "stdio",
      "command": "serial-mcp",
      "args": ["--allowlist=/dev/ttyACM*,/dev/ttyUSB*"]
    }
  }
}
```

## VS Code (Copilot)

[Schema →](https://code.visualstudio.com/docs/agents/reference/mcp-configuration). `.vscode/mcp.json` in your workspace:

```json
{
  "servers": {
    "serial": {
      "type": "stdio",
      "command": "serial-mcp",
      "args": ["--allowlist=/dev/ttyACM*,/dev/ttyUSB*"]
    }
  }
}
```

## Zed

[Schema →](https://zed.dev/docs/ai/mcp). `~/.config/zed/settings.json` under `"context_servers"`:

```json
{
  "context_servers": {
    "serial-mcp": {
      "command": "serial-mcp",
      "args": ["--allowlist=/dev/ttyACM*,/dev/ttyUSB*"]
    }
  }
}
```

## opencode

[Schema →](https://opencode.ai/config.json). `opencode.json` / `opencode.jsonc` (project) or `~/.config/opencode/opencode.json`:

```json
{
  "mcpServers": {
    "serial": {
      "type": "local",
      "command": ["serial-mcp", "--allowlist=/dev/ttyACM*,/dev/ttyUSB*"]
    }
  }
}
```

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

## Troubleshooting

- `Failed to open port` or `Unable to acquire exclusive lock on serial port`: another program already owns the device. Close tools like `picocom`, `screen`, `minicom`, serial monitors, or another `serial-mcp` instance.
- `Connection busy: ... already owns RX`: one receive-side MCP operation is already active on that connection. Finish or unsubscribe the current `read`, `read_line`, `wait_for`, or `subscribe` operation before starting another.

## Dev one-liner (no install, cargo run from source)

```json
{
  "mcpServers": {
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
