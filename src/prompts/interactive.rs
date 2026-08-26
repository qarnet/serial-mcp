use rmcp::model::*;

use crate::prompts::types::InteractiveTerminalArgs;

/// Build an interactive terminal REPL prompt for an open serial connection.
///
/// Uses one bounded `transact` call for each command/response instead of
/// separate `write` and `read` round trips.
pub fn build_interactive_prompt(args: InteractiveTerminalArgs) -> GetPromptResult {
    let line_ending = args.line_ending.as_deref().unwrap_or("\\r\\n");
    let device_prompt = args
        .device_prompt
        .as_deref()
        .map(|p| format!("`{p}`"))
        .unwrap_or_else(|| "the device's prompt string (e.g. `OK>`, `$ `)".to_string());
    let user = format!(
        "Act as a serial terminal client for connection `{id}`. Use the serial MCP \
tools. Rules:\n\
\n\
- Append `{line_ending}` to every line the user wants to send.\n\
- For each user command use one `transact(connection_id=\"{id}\", data=..., \
match={{pattern: {prompt}}}, timeout_ms=2000)` call. It writes and awaits the response \
up to {prompt} in one round trip. Use `transact` instead of separate `write`+`read`.\n\
- If the transact read times out, surface the partial buffer and ask how to proceed; do not retry \
blindly.\n\
- Decode response data as UTF-8. If the codec rejects bytes, use lossless hex fallback and tell the \
user.\n\
- Never call `close` unless the user explicitly says so.\n\
- If the connection vanishes (tool returns Connection ID not found), tell the user \
and stop. Do not silently reopen.\n\
- Optional: for modern clients, `subscriptions/listen` on \
`serial://connections/{id}` can wake the session when new RX bytes arrive. \
Notifications are hints only. Always use `transact`/`read` to fetch data.\n\
\n\
Begin by sending an empty line (transact with `{line_ending}` and the match pattern) to surface \
the current prompt, then report back and wait for the user's first command.",
        id = args.connection_id,
        line_ending = line_ending,
        prompt = device_prompt
    );
    GetPromptResult::new(vec![PromptMessage::new_text(Role::User, user)]).with_description(format!(
        "Interactive REPL session over connection {}",
        args.connection_id
    ))
}
