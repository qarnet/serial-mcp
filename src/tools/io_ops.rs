use std::sync::Arc;

use rmcp::{model::Meta, Json, Peer, RoleServer};
use tracing::{debug, info};

use crate::buffer_budget::BufferBudget;
use crate::codec;
use crate::rx_session::RxSessionManager;
use crate::serial::ConnectionManager;
use crate::serial::FlushTarget;
use crate::tools::helpers::{
    build_read_result, clamp_or_err, log_tool_err, lookup_connection, map_budget_err,
    parse_encoding, read_bytes_via_session, validate_rx_request, ResolvedRxArgs, RxLimits,
    MAX_READ_BYTES, MAX_WRITE_BYTES, MIN_READ_BYTES,
};
use crate::tools::types::{FlushArgs, FlushResult, ReadArgs, ReadResult, WriteArgs, WriteResult};

use crate::tx_session::TxSessionManager;
pub async fn write(
    connections: &Arc<ConnectionManager>,
    tx_sessions: &Arc<TxSessionManager>,
    args: WriteArgs,
) -> Result<Json<WriteResult>, String> {
    debug!("Write to {} ({})", args.connection_id, args.encoding);

    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;
    let bytes =
        codec::decode(encoding, &args.data).map_err(|e| format!("Data decoding failed - {e}"))?;
    clamp_or_err("write.data.len()", bytes.len(), MAX_WRITE_BYTES)?;

    let data: Arc<[u8]> = Arc::from(bytes.as_slice());
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
    )
    .await?;

    // Reserve budget before registering consumer.
    let _reservation = budget
        .try_reserve(max_buffered_bytes)
        .map_err(|e| map_budget_err("read.max_buffered_bytes", e))?;

    let progress_token = meta.get_progress_token();

    let session = rx_sessions.get_or_create(Arc::clone(&connection)).await;
    let event_rx = session.register_blocking();

    let outcome = read_bytes_via_session(
        event_rx,
        max_buffered_bytes,
        args.timeout_ms,
        &ct,
        progress_token,
        Some(&peer),
        matcher,
        args.no_new_rx_timeout_ms,
        Some(Arc::clone(&connection)),
        args.framing,
    )
    .await?;

    session.prune_consumers();

    let result = build_read_result(
        outcome,
        args.connection_id,
        connection.name().map(str::to_string),
        encoding,
        args.timeout_ms,
        args.no_new_rx_timeout_ms,
    )?;
    connection.record_read_op();
    let log = connection.log();
    log.rx_data(result.0.bytes_read);
    if result.0.truncated {
        connection.record_truncation();
        log.truncated(result.0.bytes_observed, result.0.bytes_returned);
    }
    if result.0.matched {
        // Extract pattern info from the result
        if let Some(ref m) = args.r#match {
            log.match_found(&m.pattern, &m.config.mode.to_string());
        }
    }
    Ok(result)
}

pub async fn flush(
    connections: &Arc<ConnectionManager>,
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
        }
    }
    info!("Flushed {} ({:?})", args.connection_id, args.target);

    Ok(Json(FlushResult {
        connection_id: args.connection_id,
        name: connection.name().map(str::to_string),
        target: args.target,
    }))
}
