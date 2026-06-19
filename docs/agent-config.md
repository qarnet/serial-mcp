# Agent Configuration

Note: `serial-mcp` must be on your `PATH`. If installed via `cargo install`, it should already be available as `serial-mcp`.

Config schemas vary by tool. Where a published JSON schema exists, we provide an example config in [`example-configs/`](../example-configs/) and validate it via a Rust integration test. Where none exists, we link the official docs. If a config stops working, check the linked docs — schemas can change.

## Port names by platform

| Platform | Example ports | Notes |
|---|---|---|
| Linux | `/dev/ttyACM0`, `/dev/ttyUSB0` | Add user to `dialout` group: `sudo usermod -aG dialout $USER` |
| macOS | `/dev/tty.usbmodem1101`, `/dev/tty.usbserial-*` | Grant serial permission on first use |
| Windows | `COM3`, `COM4` | No extra setup needed |

---

## Claude Code / Desktop

**File:**
- Claude Code: `.mcp.json` (project) or `~/.claude.json` (global)
- Claude Desktop:
  - Linux: `~/.config/claude-desktop/claude_desktop_config.json`
  - macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
  - Windows: `%APPDATA%\Claude\claude_desktop_config.json`

**Docs:** [code.claude.com/docs/en/mcp](https://code.claude.com/docs/en/mcp)

**Schema:** `https://json.schemastore.org/claude-code-settings.json`

**Example:** [`example-configs/claude_code.json`](../example-configs/claude_code.json)

## Cursor

**File:** `.cursor/mcp.json` (project) or `~/.cursor/mcp.json` (global)

**Docs:** [cursor.com/docs/mcp](https://cursor.com/docs/mcp)

**Schema:** none published — no example provided

## VS Code (Copilot)

**File:** `.vscode/mcp.json` in your workspace

**Docs:** [code.visualstudio.com/docs/agents/reference/mcp-configuration](https://code.visualstudio.com/docs/agents/reference/mcp-configuration)

**Schema:** none published — VS Code has built-in IntelliSense

**Note:** Uses `"servers"` as the top-level key, not `"mcpServers"`.

## Zed

**File:** `~/.config/zed/settings.json` under `"context_servers"`

**Docs:** [zed.dev/docs/ai/mcp](https://zed.dev/docs/ai/mcp)

**Schema:** none published — no example provided

**Note:** Uses `"context_servers"` as the top-level key. No `type` field — inferred from `command` vs `url`.

## opencode

**File:** `opencode.json` / `opencode.jsonc` (project) or `~/.config/opencode/opencode.json`

**Docs:** [opencode.ai/config.json](https://opencode.ai/config.json) (the schema is the docs)

**Schema:** `https://opencode.ai/config.json`

**Example:** [`example-configs/opencode.json`](../example-configs/opencode.json)

**Note:** Uses `"mcp"` as the top-level key, not `"mcpServers"`.

## OpenAI Codex

**File:** `~/.codex/config.json` (global) or `.codex/config.json` (project)

**Docs:** [developers.openai.com/codex](https://developers.openai.com/codex)

**Schema:** `https://developers.openai.com/codex/config-schema.json`

**Example:** [`example-configs/codex.json`](../example-configs/codex.json)

**Note:** Uses `"mcp_servers"` (underscore) as the top-level key. No `type` field — transport inferred from `command` vs `url`.

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
- `Connection busy: ... already owns RX`: a receive-side operation (`read` or `subscribe`) is already active on that connection. Each connection has a single shared RX pump; only one `read` or `subscribe` stream task can be active at a time. Finish or unsubscribe the current operation before starting another. See the [README](../README.md#how-rx-works) for RX model details.

## Schema validation

Config examples are validated against their published JSON schemas as a Rust integration test. Schemas used:

```
claude_code_settings = "https://json.schemastore.org/claude-code-settings.json"
opencode_config     = "https://opencode.ai/config.json"
codex_config        = "https://developers.openai.com/codex/config-schema.json"
```

Run locally: `cargo test --locked --test config_schema_validation`

Vendored schemas live in [`schemas/`](../schemas/). To refresh them against the latest upstream: `./scripts/update-config-schemas.sh`.

A scheduled [GitHub Actions workflow](../.github/workflows/schema-drift.yml) checks daily whether upstream schemas have changed.

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
