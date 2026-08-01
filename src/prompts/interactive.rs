use rmcp::model::*;

use crate::prompts::types::InteractiveTerminalArgs;

/// Build an interactive terminal REPL prompt for an open serial connection.
///
/// Drives bounded command/response with `transact` (one call per command)
/// instead of separate `write`+`read` round trips.
pub fn build_interactive_prompt(args: InteractiveTerminalArgs) -> GetPromptResult {
    let line_ending = args.line_ending.as_deref().unwrap_or("\\r\\n");
    let device_prompt = args
        .device_prompt
        .as_deref()
        .map(|p| format!("`{p}`"))
        .unwrap_or_else(|| "the device's prompt string (e.g. `OK>`, `$ `)".to_string());
    let user = format!(
        "Act as a serial terminal client against connection `{id}`. Use the serial \
MCP tools. Conventions:\n\
\n\
- Append `{line_ending}` to every line the user wants to send.\n\
- For each user command use ONE `transact(connection_id=\"{id}\", data=..., \
match={{pattern: {prompt}}}, timeout_ms=2000)` call — it writes and awaits the response \
up to {prompt} in a single round trip. Prefer `transact` over separate `write`+`read`.\n\
- If the transact read times out, surface the partial buffer and ask the user how to proceed \
instead of retrying blindly.\n\
- Decode the response data as UTF-8 unless it contains bytes the codec rejects, in \
which case fall back to hex and tell the user.\n\
- Never call `close` unless the user explicitly says so.\n\
- If the connection vanishes (tool returns Connection ID not found), tell the user \
and stop; do not silently reopen.\n\
\n\
Begin by sending an empty line (transact with `{line_ending}` and the match pattern) to surface \
the current prompt, then report back and wait for the user's first command.",
        id = args.connection_id,
        line_ending = line_ending,
        prompt = device_prompt
    );
    GetPromptResult::new(vec![PromptMessage::new_text(PromptMessageRole::User, user)])
        .with_description(format!(
            "Interactive REPL session over connection {}",
            args.connection_id
        ))
}
