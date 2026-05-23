//! MCP server tool surface for serial communication.
//!
//! Each `#[tool]` method below corresponds to one MCP tool. Tools return
//! structured JSON via [`Json<T>`] so MCP clients can index fields directly
//! instead of parsing free-form text.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, Json, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::codec::{self, Encoding};
use crate::error::SerialError;
use crate::serial::{
    ConnectionConfig, ConnectionManager, DataBits, FlowControl, FlushTarget, Parity, PortInfo,
    SerialConnection, StopBits,
};

/// Default read timeout used in the response when the caller did not specify one.
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FlushArgs {
    pub connection_id: String,
    #[serde(default = "default_flush_target")]
    pub target: FlushTarget,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetDtrRtsArgs {
    pub connection_id: String,
    pub dtr: bool,
    pub rts: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendBreakArgs {
    pub connection_id: String,
    #[serde(default = "default_break_duration_ms")]
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitForArgs {
    pub connection_id: String,
    /// Byte pattern to wait for, in the encoding given by `pattern_encoding`.
    pub pattern: String,
    #[serde(default = "default_encoding")]
    pub pattern_encoding: String,
    #[serde(default = "default_wait_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_wait_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_encoding")]
    pub response_encoding: String,
}

fn default_data_bits() -> String { "8".into() }
fn default_stop_bits() -> String { "1".into() }
fn default_parity() -> String { "none".into() }
fn default_flow_control() -> String { "none".into() }
fn default_encoding() -> String { "utf8".into() }
fn default_max_bytes() -> usize { 1024 }
fn default_flush_target() -> FlushTarget { FlushTarget::Both }
fn default_break_duration_ms() -> u64 { 250 }
fn default_wait_timeout_ms() -> u64 { 5000 }
fn default_wait_max_bytes() -> usize { 4096 }

// ---- Tool response structs --------------------------------------------------

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListPortsResult {
    pub count: usize,
    pub ports: Vec<PortInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenResult {
    pub connection_id: String,
    pub port: String,
    pub baud_rate: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CloseResult {
    pub connection_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteResult {
    pub connection_id: String,
    pub bytes_written: usize,
    pub encoding: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadResult {
    pub connection_id: String,
    pub bytes_read: usize,
    pub encoding: String,
    pub data: String,
    pub timed_out: bool,
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FlushResult {
    pub connection_id: String,
    pub target: FlushTarget,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SetDtrRtsResult {
    pub connection_id: String,
    pub dtr: bool,
    pub rts: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SendBreakResult {
    pub connection_id: String,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WaitForResult {
    pub connection_id: String,
    pub matched: bool,
    pub timed_out: bool,
    /// All bytes accumulated up to and including the match (when matched).
    /// Encoded with `response_encoding` from the request.
    pub data: String,
    pub bytes_read: usize,
    /// Byte offset of the start of the matched pattern within the response
    /// buffer, when matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_index: Option<usize>,
    pub timeout_ms: u64,
    pub response_encoding: String,
}

// ---- Handler ---------------------------------------------------------------

#[derive(Clone)]
pub struct SerialHandler {
    connections: Arc<ConnectionManager>,
    #[allow(dead_code)]
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
    async fn list_ports(&self) -> Result<Json<ListPortsResult>, String> {
        debug!("Listing serial ports");
        let ports = PortInfo::list_available()
            .map_err(|e| log_tool_err("list_ports", "Failed to list ports", e))?;
        info!("Found {} serial ports", ports.len());
        Ok(Json(ListPortsResult { count: ports.len(), ports }))
    }

    #[tool(description = "Open a serial port connection with specified configuration")]
    async fn open(
        &self,
        Parameters(args): Parameters<OpenArgs>,
    ) -> Result<Json<OpenResult>, String> {
        let config = parse_open_args(args)?;
        let port = config.port.clone();
        let baud_rate = config.baud_rate;
        debug!("Opening {} @ {}", port, baud_rate);

        let connection_id = self.connections.open(config).await.map_err(|e| {
            log_tool_err("open", &format!("Failed to open port {}", port), e)
        })?;
        info!("Opened connection {} -> {}", connection_id, port);
        Ok(Json(OpenResult { connection_id, port, baud_rate }))
    }

    #[tool(description = "Close an open serial port connection")]
    async fn close(
        &self,
        Parameters(args): Parameters<CloseArgs>,
    ) -> Result<Json<CloseResult>, String> {
        debug!("Closing {}", args.connection_id);
        self.connections.close(&args.connection_id).await.map_err(|e| {
            log_tool_err(
                "close",
                &format!("Failed to close connection {}", args.connection_id),
                e,
            )
        })?;
        info!("Closed connection {}", args.connection_id);
        Ok(Json(CloseResult { connection_id: args.connection_id }))
    }

    #[tool(description = "Write data to a serial port connection")]
    async fn write(
        &self,
        Parameters(args): Parameters<WriteArgs>,
    ) -> Result<Json<WriteResult>, String> {
        debug!("Write to {} ({})", args.connection_id, args.encoding);
        let encoding = parse_encoding(&args.encoding)?;
        let connection = self.lookup_connection(&args.connection_id).await?;
        let bytes = codec::decode(encoding, &args.data)
            .map_err(|e| format!("Data decoding failed - {}", e))?;
        let bytes_written = connection.write(&bytes).await.map_err(|e| {
            log_tool_err(
                "write",
                &format!("Data sending failed on {}", args.connection_id),
                e,
            )
        })?;
        debug!("Wrote {} bytes to {}", bytes_written, args.connection_id);
        Ok(Json(WriteResult {
            connection_id: args.connection_id,
            bytes_written,
            encoding: encoding.to_string(),
        }))
    }

    #[tool(description = "Read data from a serial port connection")]
    async fn read(
        &self,
        Parameters(args): Parameters<ReadArgs>,
    ) -> Result<Json<ReadResult>, String> {
        debug!("Read from {} (timeout {:?})", args.connection_id, args.timeout_ms);
        let encoding = parse_encoding(&args.encoding)?;
        let connection = self.lookup_connection(&args.connection_id).await?;
        let outcome = read_bytes(&connection, args.max_bytes, args.timeout_ms).await?;
        build_read_result(outcome, args.connection_id, encoding, args.timeout_ms)
    }

    #[tool(
        description = "Discard buffered serial data. target=input clears OS read buffer (data the device sent that the app hasn't consumed); target=output clears the OS write queue; target=both clears both."
    )]
    async fn flush(
        &self,
        Parameters(args): Parameters<FlushArgs>,
    ) -> Result<Json<FlushResult>, String> {
        debug!("Flush {} target={:?}", args.connection_id, args.target);
        let connection = self.lookup_connection(&args.connection_id).await?;
        connection.flush_buffers(args.target).await.map_err(|e| {
            log_tool_err(
                "flush",
                &format!("Failed to flush {}", args.connection_id),
                e,
            )
        })?;
        info!("Flushed {} ({:?})", args.connection_id, args.target);
        Ok(Json(FlushResult { connection_id: args.connection_id, target: args.target }))
    }

    #[tool(
        description = "Set the DTR and RTS modem-control lines. Common patterns: pulse DTR low for Arduino auto-reset; hold both low to enter ESP32 bootloader."
    )]
    async fn set_dtr_rts(
        &self,
        Parameters(args): Parameters<SetDtrRtsArgs>,
    ) -> Result<Json<SetDtrRtsResult>, String> {
        debug!("set_dtr_rts {} dtr={} rts={}", args.connection_id, args.dtr, args.rts);
        let connection = self.lookup_connection(&args.connection_id).await?;
        connection.set_dtr_rts(args.dtr, args.rts).await.map_err(|e| {
            log_tool_err(
                "set_dtr_rts",
                &format!("Failed to set control lines on {}", args.connection_id),
                e,
            )
        })?;
        info!("Control lines on {} set to dtr={} rts={}", args.connection_id, args.dtr, args.rts);
        Ok(Json(SetDtrRtsResult {
            connection_id: args.connection_id,
            dtr: args.dtr,
            rts: args.rts,
        }))
    }

    #[tool(
        description = "Assert a BREAK condition on the TX line for duration_ms milliseconds (default 250ms), then release it. Used to signal attention on some legacy serial protocols."
    )]
    async fn send_break(
        &self,
        Parameters(args): Parameters<SendBreakArgs>,
    ) -> Result<Json<SendBreakResult>, String> {
        debug!("send_break {} duration={}ms", args.connection_id, args.duration_ms);
        let connection = self.lookup_connection(&args.connection_id).await?;
        connection.send_break(args.duration_ms).await.map_err(|e| {
            log_tool_err(
                "send_break",
                &format!("Failed to send break on {}", args.connection_id),
                e,
            )
        })?;
        info!("Sent break on {} for {}ms", args.connection_id, args.duration_ms);
        Ok(Json(SendBreakResult {
            connection_id: args.connection_id,
            duration_ms: args.duration_ms,
        }))
    }

    #[tool(
        description = "Read bytes from a connection until a pattern matches or timeout. Pattern is interpreted with pattern_encoding (utf8/hex/base64). Returns the accumulated bytes (re-encoded with response_encoding) and the byte offset where the match started. Use for prompt/response interactions, e.g. send 'reset\\r\\n' then wait_for pattern='OK>'."
    )]
    async fn wait_for(
        &self,
        Parameters(args): Parameters<WaitForArgs>,
    ) -> Result<Json<WaitForResult>, String> {
        debug!(
            "wait_for {} pattern_encoding={} timeout={}ms max_bytes={}",
            args.connection_id, args.pattern_encoding, args.timeout_ms, args.max_bytes
        );
        let pattern_encoding = parse_encoding(&args.pattern_encoding)?;
        let response_encoding = parse_encoding(&args.response_encoding)?;
        let pattern = codec::decode(pattern_encoding, &args.pattern)
            .map_err(|e| format!("Pattern decoding failed - {}", e))?;
        if pattern.is_empty() {
            return Err("Pattern must not be empty".into());
        }

        let connection = self.lookup_connection(&args.connection_id).await?;
        let outcome =
            read_until_pattern(&connection, &pattern, args.timeout_ms, args.max_bytes).await?;
        let bytes_read = outcome.bytes.len();
        let data = codec::encode(response_encoding, &outcome.bytes)
            .map_err(|e| format!("Response encoding failed - {}", e))?;
        Ok(Json(WaitForResult {
            connection_id: args.connection_id,
            matched: outcome.match_index.is_some(),
            timed_out: outcome.timed_out,
            data,
            bytes_read,
            match_index: outcome.match_index,
            timeout_ms: args.timeout_ms,
            response_encoding: response_encoding.to_string(),
        }))
    }
}

// Lookup is split out so the macro-generated tool methods stay focused.
impl SerialHandler {
    /// Resolve an MCP connection id into a live [`SerialConnection`].
    async fn lookup_connection(&self, id: &str) -> Result<Arc<SerialConnection>, String> {
        self.connections
            .get(id)
            .await
            .map_err(|_| format!("Connection ID {} not found", id))
    }
}

// ---- Tool helpers (free fns) ------------------------------------------------

/// Outcome of a read call. `timed_out` distinguishes the genuine
/// read-timeout case from a successful read of `bytes`.
struct ReadOutcome {
    bytes: Vec<u8>,
    timed_out: bool,
}

async fn read_bytes(
    connection: &SerialConnection,
    max_bytes: usize,
    timeout_ms: Option<u64>,
) -> Result<ReadOutcome, String> {
    let mut buf = vec![0u8; max_bytes];
    match connection.read(&mut buf, timeout_ms).await {
        Ok(n) => {
            buf.truncate(n);
            Ok(ReadOutcome { bytes: buf, timed_out: n == 0 })
        }
        Err(SerialError::ReadTimeout) => Ok(ReadOutcome { bytes: Vec::new(), timed_out: true }),
        Err(e) => Err(log_tool_err("read", "Data reading failed", e)),
    }
}

/// Read incrementally from `connection` until `pattern` appears in the
/// accumulated buffer, `max_bytes` are buffered without a match, or
/// `timeout_ms` elapses since this call began.
async fn read_until_pattern(
    connection: &SerialConnection,
    pattern: &[u8],
    timeout_ms: u64,
    max_bytes: usize,
) -> Result<WaitOutcome, String> {
    const CHUNK_CAPACITY: usize = 256;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut accumulated: Vec<u8> = Vec::with_capacity(CHUNK_CAPACITY.min(max_bytes));

    loop {
        if let Some(idx) = find_subslice(&accumulated, pattern) {
            return Ok(WaitOutcome {
                bytes: accumulated,
                match_index: Some(idx),
                timed_out: false,
            });
        }
        if accumulated.len() >= max_bytes {
            return Ok(WaitOutcome { bytes: accumulated, match_index: None, timed_out: false });
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(WaitOutcome { bytes: accumulated, match_index: None, timed_out: true });
        }

        let remaining_ms = (deadline - now).as_millis() as u64;
        let room = (max_bytes - accumulated.len()).min(CHUNK_CAPACITY);
        let mut chunk = vec![0u8; room];
        match connection.read(&mut chunk, Some(remaining_ms)).await {
            Ok(0) => continue,
            Ok(n) => {
                chunk.truncate(n);
                accumulated.extend_from_slice(&chunk);
            }
            Err(SerialError::ReadTimeout) => {
                return Ok(WaitOutcome { bytes: accumulated, match_index: None, timed_out: true });
            }
            Err(e) => return Err(log_tool_err("wait_for", "Read failed during wait", e)),
        }
    }
}

struct WaitOutcome {
    bytes: Vec<u8>,
    match_index: Option<usize>,
    timed_out: bool,
}

/// Find the first byte offset where `needle` appears in `haystack`. Returns
/// `None` if `needle` is empty or absent.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn build_read_result(
    outcome: ReadOutcome,
    connection_id: String,
    encoding: Encoding,
    requested_timeout_ms: Option<u64>,
) -> Result<Json<ReadResult>, String> {
    let timeout_ms = requested_timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    if outcome.timed_out {
        return Ok(Json(ReadResult {
            connection_id,
            bytes_read: 0,
            encoding: encoding.to_string(),
            data: String::new(),
            timed_out: true,
            timeout_ms,
        }));
    }
    let bytes_read = outcome.bytes.len();
    let data = codec::encode(encoding, &outcome.bytes)
        .map_err(|e| format!("Data encoding failed - {}", e))?;
    Ok(Json(ReadResult {
        connection_id,
        bytes_read,
        encoding: encoding.to_string(),
        data,
        timed_out: false,
        timeout_ms,
    }))
}

fn parse_encoding(raw: &str) -> Result<Encoding, String> {
    raw.parse::<Encoding>()
        .map_err(|e| format!("Unsupported encoding - {}", e))
}

/// Strictly parse [`OpenArgs`] into a [`ConnectionConfig`]. An unrecognised
/// value here is an error rather than a silent fallback to defaults.
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

// ---- Tiny error builders ----------------------------------------------------

/// Log a tool-level failure and format a user-visible error string that the
/// rmcp router will surface as a `CallToolResult { isError: true, ... }`.
fn log_tool_err<E: std::fmt::Display>(op: &str, context: &str, err: E) -> String {
    error!("{} failed: {}", op, err);
    format!("{} - {}", context, err)
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
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "A serial port communication MCP server. Use list_ports to discover available serial ports, then open connections to communicate with serial devices."
                    .to_string(),
            )
    }

    async fn initialize(
        &self,
        _req: InitializeRequestParams,
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
    fn parse_encoding_rejects_garbage() {
        assert!(parse_encoding("rot13").is_err());
        assert!(parse_encoding("utf-8").is_ok());
    }

    #[test]
    fn build_read_result_timeout_branch() {
        let outcome = ReadOutcome { bytes: Vec::new(), timed_out: true };
        let Json(result) = build_read_result(outcome, "abc".into(), Encoding::Utf8, Some(250))
            .expect("timeout result must build");
        assert!(result.timed_out);
        assert_eq!(result.bytes_read, 0);
        assert_eq!(result.timeout_ms, 250);
        assert!(result.data.is_empty());
    }

    #[test]
    fn build_read_result_timeout_uses_default() {
        let outcome = ReadOutcome { bytes: Vec::new(), timed_out: true };
        let Json(result) = build_read_result(outcome, "abc".into(), Encoding::Hex, None)
            .expect("timeout result must build");
        assert_eq!(result.timeout_ms, DEFAULT_READ_TIMEOUT_MS);
    }

    #[test]
    fn build_read_result_data_branch_encodes_hex() {
        let outcome = ReadOutcome { bytes: b"Hi".to_vec(), timed_out: false };
        let Json(result) = build_read_result(outcome, "abc".into(), Encoding::Hex, Some(500))
            .expect("data result must build");
        assert!(!result.timed_out);
        assert_eq!(result.bytes_read, 2);
        assert_eq!(result.encoding, "hex");
        assert_eq!(result.data, "48 69");
    }

    #[test]
    fn find_subslice_locates_pattern() {
        assert_eq!(find_subslice(b"hello OK> world", b"OK>"), Some(6));
        assert_eq!(find_subslice(b"OK>at-start", b"OK>"), Some(0));
        assert_eq!(find_subslice(b"trailing OK>", b"OK>"), Some(9));
    }

    #[test]
    fn find_subslice_missing_returns_none() {
        assert_eq!(find_subslice(b"hello world", b"OK>"), None);
        assert_eq!(find_subslice(b"", b"x"), None);
    }

    #[test]
    fn find_subslice_empty_needle_returns_none() {
        assert_eq!(find_subslice(b"hello", b""), None);
    }

    #[test]
    fn find_subslice_needle_longer_than_haystack() {
        assert_eq!(find_subslice(b"hi", b"hello"), None);
    }
}
