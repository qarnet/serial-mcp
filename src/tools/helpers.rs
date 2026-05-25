use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{model::LoggingLevel, model::LoggingMessageNotificationParam, service::Peer, Json};
use tracing::error;

use crate::codec::{self, Encoding};
use crate::error::SerialError;
use crate::serial::{
    ConnectionConfig, ConnectionManager, DataBits, FlowControl, Parity, SerialConnection, StopBits,
};
use crate::tools::types::*;

pub(crate) const DEFAULT_READ_TIMEOUT_MS: u64 = 1000;

// ------------------------------------------------------------------
// Connection lookup
// ------------------------------------------------------------------

pub async fn lookup_connection(
    connections: &Arc<ConnectionManager>,
    id: &str,
) -> Result<Arc<SerialConnection>, String> {
    connections
        .get(id)
        .await
        .map_err(|_| format!("Connection ID {id} not found"))
}

// ------------------------------------------------------------------
// Read helpers
// ------------------------------------------------------------------

/// Outcome of a read call. `timed_out` distinguishes the genuine
/// read-timeout case from a successful read of `bytes`.
pub struct ReadOutcome {
    pub bytes: Vec<u8>,
    pub timed_out: bool,
}

pub async fn read_bytes(
    connection: &SerialConnection,
    max_bytes: usize,
    timeout_ms: Option<u64>,
) -> Result<ReadOutcome, String> {
    const SETTLE_MS: u64 = 50;

    let effective_timeout = timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(effective_timeout);
    let mut buf = vec![0u8; max_bytes];

    let first_n = match connection.read(&mut buf, Some(effective_timeout)).await {
        Ok(n) => n,
        Err(SerialError::ReadTimeout) => {
            return Ok(ReadOutcome {
                bytes: Vec::new(),
                timed_out: true,
            })
        }
        Err(e) => return Err(log_tool_err("read", "Data reading failed", e)),
    };

    let mut total = first_n;
    while total < max_bytes {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        let settle = remaining.min(SETTLE_MS);
        if settle == 0 {
            break;
        }
        match connection.read(&mut buf[total..], Some(settle)).await {
            Ok(n) if n > 0 => total += n,
            _ => break,
        }
    }

    buf.truncate(total);
    Ok(ReadOutcome {
        bytes: buf,
        timed_out: false,
    })
}

pub struct WaitOutcome {
    pub bytes: Vec<u8>,
    pub match_index: Option<usize>,
    pub timed_out: bool,
}

pub async fn read_until_pattern(
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
            return Ok(WaitOutcome {
                bytes: accumulated,
                match_index: None,
                timed_out: false,
            });
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(WaitOutcome {
                bytes: accumulated,
                match_index: None,
                timed_out: true,
            });
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
                return Ok(WaitOutcome {
                    bytes: accumulated,
                    match_index: None,
                    timed_out: true,
                });
            }
            Err(e) => return Err(log_tool_err("wait_for", "Read failed during wait", e)),
        }
    }
}

pub(crate) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ------------------------------------------------------------------
// Streaming helper
// ------------------------------------------------------------------

pub async fn stream_rx(
    peer: Peer<rmcp::RoleServer>,
    connection: Arc<SerialConnection>,
    encoding: Encoding,
    max_chunk_bytes: usize,
    poll_interval_ms: u64,
) {
    let logger = format!("serial:{}", connection.id());
    let mut buf = vec![0u8; max_chunk_bytes];
    loop {
        match connection.read(&mut buf, Some(poll_interval_ms)).await {
            Ok(0) | Err(SerialError::ReadTimeout) => continue,
            Ok(n) => {
                let chunk = &buf[..n];
                let encoded = match codec::encode(encoding, chunk) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let payload = serde_json::json!({
                    "connection_id": connection.id(),
                    "bytes_read": n,
                    "encoding": encoding.to_string(),
                    "data": encoded,
                });
                let param = LoggingMessageNotificationParam {
                    level: LoggingLevel::Info,
                    logger: Some(logger.clone()),
                    data: payload,
                };
                if let Err(e) = peer.notify_logging_message(param).await {
                    error!("RX stream peer disconnected: {}", e);
                    break;
                }
            }
            Err(e) => {
                error!("RX stream read error on {}: {}", connection.id(), e);
                break;
            }
        }
    }
}

// ------------------------------------------------------------------
// Result builders
// ------------------------------------------------------------------

pub fn build_read_result(
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
        .map_err(|e| format!("Data encoding failed - {e}"))?;
    Ok(Json(ReadResult {
        connection_id,
        bytes_read,
        encoding: encoding.to_string(),
        data,
        timed_out: false,
        timeout_ms,
    }))
}

// ------------------------------------------------------------------
// Parsers
// ------------------------------------------------------------------

pub fn parse_encoding(raw: &str) -> Result<Encoding, String> {
    raw.parse::<Encoding>()
        .map_err(|e| format!("Unsupported encoding - {e}"))
}

pub fn parse_open_args(args: OpenArgs) -> Result<ConnectionConfig, String> {
    Ok(ConnectionConfig {
        port: args.port,
        baud_rate: args.baud_rate,
        data_bits: parse_data_bits(&args.data_bits)?,
        stop_bits: parse_stop_bits(&args.stop_bits)?,
        parity: parse_parity(&args.parity)?,
        flow_control: parse_flow_control(&args.flow_control)?,
    })
}

pub fn parse_data_bits(raw: &str) -> Result<DataBits, String> {
    match raw {
        "5" => Ok(DataBits::Five),
        "6" => Ok(DataBits::Six),
        "7" => Ok(DataBits::Seven),
        "8" => Ok(DataBits::Eight),
        other => Err(format!("Invalid data_bits {other:?} (expected 5/6/7/8)")),
    }
}

pub fn parse_stop_bits(raw: &str) -> Result<StopBits, String> {
    match raw {
        "1" => Ok(StopBits::One),
        "2" => Ok(StopBits::Two),
        other => Err(format!("Invalid stop_bits {other:?} (expected 1/2)")),
    }
}

pub fn parse_parity(raw: &str) -> Result<Parity, String> {
    match raw.to_lowercase().as_str() {
        "none" => Ok(Parity::None),
        "odd" => Ok(Parity::Odd),
        "even" => Ok(Parity::Even),
        other => Err(format!("Invalid parity {other:?} (expected none/odd/even)")),
    }
}

pub fn parse_flow_control(raw: &str) -> Result<FlowControl, String> {
    match raw.to_lowercase().as_str() {
        "none" => Ok(FlowControl::None),
        "software" => Ok(FlowControl::Software),
        "hardware" => Ok(FlowControl::Hardware),
        other => Err(format!(
            "Invalid flow_control {other:?} (expected none/software/hardware)"
        )),
    }
}

// ------------------------------------------------------------------
// Error helper
// ------------------------------------------------------------------

pub fn log_tool_err<E: std::fmt::Display>(op: &str, context: &str, err: E) -> String {
    error!("{op} failed: {err}");
    format!("{context} - {err}")
}
