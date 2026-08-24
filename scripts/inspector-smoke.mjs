#!/usr/bin/env node
// Inspector 2.0.0 interoperability smoke, not conformance.
//
// Drive the exact pinned `@modelcontextprotocol/inspector@2.0.0` CLI in
// `--cli --format json` mode against a running Streamable HTTP server. Assert:
//
//   - initialize returns server name "serial-mcp" and negotiated modern 2026-07-28
//   - tools/list returns exactly 25 unique tools, including compute_checksum
//   - resources/list contains serial://ports and serial://connections
//   - prompts/list contains diagnose_port and interactive_terminal
//   - tools/call compute_checksum {"algorithm":"xor","data":"$GPGGA,1","encoding":"utf8"}
//     returns raw 111 / hex "6F" from the actual JSON envelope
//
// Use Node standard library only. No jq, no prose snapshots, no web/TUI/Playwright.
// Invoke the CLI non-interactively with MCP_AUTO_OPEN_ENABLED=false, non-TTY
// stdio, and a bounded --connect-timeout. Kill each command on its own timeout;
// a nonzero CLI exit or failed assertion exits the script nonzero.
//
// Usage:
//   node scripts/inspector-smoke.mjs <server-url>
//       [--inspector-cmd <command...> | --inspector-cmd=<path>]
//
// The first positional argument is the server URL. Resolve the exact Inspector
// CLI invocation in this order:
//   - `--inspector-cmd <command...>` consumes every argv token after the flag as
//     the command and its fixed args. At least one token is required, and tokens
//     are never whitespace-split.
//   - `--inspector-cmd=<path>` names one executable path and keeps it intact;
//     paths with spaces stay one token. The forms are mutually exclusive.
//   - INSPECTOR_CMD names one executable path and is never whitespace-split;
//     use --inspector-cmd when the command needs args.
//   - Otherwise use the local locked binary
//     `compat/mcp-validation/node_modules/.bin/mcp-inspector`, resolved relative
//     to this script and installed from the committed package-lock.json via
//     `npm ci --ignore-scripts`. Do not use npx or dynamic package resolution;
//     the validation tree is lockfile-pinned with lifecycle scripts disabled.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const PINNED_INSPECTOR_PACKAGE = "@modelcontextprotocol/inspector@2.0.0";
const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const LOCKED_INSPECTOR_BIN = join(
  SCRIPT_DIR,
  "..",
  "compat",
  "mcp-validation",
  "node_modules",
  ".bin",
  "mcp-inspector",
);
const SERVER_NAME = "serial-mcp";
const MODERN_VERSION = "2026-07-28";
const EXPECTED_TOOLS = 25;
const CHECKSUM_ARGUMENTS = '{"algorithm":"xor","data":"$GPGGA,1","encoding":"utf8"}';

const argv = process.argv.slice(2);
const serverUrl = argv[0];
if (!serverUrl || serverUrl.startsWith("--")) {
  console.error(
    "usage: node scripts/inspector-smoke.mjs <server-url> [--inspector-cmd <command...> | --inspector-cmd=<path>]",
  );
  process.exit(2);
}

// Resolve the exact Inspector CLI invocation using the header precedence. The
// standalone `--inspector-cmd` consumes every following argv token verbatim;
// the `=` form and INSPECTOR_CMD each name one path and never whitespace-split.
function inspectorCommand(argv) {
  const standaloneIndex = argv.indexOf("--inspector-cmd");
  const equalsIndex = argv.findIndex((arg) => arg.startsWith("--inspector-cmd="));
  if (standaloneIndex !== -1 && equalsIndex !== -1) {
    console.error(
      "error: --inspector-cmd and --inspector-cmd=<path> are mutually exclusive",
    );
    process.exit(2);
  }
  if (standaloneIndex !== -1) {
    const parts = argv.slice(standaloneIndex + 1);
    if (parts.length === 0) {
      console.error("error: --inspector-cmd requires at least one command token");
      process.exit(2);
    }
    return parts;
  }
  if (equalsIndex !== -1) {
    const path = argv[equalsIndex].slice("--inspector-cmd=".length);
    if (path.length === 0) {
      console.error("error: --inspector-cmd=<path> requires a non-empty path");
      process.exit(2);
    }
    return [path];
  }
  const envCmd = process.env.INSPECTOR_CMD;
  if (envCmd) {
    return [envCmd];
  }
  if (!existsSync(LOCKED_INSPECTOR_BIN)) {
    console.error(
      `error: locked Inspector binary not found at ${LOCKED_INSPECTOR_BIN};\n` +
        "  install the lockfile-pinned validation tree first:\n" +
        "    npm ci --ignore-scripts --prefix compat/mcp-validation\n" +
        `  (exact package: ${PINNED_INSPECTOR_PACKAGE}; never resolved dynamically)`,
    );
    process.exit(2);
  }
  return [LOCKED_INSPECTOR_BIN];
}

// Run one CLI invocation with a hard per-command timeout. Resolve with parsed
// JSON; reject on nonzero exit, timeout, or unparseable output.
function runInspector(cmd, args, timeoutMs) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd[0], [...cmd.slice(1), "--cli", ...args], {
      env: {
        ...process.env,
        MCP_AUTO_OPEN_ENABLED: "false",
        MCP_CATALOG_PATH: "",
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      child.kill("SIGKILL");
      reject(new Error(`inspector command timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.on("error", (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      reject(error);
    });
    child.on("close", (code) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (code !== 0) {
        reject(new Error(`inspector exit ${code}: ${stderr.trim() || stdout.slice(-400)}`));
        return;
      }
      const trimmed = stdout.trim();
      if (!trimmed) {
        reject(new Error("inspector produced no output"));
        return;
      }
      let parsed;
      try {
        parsed = JSON.parse(trimmed);
      } catch (error) {
        reject(new Error(`inspector output is not JSON: ${trimmed.slice(0, 400)}`));
        return;
      }
      resolve(parsed);
    });
  });
}

function fail(message) {
  console.error(`inspector-smoke FAIL: ${message}`);
  process.exitCode = 1;
}

let failures = 0;

// Shared session config forces modern protocol negotiation. The Inspector's
// ad-hoc --server-url default is legacy, so this selects 2026-07-28.
const workDir = mkdtempSync(join(tmpdir(), "inspector-smoke-"));
const configPath = join(workDir, "inspector-config.json");
writeFileSync(
  configPath,
  JSON.stringify({
    mcpServers: {
      "serial-mcp": {
        type: "http",
        url: serverUrl,
        protocolEra: "modern",
      },
    },
  }),
);

const cmd = inspectorCommand(argv);
const base = ["--config", configPath, "--server", "serial-mcp", "--connect-timeout", "15000"];
const timeoutMs = 60000;

try {
  // Verify initialize identity and modern negotiated version.
  const init = await runInspector(cmd, [...base, "--method", "initialize", "--format", "json"], timeoutMs);
  if (init.result?.serverInfo?.name !== SERVER_NAME) {
    failures++;
    fail(`initialize server name: expected ${SERVER_NAME}, got ${JSON.stringify(init.result?.serverInfo)}`);
  } else {
    console.log(`initialize: server name "${SERVER_NAME}" ok`);
  }
  if (init.result?.protocolVersion !== MODERN_VERSION) {
    failures++;
    fail(`initialize negotiated version: expected ${MODERN_VERSION}, got ${JSON.stringify(init.result?.protocolVersion)}`);
  } else {
    console.log(`initialize: negotiated ${MODERN_VERSION} ok`);
  }

  // Verify tools/list count, uniqueness, and compute_checksum.
  const tools = await runInspector(cmd, [...base, "--method", "tools/list", "--format", "json"], timeoutMs);
  const toolNames = (tools.result?.tools ?? []).map((t) => t.name);
  if (toolNames.length !== EXPECTED_TOOLS) {
    failures++;
    fail(`tools/list count: expected ${EXPECTED_TOOLS}, got ${toolNames.length}`);
  } else {
    console.log(`tools/list: exactly ${EXPECTED_TOOLS} tools ok`);
  }
  if (new Set(toolNames).size !== toolNames.length) {
    failures++;
    fail(`tools/list names not unique (${toolNames.length} entries)`);
  }
  if (!toolNames.includes("compute_checksum")) {
    failures++;
    fail("tools/list: compute_checksum missing");
  } else {
    console.log("tools/list: compute_checksum present ok");
  }

  // Verify both public resource URIs.
  const resources = await runInspector(cmd, [...base, "--method", "resources/list", "--format", "json"], timeoutMs);
  const resourceUris = (resources.result?.resources ?? []).map((r) => r.uri);
  for (const expectedUri of ["serial://ports", "serial://connections"]) {
    if (!resourceUris.includes(expectedUri)) {
      failures++;
      fail(`resources/list missing ${expectedUri} (got ${JSON.stringify(resourceUris)})`);
    } else {
      console.log(`resources/list: ${expectedUri} present ok`);
    }
  }

  // Verify both public prompts.
  const prompts = await runInspector(cmd, [...base, "--method", "prompts/list", "--format", "json"], timeoutMs);
  const promptNames = (prompts.result?.prompts ?? []).map((p) => p.name);
  for (const expectedName of ["diagnose_port", "interactive_terminal"]) {
    if (!promptNames.includes(expectedName)) {
      failures++;
      fail(`prompts/list missing ${expectedName} (got ${JSON.stringify(promptNames)})`);
    } else {
      console.log(`prompts/list: ${expectedName} present ok`);
    }
  }

  // Verify compute_checksum raw 111 / hex "6F" from the JSON envelope.
  const call = await runInspector(
    cmd,
    [...base, "--method", "tools/call", "--tool-name", "compute_checksum", "--tool-args-json", CHECKSUM_ARGUMENTS, "--format", "json"],
    timeoutMs,
  );
  const structured = call.result?.structuredContent;
  if (structured?.checksum !== 111 || structured?.checksum_hex !== "6F") {
    failures++;
    fail(`compute_checksum: expected raw 111 / hex 6F, got ${JSON.stringify(structured)}`);
  } else {
    console.log("tools/call compute_checksum: raw 111 / hex 6F ok");
  }
} catch (error) {
  failures++;
  fail(String(error.message ?? error));
} finally {
  rmSync(workDir, { recursive: true, force: true });
}

if (failures === 0) {
  console.log("inspector-smoke: all assertions passed");
} else {
  console.error(`inspector-smoke: ${failures} assertion(s) failed`);
  process.exitCode = 1;
}
