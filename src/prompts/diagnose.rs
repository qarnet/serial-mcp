use rmcp::model::*;

use crate::prompts::types::DiagnosePortArgs;

/// Build a diagnosis plan prompt for probing an unknown serial device.
///
/// Guides `list_ports` profile preview, bare `open` with profile and baud
/// fallbacks, unsolicited `read`, atomic `capture_boot` with optional DTR/RTS
/// reset, bounded `transact` probes, profile rollback, and clean close.
pub fn build_diagnose_prompt(args: DiagnosePortArgs) -> GetPromptResult {
    let starting = args
        .baud_rate
        .map(|b| b.to_string())
        .unwrap_or_else(|| "115200".into());
    let user = format!(
        "Diagnose serial port `{port}` with the serial MCP tools.\n\
\n\
Plan:\n\
1. Call `list_ports`. Confirm `{port}` is present; otherwise stop and report. Inspect \
`profile_matches`: `selected` means bare open reuses that profile; `none` means a fresh generated \
session starts.\n\
2. Call the bare `open(port=\"{port}\")`. Baud defaults to {starting}; matching profiles apply \
automatically. Report the result's `profile` (name, source, confidence) and whether the session \
was selected or created. If open fails, try 9600, 38400, 115200, 230400, 460800 explicitly until \
one succeeds.\n\
3. Sample unsolicited output with `read(connection_id, timeout_ms=500)`. Devices may print a \
banner on boot or when DTR toggles.\n\
4. For boot/reset capture, use one `capture_boot(connection_id, reset={{assert_dtr=false, \
assert_rts=false, release_dtr=true, release_rts=true, hold_ms=100}}, match=..., timeout_ms=5000)` \
call. It atomically marks the live edge, pulses DTR/RTS (Arduino-style reset), and captures only \
post-reset bytes with a private cursor. No arm/reset race or stale bytes. `reset=null` arms capture \
for externally reset or power-cycled devices.\n\
5. Probe with `transact(connection_id, data=\"AT\\r\\n\", match={{pattern=\"OK\", \
config={{mode=\"literal_substring\", pattern_encoding=\"utf8\"}}}}, timeout_ms=1000)` for one \
bounded write and awaited response. Try `?\\r\\n`, `help\\r\\n`, `\\r\\n` as alternatives.\n\
6. Characterise captured bytes: BOM/banner string, ANSI escapes, hex-only output, and line-ending \
convention.\n\
7. If a probe changed settings and the device misbehaves, `list_profiles` shows the session's \
revision history. Restore a prior snapshot with `rollback_profile`, passing the current revision \
as `expected_revision`.\n\
8. Close cleanly with `close(connection_id)` before reporting.\n\
\n\
Optional for modern clients: `subscriptions/listen` on `serial://ports` or the \
connection's `serial://connections/{{id}}` URI wakes the session when ports change \
or new bytes arrive. Treat it as a hint; always read data with `read`/`transact`.\n\
\n\
Report: device identification (vendor, role, protocol), the working serial parameters \
(baud rate + framing), the bound profile name and persistence state, the prompt string \
(if any), and any anomalies.",
        port = args.port,
        starting = starting
    );
    GetPromptResult::new(vec![PromptMessage::new_text(Role::User, user)])
        .with_description(format!("Diagnosis plan for port {}", args.port))
}
