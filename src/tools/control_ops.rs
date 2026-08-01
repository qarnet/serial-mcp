use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rmcp::{model::Meta, Json, Peer, RoleServer};
use tokio::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::learning;
use crate::profiles::ProfilePersistenceOperation;
use crate::serial::{ConnectionManager, FlushTarget, SerialConnection};
use crate::tools::helpers::{
    build_read_result, clamp_or_err, clamp_timeout_or_err, log_tool_err, lookup_connection,
    map_budget_err, parse_encoding, read_from_private_cursor, require_min_or_err, MAX_READ_BYTES,
    MAX_TIMEOUT_MS, MIN_READ_BYTES,
};
use crate::tools::types::{
    CaptureBootArgs, CaptureBootResult, SendBreakArgs, SendBreakResult, SetDtrRtsArgs,
    SetDtrRtsResult, SetFlowControlArgs, SetFlowControlResult,
};

/// Default read-phase timeout for `capture_boot` (omitted/null both resolve
/// here) so the whole operation stays bounded: hold + settle + timeout.
const DEFAULT_CAPTURE_BOOT_TIMEOUT_MS: u64 = 5000;

/// Cancellation-safe release guard for the `capture_boot` reset pulse.
///
/// Armed with the configured RELEASE state BEFORE the assertion (production
/// `set_dtr_rts` sets DTR then RTS, so an RTS failure can leave DTR changed).
/// On drop while armed, spawns a best-effort release using the configured
/// release state. Every explicit path attempts the release and disarms the
/// guard only after success.
struct ResetReleaseGuard {
    connection: Arc<SerialConnection>,
    release_dtr: bool,
    release_rts: bool,
    disarmed: AtomicBool,
}

impl ResetReleaseGuard {
    fn disarm(&self) {
        self.disarmed.store(true, Ordering::Relaxed);
    }
}

impl Drop for ResetReleaseGuard {
    fn drop(&mut self) {
        if self.disarmed.load(Ordering::Relaxed) {
            return;
        }
        let connection = Arc::clone(&self.connection);
        let release_dtr = self.release_dtr;
        let release_rts = self.release_rts;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(e) = connection
                    .set_dtr_rts_unlocked(release_dtr, release_rts)
                    .await
                {
                    warn!("capture_boot: best-effort reset-line release failed: {e}");
                }
            });
        }
    }
}

/// Attempt the configured release, disarming the guard on success. On
/// failure the guard stays armed so its drop still attempts cleanup; the
/// caller decides whether to surface the failure as a tool error.
///
/// A closed/disconnected port counts as released: the lines are gone with
/// the port, so no cleanup is needed (this is what makes a close-during-
/// capture return a clean `connection_closed` result instead of an error).
async fn release_reset_lines(
    connection: &SerialConnection,
    dtr: bool,
    rts: bool,
    guard: &mut Option<ResetReleaseGuard>,
) -> Result<(), String> {
    match connection.set_dtr_rts_unlocked(dtr, rts).await {
        Ok(()) => {
            if let Some(g) = guard.as_ref() {
                g.disarm();
            }
            Ok(())
        }
        Err(crate::error::SerialError::ConnectionClosed(_)) => {
            // Port is gone — nothing left to release.
            if let Some(g) = guard.as_ref() {
                g.disarm();
            }
            Ok(())
        }
        Err(e) => Err(log_tool_err(
            "capture_boot",
            "Failed to release reset lines",
            e,
        )),
    }
}

/// Atomic boot capture: purge OS input, mark the RX live edge under the pump
/// gate, optionally pulse DTR/RTS, then capture only post-mark bytes through
/// the existing read pipeline with a PRIVATE cursor.
///
/// The shared `read` cursor is never moved and ring history stays readable.
/// The result stays in memory (`max_buffered_bytes` bounds it); there is no
/// file output. Line release is guaranteed on every path via
/// [`ResetReleaseGuard`].
#[allow(clippy::too_many_arguments)]
pub async fn capture_boot(
    connections: &Arc<ConnectionManager>,
    rx_sessions: &Arc<crate::rx_session::RxSessionManager>,
    budget: &Arc<dyn crate::buffer_budget::BufferBudget>,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: CaptureBootArgs,
) -> Result<Json<CaptureBootResult>, String> {
    debug!(
        "capture_boot {} (reset {:?}, settle {:?}, timeout {:?})",
        args.connection_id, args.reset, args.settle_ms, args.timeout_ms
    );

    // ── 1. Validate everything BEFORE any line transition ─────────────────
    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;

    // The connection's max_buffered_bytes default bounds the in-memory result.
    let max_buffered_bytes = require_min_or_err(
        "capture_boot.max_buffered_bytes",
        connection.max_buffered_bytes_default(),
        MIN_READ_BYTES,
    )?;
    let max_buffered_bytes = clamp_or_err(
        "capture_boot.max_buffered_bytes",
        max_buffered_bytes,
        MAX_READ_BYTES,
    )?;

    // Omitted/null timeout resolves to the bounded default (5000ms); serde
    // cannot distinguish an omitted field from an explicit null on
    // Option<u64>, so both take the default and the operation stays bounded.
    let timeout_ms = Some(args.timeout_ms.unwrap_or(DEFAULT_CAPTURE_BOOT_TIMEOUT_MS));
    if let Some(ms) = args.timeout_ms {
        clamp_timeout_or_err("capture_boot.timeout_ms", ms, MAX_TIMEOUT_MS)?;
    }
    if let Some(ms) = args.no_new_rx_timeout_ms {
        if ms == 0 {
            return Err("capture_boot.no_new_rx_timeout_ms must be > 0".into());
        }
        clamp_timeout_or_err("capture_boot.no_new_rx_timeout_ms", ms, MAX_TIMEOUT_MS)?;
    }
    if let Some(ms) = args.settle_ms {
        clamp_timeout_or_err("capture_boot.settle_ms", ms, MAX_TIMEOUT_MS)?;
    }
    if let Some(reset) = &args.reset {
        if reset.hold_ms == 0 {
            return Err("capture_boot.reset.hold_ms must be >= 1".into());
        }
        clamp_timeout_or_err("capture_boot.reset.hold_ms", reset.hold_ms, MAX_TIMEOUT_MS)?;
    }

    let matcher = match args.r#match {
        Some(ref m) => Some(
            crate::match_config::validate_match_request(m)
                .map_err(|e| format!("capture_boot.match: {e}"))?,
        ),
        None => None,
    };

    // Framing/parser/protocol precedence (same 4 layers as read).
    let rx_framing = crate::precedence::resolve_field(
        args.rx_framing.clone(),
        args.protocol,
        crate::framing::preset_rx_framing,
        connection.rx_framing_default(),
        connection.protocol_default(),
    );
    let rx_parser = crate::precedence::resolve_field(
        args.rx_parser.clone(),
        args.protocol,
        crate::framing::preset_rx_parser,
        connection.rx_parser_default(),
        connection.protocol_default(),
    );

    // Construction validation BEFORE any reset: an invalid framing config
    // must never touch lines or pulse the device.
    if let Some(ref cfg) = rx_framing {
        crate::framing::FrameDecoder::new(cfg, rx_parser.as_ref())
            .map_err(|e| format!("capture_boot.rx_framing: {e}"))?;
    }

    // ── 2. Reserve output budget and get the RX session ───────────────────
    let _reservation = budget
        .try_reserve(max_buffered_bytes)
        .map_err(|e| map_budget_err("capture_boot.max_buffered_bytes", e))?;

    let progress_token = meta.get_progress_token();

    // Same ring size the connection was opened with (matches `open`).
    let session = rx_sessions
        .get_or_create(Arc::clone(&connection), connection.rx_buffer_size())
        .await
        .map_err(|e| format!("capture_boot: {e}"))?;

    let has_reset = args.reset.is_some();

    // ── 3. Serialize line control when a reset is configured ──────────────
    let _control_guard = if has_reset {
        Some(connection.control_lock().lock().await)
    } else {
        None
    };

    // ── 4. Pump gate: wait for any in-flight pump read+append; block the
    //        pump until mark/reset setup completes ────────────────────────
    let pump_guard = session.pump_gate_guard().await;

    // ── 5. Purge unread OS input (bytes that predate capture but have not
    //        yet entered the ring). Failure is a tool error BEFORE any line
    //        assertion. ───────────────────────────────────────────────────
    connection
        .flush_buffers(FlushTarget::Input)
        .await
        .map_err(|e| {
            log_tool_err(
                "capture_boot",
                &format!("Failed to purge OS input on {}", args.connection_id),
                e,
            )
        })?;
    let os_input_flushed = true;

    // ── 6. Atomic mark at the live edge (pump is quiesced) ────────────────
    let mark_offset = session.ring().end_offset();
    let pre_mark_bytes = mark_offset;

    // ── 7. Arm the release guard and assert reset lines while still
    //        holding the pump gate ────────────────────────────────────────
    let mut release_guard = args.reset.as_ref().map(|reset| ResetReleaseGuard {
        connection: Arc::clone(&connection),
        release_dtr: reset.release_dtr,
        release_rts: reset.release_rts,
        disarmed: AtomicBool::new(false),
    });
    if let Some(reset) = &args.reset {
        connection
            .set_dtr_rts_unlocked(reset.assert_dtr, reset.assert_rts)
            .await
            .map_err(|e| {
                log_tool_err(
                    "capture_boot",
                    &format!("Failed to assert reset lines on {}", args.connection_id),
                    e,
                )
            })?;
    }

    // ── 8. Release the pump gate immediately so the pump captures bytes
    //        during hold and line release ─────────────────────────────────
    drop(pump_guard);

    // ── 9. Hold the reset state; ALWAYS release lines afterwards ─────────
    if let Some(reset) = &args.reset {
        let hold = tokio::time::sleep(Duration::from_millis(reset.hold_ms));
        tokio::pin!(hold);
        let cancelled = ct.cancelled();
        tokio::pin!(cancelled);
        tokio::select! {
            _ = &mut cancelled => {
                // Cancellation during hold: release lines first, then route
                // the already-cancelled token through the read path so the
                // result is a structured `cancelled` outcome, not an
                // ad-hoc error.
                if let Err(e) = release_reset_lines(&connection, reset.release_dtr, reset.release_rts, &mut release_guard).await {
                    warn!("capture_boot: release during cancellation failed (cleanup armed): {e}");
                }
            }
            _ = &mut hold => {
                release_reset_lines(&connection, reset.release_dtr, reset.release_rts, &mut release_guard).await?;
            }
        }
    }

    // ── 10. Optional settle delay while the pump keeps appending ─────────
    if let Some(ms) = args.settle_ms {
        if ms > 0 {
            let settle = tokio::time::sleep(Duration::from_millis(ms));
            tokio::pin!(settle);
            let cancelled = ct.cancelled();
            tokio::pin!(cancelled);
            tokio::select! {
                _ = &mut cancelled => {}
                _ = &mut settle => {}
            }
        }
    }

    // ── 11. Consume from the mark with a PRIVATE cursor ──────────────────
    let (outcome, _final_private_cursor) = read_from_private_cursor(
        &session,
        mark_offset,
        max_buffered_bytes,
        timeout_ms,
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

    // Lines were released explicitly above; disarm the drop-time cleanup.
    if let Some(guard) = release_guard.as_ref() {
        guard.disarm();
    }

    // ── 12. Build the nested ReadResult and record read logs like `read` ─
    let result = build_read_result(
        outcome,
        args.connection_id.clone(),
        connection.name().map(str::to_string),
        encoding,
        timeout_ms,
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
        if let Some(ref m) = args.r#match {
            log.match_found(&m.pattern, &m.config.mode.to_string());
        }
    }

    info!(
        "capture_boot {} mark={} pre_mark={} stop_reason={}",
        args.connection_id, mark_offset, pre_mark_bytes, result.0.stop_reason
    );

    Ok(Json(CaptureBootResult {
        connection_id: args.connection_id,
        name: connection.name().map(str::to_string),
        reset: args.reset,
        mark_offset,
        pre_mark_bytes,
        os_input_flushed,
        read: result.0,
    }))
}

pub async fn set_dtr_rts(
    connections: &Arc<ConnectionManager>,
    args: SetDtrRtsArgs,
) -> Result<Json<SetDtrRtsResult>, String> {
    debug!(
        "set_dtr_rts {} dtr={} rts={}",
        args.connection_id, args.dtr, args.rts
    );

    let connection = lookup_connection(connections, &args.connection_id).await?;
    connection
        .set_dtr_rts(args.dtr, args.rts)
        .await
        .map_err(|e| {
            log_tool_err(
                "set_dtr_rts",
                &format!("Failed to set control lines on {}", args.connection_id),
                e,
            )
        })?;

    info!(
        "Control lines on {} set to dtr={} rts={}",
        args.connection_id, args.dtr, args.rts
    );

    Ok(Json(SetDtrRtsResult {
        connection_id: args.connection_id,
        name: connection.name().map(str::to_string),
        dtr: args.dtr,
        rts: args.rts,
    }))
}

pub async fn set_flow_control(
    connections: &Arc<ConnectionManager>,
    store: &Arc<crate::profile_store::ProfileStore>,
    args: SetFlowControlArgs,
) -> Result<Json<SetFlowControlResult>, String> {
    debug!(
        "set_flow_control {} flow_control={}",
        args.connection_id, args.flow_control
    );

    let flow_control = args.flow_control.parse::<crate::serial::FlowControl>()?;
    let connection = lookup_connection(connections, &args.connection_id).await?;

    // Hold the learning lock across the hardware mutation, the effective
    // snapshot, and the CAS persistence attempt.
    let _learning_guard = connection.learning_lock().lock().await;

    connection
        .set_flow_control(flow_control)
        .await
        .map_err(|e| {
            log_tool_err(
                "set_flow_control",
                &format!("Failed to set flow control on {}", args.connection_id),
                e,
            )
        })?;

    // Write-through learning: hardware mutation succeeded; persist flow
    // control through the bound profile (if any). Failure keeps the result
    // successful with `state="failed"`.
    let (profile, persistence) =
        learning::learn(store, &connection, ProfilePersistenceOperation::Learned).await;

    Ok(Json(SetFlowControlResult {
        connection_id: args.connection_id,
        name: connection.name().map(str::to_string),
        flow_control,
        profile,
        profile_persistence: Some(persistence),
    }))
}

pub async fn send_break(
    connections: &Arc<ConnectionManager>,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: SendBreakArgs,
) -> Result<Json<SendBreakResult>, String> {
    debug!(
        "send_break {} duration={}ms",
        args.connection_id, args.duration_ms
    );

    clamp_timeout_or_err("send_break.duration_ms", args.duration_ms, MAX_TIMEOUT_MS)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;

    struct BreakResetGuard {
        connection: Arc<SerialConnection>,
        disarmed: AtomicBool,
    }

    impl BreakResetGuard {
        fn disarm(&self) {
            self.disarmed.store(true, Ordering::Relaxed);
        }
    }

    impl Drop for BreakResetGuard {
        fn drop(&mut self) {
            if self.disarmed.load(Ordering::Relaxed) {
                return;
            }
            let connection = Arc::clone(&self.connection);
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = connection.set_break_state(false).await;
                });
            }
        }
    }

    connection
        .set_break_state(true)
        .await
        .map_err(|e| log_tool_err("send_break", "Failed to assert BREAK", e))?;
    let guard = BreakResetGuard {
        connection: Arc::clone(&connection),
        disarmed: AtomicBool::new(false),
    };

    let progress_token = meta.get_progress_token();
    if let Some(token) = progress_token.clone() {
        let _ = peer
            .notify_progress(rmcp::model::ProgressNotificationParam {
                progress_token: token,
                progress: 0.0,
                total: Some(args.duration_ms as f64),
                message: Some("break asserted".into()),
            })
            .await;
    }

    let start = Instant::now();
    let deadline = start + Duration::from_millis(args.duration_ms);
    let mut progress_ticker = tokio::time::interval(Duration::from_millis(250));
    progress_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut progress_emitted = false;

    loop {
        tokio::select! {
            _ = ct.cancelled() => return Err("Cancelled".into()),
            _ = tokio::time::sleep_until(deadline) => break,
            _ = progress_ticker.tick() => {
                let elapsed = start.elapsed().as_millis() as u64;
                if let Some(token) = progress_token.clone() {
                    // Skip emitting progress at t=0 (redundant with initial message)
                    if progress_emitted || elapsed > 0 {
                        progress_emitted = true;
                        let _ = peer
                            .notify_progress(rmcp::model::ProgressNotificationParam {
                                progress_token: token,
                                progress: elapsed as f64,
                                total: Some(args.duration_ms as f64),
                                message: Some("holding break".into()),
                            })
                            .await;
                    }
                }
            }
        }
    }

    connection
        .set_break_state(false)
        .await
        .map_err(|e| log_tool_err("send_break", "Failed to release BREAK", e))?;
    guard.disarm();

    let actual_duration_ms = start.elapsed().as_millis() as u64;

    info!(
        "Sent break on {} for {}ms (actual {}ms)",
        args.connection_id, args.duration_ms, actual_duration_ms
    );

    if let Some(token) = progress_token {
        let _ = peer
            .notify_progress(rmcp::model::ProgressNotificationParam {
                progress_token: token,
                progress: args.duration_ms as f64,
                total: Some(args.duration_ms as f64),
                message: Some("break released".into()),
            })
            .await;
    }

    Ok(Json(SendBreakResult {
        connection_id: args.connection_id,
        name: connection.name().map(str::to_string),
        duration_ms: args.duration_ms,
        actual_duration_ms,
    }))
}
