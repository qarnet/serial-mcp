use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{
    model::{LoggingLevel, LoggingMessageNotificationParam, Meta},
    service::RequestContext,
    Json, Peer, RoleServer,
};
use tracing::{debug, error, info, warn};

use crate::buffer_budget::BufferBudget;
use crate::codec;
use crate::match_config::{shape_match_context, validate_match_request, ByteMatcher};
use crate::rx_metadata::RxStopMetadata;
use crate::rx_session::{RxEvent, RxSessionManager};
use crate::serial::ConnectionManager;
use crate::stop_controller::{RxStopController, RxStopDecision};
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

impl StreamHandle {
    /// Abort the streaming task and wait for it to fully terminate.
    ///
    /// Awaiting matters: it guarantees the task has dropped its RxSession
    /// consumer receiver before the caller prunes consumers. A bare `abort()`
    /// (as in `Drop`) only schedules cancellation, leaving the consumer briefly
    /// open so the pump keeps stealing RX data.
    async fn abort_and_join(mut self) {
        self.join.abort();
        let _ = (&mut self.join).await;
    }
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
    budget: &Arc<dyn BufferBudget>,
    streams: &Arc<tokio::sync::Mutex<HashMap<String, StreamHandle>>>,
    args: SubscribeArgs,
    _meta: Meta,
    _ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    _ctx: RequestContext<RoleServer>,
) -> Result<Json<SubscribeResult>, String> {
    debug!(
        "subscribe {} encoding={} max_buffered_bytes={} poll={} timeout={:?} no_new_rx_timeout={:?}",
        args.connection_id,
        args.encoding,
        args.max_buffered_bytes,
        args.poll_interval_ms,
        args.timeout_ms,
        args.no_new_rx_timeout_ms
    );

    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;

    let max_buffered_bytes = require_min_or_err(
        "subscribe.max_buffered_bytes",
        args.max_buffered_bytes,
        MIN_STREAM_CHUNK_BYTES,
    )?;
    let max_buffered_bytes = clamp_or_err(
        "subscribe.max_buffered_bytes",
        max_buffered_bytes,
        MAX_STREAM_CHUNK_BYTES,
    )?;
    let poll_ms = clamp_poll_interval_or_err(
        "subscribe.poll_interval_ms",
        args.poll_interval_ms,
        MIN_POLL_INTERVAL_MS,
    )?;

    if let Some(timeout_ms) = args.timeout_ms {
        clamp_timeout_or_err("subscribe.timeout_ms", timeout_ms, MAX_TIMEOUT_MS)?;
    }
    if let Some(silence_ms) = args.no_new_rx_timeout_ms {
        if silence_ms == 0 {
            return Err("subscribe.no_new_rx_timeout_ms must be > 0".into());
        }
        clamp_timeout_or_err("subscribe.no_new_rx_timeout_ms", silence_ms, MAX_TIMEOUT_MS)?;
    }

    // Resolve matcher if provided.
    let matcher: Option<ByteMatcher> = match &args.r#match {
        Some(m) => Some(validate_match_request(m)?),
        None => None,
    };

    // Drop any existing subscription on this connection FIRST.
    // This aborts the old task and releases its budget reservation
    // before we attempt to reserve for the new subscription. This
    // avoids spurious budget-exhaustion errors when replacing a
    // subscription on the same connection.
    let replaced_previous = {
        let mut streams = streams.lock().await;
        if let Some(old_handle) = streams.remove(&args.connection_id) {
            // Drop the old handle synchronously; its task aborts and
            // the reservation will release once the task finishes aborting.
            // We yield once to let the abort start, then proceed.
            drop(old_handle);
            true
        } else {
            false
        }
    };
    // Yield to allow the old task's reservation to start releasing.
    tokio::task::yield_now().await;

    let _reservation = budget.try_reserve(max_buffered_bytes).map_err(|e| {
        match e {
            crate::buffer_budget::BufferBudgetError::OverToolLimit { requested, tool_limit } => {
                format!("subscribe.max_buffered_bytes={requested} exceeds per-tool limit {tool_limit}")
            }
            crate::buffer_budget::BufferBudgetError::ZeroRequest => {
                "subscribe.max_buffered_bytes must be > 0".into()
            }
            crate::buffer_budget::BufferBudgetError::InsufficientProgramBudget {
                requested,
                available,
            } => {
                format!("insufficient program buffer budget: requested {requested}, available {available}")
            }
        }
    })?;

    let id = args.connection_id.clone();
    let name = connection.name().map(str::to_string);
    let timeout_ms = args.timeout_ms;
    let no_new_rx_timeout_ms = args.no_new_rx_timeout_ms;

    // Get or create the RX session for this connection, then register a
    // streaming consumer. The pump in the session is the *only* code that
    // reads from the serial port. This subscribe worker consumes from the
    // mpsc channel fed by the pump.
    let session = rx_sessions.get_or_create(connection).await;
    let event_rx = session.register_streaming();

    // Hold the reservation inside the spawned task so it lives for the
    // entire streaming lifetime and is released when the task finishes.
    let reservation = _reservation;

    let join = tokio::spawn(stream_rx_via_session(
        peer,
        session,
        event_rx,
        encoding,
        max_buffered_bytes,
        poll_ms,
        timeout_ms,
        no_new_rx_timeout_ms,
        reservation,
        matcher,
    ));

    let mut streams = streams.lock().await;
    // We already removed the old handle above; just insert the new one.
    // If another subscribe sneaked in between, its handle is replaced here.
    let inserted_replaced = streams.insert(id.clone(), StreamHandle { join }).is_some();
    let was_replaced = replaced_previous || inserted_replaced;
    info!(
        "subscribed RX stream for {} (replaced={}, timeout={:?})",
        id, was_replaced, timeout_ms
    );

    Ok(Json(SubscribeResult {
        connection_id: id,
        name,
        encoding: encoding.to_string(),
        max_buffered_bytes,
        poll_interval_ms: poll_ms,
        replaced_previous: was_replaced,
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
    let name = connections
        .get(&args.connection_id)
        .await
        .ok()
        .and_then(|connection| connection.name().map(str::to_string));

    let handle = {
        let mut streams = streams.lock().await;
        streams.remove(&args.connection_id)
    };
    let was_active = handle.is_some();

    // Wait for the streaming task to fully stop before pruning. Aborting alone
    // leaves its consumer receiver open, so prune_consumers below would not see
    // it as closed and the pump would keep stealing RX data that subsequent
    // read/wait_for tools need.
    if let Some(handle) = handle {
        handle.abort_and_join().await;
    }
    info!(
        "unsubscribed {} (was_active={})",
        args.connection_id, was_active
    );

    // Prune closed consumers from the RX session so the pump can exit if no
    // consumers remain. When prune cancels the pump, await its exit so the
    // serial port is quiescent before we return — otherwise a pump mid-read
    // can grab and discard bytes a following read/wait_for is waiting for.
    if let Some(session) = rx_sessions.get(&args.connection_id).await {
        if session.prune_consumers() {
            session.join_pump().await;
        }
    }

    Ok(Json(UnsubscribeResult {
        connection_id: args.connection_id,
        name,
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
///
/// When `matcher` is `Some`, the stream detects the first match and emits
/// a final stop notification with `matched=true` and `match_index`, then
/// terminates.
///
/// Uses [`RxStopController`] for all stop-condition evaluation so that
/// `subscribe` and `read` produce identical stop reasons for the same inputs.
#[allow(clippy::too_many_arguments)]
async fn stream_rx_via_session(
    peer: Peer<RoleServer>,
    session: Arc<crate::rx_session::RxSession>,
    mut event_rx: tokio::sync::mpsc::Receiver<RxEvent>,
    encoding: crate::codec::Encoding,
    _max_buffered_bytes: usize,
    poll_interval_ms: u64,
    timeout_ms: Option<u64>,
    no_new_rx_timeout_ms: Option<u64>,
    // Held for RAII: dropping releases the budget reservation.
    _reservation: Box<dyn crate::buffer_budget::BufferReservation>,
    mut matcher: Option<ByteMatcher>,
) {
    let conn_id = session.connection_id().to_string();
    let logger = format!("serial:{conn_id}");
    let start = Instant::now();
    // Subscribe does not use max_buffered_bytes as a stop condition (it
    // streams each chunk immediately). We pass 0 so the controller never
    // stops on MaxBufferedBytes; instead we rely on timeout, match_found,
    // connection_closed, channel_closed, and read_error.
    let mut ctrl = RxStopController::new(start, timeout_ms, 0, no_new_rx_timeout_ms);
    let deadline = ctrl.deadline();
    let mut stop_outcome: Option<crate::stop_controller::RxStopOutcome> = None;

    // Accumulated buffer for context shaping on match. Capped by
    // _max_buffered_bytes so memory stays bounded.
    let context_amount = matcher.as_ref().and_then(|m| m.context_amount());
    let needle_len = matcher.as_ref().map(|m| m.needle().len());
    let mut accumulated: Vec<u8> = Vec::new();

    loop {
        if let RxStopDecision::Stop(outcome) = ctrl.check_timeout() {
            stop_outcome = Some(outcome);
            break;
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            stop_outcome = Some(outcome);
            break;
        }

        let recv_deadline = match deadline {
            Some(dl) => tokio::time::Instant::from_std(dl),
            None => tokio::time::Instant::now() + Duration::from_millis(poll_interval_ms),
        };

        let event = tokio::select! {
            msg = tokio::time::timeout_at(recv_deadline, event_rx.recv()) => match msg {
                Ok(Some(e)) => e,
                Ok(None) => {
                    stop_outcome = Some(ctrl.channel_closed());
                    break;
                }
                Err(_) => continue,
            },
        };

        match event {
            RxEvent::Data(chunk) => {
                let n = chunk.len();
                ctrl.notify_data_received();

                // Accumulate for context shaping if a matcher with context is
                // active. Cap at _max_buffered_bytes to keep memory bounded.
                if context_amount.is_some() {
                    let room = _max_buffered_bytes.saturating_sub(accumulated.len());
                    let take = chunk.len().min(room);
                    accumulated.extend_from_slice(&chunk[..take]);
                }

                // Check for match if matcher is present.
                let match_result = matcher.as_mut().map(|m| m.push(&chunk));
                // Prune matcher window to keep memory bounded. Keep only the
                // tail needed for future match detection (at most the
                // reservation budget). For subscribe, this prevents the
                // matcher from growing beyond the reserved budget.
                if let Some(m) = matcher.as_mut() {
                    let keep = m.needle().len().max(1).saturating_add(1);
                    let cap = _max_buffered_bytes.max(keep);
                    if m.len() > cap {
                        m.truncate_front(cap);
                    }
                }
                if let RxStopDecision::Stop(outcome) = ctrl.push_data(n, n, match_result) {
                    stop_outcome = Some(outcome);
                }

                // Emit data notification regardless (including on match).
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
                            stop_outcome = Some(ctrl.peer_disconnected());
                        }
                        if stop_outcome.is_some() {
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
                    stop_outcome = Some(ctrl.peer_disconnected());
                    break;
                }

                if stop_outcome.is_some() {
                    break;
                }
            }
            RxEvent::Closed => {
                stop_outcome = Some(ctrl.connection_closed());
                break;
            }
            RxEvent::Error(msg) => {
                error!("RX stream read error on {conn_id}: {msg}");
                stop_outcome = Some(ctrl.read_error());
                break;
            }
        }
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let outcome = stop_outcome.unwrap_or_else(|| ctrl.channel_closed());
    let stop_meta = RxStopMetadata {
        stop_reason: outcome.meta.stop_reason,
        truncated: outcome.meta.truncated,
        bytes_observed: outcome.meta.bytes_observed,
        bytes_returned: outcome.meta.bytes_returned,
    };

    let mut stop_payload = serde_json::json!({
        "connection_id": conn_id,
        "stop_reason": stop_meta.stop_reason.to_string(),
        "truncated": stop_meta.truncated,
        "bytes_observed": stop_meta.bytes_observed,
        "bytes_returned": stop_meta.bytes_returned,
        "elapsed_ms": elapsed_ms,
        "timeout_ms": timeout_ms,
        "no_new_rx_timeout_ms": no_new_rx_timeout_ms,
    });
    if outcome.matched {
        stop_payload["matched"] = serde_json::json!(true);

        // Apply context shaping if configured.
        let (shaped_match_index, shaped_data) = if let (Some(midx), Some(ca), Some(nlen)) =
            (outcome.match_index, context_amount, needle_len)
        {
            let shaped = shape_match_context(&accumulated, midx, nlen, Some(ca));
            (Some(shaped.match_index), Some(shaped.data))
        } else {
            (outcome.match_index, None)
        };
        stop_payload["match_index"] = serde_json::json!(shaped_match_index);
        if let Some(ref data) = shaped_data {
            match codec::encode(encoding, data) {
                Ok(encoded) => {
                    stop_payload["data"] = serde_json::json!(encoded);
                    stop_payload["bytes_returned"] = serde_json::json!(data.len());
                }
                Err(e) => {
                    warn!("RX stream match context encoding error on {conn_id}: {e}");
                }
            }
        }
    }
    let stop_param = LoggingMessageNotificationParam {
        level: LoggingLevel::Info,
        logger: Some(logger.clone()),
        data: stop_payload,
    };
    if let Err(e) = peer.notify_logging_message(stop_param).await {
        debug!("Failed to send stop notification: {e}");
    }

    info!(
        "RX stream ended for {conn_id}: reason={} bytes={} elapsed={}ms",
        stop_meta.stop_reason, stop_meta.bytes_observed, elapsed_ms
    );
}
