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
use crate::limits::DEFAULT_RX_BUFFER_SIZE;
use crate::match_config::{shape_match_context, Matcher};
use crate::rx_metadata::{RxStopMetadata, RxStopReason};
use crate::rx_session::RxSessionManager;
use crate::serial::ConnectionManager;
use crate::stop_controller::{RxStopController, RxStopDecision};
use crate::tools::helpers::{
    clamp_poll_interval_or_err, lookup_connection, map_budget_err, validate_rx_request,
    ResolvedRxArgs, RxLimits, MAX_STREAM_CHUNK_BYTES, MIN_POLL_INTERVAL_MS, MIN_STREAM_CHUNK_BYTES,
};
use crate::tools::rx_consume::{
    consume_frames, disconnect_state, frame_outcome_to_stop, DisconnectState, RxFrameSink, SinkFlow,
};
use crate::tools::types::{
    ReadFrom, SubscribeArgs, SubscribeChunkNotification, SubscribeEncodingErrorNotification,
    SubscribeFrameNotification, SubscribePartialFrameNotification, SubscribeResult,
    SubscribeStopNotification, UnsubscribeArgs, UnsubscribeResult,
};

/// RAII wrapper around a streaming task. Aborts the task on drop.
pub struct StreamHandle {
    join: Option<tokio::task::JoinHandle<()>>,
}

impl StreamHandle {
    /// Abort the streaming task and wait for it to fully terminate.
    ///
    /// Awaiting matters: it guarantees the task has dropped its RxSession
    /// consumer receiver before the caller prunes consumers. A bare `abort()`
    /// (as in `Drop`) only schedules cancellation, leaving the consumer briefly
    /// open so the pump keeps stealing RX data.
    async fn abort_and_join(mut self) {
        if let Some(j) = self.join.take() {
            j.abort();
            let _ = j.await;
        }
    }

    /// Wait for the streaming task to finish naturally (without aborting).
    /// Used by the close handler to let flush_partial run before cleanup.
    /// After this call, `drop` will not abort.
    pub fn take_join(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        self.join.take()
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        if let Some(j) = self.join.take() {
            j.abort();
        }
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
        "subscribe {} encoding={} timeout={:?} no_new_rx_timeout={:?} from={:?}",
        args.connection_id, args.encoding, args.timeout_ms, args.no_new_rx_timeout_ms, args.from,
    );

    // Look up connection early to get defaults for max_buffered_bytes and poll_interval_ms.
    let connection_pre = lookup_connection(connections, &args.connection_id).await?;
    let max_buffered_bytes_default = connection_pre.max_buffered_bytes_default();
    let poll_interval_ms_default = connection_pre.poll_interval_ms_default();
    drop(connection_pre); // validate_rx_request will re-lookup

    let ResolvedRxArgs {
        encoding,
        connection,
        max_buffered_bytes,
        matcher,
    } = validate_rx_request(
        connections,
        &args,
        RxLimits {
            tool: "subscribe",
            min_buffered: MIN_STREAM_CHUNK_BYTES,
            max_buffered: MAX_STREAM_CHUNK_BYTES,
        },
        max_buffered_bytes_default,
    )
    .await?;
    // poll_interval_ms is subscribe-specific; validated after the shared preamble.
    let poll_ms = clamp_poll_interval_or_err(
        "subscribe.poll_interval_ms",
        poll_interval_ms_default,
        MIN_POLL_INTERVAL_MS,
    )?;

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

    // Validate framing decoder construction BEFORE spawning the task.
    // This ensures construction errors hard-fail the tool call rather than
    // silently degrading to raw mode (the old fanout-era behavior).
    let decoder: Option<crate::framing::FrameDecoder> = match rx_framing.as_ref() {
        Some(cfg) => Some(
            crate::framing::FrameDecoder::new(cfg, rx_parser.as_ref())
                .map_err(|e| format!("subscribe.rx_framing: {e}"))?,
        ),
        None => None,
    };

    // Drop any existing subscription on this connection FIRST.
    // This aborts the old task and releases its budget reservation
    // before we attempt to reserve for the new subscription.
    let replaced_previous = {
        let mut streams = streams.lock().await;
        if let Some(old_handle) = streams.remove(&args.connection_id) {
            drop(old_handle);
            true
        } else {
            false
        }
    };
    // Yield to allow the old task's reservation to start releasing.
    tokio::task::yield_now().await;

    let _reservation = budget
        .try_reserve(max_buffered_bytes)
        .map_err(|e| map_budget_err("subscribe.max_buffered_bytes", e))?;

    let id = args.connection_id.clone();
    let name = connection.name().map(str::to_string);
    let timeout_ms = args.timeout_ms;
    let no_new_rx_timeout_ms = args.no_new_rx_timeout_ms;

    // Get or create the RX session for this connection.
    let conn = Arc::clone(&connection);
    connection.record_read_op();
    let session = rx_sessions
        .get_or_create(connection, DEFAULT_RX_BUFFER_SIZE)
        .await
        .map_err(|e| format!("subscribe: {e}"))?;

    // Resolve the initial private cursor from the `from` parameter.
    let ring = session.ring();
    let from = args.from.unwrap_or(ReadFrom::Now);
    let initial_cursor = match from {
        ReadFrom::Now => ring.end_offset(),
        ReadFrom::Cursor => session.read_cursor(),
        ReadFrom::BufferStart => ring.start_offset(),
        ReadFrom::Offset { offset } => offset,
    };

    // Hold the reservation inside the spawned task so it lives for the
    // entire streaming lifetime and is released when the task finishes.
    let reservation = _reservation;

    let join = tokio::spawn(stream_rx_from_ring(
        peer,
        conn,
        session,
        encoding,
        max_buffered_bytes,
        poll_ms,
        timeout_ms,
        no_new_rx_timeout_ms,
        reservation,
        matcher,
        decoder,
        rx_framing,
        rx_parser,
        initial_cursor,
    ));

    let mut streams = streams.lock().await;
    let inserted_replaced = streams
        .insert(id.clone(), StreamHandle { join: Some(join) })
        .is_some();
    let was_replaced = replaced_previous || inserted_replaced;
    info!(
        "subscribed RX stream for {} (replaced={}, timeout={:?}, initial_cursor={})",
        id, was_replaced, timeout_ms, initial_cursor
    );

    Ok(Json(SubscribeResult {
        connection_id: id,
        name,
        encoding: encoding.to_string(),
        max_buffered_bytes,
        poll_interval_ms: poll_ms,
        replaced_previous: was_replaced,
    }))
}

pub async fn unsubscribe(
    connections: &Arc<ConnectionManager>,
    _rx_sessions: &Arc<RxSessionManager>,
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

    // Wait for the streaming task to fully stop. The subscription task
    // reads from the ring independently — no need to prune consumers.
    if let Some(handle) = handle {
        handle.abort_and_join().await;
    }
    info!(
        "unsubscribed {} (was_active={})",
        args.connection_id, was_active
    );

    Ok(Json(UnsubscribeResult {
        connection_id: args.connection_id,
        name,
        was_active,
    }))
}

/// `subscribe`'s frame sink: emits one notification per decoded frame, tracking
/// cumulative returned bytes. Stops at the matching frame (preserving the legacy
/// quirk that a failed emit of the *matching* frame still reports the match).
struct SubscribeFrameSink<'a> {
    peer: Peer<RoleServer>,
    conn: &'a Arc<crate::serial::SerialConnection>,
    logger: &'a str,
    conn_id: &'a str,
    encoding: crate::codec::Encoding,
    total_returned: &'a mut usize,
    match_offset: &'a mut Option<usize>,
    match_frame_index: &'a mut Option<usize>,
}

#[async_trait::async_trait]
impl RxFrameSink for SubscribeFrameSink<'_> {
    async fn on_frame(
        &mut self,
        frame: crate::framing::Frame,
        matched: bool,
        match_index: Option<usize>,
    ) -> SinkFlow {
        let encoded = match codec::encode_or_hex(self.encoding, &frame.data) {
            Ok(payload) => {
                if let Some(reason) = &payload.fallback_reason {
                    // Lossless fallback: warn, but never count as a drop —
                    // the bytes are still represented exactly.
                    warn!(
                        "RX frame on {} not encodable as {} ({reason}); \
                         using hex",
                        self.conn_id, self.encoding
                    );
                }
                payload
            }
            Err(e) => {
                warn!("RX frame encoding error on {}: {e}", self.conn_id);
                self.conn.record_notification_drop();
                self.conn
                    .log()
                    .notification_dropped(&format!("frame encoding error: {e}"));
                return SinkFlow::Continue;
            }
        };

        let notification = SubscribeFrameNotification {
            connection_id: self.conn_id.to_string(),
            frame_index: frame.index,
            frame_type: frame.frame_type.to_string(),
            encoding: encoded.encoding.to_string(),
            data: encoded.data,
            parsed: frame.parsed,
            matched: if matched { Some(true) } else { None },
        };
        let payload = serde_json::to_value(&notification).unwrap_or_else(|e| {
            warn!(
                "RX frame notification serialization error on {}: {e}",
                self.conn_id
            );
            serde_json::json!({})
        });

        let param = LoggingMessageNotificationParam {
            level: LoggingLevel::Info,
            logger: Some(self.logger.to_string()),
            data: payload,
        };
        let emit = self.peer.notify_logging_message(param).await;

        if matched {
            // Quirk: a failed emit of the matching frame still reports the match
            // (logs + record_notification_drop only), distinct from the non-matching
            // path below which returns PeerDisconnected. Intentional — see the
            // read/subscribe framing invariants in AGENTS.md.
            // KNOWN GAP: not characterization-tested (requires a peer disconnect
            // mid-emit on the matching frame); preserved by faithful translation.
            if let Err(e) = emit {
                error!("RX frame stream peer disconnected: {e}");
                self.conn.record_notification_drop();
            }
            *self.total_returned += frame.data.len();
            *self.match_offset = match_index;
            *self.match_frame_index = Some(frame.index);
            return SinkFlow::Stop(RxStopReason::MatchFound);
        }

        if let Err(e) = emit {
            error!("RX frame stream peer disconnected: {e}");
            self.conn.record_notification_drop();
            self.conn
                .log()
                .notification_dropped(&format!("frame peer disconnected: {e}"));
            return SinkFlow::Stop(RxStopReason::PeerDisconnected);
        }
        *self.total_returned += frame.data.len();
        SinkFlow::Continue
    }
}

/// Stream RX data from the ring buffer with a private cursor.
///
/// Each subscription owns a private cursor — it does NOT move the shared
/// read cursor. Both framed and raw paths emit per-chunk/frame notifications
/// and a final stop notification with stop_reason + offset fields.
///
/// When `matcher` is `Some`, the stream detects the first match and emits
/// a final stop notification with `matched=true` and `match_index`, then
/// terminates.
///
/// When `decoder` is `Some`, data notifications are emitted per-frame
/// rather than per-chunk. Frame payloads include `frame_index`, `frame_type`,
/// `data`, and optional `parsed` fields. Raw chunk notifications are
/// suppressed when framing is active.
///
/// Gap reporting: if the private cursor falls behind the ring's
/// `start_offset`, the next notification includes `bytes_lost` and continues
/// from the clamped position (`start_offset`). The subscription never
/// silently dies — gaps are always observable.
///
/// Uses [`RxStopController`] for all stop-condition evaluation so that
/// `subscribe` and `read` produce identical stop reasons for the same inputs.
#[allow(clippy::too_many_arguments)]
async fn stream_rx_from_ring(
    peer: Peer<RoleServer>,
    conn: Arc<crate::serial::SerialConnection>,
    session: Arc<crate::rx_session::RxSession>,
    encoding: crate::codec::Encoding,
    _max_buffered_bytes: usize,
    poll_interval_ms: u64,
    timeout_ms: Option<u64>,
    no_new_rx_timeout_ms: Option<u64>,
    // Held for RAII: dropping releases the budget reservation.
    _reservation: Box<dyn crate::buffer_budget::BufferReservation>,
    mut matcher: Option<Matcher>,
    // Pre-constructed frame decoder (validated in the subscribe handler).
    // None when no framing was requested.
    decoder: Option<crate::framing::FrameDecoder>,
    // Framing config (for metadata like max_frames; decoder is already built).
    framing: Option<crate::framing::RxFramingConfig>,
    // Parser config (passed for reference; decoder already incorporated it).
    _parser: Option<crate::framing::ParserConfig>,
    // The initial private cursor position, resolved from the `from` parameter.
    initial_cursor: u64,
) {
    let conn_id = session.connection_id().to_string();
    let logger = format!("serial:{conn_id}");
    let start = Instant::now();
    let ring = session.ring();

    // Private cursor — subscriptions do NOT move the shared read cursor.
    let mut private_cursor = initial_cursor;

    // Subscribe does not use max_buffered_bytes as a stop condition (it
    // streams each chunk immediately). We pass 0 so the controller never
    // stops on MaxBufferedBytes.
    let mut ctrl = RxStopController::new(start, timeout_ms, 0, no_new_rx_timeout_ms);
    let mut stop_outcome: Option<crate::stop_controller::RxStopOutcome> = None;
    let mut match_frame_index: Option<usize> = None;
    let mut match_offset: Option<usize> = None;
    let mut frame_error_msg: Option<String> = None;

    // Track total bytes sent via per-chunk data notifications.
    let mut total_returned: usize = 0;

    // Accumulated buffer for context shaping on match.
    let context_amount = matcher.as_ref().and_then(|m| m.context_amount());
    let needle_len = matcher.as_ref().and_then(|m| m.needle_len());
    let mut accumulated: Vec<u8> = Vec::new();

    // Frame decoder state.
    let max_frames = framing.as_ref().and_then(|f| f.max_frames);
    let mut decoder = decoder;
    let mut frames_emitted: usize = 0;
    let mut frames_dropped: usize = 0;
    let peer_owned = peer.clone();

    // Total bytes lost to ring wrap over the subscription lifetime.
    let mut total_bytes_lost: u64 = 0;

    // Track raw byte offset within the stream for from_offset/next_offset.
    // The first chunk's from_offset is the clamped initial_cursor.
    let mut from_offset: Option<u64> = None;
    let mut next_offset: u64 = private_cursor;

    loop {
        // Pause timeouts while the connection is disconnected or reconnecting.
        // If reconnect is NOT enabled, exit the loop so flush_partial can run
        // and the client receives a stop notification with the partial frame.
        match disconnect_state(&conn, &mut ctrl) {
            DisconnectState::Closed => {
                stop_outcome = Some(ctrl.connection_closed());
                break;
            }
            DisconnectState::Reconnecting => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            DisconnectState::Active => {}
        }

        if let RxStopDecision::Stop(outcome) = ctrl.check_timeout() {
            stop_outcome = Some(outcome);
            break;
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            stop_outcome = Some(outcome);
            break;
        }

        // Read next data from the ring at the private cursor.
        let slice = ring.read_from(private_cursor, _max_buffered_bytes);

        // Gap reporting: if bytes_lost > 0, include it and continue.
        if slice.bytes_lost > 0 {
            total_bytes_lost += slice.bytes_lost;
        }
        // If from_offset hasn't been set yet, record it from the first slice.
        if from_offset.is_none() && !slice.bytes.is_empty() {
            from_offset = Some(slice.from_offset);
        }

        if slice.bytes.is_empty() {
            // No data available yet. Wait for new data or poll interval.
            let poll_duration = Duration::from_millis(poll_interval_ms);
            tokio::select! {
                _ = ring.wait_for_data(private_cursor) => {
                    // Data arrived — loop back to read it.
                    continue;
                }
                _ = tokio::time::sleep(poll_duration) => {
                    // Poll wakeup: check timeouts and stop conditions.
                    continue;
                }
            }
        }

        let chunk = slice.bytes;
        let n = chunk.len();
        ctrl.notify_data_received();

        // Update offset tracking.
        if from_offset.is_none() {
            from_offset = Some(slice.from_offset);
        }
        next_offset = slice.next_offset;

        // Accumulate for context shaping if a matcher with context is active.
        if context_amount.is_some() {
            let room = _max_buffered_bytes.saturating_sub(accumulated.len());
            let take = chunk.len().min(room);
            accumulated.extend_from_slice(&chunk[..take]);
        }

        // Feed to frame decoder.
        let mut suppress_chunk_notification = false;
        if let Some(ref mut dec) = decoder {
            suppress_chunk_notification = true;
            let mut sink = SubscribeFrameSink {
                peer: peer_owned.clone(),
                conn: &conn,
                logger: logger.as_str(),
                conn_id: conn_id.as_str(),
                encoding,
                total_returned: &mut total_returned,
                match_offset: &mut match_offset,
                match_frame_index: &mut match_frame_index,
            };
            let outcome = consume_frames(
                &chunk,
                dec,
                &mut matcher,
                max_frames,
                &mut frames_emitted,
                &mut sink,
                &mut frames_dropped,
            )
            .await;
            stop_outcome = frame_outcome_to_stop(
                outcome,
                &ctrl,
                total_returned,
                match_offset,
                &mut frame_error_msg,
                &conn_id,
            );
        }
        if stop_outcome.is_some() {
            break;
        }

        // When framing is NOT active, match on raw chunk bytes.
        if !suppress_chunk_notification {
            let match_result = matcher.as_mut().map(|m| m.push(&chunk));
            if let Some(m) = matcher.as_mut() {
                let keep = m
                    .needle_len()
                    .map(|n| n.max(1).saturating_add(1))
                    .unwrap_or(256);
                let cap = _max_buffered_bytes.max(keep);
                if m.len() > cap {
                    m.truncate_front(cap);
                }
            }
            if let RxStopDecision::Stop(outcome) = ctrl.push_data(n, total_returned, match_result) {
                stop_outcome = Some(outcome);
            }

            // Emit data notification (including gap info).
            let encoded = match codec::encode_or_hex(encoding, &chunk) {
                Ok(payload) => {
                    if let Some(reason) = &payload.fallback_reason {
                        // Lossless fallback: the chunk still emits as exact
                        // spaced hex and the private cursor advances. Never
                        // counted as a dropped notification.
                        warn!(
                            "RX chunk on {conn_id}: {encoding} cannot encode {n} bytes \
                             ({reason}); using hex"
                        );
                    }
                    payload
                }
                Err(e) => {
                    // True encode+hex failure: preserve the legacy error
                    // notification + drop accounting.
                    warn!(
                        "RX encoding error on {conn_id}: {encoding} cannot encode {n} bytes — dropped"
                    );
                    conn.record_notification_drop();
                    conn.log().notification_dropped(&format!(
                        "encoding error: {encoding} cannot encode {n} bytes"
                    ));
                    let notification = SubscribeEncodingErrorNotification {
                        connection_id: conn_id.to_string(),
                        encoding_error: true,
                        encoding: encoding.to_string(),
                        bytes_dropped: n,
                        reason: e.to_string(),
                        bytes_lost: if slice.bytes_lost > 0 {
                            Some(slice.bytes_lost)
                        } else {
                            None
                        },
                    };
                    let payload = serde_json::to_value(&notification).unwrap_or_else(|e| {
                        warn!(
                            "SubscribeEncodingErrorNotification serialization error on {conn_id}: {e}"
                        );
                        serde_json::json!({})
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

            let notification = SubscribeChunkNotification {
                connection_id: conn_id.to_string(),
                bytes_read: n,
                encoding: encoded.encoding.to_string(),
                data: encoded.data,
                bytes_lost: if slice.bytes_lost > 0 {
                    Some(slice.bytes_lost)
                } else {
                    None
                },
            };
            let payload = serde_json::to_value(&notification).unwrap_or_else(|e| {
                warn!("SubscribeChunkNotification serialization error on {conn_id}: {e}");
                serde_json::json!({})
            });
            let param = LoggingMessageNotificationParam {
                level: LoggingLevel::Info,
                logger: Some(logger.clone()),
                data: payload,
            };
            if let Err(e) = peer.notify_logging_message(param).await {
                error!("RX stream peer disconnected: {e}");
                conn.record_notification_drop();
                conn.log()
                    .notification_dropped(&format!("peer disconnected: {e}"));
                stop_outcome = Some(ctrl.peer_disconnected());
                break;
            }
            total_returned += n;

            if stop_outcome.is_some() {
                break;
            }
        } // end if !suppress_chunk_notification

        // Advance private cursor past consumed bytes.
        private_cursor = next_offset;
    }

    // Advance private cursor past consumed bytes on framing error
    // (same contract as read — the decoder consumed the malformed bytes).
    private_cursor = next_offset;

    // Flush partial frame from decoder before building stop payload.
    if let Some(ref mut dec) = decoder {
        if let Some(partial) = dec.flush_partial() {
            frames_emitted += 1;
            let encoded = match codec::encode_or_hex(encoding, &partial.data) {
                Ok(payload) => {
                    if let Some(reason) = &payload.fallback_reason {
                        warn!(
                            "RX partial frame on {conn_id} not encodable as {encoding} \
                             ({reason}); using hex"
                        );
                    }
                    Some(payload)
                }
                Err(e) => {
                    warn!("RX partial frame encoding error on {conn_id}: {e}");
                    conn.record_notification_drop();
                    conn.log()
                        .notification_dropped(&format!("partial frame encoding error: {e}"));
                    None
                }
            };
            if let Some(encoded) = encoded {
                let notification = SubscribePartialFrameNotification {
                    connection_id: conn_id.to_string(),
                    partial: true,
                    frame_index: partial.index,
                    frame_type: partial.frame_type.to_string(),
                    encoding: encoded.encoding.to_string(),
                    data: encoded.data,
                    parsed: partial.parsed,
                };
                let payload = serde_json::to_value(&notification).unwrap_or_else(|e| {
                    warn!(
                        "SubscribePartialFrameNotification serialization error on {conn_id}: {e}"
                    );
                    serde_json::json!({})
                });
                let param = LoggingMessageNotificationParam {
                    level: LoggingLevel::Info,
                    logger: Some(logger.clone()),
                    data: payload,
                };
                if let Err(e) = peer.notify_logging_message(param).await {
                    warn!("RX partial frame notify failed on {conn_id}: {e}");
                    conn.record_notification_drop();
                    conn.log()
                        .notification_dropped(&format!("partial frame notify: {e}"));
                }
            }
            total_returned += partial.data.len();
        }
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let outcome = stop_outcome.unwrap_or_else(|| ctrl.channel_closed());
    let bytes_observed = ctrl.bytes_observed();
    let truncated = total_returned < bytes_observed;
    if truncated {
        conn.record_truncation();
        conn.log().truncated(bytes_observed, total_returned);
    }
    let stop_meta = RxStopMetadata {
        stop_reason: outcome.meta.stop_reason,
        truncated,
        bytes_observed,
        bytes_returned: total_returned,
    };

    // Apply context shaping if configured (must be done before building the
    // struct since the shaped values differ from the raw outcome).
    let (shaped_match_index, shaped_data) = if let (Some(midx), Some(ca), Some(nlen)) =
        (outcome.match_index, context_amount, needle_len)
    {
        let shaped = shape_match_context(&accumulated, midx, nlen, Some(ca));
        (Some(shaped.match_index), Some(shaped.data))
    } else {
        (outcome.match_index, None)
    };
    // Shaped match context: `data` + its effective `encoding` are set
    // together so hex fallback remains decodable. A successful fallback is
    // warned but not counted; only a true encode+hex failure drops.
    let (match_data_encoded, match_data_encoding) = match shaped_data.as_ref() {
        Some(data) => match codec::encode_or_hex(encoding, data) {
            Ok(payload) => {
                if let Some(reason) = &payload.fallback_reason {
                    warn!(
                        "RX stream match context on {conn_id} not encodable as {encoding} \
                         ({reason}); using hex"
                    );
                }
                (Some(payload.data), Some(payload.encoding.to_string()))
            }
            Err(e) => {
                warn!("RX stream match context encoding error on {conn_id}: {e}");
                conn.record_notification_drop();
                conn.log()
                    .notification_dropped(&format!("match context encoding error: {e}"));
                (None, None)
            }
        },
        None => (None, None),
    };

    let stop_notification = SubscribeStopNotification {
        connection_id: conn_id.to_string(),
        stop_reason: stop_meta.stop_reason.to_string(),
        truncated: stop_meta.truncated,
        bytes_observed: stop_meta.bytes_observed,
        bytes_returned: stop_meta.bytes_returned,
        elapsed_ms,
        timeout_ms,
        no_new_rx_timeout_ms,
        from_offset,
        next_offset: private_cursor,
        bytes_lost: total_bytes_lost,
        error: frame_error_msg,
        matched: if outcome.matched { Some(true) } else { None },
        match_index: if outcome.matched {
            shaped_match_index
        } else {
            None
        },
        match_frame_index: if outcome.matched {
            match_frame_index
        } else {
            None
        },
        data: if outcome.matched {
            match_data_encoded
        } else {
            None
        },
        encoding: if outcome.matched {
            match_data_encoding
        } else {
            None
        },
        frames_emitted,
        frames_dropped,
        start_offset: ring.start_offset(),
        end_offset: ring.end_offset(),
    };
    let stop_payload = serde_json::to_value(&stop_notification).unwrap_or_else(|e| {
        warn!("SubscribeStopNotification serialization error on {conn_id}: {e}");
        serde_json::json!({})
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
        "RX stream ended for {conn_id}: reason={} bytes={} elapsed={}ms",
        stop_meta.stop_reason, stop_meta.bytes_observed, elapsed_ms
    );
}

#[cfg(test)]
mod tests {
    use crate::framing::ParsedFrame;
    use crate::tools::types::{
        SubscribeChunkNotification, SubscribeEncodingErrorNotification, SubscribeFrameNotification,
        SubscribePartialFrameNotification, SubscribeStopNotification,
    };

    #[test]
    fn parsed_frame_serializes_with_inlined_object_shape() {
        let at = ParsedFrame::AtCommand {
            response_type: "data".into(),
            command: Some("CGREG".into()),
            status: Some("OK".into()),
            fields: vec!["1".into(), "2".into()],
        };
        let v = serde_json::to_value(&at).unwrap();
        assert_eq!(v["parser"], "at_command");
        assert_eq!(v["response_type"], "data");
        assert_eq!(v["command"], "CGREG");
        assert_eq!(v["status"], "OK");
        assert_eq!(v["fields"], serde_json::json!(["1", "2"]));

        // command/status omitted when None.
        let at_min = ParsedFrame::AtCommand {
            response_type: "urc".into(),
            command: None,
            status: None,
            fields: vec![],
        };
        let v = serde_json::to_value(&at_min).unwrap();
        assert!(v.get("command").is_none());
        assert!(v.get("status").is_none());

        // JSON object fields are inlined alongside "parser".
        let j = ParsedFrame::Json(serde_json::json!({"sensor": "temp", "value": 25.5}));
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v["parser"], "json");
        assert_eq!(v["sensor"], "temp");
        assert_eq!(v["value"], 25.5);

        assert_eq!(
            serde_json::to_value(&ParsedFrame::Raw).unwrap()["parser"],
            "raw"
        );
    }

    #[test]
    fn subscribe_chunk_notification_serializes_with_expected_shape() {
        let n = SubscribeChunkNotification {
            connection_id: "c1".into(),
            bytes_read: 5,
            encoding: "utf8".into(),
            data: "hello".into(),
            bytes_lost: None,
        };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["connection_id"], "c1");
        assert_eq!(v["bytes_read"], 5);
        assert_eq!(v["encoding"], "utf8");
        assert_eq!(v["data"], "hello");
        assert!(v.get("bytes_lost").is_none());

        // With bytes_lost.
        let n_lost = SubscribeChunkNotification {
            connection_id: "c2".into(),
            bytes_read: 3,
            encoding: "hex".into(),
            data: "abc".into(),
            bytes_lost: Some(10),
        };
        let v = serde_json::to_value(&n_lost).unwrap();
        assert_eq!(v["bytes_lost"], 10);
    }

    #[test]
    fn subscribe_frame_notification_serializes_with_expected_shape() {
        // Basic frame — no parsed, no matched.
        let n = SubscribeFrameNotification {
            connection_id: "c1".into(),
            frame_index: 3,
            frame_type: "line".into(),
            encoding: "utf8".into(),
            data: "hello".into(),
            parsed: None,
            matched: None,
        };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["connection_id"], "c1");
        assert_eq!(v["frame_index"], 3);
        assert_eq!(v["frame_type"], "line");
        assert_eq!(v["encoding"], "utf8");
        assert_eq!(v["data"], "hello");
        assert!(v.get("parsed").is_none());
        assert!(v.get("matched").is_none());

        // With matched.
        let n_m = SubscribeFrameNotification {
            connection_id: "c2".into(),
            frame_index: 0,
            frame_type: "line2".into(),
            encoding: "hex".into(),
            data: "ff".into(),
            parsed: None,
            matched: Some(true),
        };
        let v = serde_json::to_value(&n_m).unwrap();
        assert_eq!(v["matched"], true);
    }

    #[test]
    fn subscribe_encoding_error_notification_serializes_with_expected_shape() {
        let n = SubscribeEncodingErrorNotification {
            connection_id: "c1".into(),
            encoding_error: true,
            encoding: "utf8".into(),
            bytes_dropped: 42,
            reason: "invalid utf-8".into(),
            bytes_lost: None,
        };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["connection_id"], "c1");
        assert_eq!(v["encoding_error"], true);
        assert_eq!(v["encoding"], "utf8");
        assert_eq!(v["bytes_dropped"], 42);
        assert_eq!(v["reason"], "invalid utf-8");
        assert!(v.get("bytes_lost").is_none());

        // With bytes_lost.
        let n_loss = SubscribeEncodingErrorNotification {
            connection_id: "c2".into(),
            encoding_error: true,
            encoding: "hex".into(),
            bytes_dropped: 1,
            reason: "bad".into(),
            bytes_lost: Some(5),
        };
        let v = serde_json::to_value(&n_loss).unwrap();
        assert_eq!(v["bytes_lost"], 5);
    }

    #[test]
    fn subscribe_partial_frame_notification_serializes_with_expected_shape() {
        let n = SubscribePartialFrameNotification {
            connection_id: "c1".into(),
            partial: true,
            frame_index: 7,
            frame_type: "cobs".into(),
            encoding: "hex".into(),
            data: "deadbeef".into(),
            parsed: None,
        };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["connection_id"], "c1");
        assert_eq!(v["partial"], true);
        assert_eq!(v["frame_index"], 7);
        assert_eq!(v["frame_type"], "cobs");
        assert_eq!(v["encoding"], "hex");
        assert_eq!(v["data"], "deadbeef");
        assert!(v.get("parsed").is_none());
    }

    #[test]
    fn subscribe_stop_notification_serializes_with_expected_shape() {
        let n = SubscribeStopNotification {
            connection_id: "c1".into(),
            stop_reason: "timeout".into(),
            truncated: false,
            bytes_observed: 10,
            bytes_returned: 10,
            elapsed_ms: 100,
            timeout_ms: Some(1000),
            no_new_rx_timeout_ms: None,
            from_offset: Some(0),
            next_offset: 10,
            bytes_lost: 0,
            error: None,
            matched: None,
            match_index: None,
            match_frame_index: None,
            data: None,
            encoding: None,
            frames_emitted: 0,
            frames_dropped: 0,
            start_offset: 0,
            end_offset: 10,
        };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["stop_reason"], "timeout");
        assert_eq!(v["truncated"], false);
        assert_eq!(v["next_offset"], 10);
        assert_eq!(v["start_offset"], 0);
        assert_eq!(v["end_offset"], 10);
        assert!(v.get("error").is_none());
        assert!(v.get("matched").is_none());
        assert!(v.get("no_new_rx_timeout_ms").is_none());
        assert!(v.get("match_index").is_none());
        assert!(v.get("match_frame_index").is_none());
        assert!(v.get("data").is_none());
        // encoding is only serialized alongside data.
        assert!(v.get("encoding").is_none());

        // With match.
        let n_m = SubscribeStopNotification {
            connection_id: "c2".into(),
            stop_reason: "match_found".into(),
            truncated: false,
            bytes_observed: 20,
            bytes_returned: 20,
            elapsed_ms: 50,
            timeout_ms: Some(500),
            no_new_rx_timeout_ms: None,
            from_offset: Some(0),
            next_offset: 20,
            bytes_lost: 0,
            error: None,
            matched: Some(true),
            match_index: Some(5),
            match_frame_index: Some(2),
            data: Some("TARGET".into()),
            encoding: Some("utf8".into()),
            frames_emitted: 3,
            frames_dropped: 0,
            start_offset: 0,
            end_offset: 20,
        };
        let v = serde_json::to_value(&n_m).unwrap();
        assert_eq!(v["matched"], true);
        assert_eq!(v["match_index"], 5);
        assert_eq!(v["match_frame_index"], 2);
        assert_eq!(v["data"], "TARGET");
        assert_eq!(v["encoding"], "utf8");
    }
}
