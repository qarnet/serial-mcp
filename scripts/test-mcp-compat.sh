#!/usr/bin/env bash
#
# MCP version compatibility runner — the single executable local/CI gate for
# both supported protocol versions (2025-11-25 legacy sessions and 2026-07-28
# modern discovery/stateless requests). Linux/CI scope.
#
# Runs, in order:
#   1. locked serial-mcp binary build (unless SERIAL_MCP_BIN names one)
#   2. focused Rust gates: protocol_compatibility, stdio_integration,
#      resource_subscriptions
#   3. locked historical rmcp 1.7.0 fixture build into a stable compat target
#   4. historical fixture stdio smoke against the current binary
#   5. isolated loopback HTTP server (temporary profile path) with a bounded
#      modern server/discover readiness probe
#   6. historical fixture HTTP smoke against /mcp
#   7. official legacy conformance scenarios at --spec-version 2025-11-25
#   8. official modern conformance scenarios at --spec-version 2026-07-28
#   9. pinned Inspector 2.0.0 interoperability smoke against the preferred
#      (modern) version
#
# Protocol versions, package pins, scenario lists, the expected-failure path,
# and all assertions are FIXED in this file. Environment overrides may select
# paths/port only:
#   SERIAL_MCP_BIN          default: target/debug/serial-mcp
#   MCP_COMPAT_PORT         default: 8931
#   MCP_COMPAT_REPORT_DIR   default: target/conformance-results
#   MCP_COMPAT_CARGO_TARGET default: target/mcp-compat-rmcp-1
#
# The script never runs `--suite all`, never suppresses a runner exit status
# (the readiness probe may keep its `curl ... || true` because it explicitly
# polls and validates the HTTP status), and never adds fixture endpoints to
# the product. Every conformance scenario is wrapped in GNU timeout 180 and
# fails the run on any nonzero exit. A trap always stops/waits for the HTTP
# server and removes only this run's own `mktemp -d` directory.

set -euo pipefail

# Resolve the repository root from this script's path (never cwd).
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

# --- Fixed contract (not overridable) -----------------------------------------
# The exact pinned official conformance package (no floating tags).
CONFORMANCE_PACKAGE="@modelcontextprotocol/conformance@0.2.0-alpha.10"
# Expected-failure baseline: exactly the four documented fixture-dependent
# checks (see conformance/expected-failures.yaml). A baseline entry that
# starts passing fails the run as stale; any other failure is unexpected.
EXPECTED_FAILURES="$ROOT/conformance/expected-failures.yaml"
# Scenario set for MCP protocol version 2025-11-25 (legacy initialize/session
# lifecycle). server-initialize covers the legacy initialize/session
# lifecycle in the pinned package (the handoff's `server-session-lifecycle`
# name does not exist in @modelcontextprotocol/conformance@0.2.0-alpha.10).
SCENARIOS_2025_11_25="server-initialize ping completion-complete tools-list resources-list prompts-list"
# Scenario set for MCP protocol version 2026-07-28 (modern discovery /
# stateless lifecycle).
SCENARIOS_2026_07_28="server-stateless completion-complete tools-list resources-list prompts-list caching sep-2164-resource-not-found"
# GNU timeout for every fixture invocation and conformance scenario.
GATE_TIMEOUT=180

# --- Environment overrides (paths/port only) ----------------------------------
BIN="${SERIAL_MCP_BIN:-target/debug/serial-mcp}"
PORT="${MCP_COMPAT_PORT:-8931}"
REPORT_DIR="${MCP_COMPAT_REPORT_DIR:-target/conformance-results}"
FIXTURE_TARGET="${MCP_COMPAT_CARGO_TARGET:-target/mcp-compat-rmcp-1}"

# --- Server lifecycle ---------------------------------------------------------
SERVER_PID=""
PROFILES_DIR=""

cleanup() {
  # Always stop/waits for the HTTP server we started.
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
  fi
  # Remove only this run's own temporary directory (never an override dir).
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

# --- 1. locked serial-mcp binary build ---------------------------------------
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

# --- 2. focused Rust gates ----------------------------------------------------
step "focused Rust gates: protocol_compatibility"
cargo test --locked --test protocol_compatibility
step "focused Rust gates: stdio_integration"
cargo test --locked --test stdio_integration
step "focused Rust gates: resource_subscriptions"
cargo test --locked --test resource_subscriptions

# --- 3. locked historical fixture build ---------------------------------------
step "building historical rmcp 1.7.0 fixture"
cargo build --locked --manifest-path compat/rmcp-1-client/Cargo.toml \
  --target-dir "$FIXTURE_TARGET"
FIXTURE_BIN="$FIXTURE_TARGET/debug/rmcp-1-client"
if [ ! -x "$FIXTURE_BIN" ]; then
  echo "error: fixture binary not produced: $FIXTURE_BIN" >&2
  exit 1
fi

# --- 4. historical fixture stdio smoke ----------------------------------------
step "historical rmcp 1.7.0 fixture over stdio"
timeout "$GATE_TIMEOUT" "$FIXTURE_BIN" stdio "$BIN"

# --- 5. loopback HTTP server + readiness probe --------------------------------
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
# Bounded readiness probe: a modern server/discover (the only session-less
# request the server accepts) must return HTTP 200. `|| true` is retained
# because the loop polls and validates the HTTP status itself.
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

# --- 6. historical fixture HTTP smoke -----------------------------------------
step "historical rmcp 1.7.0 fixture over HTTP ($MCP_URL)"
timeout "$GATE_TIMEOUT" "$FIXTURE_BIN" http "$MCP_URL"

# --- 7. conformance (2025-11-25) ----------------------------------------------
for sc in $SCENARIOS_2025_11_25; do
  step "conformance $sc (2025-11-25)"
  timeout "$GATE_TIMEOUT" npx -y "$CONFORMANCE_PACKAGE" server \
    --url "$MCP_URL" --scenario "$sc" --spec-version 2025-11-25 \
    --expected-failures "$EXPECTED_FAILURES" \
    -o "$REPORT_DIR/$sc-2025-11-25"
done

# --- 8. conformance (2026-07-28) ----------------------------------------------
for sc in $SCENARIOS_2026_07_28; do
  step "conformance $sc (2026-07-28)"
  timeout "$GATE_TIMEOUT" npx -y "$CONFORMANCE_PACKAGE" server \
    --url "$MCP_URL" --scenario "$sc" --spec-version 2026-07-28 \
    --expected-failures "$EXPECTED_FAILURES" \
    -o "$REPORT_DIR/$sc-2026-07-28"
done

# --- 9. Inspector 2.0.0 interoperability smoke --------------------------------
step "Inspector 2.0.0 interoperability smoke (2026-07-28)"
node "$ROOT/scripts/inspector-smoke.mjs" "$MCP_URL"

# --- success summary ----------------------------------------------------------
printf '\n==== mcp-compat: all gates passed ====\n'
echo "  rmcp-1 stdio smoke:               ok"
echo "  rmcp-1 http smoke:                ok"
echo "  conformance 2025-11-25:           ok ($(echo "$SCENARIOS_2025_11_25" | wc -w) scenarios)"
echo "  conformance 2026-07-28:           ok ($(echo "$SCENARIOS_2026_07_28" | wc -w) scenarios)"
echo "  inspector smoke:                  ok"
echo "  reports: $REPORT_DIR"
