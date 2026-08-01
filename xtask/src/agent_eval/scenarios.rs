//! Fixed call-shape scenarios for the agent-interface evaluation.
//!
//! Every scenario is a static sequence of MCP `tools/call` requests with
//! normalized placeholders (`/dev/ttyACM0`, a fixed UUID connection id), so
//! request bytes are deterministic across runs. Current variants use the
//! shipped tool names and shapes; hypothetical shorthand/recipe/facade/
//! capture variants are marked `modeled` and always carry their expansion
//! into current calls. A static harness cannot measure model
//! misunderstanding — the report states this limitation.

use serde::Serialize;

use crate::agent_eval::{FIXED_CONNECTION_ID, FIXED_ENVELOPE_ID, FIXED_PORT};

/// One fixed MCP `tools/call` request.
#[derive(Debug, Clone)]
pub struct Call {
    pub tool: &'static str,
    pub args: serde_json::Value,
}

/// A hypothetical, NOT-implemented call shape with its expansion into
/// current calls.
#[derive(Debug, Clone)]
pub struct ModeledVariant {
    /// `shorthand` | `recipe` | `facade` | `capture_boot`.
    pub kind: &'static str,
    pub label: &'static str,
    pub calls: Vec<Call>,
    pub expansion_calls: Vec<Call>,
    pub note: &'static str,
}

/// One task scenario.
#[derive(Debug, Clone)]
pub struct Scenario {
    pub id: &'static str,
    pub label: &'static str,
    pub calls: Vec<Call>,
    pub modeled: Option<ModeledVariant>,
    /// Name of an existing public behavior test that proves this
    /// composition completes (or "modeled" for hypothetical variants).
    pub completion_ref: &'static str,
    /// Stale-data/race risk flag (e.g. arm-reset-capture composition).
    pub stale_race: bool,
    pub retries: usize,
    pub invalid_calls: usize,
    /// Belongs to the "common task" set used by the facade decision.
    pub common: bool,
}

/// Measured metrics for one scenario.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioMetrics {
    pub id: String,
    pub label: String,
    pub tool_calls: usize,
    /// Compact JSON bytes of the fixed-ID `tools/call` envelopes.
    pub request_bytes: usize,
    pub invalid_calls: usize,
    pub retries: usize,
    /// Occurrences of advanced option fields across the call arguments.
    pub advanced_fields: usize,
    pub stale_race: bool,
    pub completion_ref: String,
    pub common: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modeled: Option<ModeledMetrics>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModeledMetrics {
    pub kind: String,
    pub label: String,
    pub tool_calls: usize,
    pub request_bytes: usize,
    pub expansion_calls: usize,
    pub expansion_bytes: usize,
    pub note: String,
}

/// Advanced option fields counted per call (framing/parser/protocol
/// overrides and match config — the "escalation" surface).
const ADVANCED_KEYS: [&str; 7] = [
    "rx_framing",
    "tx_framing",
    "rx_parser",
    "protocol",
    "match",
    "reconnect_policy",
    "profile_mode",
];

/// Compact JSON bytes of one fixed-ID `tools/call` envelope.
pub fn envelope_bytes(call: &Call) -> usize {
    let envelope = serde_json::json!({
        "jsonrpc": "2.0",
        "id": FIXED_ENVELOPE_ID,
        "method": "tools/call",
        "params": { "name": call.tool, "arguments": call.args }
    });
    serde_json::to_string(&envelope)
        .expect("envelope serializes")
        .len()
}

fn advanced_fields(calls: &[Call]) -> usize {
    calls
        .iter()
        .flat_map(|call| call.args.as_object().expect("args is an object").keys())
        .filter(|k| ADVANCED_KEYS.contains(&k.as_str()))
        .count()
}

fn total_bytes(calls: &[Call]) -> usize {
    calls.iter().map(envelope_bytes).sum()
}

/// Compute metrics for one scenario (current + modeled variant).
pub fn scenario_metrics(scenario: &Scenario) -> ScenarioMetrics {
    let modeled = scenario.modeled.as_ref().map(|m| ModeledMetrics {
        kind: m.kind.to_string(),
        label: m.label.to_string(),
        tool_calls: m.calls.len(),
        request_bytes: total_bytes(&m.calls),
        expansion_calls: m.expansion_calls.len(),
        expansion_bytes: total_bytes(&m.expansion_calls),
        note: m.note.to_string(),
    });
    ScenarioMetrics {
        id: scenario.id.to_string(),
        label: scenario.label.to_string(),
        tool_calls: scenario.calls.len(),
        request_bytes: total_bytes(&scenario.calls),
        invalid_calls: scenario.invalid_calls,
        retries: scenario.retries,
        advanced_fields: advanced_fields(&scenario.calls),
        stale_race: scenario.stale_race,
        completion_ref: scenario.completion_ref.to_string(),
        common: scenario.common,
        modeled,
    }
}

fn call(tool: &'static str, args: serde_json::Value) -> Call {
    Call { tool, args }
}

fn conn(id: &str) -> serde_json::Value {
    serde_json::json!({ "connection_id": id })
}

const CID: &str = FIXED_CONNECTION_ID;

/// The fixed scenario set (order is stable and part of the baseline).
pub fn scenarios() -> Vec<Scenario> {
    let match_ok = serde_json::json!({
        "pattern": "OK>",
        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
    });
    vec![
        Scenario {
            id: "first_console_discovery_open",
            label: "First console: discovery + open",
            calls: vec![
                call("list_ports", serde_json::json!({})),
                call("open", serde_json::json!({ "port": FIXED_PORT })),
            ],
            modeled: None,
            completion_ref:
                "tests/serial_pty.rs::auto_session_first_open_creates_generated_profile_and_pty_traffic_flows",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: true,
        },
        Scenario {
            id: "returning_known_console_automatic",
            label: "Returning device: automatic profile reuse",
            calls: vec![
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call(
                    "transact",
                    serde_json::json!({
                        "connection_id": CID,
                        "data": "status\r\n",
                        "match": match_ok.clone(),
                        "timeout_ms": 3000,
                    }),
                ),
            ],
            modeled: None,
            completion_ref:
                "tests/serial_pty.rs::list_ports_preview_selected_winner_matches_later_bare_open",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: true,
        },
        Scenario {
            id: "explicit_profile_management",
            label: "Returning device: explicit profile management",
            calls: vec![
                call("list_profiles", serde_json::json!({})),
                call("open_profile", serde_json::json!({ "profile": "console-dev" })),
                call(
                    "transact",
                    serde_json::json!({
                        "connection_id": CID,
                        "data": "status\r\n",
                        "match": match_ok.clone(),
                        "timeout_ms": 3000,
                    }),
                ),
            ],
            modeled: None,
            completion_ref: "tests/serial_pty.rs::open_profile_explicit_binding_reports_matched_port_confidence",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: true,
        },
        Scenario {
            id: "command_response_transact",
            label: "Command/response via transact",
            calls: vec![
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call(
                    "transact",
                    serde_json::json!({
                        "connection_id": CID,
                        "data": "status\r\n",
                        "match": match_ok.clone(),
                        "timeout_ms": 3000,
                    }),
                ),
            ],
            modeled: Some(ModeledVariant {
                kind: "shorthand",
                label: "transact with string shorthand (match/from)",
                calls: vec![
                    call("open", serde_json::json!({ "port": FIXED_PORT })),
                    call(
                        "transact",
                        serde_json::json!({
                            "connection_id": CID,
                            "data": "status\r\n",
                            "match": "OK>",
                            "from": "now",
                            "timeout_ms": 3000,
                        }),
                    ),
                ],
                expansion_calls: vec![
                    call("open", serde_json::json!({ "port": FIXED_PORT })),
                    call(
                        "transact",
                        serde_json::json!({
                            "connection_id": CID,
                            "data": "status\r\n",
                            "match": match_ok.clone(),
                            "timeout_ms": 3000,
                        }),
                    ),
                ],
                note: "String forms for `match`/`from` would expand to the current tagged objects.",
            }),
            completion_ref: "tests/serial_pty.rs::pty_transact_writes_then_reads_response",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: true,
        },
        Scenario {
            id: "command_response_write_read",
            label: "Command/response via write + read",
            calls: vec![
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call(
                    "write",
                    serde_json::json!({ "connection_id": CID, "data": "status\r\n" }),
                ),
                call(
                    "read",
                    serde_json::json!({
                        "connection_id": CID,
                        "match": match_ok.clone(),
                        "timeout_ms": 3000,
                    }),
                ),
            ],
            modeled: None,
            completion_ref: "tests/serial_pty.rs::pty_transact_writes_then_reads_response",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: true,
        },
        Scenario {
            id: "line_capture",
            label: "Line capture",
            calls: vec![
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call(
                    "read",
                    serde_json::json!({
                        "connection_id": CID,
                        "rx_framing": { "type": "line", "ending": "auto" },
                        "timeout_ms": 1000,
                    }),
                ),
            ],
            modeled: None,
            completion_ref: "tests/native_sim_validation/unix.rs::native_read_line_framing_splits_lines",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: true,
        },
        Scenario {
            id: "at_modem",
            label: "AT modem (protocol preset)",
            calls: vec![
                call(
                    "open",
                    serde_json::json!({
                        "port": FIXED_PORT,
                        "protocol": { "type": "at_command" },
                    }),
                ),
                call(
                    "transact",
                    serde_json::json!({
                        "connection_id": CID,
                        "data": "ATI",
                        "match": serde_json::json!({ "pattern": "OK", "config": { "mode": "literal_substring", "pattern_encoding": "utf8" } }),
                        "timeout_ms": 3000,
                    }),
                ),
            ],
            modeled: Some(ModeledVariant {
                kind: "shorthand",
                label: "open with string protocol shorthand",
                calls: vec![
                    call(
                        "open",
                        serde_json::json!({
                            "port": FIXED_PORT,
                            "protocol": "at_command",
                        }),
                    ),
                    call(
                        "transact",
                        serde_json::json!({
                            "connection_id": CID,
                            "data": "ATI",
                            "match": serde_json::json!({ "pattern": "OK", "config": { "mode": "literal_substring", "pattern_encoding": "utf8" } }),
                            "timeout_ms": 3000,
                        }),
                    ),
                ],
                expansion_calls: vec![
                    call(
                        "open",
                        serde_json::json!({
                            "port": FIXED_PORT,
                            "protocol": { "type": "at_command" },
                        }),
                    ),
                    call(
                        "transact",
                        serde_json::json!({
                            "connection_id": CID,
                            "data": "ATI",
                            "match": serde_json::json!({ "pattern": "OK", "config": { "mode": "literal_substring", "pattern_encoding": "utf8" } }),
                            "timeout_ms": 3000,
                        }),
                    ),
                ],
                note: "A bare string `protocol` would expand to the current tagged preset object.",
            }),
            completion_ref: "tests/native_sim_validation/unix.rs::native_read_at_parser_parses_pong",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: false,
        },
        Scenario {
            id: "at_modem_recipe",
            label: "AT modem via connection recipe (modeled)",
            calls: vec![
                call(
                    "open",
                    serde_json::json!({
                        "port": FIXED_PORT,
                        "protocol": { "type": "at_command" },
                    }),
                ),
                call(
                    "transact",
                    serde_json::json!({
                        "connection_id": CID,
                        "data": "ATI",
                        "match": serde_json::json!({ "pattern": "OK", "config": { "mode": "literal_substring", "pattern_encoding": "utf8" } }),
                        "timeout_ms": 3000,
                    }),
                ),
            ],
            modeled: Some(ModeledVariant {
                kind: "recipe",
                label: "open with recipe instead of protocol object",
                calls: vec![
                    call(
                        "open",
                        serde_json::json!({
                            "port": FIXED_PORT,
                            "recipe": { "type": "at_modem" },
                        }),
                    ),
                    call(
                        "transact",
                        serde_json::json!({
                            "connection_id": CID,
                            "data": "ATI",
                            "match": serde_json::json!({ "pattern": "OK", "config": { "mode": "literal_substring", "pattern_encoding": "utf8" } }),
                            "timeout_ms": 3000,
                        }),
                    ),
                ],
                expansion_calls: vec![
                    call(
                        "open",
                        serde_json::json!({
                            "port": FIXED_PORT,
                            "protocol": { "type": "at_command" },
                        }),
                    ),
                    call(
                        "transact",
                        serde_json::json!({
                            "connection_id": CID,
                            "data": "ATI",
                            "match": serde_json::json!({ "pattern": "OK", "config": { "mode": "literal_substring", "pattern_encoding": "utf8" } }),
                            "timeout_ms": 3000,
                        }),
                    ),
                ],
                note: "A `recipe` would replace the repeated protocol-preset object (at_modem = at_command preset + bounded timeouts).",
            }),
            completion_ref: "tests/native_sim_validation/unix.rs::native_read_at_parser_parses_pong",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: false,
        },
        Scenario {
            id: "ndjson_stream",
            label: "NDJSON stream (protocol preset)",
            calls: vec![
                call(
                    "open",
                    serde_json::json!({
                        "port": FIXED_PORT,
                        "protocol": { "type": "ndjson" },
                    }),
                ),
                call(
                    "read",
                    serde_json::json!({ "connection_id": CID, "timeout_ms": 1000 }),
                ),
            ],
            modeled: Some(ModeledVariant {
                kind: "recipe",
                label: "open with ndjson_stream recipe instead of protocol object",
                calls: vec![
                    call(
                        "open",
                        serde_json::json!({
                            "port": FIXED_PORT,
                            "recipe": { "type": "ndjson_stream" },
                        }),
                    ),
                    call(
                        "read",
                        serde_json::json!({ "connection_id": CID, "timeout_ms": 1000 }),
                    ),
                ],
                expansion_calls: vec![
                    call(
                        "open",
                        serde_json::json!({
                            "port": FIXED_PORT,
                            "protocol": { "type": "ndjson" },
                        }),
                    ),
                    call(
                        "read",
                        serde_json::json!({ "connection_id": CID, "timeout_ms": 1000 }),
                    ),
                ],
                note: "ndjson_stream recipe = ndjson preset (line framing + JSON parser, skip_empty).",
            }),
            completion_ref: "tests/native_sim_validation/unix.rs::native_read_ndjson_preset_decodes_json_frames",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: false,
        },
        Scenario {
            id: "rollback_recovery",
            label: "Rollback recovery after a bad learned setting",
            calls: vec![
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call(
                    "reconfigure",
                    serde_json::json!({ "connection_id": CID, "baud_rate": 9600 }),
                ),
                call("close", conn(CID)),
                call("list_profiles", serde_json::json!({})),
                call(
                    "rollback_profile",
                    serde_json::json!({
                        "profile_name": "console-dev",
                        "expected_revision": 2,
                        "revision": 1,
                    }),
                ),
                call("open", serde_json::json!({ "port": FIXED_PORT })),
            ],
            modeled: None,
            completion_ref: "tests/serial_pty.rs::rollback_with_no_active_connections_reports_zero",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: false,
        },
        Scenario {
            id: "boot_reset_prompt_capture",
            label: "Boot-reset prompt capture (current multi-call composition)",
            calls: vec![
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call(
                    "read",
                    serde_json::json!({
                        "connection_id": CID,
                        "from": { "type": "now" },
                        "timeout_ms": 100,
                    }),
                ),
                call(
                    "set_dtr_rts",
                    serde_json::json!({ "connection_id": CID, "dtr": false, "rts": false }),
                ),
                call(
                    "set_dtr_rts",
                    serde_json::json!({ "connection_id": CID, "dtr": true, "rts": true }),
                ),
                call(
                    "read",
                    serde_json::json!({
                        "connection_id": CID,
                        "match": serde_json::json!({ "pattern": "boot>", "config": { "mode": "literal_substring", "pattern_encoding": "utf8" } }),
                        "timeout_ms": 5000,
                    }),
                ),
            ],
            modeled: Some(ModeledVariant {
                kind: "capture_boot",
                label: "atomic capture_boot (armed reset + capture in one call)",
                calls: vec![call(
                    "capture_boot",
                    serde_json::json!({
                        "connection_id": CID,
                        "reset": { "dtr": true, "rts": true },
                        "match": "boot>",
                        "timeout_ms": 5000,
                    }),
                )],
                expansion_calls: vec![
                    call("open", serde_json::json!({ "port": FIXED_PORT })),
                    call(
                        "read",
                        serde_json::json!({
                            "connection_id": CID,
                            "from": { "type": "now" },
                            "timeout_ms": 100,
                        }),
                    ),
                    call(
                        "set_dtr_rts",
                        serde_json::json!({ "connection_id": CID, "dtr": false, "rts": false }),
                    ),
                    call(
                        "set_dtr_rts",
                        serde_json::json!({ "connection_id": CID, "dtr": true, "rts": true }),
                    ),
                    call(
                        "read",
                        serde_json::json!({
                            "connection_id": CID,
                            "match": serde_json::json!({ "pattern": "boot>", "config": { "mode": "literal_substring", "pattern_encoding": "utf8" } }),
                            "timeout_ms": 5000,
                        }),
                    ),
                ],
                note: "One server-side operation would snapshot the live edge, pulse DTR/RTS, and capture only post-reset bytes — removing the arm/reset race between the seek and the reset.",
            }),
            completion_ref:
                "tests/serial_pty.rs::pty_transact_from_now_skips_pre_write_buffer",
            stale_race: true,
            retries: 0,
            invalid_calls: 0,
            common: false,
        },
        Scenario {
            id: "permission_busy_disconnected",
            label: "Permission/busy/disconnected errors with retry",
            calls: vec![
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call(
                    "read",
                    serde_json::json!({ "connection_id": CID, "timeout_ms": 1000 }),
                ),
                call("reconnect", conn(CID)),
            ],
            modeled: None,
            completion_ref: "tests/native_sim_validation/unix.rs::native_auto_reconnect_preserves_connection",
            stale_race: false,
            retries: 1,
            invalid_calls: 0,
            common: false,
        },
        Scenario {
            id: "command_response_facade",
            label: "Command/response via facade `command` (modeled)",
            calls: vec![
                call("open", serde_json::json!({ "port": FIXED_PORT })),
                call(
                    "transact",
                    serde_json::json!({
                        "connection_id": CID,
                        "data": "status\r\n",
                        "match": match_ok.clone(),
                        "timeout_ms": 3000,
                    }),
                ),
            ],
            modeled: Some(ModeledVariant {
                kind: "facade",
                label: "concise `command` tool (transact alias)",
                calls: vec![
                    call("open", serde_json::json!({ "port": FIXED_PORT })),
                    call(
                        "command",
                        serde_json::json!({
                            "connection_id": CID,
                            "data": "status\r\n",
                            "match": "OK>",
                            "timeout_ms": 3000,
                        }),
                    ),
                ],
                expansion_calls: vec![
                    call("open", serde_json::json!({ "port": FIXED_PORT })),
                    call(
                        "transact",
                        serde_json::json!({
                            "connection_id": CID,
                            "data": "status\r\n",
                            "match": match_ok.clone(),
                            "timeout_ms": 3000,
                        }),
                    ),
                ],
                note: "A facade `command` would be a 1:1 alias of `transact` with string `match` — same call count, fewer bytes.",
            }),
            completion_ref: "tests/serial_pty.rs::pty_transact_writes_then_reads_response",
            stale_race: false,
            retries: 0,
            invalid_calls: 0,
            common: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_ids_are_unique_and_stable() {
        let all = scenarios();
        let mut ids: Vec<&str> = all.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "scenario ids must be unique");
        // The fixed order is part of the committed baseline.
        assert_eq!(all[0].id, "first_console_discovery_open");
        assert_eq!(all[all.len() - 1].id, "command_response_facade");
    }

    #[test]
    fn metrics_are_deterministic() {
        let all = scenarios();
        let first: Vec<ScenarioMetrics> = all.iter().map(scenario_metrics).collect();
        let second: Vec<ScenarioMetrics> = all.iter().map(scenario_metrics).collect();
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap(),
            "scenario metrics must be byte-identical across runs"
        );
    }

    #[test]
    fn every_scenario_has_calls_and_completion_reference() {
        for s in scenarios() {
            assert!(!s.calls.is_empty(), "{} has no calls", s.id);
            assert!(
                !s.completion_ref.is_empty() && s.completion_ref != "modeled",
                "{} must cite a real public behavior test",
                s.id
            );
            if let Some(m) = &s.modeled {
                assert!(!m.calls.is_empty(), "{} modeled variant has no calls", s.id);
                assert!(
                    !m.expansion_calls.is_empty(),
                    "{} modeled variant must carry an expansion",
                    s.id
                );
            }
        }
    }

    #[test]
    fn transact_scenario_is_shorter_than_write_read() {
        let all = scenarios();
        let transact = scenario_metrics(
            all.iter()
                .find(|s| s.id == "command_response_transact")
                .unwrap(),
        );
        let write_read = scenario_metrics(
            all.iter()
                .find(|s| s.id == "command_response_write_read")
                .unwrap(),
        );
        assert!(transact.tool_calls < write_read.tool_calls);
        assert!(transact.request_bytes < write_read.request_bytes);
    }
}
