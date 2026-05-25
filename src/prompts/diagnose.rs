use rmcp::model::*;

use crate::prompts::types::DiagnosePortArgs;

/// Build a diagnosis plan prompt for probing an unknown serial device.
pub fn build_diagnose_prompt(args: DiagnosePortArgs) -> GetPromptResult {
    let starting = args
        .baud_rate
        .map(|b| b.to_string())
        .unwrap_or_else(|| "115200".into());
    let user = format!(
        "Diagnose what's on serial port `{port}`. Use the serial MCP tools.\n\
\n\
Plan:\n\
1. Call `list_ports` and confirm `{port}` is present; if not, stop and report.\n\
2. Open the port with `open(port=\"{port}\", baud_rate={starting})`. If it fails, try \
9600, 38400, 115200, 230400, 460800 in turn until one succeeds.\n\
3. Call `read(connection_id, timeout_ms=500, max_bytes=512)` to sample unsolicited \
output. Many devices print a banner on boot or when DTR toggles.\n\
4. If silent, toggle DTR with `set_dtr_rts(connection_id, dtr=false, rts=false)` then \
`set_dtr_rts(connection_id, dtr=true, rts=true)` to soft-reset Arduino-style boards, \
and re-read.\n\
5. If still silent, send a benign probe via `write(connection_id, data=\"AT\\r\\n\", \
encoding=\"utf8\")` then `wait_for(connection_id, pattern=\"OK\", timeout_ms=1000)`. \
Try `?\\r\\n`, `help\\r\\n`, `\\r\\n` as alternatives.\n\
6. From the captured bytes, characterise the device: BOM/banner string, presence of \
ANSI escapes, hex-only output, line-ending convention.\n\
7. Close the connection cleanly with `close(connection_id)` before reporting.\n\
\n\
Report: device identification (vendor, role, protocol), the working serial parameters \
(baud rate + framing), the prompt string (if any), and any anomalies.",
        port = args.port,
        starting = starting
    );
    GetPromptResult::new(vec![PromptMessage::new_text(PromptMessageRole::User, user)])
        .with_description(format!("Diagnosis plan for port {}", args.port))
}
