use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{
    model::{LoggingLevel, LoggingMessageNotificationParam, Meta},
    service::RequestContext,
    Json, Peer, RoleServer,
};
use tracing::{debug, error, info, warn};

use crate::codec;
use crate::rx_session::{RxEvent, RxSessionManager};
use crate::serial::ConnectionManager;
use crate::tools::helpers::{
    clamp_or_err, clamp_poll_interval_or_err, clamp_timeout_or_err, lookup_connection,
    parse_encoding, require_min_or_err, MAX_STREAM_CHUNK_BYTES, MAX_TIMEOUT_MS,
    MIN_POLL_INTERVAL_MS, MIN_STREAM_CHUNK_BYTES,
};
use crate::tools::types::{SubscribeArgs, SubscribeResult, UnsubscribeArgs, UnsubscribeResult};

/// RAII wrapper around a streaming task. Aborts the task on drop.
pub struct StreamHandle {
    join: tokio::task::JoinHandle<()>,
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.join.abort();
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn subscribe(
    connections: &Arc<ConnectionManager>,
    rx_sessions: &Arc<RxSessionManager>,
    streams: &Arc<tokio::sync::Mutex<HashMap<String, StreamHandle>>>,
    args: SubscribeArgs,
    _meta: Meta,
    _ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    _ctx: RequestContext<RoleServer>,
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

    if let Some(timeout_ms) = args.timeout_ms {
        clamp_timeout_or_err("subscribe.timeout_ms", timeout_ms, MAX_TIMEOUT_MS)?;
    }

    let id = args.connection_id.clone();
    let name = connection.name().map(str::to_string);
    let timeout_ms = args.timeout_ms;

    // Get or create the RX session for this connection, then register a
    // streaming consumer. The pump in the session is the *only* code that
    // reads from the serial port. This subscribe worker consumes from the
    // mpsc channel fed by the pump.
    let session = rx_sessions.get_or_create(connection).await;
    let event_rx = session.register_streaming();

    let join = tokio::spawn(stream_rx_via_session(
        peer,
        session,
        event_rx,
        encoding,
        chunk_bytes,
        poll_ms,
        timeout_ms,
    ));

    let mut streams = streams.lock().await;
    streams.insert(id.clone(), StreamHandle { join });
    info!(
        "subscribed RX stream for {} (replaced={}, timeout={:?})",
        id, replaced_previous, timeout_ms
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
    connections: &Arc<ConnectionManager>,
    rx_sessions: &Arc<RxSessionManager>,
    streams: &Arc<tokio::sync::Mutex<HashMap<String, StreamHandle>>>,
    args: UnsubscribeArgs,
    ) -> Result<Json<UnsubscribeResult>, String> {
    debug!("unsubscribe {}", args.connection_id);

    let _connection_exists = connections.get(&args.connection_id).await.ok();

    let mut streams = streams.lock().await;
    let was_active = streams.remove(&args.connection_id).is_some();
    drop(streams);
    info!(
        "unsubscribed {} (was_active={})",
        args.connection_id, was_active
    );

    // Prune closed consumers from the RX session so the pump can
    // exit if no consumers remain. This prevents the pump from
    // stealing RX data that read/wait_for tools need.
    if let Some(session) = rx_sessions.get(&args.connection_id).await {
        session.prune_consumers();
    }

    Ok(Json(UnsubscribeResult {
        connection_id: args.connection_id,
        was_active,
    }))
}

/// Stream RX data sourced from an [`RxSession`] consumer channel.
///
/// Replaces the old `stream_rx` helper that read directly from
/// `SerialConnection`. Both timed and untimed subscribe share this one
/// background path; the only difference is whether a deadline is set.
///
/// Stop reasons are communicated via a final logging notification at `Info`
/// level with a `stop_reason` field so clients can distinguish normal
/// timeout from error or cancellation.
async fn stream_rx_via_session(
    peer: Peer<RoleServer>,
    session: Arc<crate::rx_session::RxSession>,
    mut event_rx: tokio::sync::mpsc::Receiver<RxEvent>,
    encoding: crate::codec::Encoding,
    _max_chunk_bytes: usize,
    poll_interval_ms: u64,
    timeout_ms: Option<u64>,
) {
    let conn_id = session.connection_id().to_string();
    let logger = format!("serial:{conn_id}");
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    let mut total_bytes: usize = 0;
    #[allow(unused_assignments)]
    let mut stop_reason: String = "unknown".into();
    let start = Instant::now();

    loop {
        // Check timeout deadline.
        if let Some(dl) = deadline {
            if Instant::now() >= dl {
                stop_reason = "timeout".into();
                break;
            }
        }

        let recv_deadline = match deadline {
            Some(dl) => tokio::time::Instant::from_std(dl),
            None => tokio::time::Instant::now() + Duration::from_millis(poll_interval_ms),
        };

        let event = tokio::select! {
            msg = tokio::time::timeout_at(recv_deadline, event_rx.recv()) => match msg {
                Ok(Some(e)) => e,
                Ok(None) => {
                    stop_reason = "channel_closed".into();
                    break;
                }
                Err(_) => continue,
            },
        };

        match event {
            RxEvent::Data(chunk) => {
                let n = chunk.len();
                total_bytes += n;

                let encoded = match codec::encode(encoding, &chunk) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(
                            "RX encoding error on {conn_id}: {encoding} cannot encode {n} bytes — dropped"
                        );
                        let payload = serde_json::json!({
                            "connection_id": conn_id,
                            "encoding_error": true,
                            "encoding": encoding.to_string(),
                            "bytes_dropped": n,
                            "reason": e.to_string(),
                        });
                        let param = LoggingMessageNotificationParam {
                            level: LoggingLevel::Warning,
                            logger: Some(logger.clone()),
                            data: payload,
                        };
                        if let Err(e) = peer.notify_logging_message(param).await {
                            error!("RX stream peer disconnected: {e}");
                            stop_reason = "peer_disconnected".into();
                            break;
                        }
                        continue;
                    }
                };

                let payload = serde_json::json!({
                    "connection_id": conn_id,
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
                    error!("RX stream peer disconnected: {e}");
                    stop_reason = "peer_disconnected".into();
                    break;
                }
            }
            RxEvent::Closed => {
                stop_reason = "connection_closed".into();
                break;
            }
            RxEvent::Error(msg) => {
                error!("RX stream read error on {conn_id}: {msg}");
                stop_reason = format!("read_error: {msg}");
                break;
            }
        }
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;

    let stop_payload = serde_json::json!({
        "connection_id": conn_id,
        "stop_reason": stop_reason,
        "bytes_read": total_bytes,
        "elapsed_ms": elapsed_ms,
        "timeout_ms": timeout_ms,
    });
    let stop_param = LoggingMessageNotificationParam {
        level: LoggingLevel::Info,
        logger: Some(logger.clone()),
        data: stop_payload,
    };
    if let Err(e) = peer.notify_logging_message(stop_param).await {
        debug!("Failed to send stop notification: {e}");
    }

    info!(
        "RX stream ended for {conn_id}: reason={stop_reason} bytes={total_bytes} elapsed={elapsed_ms}ms"
    );
}
