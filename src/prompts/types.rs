use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DiagnosePortArgs {
    /// OS-level port name to probe (e.g. "COM3", "/dev/ttyUSB0").
    pub port: String,
    /// Optional baud rate to try first. Defaults are tried otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baud_rate: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct InteractiveTerminalArgs {
    /// Existing connection_id returned by the `open` tool.
    pub connection_id: String,
    /// Optional line ending to append when writing user-typed lines.
    /// Defaults to `\r\n`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_ending: Option<String>,
    /// Optional prompt the device emits at the end of each response
    /// (e.g. "OK>", "$ "). Used by `wait_for` between commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_prompt: Option<String>,
}
