use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{
    model::{ProgressNotificationParam, ProgressToken},
    service::Peer,
    Json, RoleServer,
};
use tracing::error;

use tokio::sync::mpsc;

use crate::codec::{self, Encoding};
use crate::match_config::MatchResult;
use crate::match_config::{shape_match_context, validate_match_request, MatchRequest, Matcher};
use crate::rx_metadata::RxStopMetadata;
use crate::rx_session::{RxEvent, RxSession};
use crate::serial::{ConnectionConfig, ConnectionManager, SerialConnection};
use crate::stop_controller::{RxStopController, RxStopDecision};
use crate::tools::rx_consume::{
    consume_frames, disconnect_state, DisconnectState, FrameOutcome, RxFrameSink, SinkFlow,
};
use crate::tools::types::*;

pub use crate::limits::{
    MAX_READ_BYTES, MAX_STREAM_CHUNK_BYTES, MAX_TIMEOUT_MS, MAX_WRITE_BYTES, MIN_POLL_INTERVAL_MS,
    MIN_READ_BYTES, MIN_STREAM_CHUNK_BYTES,
};

pub(crate) const DEFAULT_READ_TIMEOUT_MS: u64 = 1000;

pub fn clamp_or_err(name: &str, value: usize, max: usize) -> Result<usize, String> {
    if value > max {
        Err(format!("{name}={value} exceeds maximum {max}"))
    } else {
        Ok(value)
    }
}

pub fn require_min_or_err(name: &str, value: usize, min: usize) -> Result<usize, String> {
    if value < min {
        Err(format!("{name}={value} is below minimum {min}"))
    } else {
        Ok(value)
    }
}

pub fn clamp_timeout_or_err(name: &str, value: u64, max: u64) -> Result<u64, String> {
    if value > max {
        Err(format!("{name}={value}ms exceeds maximum {max}ms"))
    } else {
        Ok(value)
    }
}

pub fn clamp_poll_interval_or_err(name: &str, value: u64, min: u64) -> Result<u64, String> {
    if value < min {
        Err(format!("{name}={value}ms is below minimum {min}ms"))
    } else {
        Ok(value)
    }
}

// ------------------------------------------------------------------
// Budget error mapping
// ------------------------------------------------------------------

/// Map a [`crate::buffer_budget::BufferBudgetError`] to a user-facing error
/// string. `field` is the fully-qualified argument name
/// (e.g. `"read.max_buffered_bytes"`) used to prefix the limit/zero messages.
pub fn map_budget_err(field: &str, e: crate::buffer_budget::BufferBudgetError) -> String {
    use crate::buffer_budget::BufferBudgetError;
    match e {
        BufferBudgetError::OverToolLimit {
            requested,
            tool_limit,
        } => format!("{field}={requested} exceeds per-tool limit {tool_limit}"),
        BufferBudgetError::ZeroRequest => format!("{field} must be > 0"),
        BufferBudgetError::InsufficientProgramBudget {
            requested,
            available,
        } => format!(
            "insufficient program buffer budget: requested {requested}, available {available}"
        ),
    }
}

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
// RX request validation (shared by read and subscribe)
// ------------------------------------------------------------------

/// Per-tool limits and the error-message label for [`validate_rx_request`].
pub struct RxLimits {
    /// Tool name used to prefix error messages ("read" or "subscribe").
    pub tool: &'static str,
    /// Minimum allowed `max_buffered_bytes`.
    pub min_buffered: usize,
    /// Maximum allowed `max_buffered_bytes`.
    pub max_buffered: usize,
}

/// The common, validated inputs shared by `read` and `subscribe`.
#[derive(Debug)]
pub struct ResolvedRxArgs {
    pub encoding: Encoding,
    pub connection: Arc<SerialConnection>,
    pub max_buffered_bytes: usize,
    pub matcher: Option<Matcher>,
}

/// Accessors for the request fields common to `read` and `subscribe`.
pub trait RxRequestArgs {
    fn connection_id(&self) -> &str;
    fn encoding(&self) -> &str;
    fn max_buffered_bytes(&self) -> usize;
    fn timeout_ms(&self) -> Option<u64>;
    fn no_new_rx_timeout_ms(&self) -> Option<u64>;
    fn match_request(&self) -> Option<&MatchRequest>;
}

impl RxRequestArgs for ReadArgs {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }
    fn encoding(&self) -> &str {
        &self.encoding
    }
    fn max_buffered_bytes(&self) -> usize {
        self.max_buffered_bytes
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.timeout_ms
    }
    fn no_new_rx_timeout_ms(&self) -> Option<u64> {
        self.no_new_rx_timeout_ms
    }
    fn match_request(&self) -> Option<&MatchRequest> {
        self.r#match.as_ref()
    }
}

impl RxRequestArgs for SubscribeArgs {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }
    fn encoding(&self) -> &str {
        &self.encoding
    }
    fn max_buffered_bytes(&self) -> usize {
        self.max_buffered_bytes
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.timeout_ms
    }
    fn no_new_rx_timeout_ms(&self) -> Option<u64> {
        self.no_new_rx_timeout_ms
    }
    fn match_request(&self) -> Option<&MatchRequest> {
        self.r#match.as_ref()
    }
}

/// Validate and resolve the inputs common to `read` and `subscribe`: encoding,
/// connection lookup, `max_buffered_bytes` bounds, `timeout_ms` / silence
/// timeout, and matcher resolution. Error messages are prefixed with
/// `limits.tool` to match each tool's existing wording.
///
/// Does NOT reserve the buffer budget — the caller does that (subscribe must
/// drop any prior subscription before reserving).
pub async fn validate_rx_request<A: RxRequestArgs>(
    connections: &Arc<ConnectionManager>,
    args: &A,
    limits: RxLimits,
) -> Result<ResolvedRxArgs, String> {
    let encoding = parse_encoding(args.encoding())?;
    let connection = lookup_connection(connections, args.connection_id()).await?;

    let max_buffered_bytes = require_min_or_err(
        &format!("{}.max_buffered_bytes", limits.tool),
        args.max_buffered_bytes(),
        limits.min_buffered,
    )?;
    let max_buffered_bytes = clamp_or_err(
        &format!("{}.max_buffered_bytes", limits.tool),
        max_buffered_bytes,
        limits.max_buffered,
    )?;

    if let Some(timeout_ms) = args.timeout_ms() {
        clamp_timeout_or_err(
            &format!("{}.timeout_ms", limits.tool),
            timeout_ms,
            MAX_TIMEOUT_MS,
        )?;
    }
    if let Some(silence_ms) = args.no_new_rx_timeout_ms() {
        if silence_ms == 0 {
            return Err(format!("{}.no_new_rx_timeout_ms must be > 0", limits.tool));
        }
        clamp_timeout_or_err(
            &format!("{}.no_new_rx_timeout_ms", limits.tool),
            silence_ms,
            MAX_TIMEOUT_MS,
        )?;
    }

    let matcher = match args.match_request() {
        Some(m) => Some(validate_match_request(m)?),
        None => None,
    };

    Ok(ResolvedRxArgs {
        encoding,
        connection,
        max_buffered_bytes,
        matcher,
    })
}

// ------------------------------------------------------------------
// Read helpers
// ------------------------------------------------------------------

/// Outcome of a read call. `timed_out` distinguishes the genuine
/// read-timeout case from a successful read of `bytes`.
pub struct ReadOutcome {
    pub bytes: Vec<u8>,
    pub elapsed_ms: u64,
    pub meta: RxStopMetadata,
    /// Whether a match pattern was found. `false` when no matcher was provided.
    pub matched: bool,
    /// Byte offset within `bytes` where the match starts, or `None`.
    pub match_index: Option<usize>,
    /// When framing is active and match was found, the index of the frame
    /// that contained the match.
    pub match_frame_index: Option<usize>,
    /// Decoded frames, empty when framing was not configured.
    pub frames: Vec<crate::framing::Frame>,
    /// Number of frames dropped by the decoder (currently only checksum
    /// mismatches with `validate: true`). Does NOT include encoding
    /// drops — those are counted separately in `build_read_result`.
    pub frames_dropped: usize,
    /// Framing/decode error text. `Some` when the read stopped with
    /// `stop_reason: framing_error`, else `None`. Surfaced as
    /// `ReadResult.error` by `build_read_result`.
    pub error: Option<String>,
    /// Absolute stream offset where this read's data starts.
    pub from_offset: Option<u64>,
    /// Cursor value after this read: `from_offset + bytes.len()`.
    pub next_offset: Option<u64>,
    /// Bytes lost to ring wrap since the cursor's original position.
    pub bytes_lost: u64,
    /// Unread bytes remaining in the ring after this read.
    pub buffered_remaining: u64,
}

/// `read`'s frame sink: collects every frame and records the first match so the
/// caller can return it. Always returns `Continue` — read includes frames
/// decoded after the matching one (legacy behavior).
struct ReadFrameSink<'a> {
    collected: &'a mut Vec<crate::framing::Frame>,
    match_data: Option<Vec<u8>>,
    match_index: Option<usize>,
    match_frame_index: Option<usize>,
}

impl<'a> ReadFrameSink<'a> {
    fn new(collected: &'a mut Vec<crate::framing::Frame>) -> Self {
        Self {
            collected,
            match_data: None,
            match_index: None,
            match_frame_index: None,
        }
    }
}

#[async_trait::async_trait]
impl RxFrameSink for ReadFrameSink<'_> {
    async fn on_frame(
        &mut self,
        frame: crate::framing::Frame,
        matched: bool,
        match_index: Option<usize>,
    ) -> SinkFlow {
        if matched && self.match_data.is_none() {
            self.match_data = Some(frame.data.clone());
            self.match_index = match_index;
            self.match_frame_index = Some(frame.index);
        }
        self.collected.push(frame);
        SinkFlow::Continue
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn read_bytes_via_session(
    mut event_rx: mpsc::Receiver<RxEvent>,
    max_bytes: usize,
    timeout_ms: Option<u64>,
    ct: &tokio_util::sync::CancellationToken,
    progress_token: Option<ProgressToken>,
    peer: Option<&Peer<RoleServer>>,
    mut matcher: Option<Matcher>,
    no_new_rx_timeout_ms: Option<u64>,
    conn: Option<Arc<crate::serial::SerialConnection>>,
    framing: Option<crate::framing::RxFramingConfig>,
    parser: Option<crate::framing::ParserConfig>,
) -> Result<ReadOutcome, String> {
    const SETTLE_MS: u64 = 50;

    let effective_timeout = timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    let read_start = Instant::now();
    let mut ctrl = RxStopController::new(read_start, timeout_ms, max_bytes, no_new_rx_timeout_ms);
    let deadline = ctrl
        .deadline()
        .unwrap_or_else(|| read_start + Duration::from_millis(effective_timeout));

    let mut last_progress = Instant::now();
    let mut accumulated: Vec<u8> = Vec::with_capacity(max_bytes);

    let context_amount = matcher.as_ref().and_then(|m| m.context_amount());
    let needle_len = matcher.as_ref().and_then(|m| m.needle_len());

    // Frame decoder state.
    let max_frames = framing.as_ref().and_then(|f| f.max_frames);
    let mut decoder: Option<crate::framing::FrameDecoder> = match framing.as_ref() {
        // read is a synchronous request/response: propagate decoder-init errors
        // to the caller. (subscribe degrades to raw mode instead — see stream_ops.rs.)
        Some(cfg) => Some(crate::framing::FrameDecoder::new(cfg, parser.as_ref())?),
        None => None,
    };
    let mut collected_frames: Vec<crate::framing::Frame> = Vec::new();
    let mut frames_seen: usize = 0;
    let mut frames_dropped: usize = 0;
    let make_outcome = |frames: Vec<crate::framing::Frame>,
                        bytes: Vec<u8>,
                        elapsed_ms: u64,
                        meta: RxStopMetadata,
                        matched: bool,
                        match_index: Option<usize>,
                        match_frame_index: Option<usize>,
                        fdropped: usize,
                        error: Option<String>| {
        let outcome = ReadOutcome {
            bytes,
            elapsed_ms,
            meta,
            matched,
            match_index,
            match_frame_index,
            frames,
            frames_dropped: fdropped,
            error,
            from_offset: None,
            next_offset: None,
            bytes_lost: 0,
            buffered_remaining: 0,
        };
        if !outcome.matched || context_amount.is_none() {
            return outcome;
        }
        let Some(match_idx) = outcome.match_index else {
            return outcome;
        };
        let Some(nlen) = needle_len else {
            return outcome;
        };
        let shaped = shape_match_context(&outcome.bytes, match_idx, nlen, context_amount);
        let bytes_returned = shaped.data.len();
        ReadOutcome {
            bytes: shaped.data,
            elapsed_ms: outcome.elapsed_ms,
            meta: RxStopMetadata::match_found(outcome.meta.bytes_observed, bytes_returned),
            matched: true,
            match_index: Some(shaped.match_index),
            match_frame_index: outcome.match_frame_index,
            frames: outcome.frames,
            frames_dropped: outcome.frames_dropped,
            error: outcome.error,
            from_offset: None,
            next_offset: None,
            bytes_lost: 0,
            buffered_remaining: 0,
        }
    };

    // Helper to flush any partial frame from the decoder and take collected frames.
    // Call at every return point to ensure incomplete frames aren't dropped.
    fn finalize_frames(
        decoder: &mut Option<crate::framing::FrameDecoder>,
        collected: &mut Vec<crate::framing::Frame>,
    ) -> Vec<crate::framing::Frame> {
        if let Some(ref mut dec) = *decoder {
            if let Some(partial) = dec.flush_partial() {
                collected.push(partial);
            }
        }
        std::mem::take(collected)
    }

    // Collapse the repeated "flush partial frames → build outcome → return"
    // tail. Captures `decoder`, `collected_frames`, `read_start`, and
    // `make_outcome` from the enclosing scope. Valid only in return position.
    macro_rules! finish {
        ($bytes:expr, $meta:expr, $matched:expr, $match_index:expr, $match_frame_index:expr, $dropped:expr, $error:expr) => {
            return Ok(make_outcome(
                finalize_frames(&mut decoder, &mut collected_frames),
                $bytes,
                read_start.elapsed().as_millis() as u64,
                $meta,
                $matched,
                $match_index,
                $match_frame_index,
                $dropped,
                $error,
            ))
        };
    }

    loop {
        // Pause timeouts while the connection is disconnected or reconnecting.
        // If reconnect is NOT enabled, exit with connection_closed so the
        // caller receives partial data and any buffered frames.
        if let Some(ref conn) = conn {
            match disconnect_state(conn, &mut ctrl) {
                DisconnectState::Closed => {
                    let outcome = ctrl.connection_closed();
                    finish!(
                        accumulated,
                        outcome.meta,
                        outcome.matched,
                        outcome.match_index,
                        None,
                        frames_dropped,
                        None
                    );
                }
                DisconnectState::Reconnecting => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
                DisconnectState::Active => {}
            }
        }

        if let RxStopDecision::Stop(outcome) = ctrl.check_timeout() {
            finish!(
                accumulated,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
                None,
                frames_dropped,
                None
            );
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            finish!(
                accumulated,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
                None,
                frames_dropped,
                None
            );
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        let remaining_ms = remaining.as_millis() as u64;
        let wait = remaining_ms.saturating_sub(1).clamp(1, 250);

        let event = tokio::select! {
            _ = ct.cancelled() => {
                let outcome = ctrl.cancelled();
                finish!(accumulated, outcome.meta, outcome.matched, outcome.match_index, None, frames_dropped, None);
            }
            msg = tokio::time::timeout(Duration::from_millis(wait), event_rx.recv()) => match msg {
                Ok(Some(e)) => e,
                Ok(None) => {
                    let outcome = ctrl.channel_closed();
                    finish!(accumulated, outcome.meta, outcome.matched, outcome.match_index, None, frames_dropped, None);
                }
                Err(_) => {
                    if let (Some(token), Some(peer)) = (progress_token.clone(), peer) {
                        if last_progress.elapsed() >= Duration::from_millis(250) {
                            last_progress = Instant::now();
                            let elapsed_ms = effective_timeout.saturating_sub(
                                deadline.saturating_duration_since(Instant::now()).as_millis() as u64
                            );
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
                    continue;
                }
            },
        };

        match event {
            RxEvent::Data(chunk) => {
                ctrl.notify_data_received();
                let chunk_len = chunk.len();
                let room = max_bytes.saturating_sub(accumulated.len());
                let take = chunk.len().min(room);
                accumulated.extend_from_slice(&chunk[..take]);

                // Feed to frame decoder via the shared consumer.
                if let Some(ref mut dec) = decoder {
                    let mut sink = ReadFrameSink::new(&mut collected_frames);
                    let outcome = consume_frames(
                        &chunk,
                        dec,
                        &mut matcher,
                        max_frames,
                        &mut frames_seen,
                        &mut sink,
                        &mut frames_dropped,
                    )
                    .await;
                    let ReadFrameSink {
                        match_data,
                        match_index,
                        match_frame_index,
                        ..
                    } = sink;
                    if let Some(data) = match_data {
                        let meta =
                            RxStopMetadata::match_found(ctrl.bytes_observed(), accumulated.len());
                        finish!(
                            data,
                            meta,
                            true,
                            match_index,
                            match_frame_index,
                            frames_dropped,
                            None
                        )
                    }
                    if let FrameOutcome::MaxFrames = outcome {
                        let meta =
                            RxStopMetadata::max_frames(ctrl.bytes_observed(), accumulated.len());
                        finish!(accumulated, meta, false, None, None, frames_dropped, None);
                    }
                    if let FrameOutcome::DecodeError(e) = outcome {
                        let meta = crate::rx_metadata::RxStopMetadata::framing_error(
                            ctrl.bytes_observed(),
                        );
                        tracing::error!("RX framing decode error: {e}");
                        let err_text = e.to_string();
                        finish!(
                            accumulated,
                            meta,
                            false,
                            None,
                            None,
                            frames_dropped,
                            Some(err_text)
                        );
                    }
                }

                // When framing is NOT active, match on raw chunk bytes.
                if decoder.is_none() {
                    let match_result = matcher.as_mut().map(|m| m.push(&chunk[..take]));
                    let buffered_len = accumulated.len();

                    if let RxStopDecision::Stop(outcome) =
                        ctrl.push_data(chunk_len, buffered_len, match_result)
                    {
                        finish!(
                            accumulated,
                            outcome.meta,
                            outcome.matched,
                            outcome.match_index,
                            None,
                            frames_dropped,
                            None
                        );
                    }
                }

                // Without a matcher or framing, first-byte-then-settle semantics.
                if matcher.is_none() && decoder.is_none() {
                    break;
                }
                // With a matcher or framing, push_data already checked max_buffered_bytes.
                // Loop continues to accumulate until match/frames/timeout/close.
                // Prune the matcher's internal window to prevent unbounded growth.
                if let Some(m) = matcher.as_mut() {
                    let keep = m
                        .needle_len()
                        .map(|n| n.max(1).saturating_add(1))
                        .unwrap_or(256);
                    let cap = max_bytes.max(keep);
                    if m.len() > cap {
                        m.truncate_front(cap);
                    }
                }
            }
            RxEvent::Closed => {
                let outcome = ctrl.connection_closed();
                finish!(
                    accumulated,
                    outcome.meta,
                    outcome.matched,
                    outcome.match_index,
                    None,
                    frames_dropped,
                    None
                );
            }
            RxEvent::Error(msg) => {
                return Err(log_tool_err("read", "Data reading failed", msg));
            }
        }
    }

    // Settle phase: plain-read burst gather. Reached only when neither a matcher
    // nor framing is active (the sole `break` above requires both to be None).
    debug_assert!(
        decoder.is_none() && matcher.is_none(),
        "settle phase reached with active decoder/matcher"
    );
    while accumulated.len() < max_bytes {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        let settle = remaining.min(SETTLE_MS);
        if settle == 0 {
            break;
        }

        if let RxStopDecision::Stop(outcome) = ctrl.check_timeout() {
            finish!(
                accumulated,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
                None,
                frames_dropped,
                None
            );
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            finish!(
                accumulated,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
                None,
                frames_dropped,
                None
            );
        }

        let event = tokio::select! {
            _ = ct.cancelled() => {
                let outcome = ctrl.cancelled();
                finish!(accumulated, outcome.meta, outcome.matched, outcome.match_index, None, frames_dropped, None);
            }
            msg = tokio::time::timeout(Duration::from_millis(settle), event_rx.recv()) => match msg {
                Ok(Some(e)) => Some(e),
                Ok(None) | Err(_) => None,
            },
        };
        match event {
            Some(RxEvent::Data(chunk)) => {
                ctrl.notify_data_received();
                ctrl.record_data(chunk.len(), accumulated.len());
                let room = max_bytes.saturating_sub(accumulated.len());
                let take = chunk.len().min(room);
                accumulated.extend_from_slice(&chunk[..take]);
                ctrl.record_data(0, accumulated.len());

                if let (Some(token), Some(peer)) = (progress_token.clone(), peer) {
                    let elapsed_ms = effective_timeout.saturating_sub(
                        deadline
                            .saturating_duration_since(Instant::now())
                            .as_millis() as u64,
                    );
                    if last_progress.elapsed() >= Duration::from_millis(250) {
                        last_progress = Instant::now();
                        let _ = peer
                            .notify_progress(ProgressNotificationParam {
                                progress_token: token,
                                progress: elapsed_ms as f64,
                                total: Some(effective_timeout as f64),
                                message: Some(format!("read {} bytes", accumulated.len())),
                            })
                            .await;
                    }
                }
            }
            Some(RxEvent::Closed) => break,
            Some(RxEvent::Error(msg)) => {
                return Err(log_tool_err("read", "Data reading failed", msg))
            }
            None => break,
        }
    }

    // After settle phase, determine the stop reason.
    // If we filled the buffer during settle, it's MaxBufferedBytes;
    // otherwise it's DataComplete.
    if let RxStopDecision::Stop(outcome) = ctrl.check_max_buffered_bytes() {
        finish!(
            accumulated,
            outcome.meta,
            outcome.matched,
            outcome.match_index,
            None,
            frames_dropped,
            None
        );
    }
    let outcome = ctrl.data_complete();
    finish!(
        accumulated,
        outcome.meta,
        outcome.matched,
        outcome.match_index,
        None,
        frames_dropped,
        None
    );
}

/// Re-export of the shared [`crate::util::find_subsequence`] under the
/// legacy `find_subslice` name to keep existing import paths stable.
pub(crate) use crate::util::find_subsequence as find_subslice;

// ------------------------------------------------------------------
// Ring-based read (Phase 1.3)
// ------------------------------------------------------------------

/// Drive a `read` from the ring buffer, with cat semantics: buffered-but-
/// unread bytes are returned immediately. Pattern matching checks history
/// first, then waits for new bytes. Peek never advances the cursor.
#[allow(clippy::too_many_arguments)]
pub async fn read_bytes_from_ring(
    session: Arc<RxSession>,
    max_bytes: usize,
    timeout_ms: Option<u64>,
    ct: &tokio_util::sync::CancellationToken,
    _progress_token: Option<ProgressToken>,
    _peer: Option<&Peer<RoleServer>>,
    mut matcher: Option<Matcher>,
    no_new_rx_timeout_ms: Option<u64>,
    conn: Option<Arc<SerialConnection>>,
    framing: Option<crate::framing::RxFramingConfig>,
    parser: Option<crate::framing::ParserConfig>,
    peek: bool,
) -> Result<ReadOutcome, String> {
    let effective_timeout_ms = timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    let read_start = Instant::now();
    let ring = session.ring();
    let cursor = session.read_cursor();

    let initial_slice = ring.read_from(cursor, max_bytes);

    let mut ctrl = RxStopController::new(read_start, timeout_ms, max_bytes, no_new_rx_timeout_ms);

    let context_amount = matcher.as_ref().and_then(|m| m.context_amount());
    let needle_len = matcher.as_ref().and_then(|m| m.needle_len());

    // Frame decoder state.
    let max_frames = framing.as_ref().and_then(|f| f.max_frames);
    let mut decoder: Option<crate::framing::FrameDecoder> = match framing.as_ref() {
        Some(cfg) => Some(crate::framing::FrameDecoder::new(cfg, parser.as_ref())?),
        None => None,
    };
    let mut collected_frames: Vec<crate::framing::Frame> = Vec::new();
    let mut frames_seen: usize = 0;
    let mut frames_dropped: usize = 0;

    // We track how many raw bytes we consumed from the ring (for cursor advancement)
    // and how many we return in the result.
    let mut consumed_offset: u64 = 0; // raw bytes consumed from ring cursor
    let mut returned_bytes: Vec<u8> = Vec::with_capacity(max_bytes);

    // Helper: build the ReadOutcome from current state.
    // Advances cursor unless `peek` is true.
    let make_read_outcome = |returned_bytes: Vec<u8>,
                             consumed_offset: u64,
                             _ctrl: &RxStopController,
                             elapsed_ms: u64,
                             meta: RxStopMetadata,
                             matched: bool,
                             match_index: Option<usize>,
                             match_frame_index: Option<usize>,
                             frames: Vec<crate::framing::Frame>,
                             frames_dropped: usize,
                             error: Option<String>,
                             ring: &crate::rx_ring::RxRing,
                             cursor: u64,
                             peek: bool|
     -> ReadOutcome {
        let start_off = ring.start_offset();
        let end_off = ring.end_offset();
        let clamped_from = cursor.max(start_off).min(end_off);
        let bytes_lost = start_off.saturating_sub(cursor);
        let used = consumed_offset.min(max_bytes as u64);
        let next_off = if peek {
            clamped_from
        } else {
            clamped_from + used
        };
        let from_off = if returned_bytes.is_empty() && consumed_offset == 0 {
            None
        } else {
            Some(clamped_from)
        };
        let next_off_out = if returned_bytes.is_empty() && consumed_offset == 0 {
            None
        } else {
            Some(next_off)
        };
        let buffered_remaining = end_off.saturating_sub(next_off);
        ReadOutcome {
            bytes: returned_bytes,
            elapsed_ms,
            meta,
            matched,
            match_index,
            match_frame_index,
            frames,
            frames_dropped,
            error,
            from_offset: from_off,
            next_offset: next_off_out,
            bytes_lost,
            buffered_remaining,
        }
    };

    // Check if we have immediate cat-path data (no match, bytes available).
    let has_immediate_data = !initial_slice.bytes.is_empty() && matcher.is_none();

    if has_immediate_data && decoder.is_none() {
        // Cat path: return buffered bytes immediately.
        let take = initial_slice.bytes.len().min(max_bytes);
        let data: Vec<u8> = initial_slice.bytes[..take].to_vec();
        let consumed = data.len() as u64;
        let meta = RxStopMetadata::drained(cursor + consumed, consumed as usize, consumed as usize);
        if !peek {
            session.set_read_cursor(
                initial_slice
                    .from_offset
                    .wrapping_add(consumed)
                    .min(ring.end_offset()),
            );
        }
        return Ok(make_read_outcome(
            data,
            consumed,
            &ctrl,
            read_start.elapsed().as_millis() as u64,
            meta,
            false,
            None,
            None,
            Vec::new(),
            0,
            None,
            ring,
            cursor,
            peek,
        ));
    }

    // Match-check history first if a matcher is present.
    if matcher.is_some() && !initial_slice.bytes.is_empty() {
        let take = initial_slice.bytes.len().min(max_bytes);
        let hist = &initial_slice.bytes[..take];
        let match_result = matcher.as_mut().map(|m| m.push(hist));
        if let Some(MatchResult::Found(idx)) = match_result {
            let match_end = idx + needle_len.unwrap_or(0);
            let consumed = match_end as u64;
            let data = hist[..match_end].to_vec();
            let meta = RxStopMetadata::match_found(consumed as usize, consumed as usize);
            if !peek {
                session.set_read_cursor(
                    initial_slice
                        .from_offset
                        .wrapping_add(consumed)
                        .min(ring.end_offset()),
                );
            }
            // Handle context shaping
            if let Some(context) = context_amount {
                let shaped = shape_match_context(hist, idx, needle_len.unwrap_or(0), Some(context));
                let shaped_consumed = shaped.data.len() as u64;
                return Ok(ReadOutcome {
                    bytes: shaped.data,
                    elapsed_ms: read_start.elapsed().as_millis() as u64,
                    meta: RxStopMetadata::match_found(consumed as usize, shaped_consumed as usize),
                    matched: true,
                    match_index: Some(shaped.match_index),
                    match_frame_index: None,
                    frames: Vec::new(),
                    frames_dropped: 0,
                    error: None,
                    from_offset: Some(initial_slice.from_offset),
                    next_offset: Some(if peek {
                        initial_slice.from_offset
                    } else {
                        initial_slice.from_offset + consumed
                    }),
                    bytes_lost: initial_slice.bytes_lost,
                    buffered_remaining: ring.end_offset().saturating_sub(if peek {
                        initial_slice.from_offset
                    } else {
                        initial_slice.from_offset + consumed
                    }),
                });
            }
            return Ok(make_read_outcome(
                data,
                consumed,
                &ctrl,
                read_start.elapsed().as_millis() as u64,
                meta,
                true,
                Some(idx),
                None,
                Vec::new(),
                0,
                None,
                ring,
                cursor,
                peek,
            ));
        }
        // Not found in history — consume what we read from the ring for the result so far.
        consumed_offset = take as u64;
        returned_bytes = hist.to_vec();
        ctrl.notify_data_received();
        ctrl.push_data(take, take, Some(MatchResult::NoMatch));
    }

    // Process initial bytes through frame decoder if active.
    if decoder.is_some() && !initial_slice.bytes.is_empty() {
        let take = initial_slice.bytes.len().min(max_bytes);
        let chunk = &initial_slice.bytes[..take];
        consumed_offset += take as u64;
        returned_bytes.extend_from_slice(chunk);

        if let Some(ref mut dec) = decoder {
            let mut sink = ReadFrameSink::new(&mut collected_frames);
            let outcome = consume_frames(
                chunk,
                dec,
                &mut matcher,
                max_frames,
                &mut frames_seen,
                &mut sink,
                &mut frames_dropped,
            )
            .await;
            let ReadFrameSink {
                match_data,
                match_index,
                match_frame_index,
                ..
            } = sink;
            if let Some(data) = match_data {
                let meta = RxStopMetadata::match_found(ctrl.bytes_observed(), returned_bytes.len());
                if !peek {
                    session.set_read_cursor(
                        initial_slice
                            .from_offset
                            .wrapping_add(consumed_offset)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    data,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    meta,
                    true,
                    match_index,
                    match_frame_index,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    None,
                    ring,
                    cursor,
                    peek,
                ));
            }
            if let FrameOutcome::MaxFrames = outcome {
                let meta = RxStopMetadata::max_frames(ctrl.bytes_observed(), returned_bytes.len());
                if !peek {
                    session.set_read_cursor(
                        initial_slice
                            .from_offset
                            .wrapping_add(consumed_offset)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    meta,
                    false,
                    None,
                    None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    None,
                    ring,
                    cursor,
                    peek,
                ));
            }
            if let FrameOutcome::DecodeError(e) = outcome {
                let meta = crate::rx_metadata::RxStopMetadata::framing_error(ctrl.bytes_observed());
                tracing::error!("RX framing decode error: {e}");
                let err_text = e.to_string();
                if !peek {
                    session.set_read_cursor(
                        initial_slice
                            .from_offset
                            .wrapping_add(consumed_offset)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    meta,
                    false,
                    None,
                    None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    Some(err_text),
                    ring,
                    cursor,
                    peek,
                ));
            }
            if let FrameOutcome::SinkStop(reason) = outcome {
                let meta =
                    RxStopMetadata::new(reason, ctrl.bytes_observed(), returned_bytes.len(), false);
                if !peek {
                    session.set_read_cursor(
                        initial_slice
                            .from_offset
                            .wrapping_add(consumed_offset)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    meta,
                    false,
                    None,
                    None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    None,
                    ring,
                    cursor,
                    peek,
                ));
            }
        }
    }

    // Wait loop: wait for new data, then process.
    let mut clocked_cursor = initial_slice.next_offset;
    loop {
        // Pause timeouts while connection is disconnected/reconnecting.
        if let Some(ref conn) = conn {
            match disconnect_state(conn, &mut ctrl) {
                DisconnectState::Closed => {
                    let outcome = ctrl.connection_closed();
                    if !peek {
                        session.set_read_cursor(
                            cursor.wrapping_add(consumed_offset).min(ring.end_offset()),
                        );
                    }
                    return Ok(make_read_outcome(
                        returned_bytes,
                        consumed_offset,
                        &ctrl,
                        read_start.elapsed().as_millis() as u64,
                        outcome.meta,
                        outcome.matched,
                        outcome.match_index,
                        None,
                        std::mem::take(&mut collected_frames),
                        frames_dropped,
                        None,
                        ring,
                        cursor,
                        peek,
                    ));
                }
                DisconnectState::Reconnecting => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
                DisconnectState::Active => {}
            }
        }

        if let RxStopDecision::Stop(outcome) = ctrl.check_timeout() {
            if !peek {
                session
                    .set_read_cursor(cursor.wrapping_add(consumed_offset).min(ring.end_offset()));
            }
            return Ok(make_read_outcome(
                returned_bytes,
                consumed_offset,
                &ctrl,
                read_start.elapsed().as_millis() as u64,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
                None,
                std::mem::take(&mut collected_frames),
                frames_dropped,
                None,
                ring,
                cursor,
                peek,
            ));
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            if !peek {
                session
                    .set_read_cursor(cursor.wrapping_add(consumed_offset).min(ring.end_offset()));
            }
            return Ok(make_read_outcome(
                returned_bytes,
                consumed_offset,
                &ctrl,
                read_start.elapsed().as_millis() as u64,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
                None,
                std::mem::take(&mut collected_frames),
                frames_dropped,
                None,
                ring,
                cursor,
                peek,
            ));
        }

        // Wait for more data on the ring, or a short poll to check timeouts.
        let deadline = ctrl
            .deadline()
            .unwrap_or_else(|| read_start + Duration::from_millis(effective_timeout_ms));
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        let poll_ms = remaining.saturating_sub(1).clamp(1, 250); // adaptive: 1-250ms
        tokio::select! {
            _ = ct.cancelled() => {
                let outcome = ctrl.cancelled();
                if !peek {
                    session.set_read_cursor(
                        cursor.wrapping_add(consumed_offset).min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    returned_bytes, consumed_offset, &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    outcome.meta, outcome.matched, outcome.match_index, None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped, None, ring, cursor, peek,
                ));
            }
            _ = ring.wait_for_data(clocked_cursor) => {}
            _ = tokio::time::sleep(std::time::Duration::from_millis(poll_ms)) => {
                // Poll wakeup: loop back to check timeouts and stop conditions.
                continue;
            }
        }

        // New data arrived — read from ring at the clocked cursor.
        let slice = ring.read_from(
            clocked_cursor,
            max_bytes.saturating_sub(returned_bytes.len()),
        );
        if slice.bytes.is_empty() {
            continue; // spurious wakeup
        }

        ctrl.notify_data_received();
        let take = slice
            .bytes
            .len()
            .min(max_bytes.saturating_sub(returned_bytes.len()));
        let chunk = &slice.bytes[..take];
        returned_bytes.extend_from_slice(chunk);
        consumed_offset = consumed_offset
            .wrapping_add(take as u64)
            .min(max_bytes as u64);

        // Feed to frame decoder if active.
        if let Some(ref mut dec) = decoder {
            let mut sink = ReadFrameSink::new(&mut collected_frames);
            let outcome = consume_frames(
                chunk,
                dec,
                &mut matcher,
                max_frames,
                &mut frames_seen,
                &mut sink,
                &mut frames_dropped,
            )
            .await;
            let ReadFrameSink {
                match_data,
                match_index,
                match_frame_index,
                ..
            } = sink;
            if let Some(data) = match_data {
                let meta = RxStopMetadata::match_found(ctrl.bytes_observed(), returned_bytes.len());
                if !peek {
                    session.set_read_cursor(
                        slice
                            .from_offset
                            .wrapping_add(take as u64)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    data,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    meta,
                    true,
                    match_index,
                    match_frame_index,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    None,
                    ring,
                    cursor,
                    peek,
                ));
            }
            if let FrameOutcome::MaxFrames = outcome {
                let meta = RxStopMetadata::max_frames(ctrl.bytes_observed(), returned_bytes.len());
                if !peek {
                    session.set_read_cursor(
                        slice
                            .from_offset
                            .wrapping_add(take as u64)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    meta,
                    false,
                    None,
                    None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    None,
                    ring,
                    cursor,
                    peek,
                ));
            }
            if let FrameOutcome::DecodeError(e) = outcome {
                let meta = crate::rx_metadata::RxStopMetadata::framing_error(ctrl.bytes_observed());
                tracing::error!("RX framing decode error: {e}");
                let err_text = e.to_string();
                if !peek {
                    session.set_read_cursor(
                        slice
                            .from_offset
                            .wrapping_add(take as u64)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    meta,
                    false,
                    None,
                    None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    Some(err_text),
                    ring,
                    cursor,
                    peek,
                ));
            }
            if let FrameOutcome::SinkStop(reason) = outcome {
                let meta =
                    RxStopMetadata::new(reason, ctrl.bytes_observed(), returned_bytes.len(), false);
                if !peek {
                    session.set_read_cursor(
                        slice
                            .from_offset
                            .wrapping_add(take as u64)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    meta,
                    false,
                    None,
                    None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    None,
                    ring,
                    cursor,
                    peek,
                ));
            }
        }

        // Raw matcher path (no framing).
        if decoder.is_none() {
            let match_result = matcher.as_mut().map(|m| m.push(chunk));
            let buffered_len = returned_bytes.len();
            let data_count = chunk.len();
            if let RxStopDecision::Stop(outcome) =
                ctrl.push_data(data_count, buffered_len, match_result)
            {
                if !peek {
                    session.set_read_cursor(
                        slice
                            .from_offset
                            .wrapping_add(take as u64)
                            .min(ring.end_offset()),
                    );
                }
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    outcome.meta,
                    outcome.matched,
                    outcome.match_index,
                    None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    None,
                    ring,
                    cursor,
                    peek,
                ));
            }
        }

        // Update clocked cursor for next iteration.
        clocked_cursor = slice.next_offset;

        // max_bytes reached -> drained
        if returned_bytes.len() >= max_bytes {
            let meta = RxStopMetadata::drained(
                cursor.wrapping_add(consumed_offset),
                returned_bytes.len(),
                returned_bytes.len(),
            );
            if !peek {
                session
                    .set_read_cursor(cursor.wrapping_add(consumed_offset).min(ring.end_offset()));
            }
            return Ok(make_read_outcome(
                returned_bytes,
                consumed_offset,
                &ctrl,
                read_start.elapsed().as_millis() as u64,
                meta,
                false,
                None,
                None,
                std::mem::take(&mut collected_frames),
                frames_dropped,
                None,
                ring,
                cursor,
                peek,
            ));
        }
    }
}

// ------------------------------------------------------------------
// Result builders
// ------------------------------------------------------------------

pub fn build_read_result(
    outcome: ReadOutcome,
    connection_id: String,
    name: Option<String>,
    encoding: Encoding,
    requested_timeout_ms: Option<u64>,
    requested_no_new_rx_timeout_ms: Option<u64>,
) -> Result<Json<ReadResult>, String> {
    let timeout_ms = requested_timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    let bytes_read = outcome.bytes.len();
    let elapsed_ms = outcome.elapsed_ms;

    let is_framing_error = outcome.error.is_some();
    let (data, effective_encoding) = match codec::encode(encoding, &outcome.bytes) {
        Ok(s) => (s, encoding),
        Err(e) if is_framing_error => {
            // Framing-error path: fall back to hex so the partial bytes and
            // the framing diagnostic both survive. Lossy UTF-8 was rejected
            // (corrupts bytes); base64 would also work but hex matches the
            // binary-protocol context that produced the framing error.
            tracing::warn!(
                "read framing-error data not encodable as {encoding} ({e}); \
                 falling back to hex"
            );
            let hex = codec::encode(Encoding::Hex, &outcome.bytes)
                .map_err(|e| format!("hex fallback encoding failed - {e}"))?;
            (hex, Encoding::Hex)
        }
        Err(e) => return Err(format!("Data encoding failed - {e}")),
    };

    let mut frames_dropped: usize = outcome.frames_dropped;
    let frames = if outcome.frames.is_empty() {
        None
    } else {
        let encoded_frames: Vec<FrameResult> = outcome
            .frames
            .iter()
            .filter_map(|f| {
                let encode = |enc: Encoding, data: &[u8]| -> Option<FrameResult> {
                    match codec::encode(enc, data) {
                        Ok(fdata) => Some(FrameResult {
                            data: fdata,
                            encoding: enc.to_string(),
                            frame_index: f.index,
                            frame_type: f.frame_type.to_string(),
                            parsed: f.parsed.clone(),
                        }),
                        Err(err) => {
                            tracing::warn!("Frame {} encoding failed: {err}", f.index);
                            None
                        }
                    }
                };
                // Try the effective encoding first.
                let mut frame = encode(effective_encoding, &f.data);
                // On framing error, fall back to hex per-frame so partial
                // frames survive alongside the raw bytes.
                if frame.is_none() && is_framing_error && effective_encoding != Encoding::Hex {
                    frame = encode(Encoding::Hex, &f.data);
                }
                if frame.is_none() {
                    frames_dropped += 1;
                }
                frame
            })
            .collect();
        if encoded_frames.is_empty() {
            None
        } else {
            Some(encoded_frames)
        }
    };

    Ok(Json(ReadResult {
        connection_id,
        name,
        bytes_read,
        encoding: effective_encoding.to_string(),
        data,
        timeout_ms,
        no_new_rx_timeout_ms: requested_no_new_rx_timeout_ms,
        elapsed_ms,
        stop_reason: outcome.meta.stop_reason.to_string(),
        truncated: outcome.meta.truncated,
        bytes_observed: outcome.meta.bytes_observed,
        bytes_returned: outcome.meta.bytes_returned,
        matched: outcome.matched,
        match_index: outcome.match_index,
        match_frame_index: outcome.match_frame_index,
        frames,
        frames_dropped,
        error: outcome.error,
        from_offset: outcome.from_offset,
        next_offset: outcome.next_offset,
        bytes_lost: outcome.bytes_lost,
        buffered_remaining: outcome.buffered_remaining,
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
        name: args.name,
        baud_rate: args.baud_rate,
        data_bits: args.data_bits.parse()?,
        stop_bits: args.stop_bits.parse()?,
        parity: args.parity.parse()?,
        flow_control: args.flow_control.parse()?,
        port_info: None,
        log_capacity: args.log_capacity,
        log_enabled: args.log_enabled,
        tx_framing: args.tx_framing,
        rx_framing: args.rx_framing,
        rx_parser: args.rx_parser,
        protocol: args.protocol,
        rx_buffer_size: args.rx_buffer_size,
    })
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
    use crate::rx_metadata::RxStopReason;
    use crate::rx_session::RxEvent;
    use tokio::sync::mpsc;

    #[test]
    fn open_args_parsed_strictly() {
        let args = OpenArgs {
            port: "/dev/ttyUSB0".into(),
            name: Some("console".into()),
            baud_rate: 115200,
            data_bits: "8".into(),
            stop_bits: "1".into(),
            parity: "none".into(),
            flow_control: "none".into(),
            log_capacity: 1024,
            log_enabled: true,
            reconnect_policy: Default::default(),
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: crate::limits::DEFAULT_RX_BUFFER_SIZE,
        };
        let config = parse_open_args(args).unwrap();
        assert_eq!(config.port, "/dev/ttyUSB0");
        assert_eq!(config.name.as_deref(), Some("console"));
        assert_eq!(config.baud_rate, 115200);
    }

    #[test]
    fn open_args_reject_invalid_data_bits() {
        let args = OpenArgs {
            port: "X".into(),
            name: None,
            baud_rate: 9600,
            data_bits: "9".into(),
            stop_bits: "1".into(),
            parity: "none".into(),
            flow_control: "none".into(),
            log_capacity: 1024,
            log_enabled: true,
            reconnect_policy: Default::default(),
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: crate::limits::DEFAULT_RX_BUFFER_SIZE,
        };
        let err = parse_open_args(args).unwrap_err();
        assert!(err.contains("data_bits"));
    }

    #[test]
    fn open_args_reject_invalid_parity() {
        use crate::serial::Parity;
        assert!("weird".parse::<Parity>().is_err());
        assert!("none".parse::<Parity>().is_ok());
        assert!("Even".parse::<Parity>().is_ok());
    }

    #[test]
    fn parse_encoding_rejects_garbage() {
        assert!(parse_encoding("rot13").is_err());
        assert!(parse_encoding("utf-8").is_ok());
    }

    #[test]
    fn build_read_result_timeout_returns_success_with_stop_reason() {
        let outcome = ReadOutcome {
            bytes: Vec::new(),
            elapsed_ms: 250,
            meta: RxStopMetadata::timeout(0),
            matched: false,
            match_index: None,
            match_frame_index: None,
            frames: vec![],
            frames_dropped: 0,
            error: None,
            from_offset: None,
            next_offset: None,
            bytes_lost: 0,
            buffered_remaining: 0,
        };
        let Json(result) =
            build_read_result(outcome, "abc".into(), None, Encoding::Utf8, Some(250), None)
                .expect("timeout must return Ok");
        assert_eq!(result.stop_reason, "timeout");
        assert_eq!(result.bytes_read, 0);
        assert!(!result.matched);
        assert!(result.match_index.is_none());
    }

    #[test]
    fn build_read_result_timeout_uses_default_timeout_ms() {
        let outcome = ReadOutcome {
            bytes: Vec::new(),
            elapsed_ms: DEFAULT_READ_TIMEOUT_MS,
            meta: RxStopMetadata::timeout(0),
            matched: false,
            match_index: None,
            match_frame_index: None,
            frames: vec![],
            frames_dropped: 0,
            error: None,
            from_offset: None,
            next_offset: None,
            bytes_lost: 0,
            buffered_remaining: 0,
        };
        let Json(result) =
            build_read_result(outcome, "abc".into(), None, Encoding::Hex, None, None)
                .expect("timeout must return Ok");
        assert_eq!(result.timeout_ms, DEFAULT_READ_TIMEOUT_MS);
        assert_eq!(result.stop_reason, "timeout");
    }

    #[test]
    fn build_read_result_data_branch_encodes_hex() {
        let outcome = ReadOutcome {
            bytes: b"Hi".to_vec(),
            elapsed_ms: 42,
            meta: RxStopMetadata::data_complete(2, 2),
            matched: false,
            match_index: None,
            match_frame_index: None,
            frames: vec![],
            frames_dropped: 0,
            error: None,
            from_offset: None,
            next_offset: None,
            bytes_lost: 0,
            buffered_remaining: 0,
        };
        let Json(result) =
            build_read_result(outcome, "abc".into(), None, Encoding::Hex, Some(500), None)
                .expect("data result must build");
        assert_eq!(result.bytes_read, 2);
        assert_eq!(result.encoding, "hex");
        assert_eq!(result.data, "48 69");
        assert_eq!(result.elapsed_ms, 42);
        assert_eq!(result.stop_reason, "data_complete");
        assert!(!result.truncated);
        assert_eq!(result.bytes_observed, 2);
        assert_eq!(result.bytes_returned, 2);
        assert!(!result.matched);
        assert!(result.match_index.is_none());
    }

    #[test]
    fn build_read_result_data_branch_includes_name() {
        let outcome = ReadOutcome {
            bytes: b"Hi".to_vec(),
            elapsed_ms: 42,
            meta: RxStopMetadata::data_complete(2, 2),
            matched: false,
            match_index: None,
            match_frame_index: None,
            frames: vec![],
            frames_dropped: 0,
            error: None,
            from_offset: None,
            next_offset: None,
            bytes_lost: 0,
            buffered_remaining: 0,
        };
        let Json(result) = build_read_result(
            outcome,
            "abc".into(),
            Some("console".into()),
            Encoding::Hex,
            Some(500),
            None,
        )
        .expect("data result must build");
        assert_eq!(result.name.as_deref(), Some("console"));
    }

    #[test]
    fn build_read_result_match_fields_populated() {
        let outcome = ReadOutcome {
            bytes: b"hello OK> world".to_vec(),
            elapsed_ms: 100,
            meta: RxStopMetadata::match_found(16, 16),
            matched: true,
            match_index: Some(6),
            match_frame_index: None,
            frames: vec![],
            frames_dropped: 0,
            error: None,
            from_offset: None,
            next_offset: None,
            bytes_lost: 0,
            buffered_remaining: 0,
        };
        let Json(result) = build_read_result(
            outcome,
            "conn".into(),
            None,
            Encoding::Utf8,
            Some(1000),
            None,
        )
        .expect("matched read result must build");
        assert!(result.matched);
        assert_eq!(result.match_index, Some(6));
        assert_eq!(result.stop_reason, "match_found");
    }

    #[test]
    fn clamp_or_err_rejects_oversized_values() {
        assert!(clamp_or_err("test.max_bytes", 1024 * 1024, MAX_READ_BYTES).is_ok());
        assert!(clamp_or_err("test.max_bytes", 1024 * 1024 + 1, MAX_READ_BYTES).is_err());
        assert!(clamp_or_err("test.max_bytes", usize::MAX, MAX_WRITE_BYTES).is_err());
    }

    #[test]
    fn require_min_or_err_rejects_undersized_values() {
        assert!(require_min_or_err("test.max_bytes", 1, MIN_READ_BYTES).is_ok());
        assert!(require_min_or_err("test.max_bytes", 0, MIN_READ_BYTES).is_err());
    }

    #[test]
    fn clamp_timeout_or_err_rejects_oversized_timeout() {
        assert!(clamp_timeout_or_err("test.timeout_ms", 1000, MAX_TIMEOUT_MS).is_ok());
        assert!(
            clamp_timeout_or_err("test.timeout_ms", MAX_TIMEOUT_MS + 1, MAX_TIMEOUT_MS).is_err()
        );
    }

    #[test]
    fn clamp_poll_interval_or_err_rejects_undersized_interval() {
        assert!(clamp_poll_interval_or_err("test.poll_ms", 10, MIN_POLL_INTERVAL_MS).is_ok());
        assert!(clamp_poll_interval_or_err("test.poll_ms", 9, MIN_POLL_INTERVAL_MS).is_err());
        assert!(clamp_poll_interval_or_err("test.poll_ms", 0, MIN_POLL_INTERVAL_MS).is_err());
    }

    #[test]
    fn shape_match_context_at_offset_zero_with_context() {
        let shaped = crate::match_config::shape_match_context(b"OK>rest", 0, 3, Some(128));
        assert_eq!(shaped.data, b"OK>");
        assert_eq!(shaped.match_index, 0);
    }

    #[test]
    fn shape_match_context_larger_than_pre_match() {
        let shaped = crate::match_config::shape_match_context(b"ABOK>x", 2, 3, Some(100));
        assert_eq!(shaped.data, b"ABOK>");
        assert_eq!(shaped.match_index, 2);
    }

    #[test]
    fn shape_match_context_exact_pre_match() {
        let shaped = crate::match_config::shape_match_context(b"XXOK>", 2, 3, Some(2));
        assert_eq!(shaped.data, b"XXOK>");
        assert_eq!(shaped.match_index, 2);
    }

    #[test]
    fn shape_match_context_truncates_post_match() {
        let shaped = crate::match_config::shape_match_context(b"preOK>post123", 3, 3, Some(3));
        // pre_start=0, match_end=6, shaped="preOK>" (6 bytes)
        assert_eq!(shaped.data, b"preOK>");
        assert_eq!(shaped.match_index, 3);
    }

    // ── Framing validation: invalid configs reject early ──────────────────

    fn make_closed_rx() -> tokio::sync::mpsc::Receiver<RxEvent> {
        let (_, rx) = tokio::sync::mpsc::channel::<RxEvent>(1);
        rx // sender dropped → channel closed
    }

    #[tokio::test]
    async fn read_via_session_rejects_empty_delimiter() {
        let rx = make_closed_rx();
        let ct = tokio_util::sync::CancellationToken::new();
        let framing = Some(crate::framing::RxFramingConfig {
            mode: crate::framing::RxFramingMode::Delimiter {
                delimiter: "".into(),
                delimiter_encoding: crate::match_config::PatternEncoding::Utf8,
            },
            ..Default::default()
        });
        let result = read_bytes_via_session(
            rx, 128, None, &ct, None, None, None, None, None, framing, None,
        )
        .await;
        match result {
            Ok(_) => panic!("empty delimiter should be rejected"),
            Err(err) => assert!(err.contains("Delimiter must not be empty"), "got: {err}"),
        }
    }

    #[tokio::test]
    async fn read_via_session_rejects_invalid_prefix_size() {
        let rx = make_closed_rx();
        let ct = tokio_util::sync::CancellationToken::new();
        let framing = Some(crate::framing::RxFramingConfig {
            mode: crate::framing::RxFramingMode::LengthPrefixed {
                prefix_size: 3,
                endianness: crate::framing::Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        });
        let result = read_bytes_via_session(
            rx, 128, None, &ct, None, None, None, None, None, framing, None,
        )
        .await;
        match result {
            Ok(_) => panic!("prefix_size=3 should be rejected"),
            Err(err) => assert!(err.contains("prefix_size must be 1, 2, or 4"), "got: {err}"),
        }
    }

    #[tokio::test]
    async fn read_via_session_rejects_empty_markers() {
        let rx = make_closed_rx();
        let ct = tokio_util::sync::CancellationToken::new();
        let framing = Some(crate::framing::RxFramingConfig {
            mode: crate::framing::RxFramingMode::StartEnd {
                start: vec!["".into()],
                end: "X".into(),
                marker_encoding: crate::match_config::PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        });
        let result = read_bytes_via_session(
            rx, 128, None, &ct, None, None, None, None, None, framing, None,
        )
        .await;
        match result {
            Ok(_) => panic!("empty markers should be rejected"),
            Err(err) => assert!(
                err.contains("Start and end markers must not be empty"),
                "got: {err}"
            ),
        }
    }

    #[tokio::test]
    async fn read_via_session_rejects_invalid_regex() {
        let rx = make_closed_rx();
        let ct = tokio_util::sync::CancellationToken::new();
        let framing = Some(crate::framing::RxFramingConfig {
            mode: crate::framing::RxFramingMode::Line {
                ending: crate::framing::LineEnding::Auto,
            },
            ..Default::default()
        });
        let parser = Some(crate::framing::ParserConfig {
            parser_type: crate::framing::ParserType::ShellPrompt,
            custom_prompt: Some("[invalid".to_string()),
            validate: false,
        });
        let result = read_bytes_via_session(
            rx, 128, None, &ct, None, None, None, None, None, framing, parser,
        )
        .await;
        match result {
            Ok(_) => panic!("invalid regex should be rejected"),
            Err(err) => assert!(err.contains("Invalid prompt regex"), "got: {err}"),
        }
    }

    fn fresh_ct() -> tokio_util::sync::CancellationToken {
        tokio_util::sync::CancellationToken::new()
    }

    // ── Plain read (settle phase) ──────────────────────────────────────────────

    #[tokio::test]
    async fn char_plain_read_single_chunk_data_complete() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"hello".to_vec())).await.unwrap();
        drop(tx); // sender closed → settle gathers the chunk, then ends normally
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.bytes, b"hello");
        assert_eq!(out.meta.stop_reason, RxStopReason::DataComplete);
        assert!(!out.matched);
        assert!(out.frames.is_empty());
    }

    #[tokio::test]
    async fn char_plain_read_timeout_empty() {
        let (_tx, rx) = mpsc::channel(8); // keep sender alive so recv blocks (no channel-close)
        let out = read_bytes_via_session(
            rx,
            256,
            Some(80),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(out.bytes.is_empty());
        assert_eq!(out.meta.stop_reason, RxStopReason::Timeout);
    }

    #[tokio::test]
    async fn char_plain_read_max_buffered_truncates() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"0123456789".to_vec()))
            .await
            .unwrap();
        let out = read_bytes_via_session(
            rx,
            4,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.bytes.len(), 4);
        assert_eq!(out.meta.stop_reason, RxStopReason::MaxBufferedBytes);
        assert!(out.meta.truncated);
        assert_eq!(out.meta.bytes_observed, 10);
        assert_eq!(out.meta.bytes_returned, 4);
    }

    // ── Lifecycle stop reasons ─────────────────────────────────────────────────

    #[tokio::test]
    async fn char_channel_closed_before_data() {
        let (tx, rx) = mpsc::channel(8);
        drop(tx); // no data ever; main loop sees recv() == None
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::ChannelClosed);
    }

    #[tokio::test]
    async fn char_connection_closed_event() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Closed).await.unwrap();
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::ConnectionClosed);
    }

    #[tokio::test]
    async fn char_cancelled() {
        let ct = fresh_ct();
        ct.cancel();
        let (_tx, rx) = mpsc::channel(8);
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &ct,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::Cancelled);
    }

    // ── Matcher (raw, no framing) ──────────────────────────────────────────────

    #[tokio::test]
    async fn char_matcher_found_raw() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"xxOKyy".to_vec())).await.unwrap();
        let matcher = Matcher::new_literal(b"OK".to_vec());
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            matcher,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::MatchFound);
        assert!(out.matched);
        assert_eq!(out.match_index, Some(2));
        assert_eq!(out.bytes, b"xxOKyy");
    }

    #[tokio::test]
    async fn char_matcher_found_raw_spans_two_chunks() {
        // Match pattern "OK" split across two RxEvent::Data chunks.
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"xxO".to_vec())).await.unwrap();
        tx.send(RxEvent::Data(b"Kyy".to_vec())).await.unwrap();
        let matcher = Matcher::new_literal(b"OK".to_vec());
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            matcher,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::MatchFound);
        assert!(out.matched);
        assert_eq!(out.match_index, Some(2));
        assert_eq!(out.bytes, b"xxOKyy");
        drop(tx);
    }

    #[tokio::test]
    async fn char_matcher_silence_timeout() {
        // With a matcher active the main loop never breaks to settle, so the
        // silence timer governs. One byte, then quiet → no_new_rx_timeout.
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"a".to_vec())).await.unwrap();
        let matcher = Matcher::new_literal(b"ZZ".to_vec()); // never matches
        let out = read_bytes_via_session(
            rx,
            256,
            Some(5000),
            &fresh_ct(),
            None,
            None,
            matcher,
            Some(60),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::NoNewRxTimeout);
        assert!(!out.matched);
        drop(tx);
    }

    // ── Framing ────────────────────────────────────────────────────────────────

    fn line_framing(max_frames: Option<usize>) -> crate::framing::RxFramingConfig {
        crate::framing::RxFramingConfig {
            mode: crate::framing::RxFramingMode::Line {
                ending: crate::framing::LineEnding::Auto,
            },
            max_frames,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn char_framing_max_frames() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"a\nb\n".to_vec())).await.unwrap();
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            Some(line_framing(Some(2))),
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::MaxFrames);
        assert_eq!(out.frames.len(), 2);
        drop(tx);
    }

    #[tokio::test]
    async fn char_framing_match_sets_frame_index() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"a\nb\n".to_vec())).await.unwrap();
        let matcher = Matcher::new_literal(b"b".to_vec());
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            matcher,
            None,
            None,
            Some(line_framing(None)),
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::MatchFound);
        assert!(out.matched);
        assert_eq!(out.match_index, Some(0));
        assert_eq!(out.match_frame_index, Some(1));
        assert_eq!(out.bytes, b"b");
        drop(tx);
    }

    #[tokio::test]
    async fn char_framing_match_includes_post_match_frames() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"hit\nafter\n".to_vec()))
            .await
            .unwrap();
        let matcher = Matcher::new_literal(b"hit".to_vec());
        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            matcher,
            None,
            None,
            Some(line_framing(None)),
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::MatchFound);
        assert_eq!(out.match_frame_index, Some(0));
        assert_eq!(out.bytes, b"hit");
        // read includes the frame decoded AFTER the matching one in the same chunk.
        assert_eq!(out.frames.len(), 2);
        drop(tx);
    }

    #[tokio::test]
    async fn char_framing_partial_frame_flushed_on_timeout() {
        // "ab" with no newline → buffered partial frame, flushed on timeout.
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"ab".to_vec())).await.unwrap();
        let out = read_bytes_via_session(
            rx,
            256,
            Some(80),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            Some(line_framing(None)),
            None,
        )
        .await
        .unwrap();
        assert_eq!(out.meta.stop_reason, RxStopReason::Timeout);
        assert_eq!(out.frames.len(), 1);
        assert_eq!(out.frames[0].data, b"ab");
    }

    struct TestRxArgs {
        connection_id: String,
        encoding: String,
        max_buffered_bytes: usize,
        timeout_ms: Option<u64>,
        no_new_rx_timeout_ms: Option<u64>,
        match_request: Option<MatchRequest>,
    }

    impl RxRequestArgs for TestRxArgs {
        fn connection_id(&self) -> &str {
            &self.connection_id
        }
        fn encoding(&self) -> &str {
            &self.encoding
        }
        fn max_buffered_bytes(&self) -> usize {
            self.max_buffered_bytes
        }
        fn timeout_ms(&self) -> Option<u64> {
            self.timeout_ms
        }
        fn no_new_rx_timeout_ms(&self) -> Option<u64> {
            self.no_new_rx_timeout_ms
        }
        fn match_request(&self) -> Option<&MatchRequest> {
            self.match_request.as_ref()
        }
    }

    fn valid_args(id: &str) -> TestRxArgs {
        TestRxArgs {
            connection_id: id.into(),
            encoding: "utf-8".into(),
            max_buffered_bytes: 256,
            timeout_ms: Some(1000),
            no_new_rx_timeout_ms: None,
            match_request: None,
        }
    }

    fn read_limits() -> RxLimits {
        RxLimits {
            tool: "read",
            min_buffered: MIN_READ_BYTES,
            max_buffered: MAX_READ_BYTES,
        }
    }

    async fn fake_conn() -> (Arc<ConnectionManager>, String, tokio::io::DuplexStream) {
        let connections = Arc::new(ConnectionManager::new());
        let (conn, peer) = crate::serial::test_support::loopback_connection("/dev/fake-validate");
        let id = connections.insert(conn).await.unwrap();
        (connections, id, peer)
    }

    #[tokio::test]
    async fn validate_rx_request_ok() {
        let (connections, id, _peer) = fake_conn().await;
        let resolved = validate_rx_request(&connections, &valid_args(&id), read_limits())
            .await
            .unwrap();
        assert_eq!(resolved.max_buffered_bytes, 256);
        assert!(resolved.matcher.is_none());
        assert_eq!(resolved.connection.port(), "/dev/fake-validate");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_bad_encoding() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.encoding = "rot13".into();
        let err = validate_rx_request(&connections, &a, read_limits())
            .await
            .unwrap_err();
        assert!(err.to_lowercase().contains("encoding"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_unknown_connection() {
        let connections = Arc::new(ConnectionManager::new());
        let err = validate_rx_request(&connections, &valid_args("nope"), read_limits())
            .await
            .unwrap_err();
        assert!(err.contains("Connection ID nope not found"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_buffered_below_min() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.max_buffered_bytes = 0;
        let err = validate_rx_request(&connections, &a, read_limits())
            .await
            .unwrap_err();
        assert!(err.contains("read.max_buffered_bytes"), "got: {err}");
        assert!(err.contains("below minimum"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_buffered_above_max() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.max_buffered_bytes = MAX_READ_BYTES + 1;
        let err = validate_rx_request(&connections, &a, read_limits())
            .await
            .unwrap_err();
        assert!(err.contains("exceeds maximum"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_zero_silence_with_tool_prefix() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.no_new_rx_timeout_ms = Some(0);
        let subscribe_limits = RxLimits {
            tool: "subscribe",
            min_buffered: MIN_STREAM_CHUNK_BYTES,
            max_buffered: MAX_STREAM_CHUNK_BYTES,
        };
        let err = validate_rx_request(&connections, &a, subscribe_limits)
            .await
            .unwrap_err();
        assert_eq!(err, "subscribe.no_new_rx_timeout_ms must be > 0");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_oversized_timeout() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.timeout_ms = Some(MAX_TIMEOUT_MS + 1);
        let err = validate_rx_request(&connections, &a, read_limits())
            .await
            .unwrap_err();
        assert!(err.contains("read.timeout_ms"), "got: {err}");
        assert!(err.contains("exceeds maximum"), "got: {err}");
    }

    // ── Auto-promotion integration: read loop ──────────────────────────────

    /// Test that auto promotes to CrMode when the read loop receives a bare
    /// `\r` followed by a non-`\n` byte across events.
    #[tokio::test]
    async fn char_framing_auto_promotes_on_bare_cr() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"line1\r".to_vec())).await.unwrap();
        tx.send(RxEvent::Data(b"x".to_vec())).await.unwrap();
        // After promotion, "more\r" splits on \r in CrMode.
        tx.send(RxEvent::Data(b"more\r".to_vec())).await.unwrap();
        drop(tx);

        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            Some(line_framing(None)),
            None,
        )
        .await
        .unwrap();

        // First frame from promotion: "line1"
        // Second frame: "xmore" (x stayed buffered + more, split on \r)
        assert_eq!(out.frames.len(), 2);
        assert_eq!(out.frames[0].data, b"line1");
        assert_eq!(out.frames[1].data, b"xmore");
    }

    /// Test that flush_partial on timeout emits a pending \r as a frame.
    #[tokio::test]
    async fn char_framing_auto_flush_partial_on_timeout_emits_pending_cr() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(b"tail\r".to_vec())).await.unwrap();
        // No more data — read times out.

        let out = read_bytes_via_session(
            rx,
            256,
            Some(80),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            Some(line_framing(None)),
            None,
        )
        .await
        .unwrap();

        assert_eq!(out.frames.len(), 1);
        assert_eq!(out.frames[0].data, b"tail\r");
        assert_eq!(out.meta.stop_reason, RxStopReason::Timeout);
    }

    fn slip_framing() -> crate::framing::RxFramingConfig {
        crate::framing::RxFramingConfig {
            mode: crate::framing::RxFramingMode::Slip,
            ..Default::default()
        }
    }

    /// Read integration: SLIP decodes a frame from the event stream.
    #[tokio::test]
    async fn char_framing_slip_decode_success() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(vec![0xC0, b'h', b'i', 0xC0]))
            .await
            .unwrap();
        drop(tx);

        let out = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            Some(slip_framing()),
            None,
        )
        .await
        .unwrap();

        assert_eq!(out.frames.len(), 1);
        assert_eq!(out.frames[0].data, b"hi");
        assert_eq!(out.frames[0].frame_type, "slip");
    }

    /// Read integration: SLIP malformed escape returns partial result
    /// with stop_reason=framing_error (was: surfaces as Err).
    #[tokio::test]
    async fn char_framing_slip_malformed_surfaces_error() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(vec![0xC0, 0xDB, 0x41, 0xC0]))
            .await
            .unwrap();
        drop(tx);

        let outcome = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            Some(slip_framing()),
            None,
        )
        .await
        .unwrap_or_else(|msg| panic!("expected Ok outcome, got Err: {msg}"));

        assert_eq!(
            outcome.meta.stop_reason,
            crate::rx_metadata::RxStopReason::FramingError,
            "expected framing_error stop reason"
        );
        assert!(
            outcome.error.is_some(),
            "expected error field to be Some, got None"
        );
        let err_text = outcome.error.as_ref().unwrap();
        assert!(
            err_text.contains("SLIP"),
            "expected error to mention SLIP: {err_text}"
        );
        assert!(
            err_text.contains("0x41"),
            "expected error to mention violating byte 0x41: {err_text}"
        );

        // Drive through build_read_result with utf8 encoding — must fall
        // back to hex and return Ok (not Err).
        let result = build_read_result(
            outcome,
            "test".into(),
            None,
            Encoding::Utf8,
            Some(1000),
            None,
        );
        let Json(read_result) =
            result.unwrap_or_else(|e| panic!("expected Ok result, got Err: {e}"));
        assert_eq!(read_result.stop_reason, "framing_error");
        assert_eq!(read_result.encoding, "hex");
        assert!(
            read_result.error.is_some(),
            "expected ReadResult.error to be Some"
        );
        let re = read_result.error.unwrap();
        assert!(re.contains("SLIP"), "error msg: {re}");
        assert!(re.contains("0x41"), "error msg: {re}");
        // Hex-encoded data: only hex digits and spaces.
        assert!(
            read_result
                .data
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == ' '),
            "data not hex-encoded: {}",
            read_result.data
        );
    }

    /// Read integration: SLIP malformed escape with a good frame first.
    /// Good frames decoded before the error must survive.
    #[tokio::test]
    async fn char_framing_slip_frames_before_malformed_error() {
        let (tx, rx) = mpsc::channel(8);
        // Good SLIP frame "hi", then ESC followed by invalid escape byte
        // (no consecutive END markers to avoid empty frames).
        tx.send(RxEvent::Data(vec![0xC0, b'h', b'i', 0xC0, 0xDB, 0x41]))
            .await
            .unwrap();
        drop(tx);

        let outcome = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            Some(slip_framing()),
            None,
        )
        .await
        .unwrap_or_else(|msg| panic!("expected Ok outcome, got Err: {msg}"));

        assert_eq!(
            outcome.meta.stop_reason,
            crate::rx_metadata::RxStopReason::FramingError,
            "expected framing_error stop reason"
        );
        assert!(outcome.error.is_some(), "expected error field to be Some");
        // The good frame should be collected.
        let hi_frame = outcome.frames.iter().find(|f| f.data == b"hi");
        assert!(
            hi_frame.is_some(),
            "expected a frame with data b\"hi\", got frames: {:?}",
            outcome.frames
        );
        assert_eq!(
            hi_frame.unwrap().frame_type,
            "slip",
            "good frame should have type slip"
        );

        // Drive through build_read_result with utf8 encoding.
        let result = build_read_result(
            outcome,
            "test".into(),
            None,
            Encoding::Utf8,
            Some(1000),
            None,
        );
        let Json(read_result) =
            result.unwrap_or_else(|e| panic!("expected Ok result, got Err: {e}"));
        assert_eq!(read_result.stop_reason, "framing_error");
        assert_eq!(read_result.encoding, "hex");
        assert!(
            read_result.error.is_some(),
            "expected ReadResult.error to be Some"
        );
        let re = read_result.error.as_ref().unwrap();
        assert!(re.contains("SLIP"), "error msg: {re}");
        // The good frame should survive in the frames array.
        let frames = read_result.frames.as_ref().expect("frames should be Some");
        assert!(
            frames
                .iter()
                .any(|f| f.data.contains("68") && f.data.contains("69")),
            "expected a frame with hex data for \"hi\", got: {frames:?}"
        );
    }

    /// Guard: a non-framing read with invalid-utf8 bytes hard-fails.
    /// Normal stops keep the existing hard-fail contract — hex fallback
    /// applies ONLY on the framing-error path.
    #[tokio::test]
    async fn non_framing_read_invalid_utf8_still_errors() {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RxEvent::Data(vec![0xFF, b'X'])).await.unwrap();
        drop(tx);

        let outcome = read_bytes_via_session(
            rx,
            256,
            Some(1000),
            &fresh_ct(),
            None,
            None,
            None,
            None,
            None,
            None, // no framing
            None,
        )
        .await
        .unwrap();

        assert_ne!(
            outcome.meta.stop_reason,
            crate::rx_metadata::RxStopReason::FramingError,
            "no framing configured, must not be framing_error"
        );

        let result = build_read_result(
            outcome,
            "test".into(),
            None,
            Encoding::Utf8,
            Some(1000),
            None,
        );
        assert!(
            result.is_err(),
            "expected Err for invalid-utf8 data with no framing"
        );
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected Err, got Ok"),
        };
        assert!(
            err.contains("Data encoding failed"),
            "expected encoding error: {err}"
        );
    }
}
