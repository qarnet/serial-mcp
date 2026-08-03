#!/usr/bin/env node
// Inspector 2.0.0 interoperability smoke (NOT conformance).
//
// Drives the exact pinned `@modelcontextprotocol/inspector@2.0.0` CLI in
// `--cli --format json` mode against a running Streamable HTTP server and
// asserts the observable MCP surface:
//
//   - initialize  -> server name "serial-mcp", negotiated modern 2026-07-28
//   - tools/list  -> exactly 25 unique tools, compute_checksum present
//   - resources/list -> serial://ports and serial://connections present
//   - prompts/list   -> diagnose_port and interactive_terminal present
//   - tools/call compute_checksum {"algorithm":"xor","data":"$GPGGA,1","encoding":"utf8"}
//                   -> raw 111 / hex "6F" (parsed from the actual JSON envelope)
//
// Node standard library only. No jq, no prose snapshots, no web/TUI/Playwright.
// The CLI is invoked non-interactively: MCP_AUTO_OPEN_ENABLED=false, non-TTY
// stdio, and a bounded --connect-timeout. Every command is killed on its own
// timeout; a nonzero CLI exit or a failed assertion exits the script nonzero.
//
// Usage:
//   node scripts/inspector-smoke.mjs <server-url>
//       [--inspector-cmd <command...> | --inspector-cmd=<path>]
//
// The first positional is the server URL. The exact Inspector CLI invocation
// resolves with this precedence:
//   - `--inspector-cmd <command...>` — every argv token AFTER the flag is the
//     command plus its fixed args (at least one token required; tokens are
//     never whitespace-split);
//   - `--inspector-cmd=<path>` — one executable path, kept intact (paths with
//     spaces stay a single token); mutually exclusive with the standalone form;
//   - the INSPECTOR_CMD env var — ONE executable path, never whitespace-split
//     (paths with spaces remain intact); use --inspector-cmd when the command
//     needs args;
//   - otherwise the exact pinned `npx` package
//     `@modelcontextprotocol/inspector@2.0.0`. No floating versions.

import { spawn } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const PINNED_INSPECTOR_PACKAGE = "@modelcontextprotocol/inspector@2.0.0";
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

// Resolve the exact Inspector CLI invocation (see header comment for the
// precedence). Deterministic: the standalone `--inspector-cmd` consumes every
// following argv token verbatim; the `=` form and INSPECTOR_CMD env each name
// ONE path and are never whitespace-split.
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
  return ["npx", "-y", PINNED_INSPECTOR_PACKAGE];
}

// One CLI invocation with a hard per-command timeout. Resolves with parsed
// JSON; rejects on nonzero exit, timeout, or unparseable output.
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

// Shared session config: forces the modern protocol era so the Inspector
// negotiates 2026-07-28 (its ad-hoc --server-url default is legacy).
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
  // 1. initialize: name + modern negotiated version.
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

  // 2. tools/list: exactly 25 unique tools, compute_checksum present.
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

  // 3. resources/list: serial://ports and serial://connections present.
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

  // 4. prompts/list: diagnose_port and interactive_terminal present.
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

  // 5. tools/call compute_checksum: raw 111 / hex "6F" from the JSON envelope.
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
