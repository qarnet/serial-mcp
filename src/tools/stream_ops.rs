use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use rmcp::{model::Meta, service::RequestContext, Json, Peer, RoleServer};
use tracing::{debug, info};

use crate::codec;
use crate::serial::{ConnectionManager, SerialConnection, SerialRxLease};
use crate::tools::helpers::{
    clamp_or_err, clamp_poll_interval_or_err, clamp_timeout_or_err, log_tool_err,
    lookup_connection, parse_encoding, require_min_or_err, stream_rx, MAX_STREAM_CHUNK_BYTES,
    MAX_TIMEOUT_MS, MIN_POLL_INTERVAL_MS, MIN_STREAM_CHUNK_BYTES,
};
use crate::tools::types::{SubscribeArgs, SubscribeResult, UnsubscribeArgs, UnsubscribeResult};

/// RAII wrapper around a streaming task. Aborts the task on drop.
pub struct StreamHandle {
    join: tokio::task::JoinHandle<()>,
    _rx_lease: SerialRxLease,
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.join.abort();
    }
}

pub async fn subscribe(
    connections: &Arc<ConnectionManager>,
    streams: &Arc<tokio::sync::Mutex<HashMap<String, StreamHandle>>>,
    args: SubscribeArgs,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    ctx: RequestContext<RoleServer>,
) -> Result<Json<SubscribeResult>, String> {
    debug!(
        "subscribe {} encoding={} chunk={} poll={} timeout={:?}",
        args.connection_id,
        args.encoding,
        args.max_chunk_bytes.0,
        args.poll_interval_ms.0,
        args.timeout_ms
    );

    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;

    let chunk_bytes = require_min_or_err(
        "subscribe.max_chunk_bytes",
        args.max_chunk_bytes.0,
        MIN_STREAM_CHUNK_BYTES,
    )?;
    let chunk_bytes = clamp_or_err(
        "subscribe.max_chunk_bytes",
        chunk_bytes,
        MAX_STREAM_CHUNK_BYTES,
    )?;
    let poll_ms = clamp_poll_interval_or_err(
        "subscribe.poll_interval_ms",
        args.poll_interval_ms.0,
        MIN_POLL_INTERVAL_MS,
    )?;

    let id = args.connection_id.clone();
    let replaced_previous = streams.lock().await.remove(&id).is_some();

    let timeout_ms_opt: Option<u64> = args.timeout_ms.into();

    // If timeout_ms is set, run in blocking mode: collect data for the
    // duration and return it inline. Otherwise, fire-and-forget background
    // streaming via logging notifications (original behaviour).
    if let Some(timeout_ms) = timeout_ms_opt {
        clamp_timeout_or_err("subscribe.timeout_ms", timeout_ms, MAX_TIMEOUT_MS)?;
        let _rx_lease = SerialConnection::acquire_rx(&connection, "subscribe")?;
        let progress_token = meta.get_progress_token();
        let start = Instant::now();
        let mut buf = vec![0u8; chunk_bytes];
        let mut total: usize = 0;
        loop {
            if ct.is_cancelled() {
                return Err("Cancelled".into());
            }
            let elapsed = start.elapsed().as_millis() as u64;
            if elapsed >= timeout_ms {
                break;
            }
            let remaining = (timeout_ms - elapsed).min(poll_ms);
            match tokio::select! {
                _ = ct.cancelled() => return Err("Cancelled".into()),
                res = connection.read(&mut buf[total..], Some(remaining)) => res,
            } {
                Ok(n) if n > 0 => {
                    total += n;
                    if total >= chunk_bytes {
                        break;
                    }
                    if let Some(token) = progress_token.clone() {
                        let _ = peer
                            .notify_progress(rmcp::model::ProgressNotificationParam {
                                progress_token: token,
                                progress: elapsed as f64,
                                total: Some(timeout_ms as f64),
                                message: Some(format!("read {total} bytes")),
                            })
                            .await;
                    }
                }
                Ok(_) => {} // 0 bytes — continue polling until timeout
                Err(crate::error::SerialError::ReadTimeout) => {} // no data this iteration
                Err(e) => return Err(log_tool_err("subscribe", "Read failed during subscribe", e)),
            }
        }

        buf.truncate(total);
        let elapsed_ms = start.elapsed().as_millis() as u64;
        let data =
            codec::encode(encoding, &buf).map_err(|e| format!("Data encoding failed - {e}"))?;

        info!(
            "subscribe {} collected {} bytes in {}ms",
            id, total, elapsed_ms
        );

        return Ok(Json(SubscribeResult {
            connection_id: id,
            encoding: encoding.to_string(),
            max_chunk_bytes: chunk_bytes,
            poll_interval_ms: poll_ms,
            replaced_previous,
            data: Some(data),
            bytes_read: Some(total),
            elapsed_ms: Some(elapsed_ms),
            timeout_ms: Some(timeout_ms),
        }));
    }

    // Fire-and-forget mode (original behaviour)
    let rx_lease = SerialConnection::acquire_rx(&connection, "subscribe")?;
    let join = tokio::spawn(stream_rx(
        ctx.peer.clone(),
        connection,
        encoding,
        chunk_bytes,
        poll_ms,
    ));

    let mut streams = streams.lock().await;
    streams.insert(
        id.clone(),
        StreamHandle {
            join,
            _rx_lease: rx_lease,
        },
    );
    info!(
        "subscribed RX stream for {} (replaced={})",
        id, replaced_previous
    );

    Ok(Json(SubscribeResult {
        connection_id: id,
        encoding: encoding.to_string(),
        max_chunk_bytes: chunk_bytes,
        poll_interval_ms: poll_ms,
        replaced_previous,
        data: None,
        bytes_read: None,
        elapsed_ms: None,
        timeout_ms: None,
    }))
}

pub async fn unsubscribe(
    streams: &Arc<tokio::sync::Mutex<HashMap<String, StreamHandle>>>,
    args: UnsubscribeArgs,
) -> Result<Json<UnsubscribeResult>, String> {
    debug!("unsubscribe {}", args.connection_id);

    let mut streams = streams.lock().await;
    let was_active = streams.remove(&args.connection_id).is_some();
    info!(
        "unsubscribed {} (was_active={})",
        args.connection_id, was_active
    );

    Ok(Json(UnsubscribeResult {
        connection_id: args.connection_id,
        was_active,
    }))
}
