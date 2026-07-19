use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{model::ProgressToken, service::Peer, Json, RoleServer};
use tracing::error;

use crate::codec::{self, Encoding};
use crate::match_config::MatchResult;
use crate::match_config::{shape_match_context, validate_match_request, MatchRequest, Matcher};
use crate::rx_metadata::RxStopMetadata;
use crate::rx_session::RxSession;
use crate::serial::{ConnectionConfig, ConnectionManager, SerialConnection};
use crate::stop_controller::{RxStopController, RxStopDecision};
use crate::tools::rx_consume::{
    consume_frames, disconnect_state, frame_outcome_to_stop, DisconnectState, RxFrameSink, SinkFlow,
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
    pub start_offset: u64,
    pub end_offset: u64,
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

/// Re-export of the shared [`crate::util::find_subsequence`] under the
/// legacy `find_subslice` name to keep existing import paths stable.
pub(crate) use crate::util::find_subsequence as find_subslice;

// ------------------------------------------------------------------
// Ring-based read (Phase 1.3)
// ------------------------------------------------------------------

/// Advance the shared read cursor by `consumed` bytes from `base`,
/// clamped to the ring's live edge.
fn advance_cursor(
    session: &crate::rx_session::RxSession,
    base: u64,
    consumed: u64,
    ring: &crate::rx_ring::RxRing,
) {
    let next = base.saturating_add(consumed).min(ring.end_offset());
    session.set_read_cursor(next);
}

/// Drive a `read` from the ring buffer, with cat semantics: buffered-but-
/// unread bytes are returned immediately. Pattern matching checks history
/// first, then waits for new bytes. Always advances the cursor.
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
    let mut frame_error_msg: Option<String> = None;
    let conn_id = session.connection_id().to_string();

    // We track how many raw bytes we consumed from the ring (for cursor advancement)
    // and how many we return in the result.
    let mut consumed_offset: u64 = 0; // raw bytes consumed from ring cursor
    let mut returned_bytes: Vec<u8> = Vec::with_capacity(max_bytes);

    // Helper: build the ReadOutcome from current state.
    // Always advances cursor.
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
                             cursor: u64|
     -> ReadOutcome {
        let start_off = ring.start_offset();
        let end_off = ring.end_offset();
        let clamped_from = cursor.max(start_off).min(end_off);
        let bytes_lost = start_off.saturating_sub(cursor);
        let used = consumed_offset.min(max_bytes as u64);
        let next_off = clamped_from + used;
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
            start_offset: ring.start_offset(),
            end_offset: ring.end_offset(),
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
        advance_cursor(&session, initial_slice.from_offset, consumed, ring);
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
            advance_cursor(&session, initial_slice.from_offset, consumed, ring);
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
                    next_offset: Some(initial_slice.from_offset + consumed),
                    bytes_lost: initial_slice.bytes_lost,
                    buffered_remaining: ring
                        .end_offset()
                        .saturating_sub(initial_slice.from_offset + consumed),
                    start_offset: ring.start_offset(),
                    end_offset: ring.end_offset(),
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
                session.set_read_cursor(
                    initial_slice
                        .from_offset
                        .wrapping_add(consumed_offset)
                        .min(ring.end_offset()),
                );
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
                ));
            }
            if let Some(stop) = frame_outcome_to_stop(
                outcome,
                &ctrl,
                returned_bytes.len(),
                match_index,
                &mut frame_error_msg,
                &conn_id,
            ) {
                session.set_read_cursor(
                    initial_slice
                        .from_offset
                        .wrapping_add(consumed_offset)
                        .min(ring.end_offset()),
                );
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    stop.meta,
                    stop.matched,
                    stop.match_index,
                    match_frame_index,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    frame_error_msg,
                    ring,
                    cursor,
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
                    session.set_read_cursor(
                        cursor.wrapping_add(consumed_offset).min(ring.end_offset()),
                    );
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
            advance_cursor(&session, cursor, consumed_offset, ring);
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
            ));
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            advance_cursor(&session, cursor, consumed_offset, ring);
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
                    session.set_read_cursor(
                        cursor.wrapping_add(consumed_offset).min(ring.end_offset()),
                    );
                return Ok(make_read_outcome(
                    returned_bytes, consumed_offset, &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    outcome.meta, outcome.matched, outcome.match_index, None,
                    std::mem::take(&mut collected_frames),
                    frames_dropped, None, ring, cursor,
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
                session.set_read_cursor(
                    slice
                        .from_offset
                        .wrapping_add(take as u64)
                        .min(ring.end_offset()),
                );
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
                ));
            }
            if let Some(stop) = frame_outcome_to_stop(
                outcome,
                &ctrl,
                returned_bytes.len(),
                match_index,
                &mut frame_error_msg,
                &conn_id,
            ) {
                session.set_read_cursor(
                    slice
                        .from_offset
                        .wrapping_add(take as u64)
                        .min(ring.end_offset()),
                );
                return Ok(make_read_outcome(
                    returned_bytes,
                    consumed_offset,
                    &ctrl,
                    read_start.elapsed().as_millis() as u64,
                    stop.meta,
                    stop.matched,
                    stop.match_index,
                    match_frame_index,
                    std::mem::take(&mut collected_frames),
                    frames_dropped,
                    frame_error_msg,
                    ring,
                    cursor,
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
                session.set_read_cursor(
                    slice
                        .from_offset
                        .wrapping_add(take as u64)
                        .min(ring.end_offset()),
                );
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
            advance_cursor(&session, cursor, consumed_offset, ring);
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
        start_offset: outcome.start_offset,
        end_offset: outcome.end_offset,
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
            start_offset: 0,
            end_offset: 0,
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
            start_offset: 0,
            end_offset: 0,
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
            start_offset: 0,
            end_offset: 0,
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
            start_offset: 0,
            end_offset: 0,
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
            start_offset: 0,
            end_offset: 0,
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

    // ── RX request validation ──────────────────────────────────────────────

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
}
