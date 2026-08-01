use std::sync::Arc;

use rmcp::{model::Meta, Json, Peer, RoleServer};
use tracing::{debug, info, warn};

use crate::buffer_budget::BufferBudget;
use crate::codec::Encoding;
use crate::limits::DEFAULT_RX_BUFFER_SIZE;
use crate::rx_session::RxSessionManager;
use crate::serial::ConnectionManager;
use crate::serial::FlushTarget;
use crate::tools::helpers::{
    clamp_or_err, log_tool_err, lookup_connection, map_budget_err, parse_encoding, MAX_READ_BYTES,
    MAX_WRITE_BYTES, MIN_READ_BYTES,
};
use crate::tools::read_loop::read_bytes_from_ring;
use crate::tools::result_builders::{build_read_result, record_read_completion};
use crate::tools::rx_validate::{validate_rx_request, ResolvedRxArgs, RxLimits};
use crate::tools::types::{
    FlushArgs, FlushResult, ReadArgs, ReadFrom, ReadResult, TransactArgs, TransactResult,
    WriteArgs, WriteResult,
};

use crate::tx_session::TxSessionManager;
pub async fn write(
    connections: &Arc<ConnectionManager>,
    tx_sessions: &Arc<TxSessionManager>,
    args: WriteArgs,
) -> Result<Json<WriteResult>, String> {
    debug!("Write to {} ({})", args.connection_id, args.encoding);

    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;
    let decoded = decode_tx_payload(encoding, &args.data, "write.data.len()")?;
    let decoded_len = decoded.len();

    // Resolve tx_framing via the shared 4-layer precedence helper.
    let tx_framing = crate::precedence::resolve_field(
        args.tx_framing,
        args.protocol,
        crate::framing::preset_tx_framing,
        connection.tx_framing_default(),
        connection.protocol_default(),
    );

    let data =
        apply_tx_framing(decoded, tx_framing.as_ref(), "write.framed_len()").map_err(|err| {
            match err {
                TxFramingError::Encode(e) => log_tool_err(
                    "write",
                    &format!("TX framing failed on {}: {e}", args.connection_id),
                    e,
                ),
                TxFramingError::Size(e) => e,
            }
        })?;

    let session = tx_sessions.get_or_create(Arc::clone(&connection)).await;
    let bytes_written = session.write(data).await.map_err(|e| {
        log_tool_err(
            "write",
            &format!("Data sending failed on {}", args.connection_id),
            e,
        )
    })?;

    debug!("Wrote {} bytes to {}", bytes_written, args.connection_id);
    connection.record_write_op();
    Ok(Json(WriteResult {
        connection_id: args.connection_id,
        name: connection.name().map(str::to_string),
        bytes_written,
        decoded_bytes: decoded_len,
        encoding: encoding.to_string(),
    }))
}

pub async fn read(
    connections: &Arc<ConnectionManager>,
    rx_sessions: &Arc<RxSessionManager>,
    budget: &Arc<dyn BufferBudget>,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: ReadArgs,
) -> Result<Json<ReadResult>, String> {
    debug!(
        "Read from {} (timeout {:?}, no_new_rx_timeout {:?})",
        args.connection_id, args.timeout_ms, args.no_new_rx_timeout_ms
    );

    // Look up connection early to get max_buffered_bytes default.
    let max_buffered_bytes_default = lookup_connection(connections, &args.connection_id)
        .await
        .map(|c| c.max_buffered_bytes_default())
        .unwrap_or(32768);

    let ResolvedRxArgs {
        encoding,
        connection,
        max_buffered_bytes,
        matcher,
    } = validate_rx_request(
        connections,
        &args,
        RxLimits {
            tool: "read",
            min_buffered: MIN_READ_BYTES,
            max_buffered: MAX_READ_BYTES,
        },
        max_buffered_bytes_default,
    )
    .await?;

    // Reserve budget before reading.
    let _reservation = budget
        .try_reserve(max_buffered_bytes)
        .map_err(|e| map_budget_err("read.max_buffered_bytes", e))?;

    let progress_token = meta.get_progress_token();

    let session = rx_sessions
        .get_or_create(Arc::clone(&connection), DEFAULT_RX_BUFFER_SIZE)
        .await
        .map_err(|e| format!("read: {e}"))?;

    // Resolve rx_framing + rx_parser via the shared 4-layer precedence helper.
    let rx_framing = crate::precedence::resolve_field(
        args.rx_framing,
        args.protocol,
        crate::framing::preset_rx_framing,
        connection.rx_framing_default(),
        connection.protocol_default(),
    );
    let rx_parser = crate::precedence::resolve_field(
        args.rx_parser,
        args.protocol,
        crate::framing::preset_rx_parser,
        connection.rx_parser_default(),
        connection.protocol_default(),
    );

    // Resolve the initial read cursor from the `from` parameter.
    // Default: Cursor (shared read cursor). Writes the cursor BEFORE calling
    // read_bytes_from_ring so the agent can re-pass the same
    // from: {"type":"offset","offset":N} to re-read non-destructively
    // (cursor gets reset on each call).
    let ring = session.ring();
    let initial_cursor = match args.from.as_ref().unwrap_or(&ReadFrom::Cursor) {
        ReadFrom::Now => ring.end_offset(),
        ReadFrom::Cursor => session.read_cursor(),
        ReadFrom::BufferStart => ring.start_offset(),
        ReadFrom::Offset { offset } => *offset,
    };
    session.set_read_cursor(initial_cursor);

    let outcome = read_bytes_from_ring(
        session,
        max_buffered_bytes,
        args.timeout_ms,
        &ct,
        progress_token,
        Some(&peer),
        matcher,
        args.no_new_rx_timeout_ms,
        Some(Arc::clone(&connection)),
        rx_framing,
        rx_parser,
    )
    .await?;

    let result = build_read_result(
        outcome,
        args.connection_id,
        connection.name().map(str::to_string),
        encoding,
        args.timeout_ms,
        args.no_new_rx_timeout_ms,
    )?;
    record_read_completion(&connection, &result.0, args.r#match.as_ref());
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub async fn transact(
    connections: &Arc<ConnectionManager>,
    tx_sessions: &Arc<TxSessionManager>,
    rx_sessions: &Arc<RxSessionManager>,
    budget: &Arc<dyn BufferBudget>,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: TransactArgs,
) -> Result<Json<TransactResult>, String> {
    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;

    // --- Write half (shared decode/validate/framing, then session I/O) ---
    let decoded = decode_tx_payload(encoding, &args.data, "transact.data.len()")?;
    let decoded_len = decoded.len();

    let tx_framing = crate::precedence::resolve_field(
        args.tx_framing,
        args.protocol,
        crate::framing::preset_tx_framing,
        connection.tx_framing_default(),
        connection.protocol_default(),
    );

    let data =
        apply_tx_framing(decoded, tx_framing.as_ref(), "transact.framed_len()").map_err(|err| {
            match err {
                TxFramingError::Encode(e) => format!("TX framing failed: {e}"),
                TxFramingError::Size(e) => e,
            }
        })?;

    let tx_session = tx_sessions.get_or_create(Arc::clone(&connection)).await;
    let bytes_written = tx_session
        .write(data)
        .await
        .map_err(|e| format!("Write failed: {e}"))?;
    connection.record_write_op();
    let write_result = WriteResult {
        connection_id: args.connection_id.clone(),
        name: connection.name().map(str::to_string),
        bytes_written,
        decoded_bytes: decoded_len,
        encoding: encoding.to_string(),
    };

    // --- Read half (inlined from read handler, default from="now") ---
    let max_buffered_bytes = connection.max_buffered_bytes_default();
    let _reservation = budget
        .try_reserve(max_buffered_bytes)
        .map_err(|e| map_budget_err("transact.max_buffered_bytes", e))?;

    let progress_token = meta.get_progress_token();
    let session = rx_sessions
        .get_or_create(Arc::clone(&connection), DEFAULT_RX_BUFFER_SIZE)
        .await
        .map_err(|e| format!("transact: {e}"))?;

    let rx_framing = crate::precedence::resolve_field(
        args.rx_framing,
        args.protocol,
        crate::framing::preset_rx_framing,
        connection.rx_framing_default(),
        connection.protocol_default(),
    );
    let rx_parser = crate::precedence::resolve_field(
        args.rx_parser,
        args.protocol,
        crate::framing::preset_rx_parser,
        connection.rx_parser_default(),
        connection.protocol_default(),
    );

    let ring = session.ring();
    let initial_cursor = match args.from.as_ref().unwrap_or(&ReadFrom::Now) {
        ReadFrom::Now => ring.end_offset(),
        ReadFrom::Cursor => session.read_cursor(),
        ReadFrom::BufferStart => ring.start_offset(),
        ReadFrom::Offset { offset } => *offset,
    };
    session.set_read_cursor(initial_cursor);

    // Resolve the matcher from the match config (inline the matcher-building).
    let matcher = match args.r#match {
        Some(ref m) => Some(
            crate::match_config::validate_match_request(m)
                .map_err(|e| format!("transact.match: {e}"))?,
        ),
        None => None,
    };

    let outcome = read_bytes_from_ring(
        session,
        max_buffered_bytes,
        args.timeout_ms,
        &ct,
        progress_token,
        Some(&peer),
        matcher,
        args.no_new_rx_timeout_ms,
        Some(Arc::clone(&connection)),
        rx_framing.clone(),
        rx_parser.clone(),
    )
    .await?;

    let result = build_read_result(
        outcome,
        args.connection_id.clone(),
        connection.name().map(str::to_string),
        encoding,
        args.timeout_ms,
        args.no_new_rx_timeout_ms,
    )?;
    record_read_completion(&connection, &result.0, args.r#match.as_ref());

    Ok(Json(TransactResult {
        connection_id: args.connection_id,
        name: connection.name().map(str::to_string),
        write: write_result,
        read: result.0,
    }))
}

pub async fn flush(
    connections: &Arc<ConnectionManager>,
    rx_sessions: &Arc<RxSessionManager>,
    tx_sessions: &Arc<TxSessionManager>,
    args: FlushArgs,
) -> Result<Json<FlushResult>, String> {
    debug!("Flush {} target={:?}", args.connection_id, args.target);

    let connection = lookup_connection(connections, &args.connection_id).await?;
    match args.target {
        FlushTarget::Input => {
            connection
                .flush_buffers(FlushTarget::Input)
                .await
                .map_err(|e| {
                    log_tool_err(
                        "flush",
                        &format!("Failed to flush {}", args.connection_id),
                        e,
                    )
                })?;
            // Discard all unread buffered RX data. To skip past data without
            // destroying it, use `read` with `from: {"type": "now"}` and
            // discard the result.
            discard_rx_backlog(rx_sessions, &args.connection_id).await;
        }
        FlushTarget::Output => {
            let session = tx_sessions.get_or_create(Arc::clone(&connection)).await;
            session.flush_output().await.map_err(|e| {
                log_tool_err(
                    "flush",
                    &format!("Failed to flush {}", args.connection_id),
                    e,
                )
            })?;
        }
        FlushTarget::Both => {
            // Output-first ordering: flush queued/OS output, then clear OS
            // input, then discard the retained ring and clamp the shared
            // cursor — the same RX semantics as target=input.
            let session = tx_sessions.get_or_create(Arc::clone(&connection)).await;
            session.flush_output().await.map_err(|e| {
                log_tool_err(
                    "flush",
                    &format!("Failed to flush {}", args.connection_id),
                    e,
                )
            })?;
            connection
                .flush_buffers(FlushTarget::Input)
                .await
                .map_err(|e| {
                    log_tool_err(
                        "flush",
                        &format!("Failed to flush {}", args.connection_id),
                        e,
                    )
                })?;
            discard_rx_backlog(rx_sessions, &args.connection_id).await;
        }
    }
    info!("Flushed {} ({:?})", args.connection_id, args.target);

    Ok(Json(FlushResult {
        connection_id: args.connection_id,
        name: connection.name().map(str::to_string),
        target: args.target,
    }))
}

/// Shared RX-discard path for `flush(target="input")` and
/// `flush(target="both")`: clear the retained RX ring and clamp the shared
/// read cursor to the ring live edge. Both targets route through this one
/// helper so their RX semantics cannot drift.
async fn discard_rx_backlog(rx_sessions: &Arc<RxSessionManager>, connection_id: &str) {
    if let Some(session) = rx_sessions.get(connection_id).await {
        session.ring().clear();
        session.set_read_cursor(session.ring().end_offset());
        warn!(
            "flush: ring cleared for {}; all unread RX data discarded",
            connection_id
        );
    }
}

// ------------------------------------------------------------------
// Shared TX preparation (used by `write` and `transact`)
// ------------------------------------------------------------------

/// Decode a tool string and validate decoded size against `MAX_WRITE_BYTES`.
fn decode_tx_payload(
    encoding: Encoding,
    input: &str,
    decoded_limit_field: &str,
) -> Result<Vec<u8>, String> {
    let bytes =
        crate::codec::decode(encoding, input).map_err(|e| format!("Data decoding failed - {e}"))?;
    clamp_or_err(decoded_limit_field, bytes.len(), MAX_WRITE_BYTES)?;
    Ok(bytes)
}

/// Failure of the shared TX framing stage: callers map `Encode` errors
/// themselves; `Size` validation errors pass through unchanged.
#[derive(Debug)]
enum TxFramingError {
    /// `TxFramingMode::encode` failed; the caller owns the error mapping.
    Encode(String),
    /// The framed size exceeded `MAX_WRITE_BYTES`; carries the ready-made
    /// `{field}={value} exceeds maximum {max}` validation error.
    Size(String),
}

/// Apply TX framing when configured; otherwise return the decoded bytes
/// unchanged. Validates the final length against `MAX_WRITE_BYTES`.
fn apply_tx_framing(
    decoded: Vec<u8>,
    framing: Option<&crate::framing::TxFramingConfig>,
    framed_limit_field: &str,
) -> Result<Arc<[u8]>, TxFramingError> {
    match framing {
        Some(cfg) => {
            let framed = cfg.mode.encode(&decoded).map_err(TxFramingError::Encode)?;
            clamp_or_err(framed_limit_field, framed.len(), MAX_WRITE_BYTES)
                .map_err(TxFramingError::Size)?;
            Ok(Arc::from(framed))
        }
        None => Ok(Arc::from(decoded)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::framing::{Endianness, TxFramingConfig, TxFramingMode, TxLineEnding};

    #[test]
    fn decode_tx_payload_preserves_decoded_bytes_across_encodings() {
        let utf8 = decode_tx_payload(Encoding::Utf8, "hello", "t.data.len()").unwrap();
        assert_eq!(utf8, b"hello");
        assert_eq!(utf8.len(), 5);

        let hex = decode_tx_payload(Encoding::Hex, "48 65 6c 6c 6f", "t.data.len()").unwrap();
        assert_eq!(hex, b"Hello");
        assert_eq!(hex.len(), 5);

        let b64 = decode_tx_payload(Encoding::Base64, "aGVsbG8=", "t.data.len()").unwrap();
        assert_eq!(b64, b"hello");
        assert_eq!(b64.len(), 5);
    }

    #[test]
    fn decode_tx_payload_decode_failure_keeps_exact_error_text() {
        let err = decode_tx_payload(Encoding::Hex, "zz", "write.data.len()").unwrap_err();
        assert!(
            err.starts_with("Data decoding failed - "),
            "decode error text: {err}"
        );
    }

    #[test]
    fn decode_tx_payload_rejects_oversized_decoded_bytes() {
        let big = "a".repeat(MAX_WRITE_BYTES + 1);
        let err = decode_tx_payload(Encoding::Utf8, &big, "write.data.len()").unwrap_err();
        assert!(
            err.starts_with("write.data.len()="),
            "oversize field label: {err}"
        );
        assert!(err.contains("exceeds maximum"), "{err}");
    }

    #[test]
    fn apply_tx_framing_unframed_retains_bytes_byte_identical() {
        let decoded = decode_tx_payload(Encoding::Utf8, "hello", "t.data.len()").unwrap();
        let data = apply_tx_framing(decoded, None, "t.framed_len()").unwrap();
        assert_eq!(&*data, b"hello");
    }

    #[test]
    fn apply_tx_framing_produces_exact_framed_bytes() {
        let decoded = decode_tx_payload(Encoding::Utf8, "hello", "t.data.len()").unwrap();
        let framing = TxFramingConfig {
            mode: TxFramingMode::Line {
                ending: TxLineEnding::Crlf,
            },
        };
        let data = apply_tx_framing(decoded, Some(&framing), "t.framed_len()").unwrap();
        assert_eq!(&*data, b"hello\r\n");
    }

    #[test]
    fn apply_tx_framing_distinguishes_framing_from_size_errors() {
        // Invalid prefix_size is a framing (encode) error, not a size error.
        let decoded = decode_tx_payload(Encoding::Utf8, "abc", "t.data.len()").unwrap();
        let framing = TxFramingConfig {
            mode: TxFramingMode::LengthPrefixed {
                prefix_size: 3,
                endianness: Endianness::Big,
            },
        };
        match apply_tx_framing(decoded, Some(&framing), "t.framed_len()") {
            Err(TxFramingError::Encode(e)) => {
                assert!(e.contains("prefix_size"), "framing error text: {e}");
            }
            other => panic!("expected Encode error, got {other:?}"),
        }

        // Framing that pushes the length over MAX_WRITE_BYTES is a size error
        // carrying the ready-made validation text.
        let decoded =
            decode_tx_payload(Encoding::Utf8, &"a".repeat(MAX_WRITE_BYTES), "t.data.len()")
                .unwrap();
        let framing = TxFramingConfig {
            mode: TxFramingMode::Line {
                ending: TxLineEnding::Crlf,
            },
        };
        match apply_tx_framing(decoded, Some(&framing), "write.framed_len()") {
            Err(TxFramingError::Size(e)) => {
                assert!(e.starts_with("write.framed_len()="), "size error: {e}");
                assert!(e.contains("exceeds maximum"), "{e}");
            }
            other => panic!("expected Size error, got {other:?}"),
        }
    }
}
