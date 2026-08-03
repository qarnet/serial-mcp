use rmcp::model::*;

use crate::prompts::types::DiagnosePortArgs;

/// Build a diagnosis plan prompt for probing an unknown serial device.
///
/// Teaches the common path: `list_ports` profile preview → bare `open`
/// (automatic 115200 fallback + profile selection) → `transact` probes →
/// inspect/roll back the learned profile when a probe misfired.
/// `capture_boot` is the boot/reset capture path (atomic mark + optional
/// DTR/RTS pulse + private-cursor capture).
pub fn build_diagnose_prompt(args: DiagnosePortArgs) -> GetPromptResult {
    let starting = args
        .baud_rate
        .map(|b| b.to_string())
        .unwrap_or_else(|| "115200".into());
    let user = format!(
        "Diagnose what's on serial port `{port}`. Use the serial MCP tools.\n\
\n\
Plan:\n\
1. Call `list_ports` and confirm `{port}` is present; if not, stop and report. Inspect its \
`profile_matches` entry: `selected` means a bare open will reuse that profile; `none` means a \
fresh generated session will start.\n\
2. Open with the bare common call `open(port=\"{port}\")` — baud defaults to {starting} \
and any matching profile is applied automatically. Read the result's `profile` (name, source, \
confidence) and report which profile session was selected or created. If open fails, try 9600, \
38400, 115200, 230400, 460800 explicitly until one succeeds.\n\
3. Sample unsolicited output with `read(connection_id, timeout_ms=500)`. Many devices print a \
banner on boot or when DTR toggles.\n\
4. For boot/reset capture use ONE `capture_boot(connection_id, reset={{assert_dtr=false, \
assert_rts=false, release_dtr=true, release_rts=true, hold_ms=100}}, match=..., timeout_ms=5000)` \
call: it atomically marks the live edge, pulses DTR/RTS (Arduino-style reset), and captures only \
post-reset bytes with a private cursor — no arm/reset race, no stale bytes. `reset=null` arms \
capture for externally reset/power-cycled devices.\n\
5. Probe with `transact(connection_id, data=\"AT\\r\\n\", match={{pattern=\"OK\", \
config={{mode=\"literal_substring\", pattern_encoding=\"utf8\"}}}}, timeout_ms=1000)` — one call \
for write + awaited response. Try `?\\r\\n`, `help\\r\\n`, `\\r\\n` as alternatives.\n\
6. From the captured bytes, characterise the device: BOM/banner string, presence of ANSI \
escapes, hex-only output, line-ending convention.\n\
7. If a probe changed settings and the device misbehaves, `list_profiles` shows the session's \
revision history; restore a prior snapshot with `rollback_profile` (pass the current revision \
as `expected_revision`).\n\
8. Close the connection cleanly with `close(connection_id)` before reporting.\n\
\n\
Optional for modern clients: `subscriptions/listen` on `serial://ports` or the \
connection's `serial://connections/{{id}}` URI wakes the session when ports change \
or new bytes arrive — a hint only; always read data with `read`/`transact`.\n\
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
