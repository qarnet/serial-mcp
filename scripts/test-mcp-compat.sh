#!/usr/bin/env bash
#
# Run compatibility checks for both supported MCP protocol versions through one
# local/CI gate: 2025-11-25 legacy sessions and 2026-07-28 modern
# discovery/stateless requests. Linux/CI scope.
#
# Run, in order:
# 1. Build the locked serial-mcp binary unless SERIAL_MCP_BIN names one.
# 2. Install lockfile-pinned MCP validation tooling with
#    `npm ci --ignore-scripts` in compat/mcp-validation. Lifecycle scripts are
#    not executed and packages are not resolved via npx.
# 3. Run focused Rust gates: protocol_compatibility, stdio_integration, and
#    resource_subscriptions.
# 4. Build the locked historical rmcp 1.7.0 fixture into a stable compat target.
# 5. Run the historical fixture stdio smoke against the current binary.
# 6. Start an isolated loopback HTTP server with a temporary profile path and a
#    bounded modern server/discover readiness probe.
# 7. Run the historical fixture HTTP smoke against /mcp.
# 8. Run official legacy conformance scenarios at --spec-version 2025-11-25.
# 9. Run official modern conformance scenarios at --spec-version 2026-07-28.
# 10. Run the pinned Inspector 2.0.0 interoperability smoke against the
#     preferred modern version.
#
# Conformance and Inspector packages come only from the committed lockfile
# compat/mcp-validation/package-lock.json (exact versions 0.2.0-alpha.10 /
# 2.0.0 with integrity hashes, private package.json). Invoke them through that
# project's local node_modules/.bin, not via npx or dynamic package resolution.
#
# Protocol versions, package pins, scenario lists, the expected-failure path,
# and assertions are fixed in this file. Environment overrides may select paths
# and port only:
#   SERIAL_MCP_BIN          default: target/debug/serial-mcp
#   MCP_COMPAT_PORT         default: 8931
#   MCP_COMPAT_REPORT_DIR   default: target/conformance-results
#   MCP_COMPAT_CARGO_TARGET default: target/mcp-compat-rmcp-1
#
# The script does not run `--suite all`, suppress a runner exit status, or add
# fixture endpoints to the product. The readiness probe keeps `curl ... || true`
# because it polls and validates the HTTP status itself. GNU timeout 180 wraps
# every conformance scenario, and any nonzero exit fails the run. A trap stops
# and waits for the HTTP server and removes only this run's own `mktemp -d`
# directory.

set -euo pipefail

# Resolve repository root from this script's path rather than cwd.
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

# Fixed contract.
# Record the exact pinned official conformance package. Install it from the
# committed lockfile and invoke it through the local binary below; this constant
# documents the package name.
CONFORMANCE_PACKAGE="@modelcontextprotocol/conformance@0.2.0-alpha.10"
# The lockfile-pinned MCP validation project keeps exact direct versions and
# integrity hashes in compat/mcp-validation/package-lock.json (private
# package.json). Install with `npm ci --ignore-scripts`; lifecycle scripts are
# disabled by policy for supply-chain hardening. This script uses only the local
# binaries for conformance and Inspector; npx and dynamic package resolution
# are not used.
VALIDATION_DIR="$ROOT/compat/mcp-validation"
CONFORMANCE_BIN="$VALIDATION_DIR/node_modules/.bin/conformance"
INSPECTOR_BIN="$VALIDATION_DIR/node_modules/.bin/mcp-inspector"
# The expected-failure baseline contains exactly four documented
# fixture-dependent checks (see conformance/expected-failures.yaml). A baseline
# entry that starts passing fails the run as stale; any other failure is
# unexpected.
EXPECTED_FAILURES="$ROOT/conformance/expected-failures.yaml"
# MCP protocol version 2025-11-25 uses the legacy initialize/session lifecycle.
# server-initialize covers that lifecycle in the pinned package; the handoff's
# `server-session-lifecycle` name does not exist in
# @modelcontextprotocol/conformance@0.2.0-alpha.10.
SCENARIOS_2025_11_25="server-initialize ping completion-complete tools-list resources-list prompts-list"
# MCP protocol version 2026-07-28 uses the modern discovery/stateless lifecycle.
SCENARIOS_2026_07_28="server-stateless completion-complete tools-list resources-list prompts-list caching sep-2164-resource-not-found"
# GNU timeout applies to every fixture invocation and conformance scenario.
GATE_TIMEOUT=180

# Environment overrides are limited to paths and port.
BIN="${SERIAL_MCP_BIN:-target/debug/serial-mcp}"
PORT="${MCP_COMPAT_PORT:-8931}"
REPORT_DIR="${MCP_COMPAT_REPORT_DIR:-target/conformance-results}"
FIXTURE_TARGET="${MCP_COMPAT_CARGO_TARGET:-target/mcp-compat-rmcp-1}"

# Server lifecycle.
SERVER_PID=""
PROFILES_DIR=""

cleanup() {
  # Always stop and wait for the HTTP server started by this run.
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
  fi
  # Remove only this run's temporary directory, never an override directory.
  if [ -n "$PROFILES_DIR" ]; then
    rm -rf -- "$PROFILES_DIR"
    PROFILES_DIR=""
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

step() {
  printf '\n==== [%s] %s ====\n' "$(date -u +%H:%M:%S)" "$*"
}

# Build the locked serial-mcp binary when no executable was supplied.
if [ -x "$BIN" ]; then
  step "using prebuilt serial-mcp binary: $BIN"
else
  step "building locked serial-mcp binary"
  cargo build --locked --bin serial-mcp
fi
if [ ! -x "$BIN" ]; then
  echo "error: SERIAL_MCP_BIN '$BIN' is not an executable" >&2
  exit 1
fi
BIN="$(realpath "$BIN")"
step "serial-mcp binary: $BIN"

# Install lockfile-pinned MCP validation tooling without lifecycle scripts.
step "installing MCP validation tooling (npm ci --ignore-scripts)"
npm ci --ignore-scripts --prefix "$VALIDATION_DIR"
if [ ! -x "$CONFORMANCE_BIN" ]; then
  echo "error: conformance binary not produced by npm ci: $CONFORMANCE_BIN" >&2
  exit 1
fi
if [ ! -x "$INSPECTOR_BIN" ]; then
  echo "error: mcp-inspector binary not produced by npm ci: $INSPECTOR_BIN" >&2
  exit 1
fi

# Run focused Rust gates.
step "focused Rust gates: protocol_compatibility"
cargo test --locked --test protocol_compatibility
step "focused Rust gates: stdio_integration"
cargo test --locked --test stdio_integration
step "focused Rust gates: resource_subscriptions"
cargo test --locked --test resource_subscriptions

# Build the locked historical fixture.
step "building historical rmcp 1.7.0 fixture"
cargo build --locked --manifest-path compat/rmcp-1-client/Cargo.toml \
  --target-dir "$FIXTURE_TARGET"
FIXTURE_BIN="$FIXTURE_TARGET/debug/rmcp-1-client"
if [ ! -x "$FIXTURE_BIN" ]; then
  echo "error: fixture binary not produced: $FIXTURE_BIN" >&2
  exit 1
fi

# Run the historical fixture stdio smoke.
step "historical rmcp 1.7.0 fixture over stdio"
timeout "$GATE_TIMEOUT" "$FIXTURE_BIN" stdio "$BIN"

# Start the loopback HTTP server and poll readiness.
mkdir -p "$REPORT_DIR"
PROFILES_DIR="$(mktemp -d)"
step "starting HTTP server on 127.0.0.1:$PORT (logs: $REPORT_DIR/server.log)"
"$BIN" --transport=http --bind=127.0.0.1:"$PORT" \
  --profiles-path "$PROFILES_DIR/profiles.toml" \
  >"$REPORT_DIR/server.log" 2>&1 &
SERVER_PID=$!
echo "$SERVER_PID" >"$REPORT_DIR/server.pid"

MCP_URL="http://127.0.0.1:$PORT/mcp"
step "readiness probe (bounded server/discover for 2026-07-28)"
# The bounded probe sends modern server/discover, the only session-less request
# the server accepts. It must return HTTP 200. `curl ... || true` lets the loop
# continue while it polls and validates the HTTP status itself.
probe_code=""
for _ in $(seq 1 60); do
  probe_code="$(curl -s -o /dev/null -w '%{http_code}' -X POST \
    "$MCP_URL" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H 'MCP-Protocol-Version: 2026-07-28' \
    -H 'Mcp-Method: server/discover' \
    -d '{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"probe","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}}}' \
    || true)"
  if [ "$probe_code" = "200" ]; then
    echo "server ready (probe 200)"
    break
  fi
  sleep 0.5
done
if [ "$probe_code" != "200" ]; then
  echo "server failed to become ready; last probe code: $probe_code" >&2
  cat "$REPORT_DIR/server.log" >&2
  exit 1
fi

# Run the historical fixture HTTP smoke.
step "historical rmcp 1.7.0 fixture over HTTP ($MCP_URL)"
timeout "$GATE_TIMEOUT" "$FIXTURE_BIN" http "$MCP_URL"

# Run legacy conformance scenarios.
for sc in $SCENARIOS_2025_11_25; do
  step "conformance $sc (2025-11-25)"
  timeout "$GATE_TIMEOUT" "$CONFORMANCE_BIN" server \
    --url "$MCP_URL" --scenario "$sc" --spec-version 2025-11-25 \
    --expected-failures "$EXPECTED_FAILURES" \
    -o "$REPORT_DIR/$sc-2025-11-25"
done

# Run modern conformance scenarios.
for sc in $SCENARIOS_2026_07_28; do
  step "conformance $sc (2026-07-28)"
  timeout "$GATE_TIMEOUT" "$CONFORMANCE_BIN" server \
    --url "$MCP_URL" --scenario "$sc" --spec-version 2026-07-28 \
    --expected-failures "$EXPECTED_FAILURES" \
    -o "$REPORT_DIR/$sc-2026-07-28"
done

# Run the Inspector 2.0.0 interoperability smoke.
step "Inspector 2.0.0 interoperability smoke (2026-07-28)"
node "$ROOT/scripts/inspector-smoke.mjs" "$MCP_URL" --inspector-cmd "$INSPECTOR_BIN"

# Print the success summary.
printf '\n==== mcp-compat: all gates passed ====\n'
echo "  validation tooling install:       ok (npm ci --ignore-scripts, locked)"
echo "  rmcp-1 stdio smoke:               ok"
echo "  rmcp-1 http smoke:                ok"
echo "  conformance 2025-11-25:           ok ($(echo "$SCENARIOS_2025_11_25" | wc -w) scenarios)"
echo "  conformance 2026-07-28:           ok ($(echo "$SCENARIOS_2026_07_28" | wc -w) scenarios)"
echo "  inspector smoke:                  ok"
echo "  reports: $REPORT_DIR"
