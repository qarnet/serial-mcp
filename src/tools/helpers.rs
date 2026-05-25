use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{
    model::{
        LoggingLevel, LoggingMessageNotificationParam, ProgressNotificationParam, ProgressToken,
    },
    service::Peer,
    Json, RoleServer,
};
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
    ct: &tokio_util::sync::CancellationToken,
    progress_token: Option<ProgressToken>,
    peer: Option<&Peer<RoleServer>>,
) -> Result<ReadOutcome, String> {
    const SETTLE_MS: u64 = 50;

    let effective_timeout = timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(effective_timeout);
    let mut buf = vec![0u8; max_bytes];

    let mut last_progress = Instant::now();

    // Read until at least one byte arrives or we time out.
    let first_n = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(ReadOutcome {
                bytes: Vec::new(),
                timed_out: true,
            });
        }

        let poll_ms = remaining.as_millis() as u64;
        let poll_ms = poll_ms.clamp(1, 250);

        match tokio::select! {
            _ = ct.cancelled() => return Err("Cancelled".into()),
            res = connection.read(&mut buf, Some(poll_ms)) => res,
        } {
            Ok(n) if n > 0 => break n,
            Ok(_) => {}
            Err(SerialError::ReadTimeout) => {}
            Err(e) => return Err(log_tool_err("read", "Data reading failed", e)),
        }

        if let (Some(token), Some(peer)) = (progress_token.clone(), peer) {
            if last_progress.elapsed() >= Duration::from_millis(250) {
                last_progress = Instant::now();
                let remaining = deadline.saturating_duration_since(Instant::now());
                let elapsed_ms = effective_timeout.saturating_sub(remaining.as_millis() as u64);
                let _ = peer
                    .notify_progress(ProgressNotificationParam {
                        progress_token: token,
                        progress: elapsed_ms as f64,
                        total: Some(effective_timeout as f64),
                        message: Some("waiting for data".into()),
                    })
                    .await;
            }
        }
    };

    if let (Some(token), Some(peer)) = (progress_token.clone(), peer) {
        let _ = peer
            .notify_progress(ProgressNotificationParam {
                progress_token: token,
                progress: 0.0,
                total: Some(effective_timeout as f64),
                message: Some("read started".to_string()),
            })
            .await;
    }

    let mut total = first_n;
    while total < max_bytes {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        let settle = remaining.min(SETTLE_MS);
        if settle == 0 {
            break;
        }
        let res = tokio::select! {
            _ = ct.cancelled() => return Err("Cancelled".into()),
            res = connection.read(&mut buf[total..], Some(settle)) => res,
        };
        match res {
            Ok(n) if n > 0 => {
                total += n;
                if let (Some(token), Some(peer)) = (progress_token.clone(), peer) {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    let elapsed_ms = effective_timeout.saturating_sub(remaining.as_millis() as u64);
                    if last_progress.elapsed() >= Duration::from_millis(250) {
                        last_progress = Instant::now();
                        let _ = peer
                            .notify_progress(ProgressNotificationParam {
                                progress_token: token,
                                progress: elapsed_ms as f64,
                                total: Some(effective_timeout as f64),
                                message: Some(format!("read {total} bytes")),
                            })
                            .await;
                    }
                }
            }
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
    ct: &tokio_util::sync::CancellationToken,
    progress_token: Option<ProgressToken>,
    peer: Option<&Peer<RoleServer>>,
) -> Result<WaitOutcome, String> {
    const CHUNK_CAPACITY: usize = 256;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut accumulated: Vec<u8> = Vec::with_capacity(CHUNK_CAPACITY.min(max_bytes));

    let mut last_progress = Instant::now();

    if let (Some(token), Some(peer)) = (progress_token.clone(), peer) {
        let _ = peer
            .notify_progress(ProgressNotificationParam {
                progress_token: token,
                progress: 0.0,
                total: Some(timeout_ms as f64),
                message: Some("wait_for started".to_string()),
            })
            .await;
    }

    loop {
        if ct.is_cancelled() {
            return Err("Cancelled".into());
        }
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

        if let (Some(token), Some(peer)) = (progress_token.clone(), peer) {
            if last_progress.elapsed() >= Duration::from_millis(250) {
                last_progress = Instant::now();
                let remaining = deadline.saturating_duration_since(Instant::now());
                let elapsed_ms = timeout_ms.saturating_sub(remaining.as_millis() as u64);
                let _ = peer
                    .notify_progress(ProgressNotificationParam {
                        progress_token: token,
                        progress: elapsed_ms as f64,
                        total: Some(timeout_ms as f64),
                        message: Some(format!("wait_for {} bytes", accumulated.len())),
                    })
                    .await;
            }
        }

        let remaining_ms = (deadline - now).as_millis() as u64;
        // Poll in short slices so we can emit progress updates even with no data.
        let remaining_ms = remaining_ms.clamp(1, 250);
        let room = (max_bytes - accumulated.len()).min(CHUNK_CAPACITY);
        let mut chunk = vec![0u8; room];
        match tokio::select! {
            _ = ct.cancelled() => return Err("Cancelled".into()),
            res = connection.read(&mut chunk, Some(remaining_ms)) => res,
        } {
            Ok(0) => continue,
            Ok(n) => {
                chunk.truncate(n);
                accumulated.extend_from_slice(&chunk);
            }
            Err(SerialError::ReadTimeout) => {
                // No data arrived during this poll slice; keep waiting until the
                // overall deadline is reached.
                continue;
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

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::io::AsyncWriteExt;

    use crate::serial::test_support::loopback_connection;

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
        let outcome = ReadOutcome {
            bytes: Vec::new(),
            timed_out: true,
        };
        let Json(result) = build_read_result(outcome, "abc".into(), Encoding::Utf8, Some(250))
            .expect("timeout result must build");
        assert!(result.timed_out);
        assert_eq!(result.bytes_read, 0);
        assert_eq!(result.timeout_ms, 250);
        assert!(result.data.is_empty());
    }

    #[test]
    fn build_read_result_timeout_uses_default() {
        let outcome = ReadOutcome {
            bytes: Vec::new(),
            timed_out: true,
        };
        let Json(result) = build_read_result(outcome, "abc".into(), Encoding::Hex, None)
            .expect("timeout result must build");
        assert_eq!(result.timeout_ms, DEFAULT_READ_TIMEOUT_MS);
    }

    #[test]
    fn build_read_result_data_branch_encodes_hex() {
        let outcome = ReadOutcome {
            bytes: b"Hi".to_vec(),
            timed_out: false,
        };
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

    #[tokio::test]
    async fn read_until_pattern_matches_when_pattern_arrives() {
        let (conn, mut peer) = loopback_connection("test");
        let writer = tokio::spawn(async move {
            peer.write_all(b"junk before OK> and tail").await.unwrap();
        });
        let ct = tokio_util::sync::CancellationToken::new();
        let outcome = read_until_pattern(&conn, b"OK>", 1_000, 1024, &ct, None, None)
            .await
            .unwrap();
        writer.await.unwrap();
        assert_eq!(outcome.match_index, Some(12));
        assert!(!outcome.timed_out);
        assert!(outcome.bytes.starts_with(b"junk before OK>"));
    }

    #[tokio::test]
    async fn read_until_pattern_times_out_with_no_match() {
        let (conn, mut peer) = loopback_connection("test");
        peer.write_all(b"only noise here").await.unwrap();
        let ct = tokio_util::sync::CancellationToken::new();
        let outcome = read_until_pattern(&conn, b"OK>", 60, 1024, &ct, None, None)
            .await
            .unwrap();
        assert!(outcome.timed_out);
        assert!(outcome.match_index.is_none());
        assert!(outcome.bytes.windows(3).all(|w| w != b"OK>"));
    }

    #[tokio::test]
    async fn read_until_pattern_stops_at_max_bytes() {
        let (conn, mut peer) = loopback_connection("test");
        let blob = vec![b'.'; 600];
        peer.write_all(&blob).await.unwrap();
        let ct = tokio_util::sync::CancellationToken::new();
        let outcome = read_until_pattern(&conn, b"OK>", 1_000, 256, &ct, None, None)
            .await
            .unwrap();
        assert!(!outcome.timed_out);
        assert!(outcome.match_index.is_none());
        assert_eq!(outcome.bytes.len(), 256);
    }
}
