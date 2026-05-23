//! MCP server tool surface for serial communication.
//!
//! Each `#[tool]` method below corresponds to one MCP tool. The methods are
//! kept small by delegating connection lookups, parsing, and response
//! formatting to helpers further down in the file.

use std::future::Future;
use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::*,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::Deserialize;
use tracing::{debug, error, info};

use crate::codec::{self, Encoding};
use crate::error::SerialError;
use crate::serial::{
    ConnectionConfig, ConnectionManager, DataBits, FlowControl, Parity, PortInfo, SerialConnection,
    StopBits,
};

/// Default read timeout used in response text when the caller did not specify one.
const DEFAULT_READ_TIMEOUT_MS: u64 = 1000;

// ---- Tool argument structs --------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenArgs {
    pub port: String,
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: String,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: String,
    #[serde(default = "default_parity")]
    pub parity: String,
    #[serde(default = "default_flow_control")]
    pub flow_control: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseArgs {
    pub connection_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteArgs {
    pub connection_id: String,
    pub data: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    pub connection_id: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

fn default_data_bits() -> String { "8".into() }
fn default_stop_bits() -> String { "1".into() }
fn default_parity() -> String { "none".into() }
fn default_flow_control() -> String { "none".into() }
fn default_encoding() -> String { "utf8".into() }
fn default_max_bytes() -> usize { 1024 }

// ---- Handler ---------------------------------------------------------------

#[derive(Clone)]
pub struct SerialHandler {
    connections: Arc<ConnectionManager>,
    tool_router: ToolRouter<SerialHandler>,
}

#[tool_router]
impl SerialHandler {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(ConnectionManager::new()),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List all available serial ports on the system")]
    async fn list_ports(&self) -> Result<CallToolResult, McpError> {
        debug!("Listing serial ports");
        let ports = PortInfo::list_available()
            .map_err(|e| log_and_internal_err("list_ports", "Failed to list ports", e))?;
        info!("Found {} serial ports", ports.len());
        Ok(success(format_port_list(&ports)))
    }

    #[tool(description = "Open a serial port connection with specified configuration")]
    async fn open(
        &self,
        Parameters(args): Parameters<OpenArgs>,
    ) -> Result<CallToolResult, McpError> {
        let config = parse_open_args(args).map_err(internal_err)?;
        let port = config.port.clone();
        let baud_rate = config.baud_rate;
        debug!("Opening {} @ {}", port, baud_rate);

        let id = self.connections.open(config).await.map_err(|e| {
            log_and_internal_err("open", &format!("Failed to open port {}", port), e)
        })?;
        info!("Opened connection {} -> {}", id, port);
        Ok(success(format_open_result(&id, &port, baud_rate)))
    }

    #[tool(description = "Close an open serial port connection")]
    async fn close(
        &self,
        Parameters(args): Parameters<CloseArgs>,
    ) -> Result<CallToolResult, McpError> {
        debug!("Closing {}", args.connection_id);
        self.connections.close(&args.connection_id).await.map_err(|e| {
            log_and_internal_err(
                "close",
                &format!("Failed to close connection {}", args.connection_id),
                e,
            )
        })?;
        info!("Closed connection {}", args.connection_id);
        Ok(success(format_close_result(&args.connection_id)))
    }

    #[tool(description = "Write data to a serial port connection")]
    async fn write(
        &self,
        Parameters(args): Parameters<WriteArgs>,
    ) -> Result<CallToolResult, McpError> {
        debug!("Write to {} ({})", args.connection_id, args.encoding);
        let encoding = parse_encoding(&args.encoding)?;
        let connection = self.lookup_connection(&args.connection_id).await?;
        let bytes = codec::decode(encoding, &args.data)
            .map_err(|e| internal_err(format!("Data decoding failed - {}", e)))?;
        let written = connection.write(&bytes).await.map_err(|e| {
            log_and_internal_err(
                "write",
                &format!("Data sending failed on {}", args.connection_id),
                e,
            )
        })?;
        debug!("Wrote {} bytes to {}", written, args.connection_id);
        Ok(success(format_write_result(&args.connection_id, written, &args.data)))
    }

    #[tool(description = "Read data from a serial port connection")]
    async fn read(
        &self,
        Parameters(args): Parameters<ReadArgs>,
    ) -> Result<CallToolResult, McpError> {
        debug!("Read from {} (timeout {:?})", args.connection_id, args.timeout_ms);
        let encoding = parse_encoding(&args.encoding)?;
        let connection = self.lookup_connection(&args.connection_id).await?;
        let outcome = read_bytes(&connection, args.max_bytes, args.timeout_ms).await?;
        format_read_outcome(outcome, &args.connection_id, encoding, args.timeout_ms)
    }
}

// Lookup is split out so the macro-generated tool methods stay focused.
impl SerialHandler {
    /// Resolve an MCP connection id into a live [`SerialConnection`].
    async fn lookup_connection(&self, id: &str) -> Result<Arc<SerialConnection>, McpError> {
        self.connections
            .get(id)
            .await
            .map_err(|_| internal_err(format!("Connection ID {} not found", id)))
    }
}

// ---- Tool helpers (free fns) ------------------------------------------------

/// Outcome of a successful read call, distinguishing actual data from an
/// empty buffer (which serial ports report on timeout when no `timeout_ms`
/// was set or when the OS reports EOF).
struct ReadOutcome {
    bytes: Vec<u8>,
    timed_out: bool,
}

async fn read_bytes(
    connection: &SerialConnection,
    max_bytes: usize,
    timeout_ms: Option<u64>,
) -> Result<ReadOutcome, McpError> {
    let mut buf = vec![0u8; max_bytes];
    match connection.read(&mut buf, timeout_ms).await {
        Ok(n) => {
            buf.truncate(n);
            Ok(ReadOutcome { bytes: buf, timed_out: n == 0 })
        }
        Err(SerialError::ReadTimeout) => Ok(ReadOutcome { bytes: Vec::new(), timed_out: true }),
        Err(e) => Err(log_and_internal_err("read", "Data reading failed", e)),
    }
}

fn format_read_outcome(
    outcome: ReadOutcome,
    connection_id: &str,
    encoding: Encoding,
    timeout_ms: Option<u64>,
) -> Result<CallToolResult, McpError> {
    if outcome.timed_out {
        return Ok(success(format_read_timeout(connection_id, timeout_ms)));
    }
    let encoded = codec::encode(encoding, &outcome.bytes)
        .map_err(|e| internal_err(format!("Data encoding failed - {}", e)))?;
    Ok(success(format_read_ok(connection_id, outcome.bytes.len(), &encoded)))
}

fn parse_encoding(raw: &str) -> Result<Encoding, McpError> {
    raw.parse::<Encoding>()
        .map_err(|e| internal_err(format!("Unsupported encoding - {}", e)))
}

/// Strictly parse [`OpenArgs`] into a [`ConnectionConfig`]. Unlike the
/// previous silent-fallback version, an unrecognised value here is an error.
fn parse_open_args(args: OpenArgs) -> Result<ConnectionConfig, String> {
    Ok(ConnectionConfig {
        port: args.port,
        baud_rate: args.baud_rate,
        data_bits: parse_data_bits(&args.data_bits)?,
        stop_bits: parse_stop_bits(&args.stop_bits)?,
        parity: parse_parity(&args.parity)?,
        flow_control: parse_flow_control(&args.flow_control)?,
    })
}

fn parse_data_bits(raw: &str) -> Result<DataBits, String> {
    match raw {
        "5" => Ok(DataBits::Five),
        "6" => Ok(DataBits::Six),
        "7" => Ok(DataBits::Seven),
        "8" => Ok(DataBits::Eight),
        other => Err(format!("Invalid data_bits {:?} (expected 5/6/7/8)", other)),
    }
}

fn parse_stop_bits(raw: &str) -> Result<StopBits, String> {
    match raw {
        "1" => Ok(StopBits::One),
        "2" => Ok(StopBits::Two),
        other => Err(format!("Invalid stop_bits {:?} (expected 1/2)", other)),
    }
}

fn parse_parity(raw: &str) -> Result<Parity, String> {
    match raw.to_lowercase().as_str() {
        "none" => Ok(Parity::None),
        "odd" => Ok(Parity::Odd),
        "even" => Ok(Parity::Even),
        other => Err(format!("Invalid parity {:?} (expected none/odd/even)", other)),
    }
}

fn parse_flow_control(raw: &str) -> Result<FlowControl, String> {
    match raw.to_lowercase().as_str() {
        "none" => Ok(FlowControl::None),
        "software" => Ok(FlowControl::Software),
        "hardware" => Ok(FlowControl::Hardware),
        other => Err(format!(
            "Invalid flow_control {:?} (expected none/software/hardware)",
            other
        )),
    }
}

// ---- Response formatting (pure) --------------------------------------------

fn format_port_list(ports: &[PortInfo]) -> String {
    if ports.is_empty() {
        return "No serial ports found on the system".to_string();
    }
    let lines: Vec<String> = ports.iter().map(format_port_line).collect();
    format!("Found {} serial ports:\n{}", ports.len(), lines.join("\n"))
}

fn format_port_line(port: &PortInfo) -> String {
    match &port.hardware_id {
        Some(hw) => format!("- {}: {} ({})", port.name, port.description, hw),
        None => format!("- {}: {}", port.name, port.description),
    }
}

fn format_open_result(id: &str, port: &str, baud_rate: u32) -> String {
    format!(
        "Serial connection opened\nConnection ID: {}\nPort: {}\nBaud rate: {}",
        id, port, baud_rate
    )
}

fn format_close_result(id: &str) -> String {
    format!("Serial connection closed\nConnection ID: {}", id)
}

fn format_write_result(id: &str, bytes_written: usize, original_data: &str) -> String {
    format!(
        "Data sent successfully\nConnection ID: {}\nBytes written: {}\nData: {:?}",
        id, bytes_written, original_data
    )
}

fn format_read_ok(id: &str, bytes_read: usize, encoded: &str) -> String {
    format!(
        "Data read successfully\nConnection ID: {}\nBytes read: {}\nData: {:?}",
        id, bytes_read, encoded
    )
}

fn format_read_timeout(id: &str, timeout_ms: Option<u64>) -> String {
    format!(
        "Read timeout\nConnection ID: {}\nTimeout: {}ms\nBytes read: 0",
        id,
        timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS)
    )
}

// ---- Tiny error / result builders -------------------------------------------

fn success(text: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text)])
}

fn internal_err<M: Into<String>>(message: M) -> McpError {
    McpError::internal_error(format!("Error: {}", message.into()), None)
}

fn log_and_internal_err<E: std::fmt::Display>(op: &str, context: &str, err: E) -> McpError {
    error!("{} failed: {}", op, err);
    internal_err(format!("{} - {}", context, err))
}

// ---- ServerHandler boilerplate ----------------------------------------------

impl Default for SerialHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler]
impl ServerHandler for SerialHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "A serial port communication MCP server. Use list_ports to discover available serial ports, then open connections to communicate with serial devices.".into(),
            ),
        }
    }

    async fn initialize(
        &self,
        _req: InitializeRequestParam,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        info!("Serial MCP server initialized");
        Ok(self.get_info())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_args_parsed_strictly() {
        let args = OpenArgs {
            port: "/dev/ttyUSB0".into(),
            baud_rate: 115200,
            data_bits: "8".into(),
            stop_bits: "1".into(),
            parity: "none".into(),
            flow_control: "none".into(),
        };
        let config = parse_open_args(args).unwrap();
        assert_eq!(config.port, "/dev/ttyUSB0");
        assert_eq!(config.baud_rate, 115200);
    }

    #[test]
    fn open_args_reject_invalid_data_bits() {
        let args = OpenArgs {
            port: "X".into(),
            baud_rate: 9600,
            data_bits: "9".into(),
            stop_bits: "1".into(),
            parity: "none".into(),
            flow_control: "none".into(),
        };
        let err = parse_open_args(args).unwrap_err();
        assert!(err.contains("data_bits"));
    }

    #[test]
    fn open_args_reject_invalid_parity() {
        assert!(parse_parity("weird").is_err());
        assert!(parse_parity("none").is_ok());
        assert!(parse_parity("Even").is_ok());
    }

    #[test]
    fn format_port_list_empty() {
        assert_eq!(
            format_port_list(&[]),
            "No serial ports found on the system"
        );
    }

    #[test]
    fn format_port_list_with_entries() {
        let ports = vec![
            PortInfo {
                name: "COM1".into(),
                description: "Serial Port".into(),
                hardware_id: None,
            },
            PortInfo {
                name: "COM3".into(),
                description: "USB Serial Device".into(),
                hardware_id: Some("USB VID:1234 PID:5678".into()),
            },
        ];
        let rendered = format_port_list(&ports);
        assert!(rendered.starts_with("Found 2 serial ports:\n"));
        assert!(rendered.contains("- COM1: Serial Port"));
        assert!(rendered.contains("- COM3: USB Serial Device (USB VID:1234 PID:5678)"));
    }

    #[test]
    fn format_read_timeout_uses_default_when_unset() {
        let text = format_read_timeout("conn-123", None);
        assert!(text.contains(&format!("{}ms", DEFAULT_READ_TIMEOUT_MS)));
        assert!(text.contains("conn-123"));
    }

    #[test]
    fn format_read_timeout_uses_supplied_value() {
        let text = format_read_timeout("conn-123", Some(50));
        assert!(text.contains("50ms"));
    }

    #[test]
    fn parse_encoding_rejects_garbage() {
        assert!(parse_encoding("rot13").is_err());
        assert!(parse_encoding("utf-8").is_ok());
    }
}
