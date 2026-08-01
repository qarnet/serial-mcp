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
/// `max_buffered_bytes` is passed explicitly (resolved from the connection
/// default by the handler).
/// Does NOT reserve the buffer budget — the caller does that (subscribe must
/// drop any prior subscription before reserving).
pub async fn validate_rx_request<A: RxRequestArgs>(
    connections: &Arc<ConnectionManager>,
    args: &A,
    limits: RxLimits,
    max_buffered_bytes: usize,
) -> Result<ResolvedRxArgs, String> {
    let encoding = parse_encoding(args.encoding())?;
    let connection = lookup_connection(connections, args.connection_id()).await?;

    let max_buffered_bytes = require_min_or_err(
        &format!("{}.max_buffered_bytes", limits.tool),
        max_buffered_bytes,
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

/// Advance a cursor by `consumed` bytes from `base`, clamped to the ring's
/// live edge. Mirrors the previous shared-cursor `advance_cursor` helper,
/// but writes to a caller-owned cursor value instead of the session.
fn advance_private_cursor(base: u64, consumed: u64, ring: &crate::rx_ring::RxRing) -> u64 {
    base.saturating_add(consumed).min(ring.end_offset())
}

/// Drive a `read` from the ring buffer using a PRIVATE cursor, with cat
/// semantics: buffered-but-unread bytes are returned immediately. Pattern
/// matching checks history first, then waits for new bytes.
///
/// The shared `read` cursor is never touched. The caller supplies the start
/// offset (`initial_cursor`) and receives the final private cursor so the
/// shared wrapper can apply it. `capture_boot` starts at its atomic mark and
/// discards the returned cursor.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn read_from_private_cursor(
    session: &crate::rx_session::RxSession,
    initial_cursor: u64,
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
) -> Result<(ReadOutcome, u64), String> {
    let effective_timeout_ms = timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    let read_start = Instant::now();
    let ring = session.ring();
    let cursor = initial_cursor;

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
    // Private cursor state: every write the shared-cursor version made to
    // `session.set_read_cursor` lands here instead. Every return path below
    // overwrites it before returning (the shared version also always
    // overwrites before reading back), so the initial value is never read.
    #[allow(unused_assignments)]
    let mut cursor_state: u64 = cursor;

    // Helper: build the ReadOutcome from current state.
    // Does not touch the shared cursor.
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
        cursor_state = advance_private_cursor(initial_slice.from_offset, consumed, ring);
        return Ok((
            make_read_outcome(
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
            ),
            cursor_state,
        ));
    }

    // Match-check history first if a matcher is present.
    if matcher.is_some() && !initial_slice.bytes.is_empty() {
        let take = initial_slice.bytes.len().min(max_bytes);
        let hist = &initial_slice.bytes[..take];
        // Bounded push: same matcher-owned window policy as the live path and
        // subscribe. The initial slice is at most `max_bytes`, so no
        // truncation occurs here and the history match stays exact.
        let match_result = matcher.as_mut().map(|m| m.push_bounded(hist, max_bytes));
        if let Some(MatchResult::Found(idx)) = match_result {
            let match_end = idx + needle_len.unwrap_or(0);
            let consumed = match_end as u64;
            let data = hist[..match_end].to_vec();
            let meta = RxStopMetadata::match_found(consumed as usize, consumed as usize);
            cursor_state = advance_private_cursor(initial_slice.from_offset, consumed, ring);
            // Handle context shaping
            if let Some(context) = context_amount {
                let shaped = shape_match_context(hist, idx, needle_len.unwrap_or(0), Some(context));
                let shaped_consumed = shaped.data.len() as u64;
                return Ok((
                    ReadOutcome {
                        bytes: shaped.data,
                        elapsed_ms: read_start.elapsed().as_millis() as u64,
                        meta: RxStopMetadata::match_found(
                            consumed as usize,
                            shaped_consumed as usize,
                        ),
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
                    },
                    cursor_state,
                ));
            }
            return Ok((
                make_read_outcome(
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
                ),
                cursor_state,
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
                cursor_state = initial_slice
                    .from_offset
                    .wrapping_add(consumed_offset)
                    .min(ring.end_offset());
                return Ok((
                    make_read_outcome(
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
                    ),
                    cursor_state,
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
                cursor_state = initial_slice
                    .from_offset
                    .wrapping_add(consumed_offset)
                    .min(ring.end_offset());
                return Ok((
                    make_read_outcome(
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
                    ),
                    cursor_state,
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
                    cursor_state = cursor.wrapping_add(consumed_offset).min(ring.end_offset());
                    return Ok((
                        make_read_outcome(
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
                        ),
                        cursor_state,
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
            cursor_state = advance_private_cursor(cursor, consumed_offset, ring);
            return Ok((
                make_read_outcome(
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
                ),
                cursor_state,
            ));
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            cursor_state = advance_private_cursor(cursor, consumed_offset, ring);
            return Ok((
                make_read_outcome(
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
                ),
                cursor_state,
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
                    cursor_state = cursor
                        .wrapping_add(consumed_offset)
                        .min(ring.end_offset());
                return Ok((
                    make_read_outcome(
                        returned_bytes, consumed_offset, &ctrl,
                        read_start.elapsed().as_millis() as u64,
                        outcome.meta, outcome.matched, outcome.match_index, None,
                        std::mem::take(&mut collected_frames),
                        frames_dropped, None, ring, cursor,
                    ),
                    cursor_state,
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
                cursor_state = slice
                    .from_offset
                    .wrapping_add(take as u64)
                    .min(ring.end_offset());
                return Ok((
                    make_read_outcome(
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
                    ),
                    cursor_state,
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
                cursor_state = slice
                    .from_offset
                    .wrapping_add(take as u64)
                    .min(ring.end_offset());
                return Ok((
                    make_read_outcome(
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
                    ),
                    cursor_state,
                ));
            }
        }

        // Raw matcher path (no framing).
        if decoder.is_none() {
            // Bounded push: same matcher-owned window policy as the
            // initial-history path and subscribe.
            let match_result = matcher.as_mut().map(|m| m.push_bounded(chunk, max_bytes));
            let buffered_len = returned_bytes.len();
            let data_count = chunk.len();
            if let RxStopDecision::Stop(outcome) =
                ctrl.push_data(data_count, buffered_len, match_result)
            {
                // Live matches apply matcher-owned context shaping (same
                // policy as subscribe). Only the returned payload and the
                // relative match_index change — cursor consumption and the
                // stream offsets stay based on the consumed bytes.
                let (match_bytes, match_index) = match outcome.match_index {
                    Some(idx) => match matcher
                        .as_ref()
                        .and_then(|m| m.shape_literal_match_context(idx))
                    {
                        Some(shaped) => (shaped.data, Some(shaped.match_index)),
                        None => (returned_bytes.clone(), Some(idx)),
                    },
                    None => (returned_bytes.clone(), None),
                };
                cursor_state = slice
                    .from_offset
                    .wrapping_add(take as u64)
                    .min(ring.end_offset());
                return Ok((
                    make_read_outcome(
                        match_bytes,
                        consumed_offset,
                        &ctrl,
                        read_start.elapsed().as_millis() as u64,
                        outcome.meta,
                        outcome.matched,
                        match_index,
                        None,
                        std::mem::take(&mut collected_frames),
                        frames_dropped,
                        None,
                        ring,
                        cursor,
                    ),
                    cursor_state,
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
            cursor_state = advance_private_cursor(cursor, consumed_offset, ring);
            return Ok((
                make_read_outcome(
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
                ),
                cursor_state,
            ));
        }
    }
}

/// Drive a `read` from the ring buffer with the SHARED read cursor: reads
/// the current cursor, delegates to [`read_from_private_cursor`], and
/// applies the returned final cursor. Preserves the pre-Phase-5 behavior
/// exactly — `read`/`transact` call this wrapper.
#[allow(clippy::too_many_arguments)]
pub async fn read_bytes_from_ring(
    session: Arc<RxSession>,
    max_bytes: usize,
    timeout_ms: Option<u64>,
    ct: &tokio_util::sync::CancellationToken,
    progress_token: Option<ProgressToken>,
    peer: Option<&Peer<RoleServer>>,
    matcher: Option<Matcher>,
    no_new_rx_timeout_ms: Option<u64>,
    conn: Option<Arc<SerialConnection>>,
    framing: Option<crate::framing::RxFramingConfig>,
    parser: Option<crate::framing::ParserConfig>,
) -> Result<ReadOutcome, String> {
    let initial_cursor = session.read_cursor();
    let (outcome, final_cursor) = read_from_private_cursor(
        &session,
        initial_cursor,
        max_bytes,
        timeout_ms,
        ct,
        progress_token,
        peer,
        matcher,
        no_new_rx_timeout_ms,
        conn,
        framing,
        parser,
    )
    .await?;
    session.set_read_cursor(final_cursor);
    Ok(outcome)
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

    let (data, effective_encoding) = match codec::encode_or_hex(encoding, &outcome.bytes) {
        Ok(payload) => {
            if let Some(reason) = &payload.fallback_reason {
                // Lossless fallback: exact spaced hex preserves every byte.
                // Warned but never counted as a drop. Lossy UTF-8 was
                // rejected (corrupts bytes); hex matches the binary-protocol
                // context that produced the unencodable payload.
                tracing::warn!(
                    "read data not encodable as {encoding} ({reason}); \
                     falling back to hex"
                );
            }
            (payload.data, payload.encoding)
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
                // Encode each frame independently from the REQUESTED
                // encoding (not the top-level effective encoding): a valid
                // UTF-8 frame preceding malformed binary SLIP stays UTF-8
                // while the top-level raw data falls back to hex.
                match codec::encode_or_hex(encoding, &f.data) {
                    Ok(payload) => {
                        if let Some(reason) = &payload.fallback_reason {
                            tracing::warn!(
                                "Frame {} not encodable as {encoding} ({reason}); \
                                 falling back to hex",
                                f.index
                            );
                        }
                        Some(FrameResult {
                            data: payload.data,
                            encoding: payload.encoding.to_string(),
                            frame_index: f.index,
                            frame_type: f.frame_type.to_string(),
                            parsed: f.parsed.clone(),
                        })
                    }
                    Err(err) => {
                        // Only a true encode+hex failure counts as a drop.
                        tracing::warn!("Frame {} encoding failed: {err}", f.index);
                        frames_dropped += 1;
                        None
                    }
                }
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

// ------------------------------------------------------------------
// Open-settings resolution (Phase 3A)
// ------------------------------------------------------------------

/// Optional open-field overlay shared by `open` and `open_profile`.
///
/// `None` fields fall through to the selected profile's defaults, then to
/// built-in defaults. `open` fills the overlay from `OpenArgs`;
/// `open_profile` fills only `name`/`log_capacity`/`log_enabled`/
/// `rx_buffer_size` from `OpenProfileArgs` (all other fields come from the
/// named profile).
#[derive(Debug, Clone, Default)]
pub struct OpenOverlay {
    pub(crate) name: Option<String>,
    pub(crate) baud_rate: Option<u32>,
    pub(crate) data_bits: Option<String>,
    pub(crate) stop_bits: Option<String>,
    pub(crate) parity: Option<String>,
    pub(crate) flow_control: Option<String>,
    pub(crate) log_capacity: Option<usize>,
    pub(crate) log_enabled: Option<bool>,
    pub(crate) reconnect_policy: Option<crate::serial::ReconnectPolicy>,
    pub(crate) tx_framing: Option<crate::framing::TxFramingConfig>,
    pub(crate) rx_framing: Option<crate::framing::RxFramingConfig>,
    pub(crate) rx_parser: Option<crate::framing::ParserConfig>,
    pub(crate) protocol: Option<crate::framing::ProtocolPreset>,
    pub(crate) rx_buffer_size: Option<usize>,
    pub(crate) max_buffered_bytes: Option<usize>,
    pub(crate) poll_interval_ms: Option<u64>,
}

impl OpenOverlay {
    /// Overlay from the bare `open` tool's arguments.
    pub(crate) fn from_open_args(args: &OpenArgs) -> Self {
        Self {
            name: args.name.clone(),
            baud_rate: args.baud_rate,
            data_bits: args.data_bits.clone(),
            stop_bits: args.stop_bits.clone(),
            parity: args.parity.clone(),
            flow_control: args.flow_control.clone(),
            log_capacity: args.log_capacity,
            log_enabled: args.log_enabled,
            reconnect_policy: args.reconnect_policy.clone(),
            tx_framing: args.tx_framing.clone(),
            rx_framing: args.rx_framing.clone(),
            rx_parser: args.rx_parser.clone(),
            protocol: args.protocol,
            rx_buffer_size: args.rx_buffer_size,
            max_buffered_bytes: args.max_buffered_bytes,
            poll_interval_ms: args.poll_interval_ms,
        }
    }

    /// Overlay from the `open_profile` tool's override arguments. Only the
    /// fields `open_profile` exposes are populated; everything else falls
    /// through to the named profile's defaults.
    pub(crate) fn from_open_profile_args(args: &OpenProfileArgs) -> Self {
        Self {
            name: args.name.clone(),
            log_capacity: args.log_capacity,
            log_enabled: args.log_enabled,
            rx_buffer_size: args.rx_buffer_size,
            ..Self::default()
        }
    }
}

/// Concrete, fully-resolved open settings after merging explicit open
/// fields, a selected profile's defaults, and built-in defaults
/// (115200/8-N-1, 256 KiB ring, etc.). `ConnectionConfig` is built from
/// this; no `unwrap_or` precedence is scattered across tool logic.
#[derive(Debug, Clone)]
pub struct ResolvedOpenSettings {
    pub port: String,
    pub name: Option<String>,
    pub baud_rate: u32,
    pub data_bits: crate::serial::DataBits,
    pub stop_bits: crate::serial::StopBits,
    pub parity: crate::serial::Parity,
    pub flow_control: crate::serial::FlowControl,
    pub log_capacity: usize,
    pub log_enabled: bool,
    pub reconnect_policy: crate::serial::ReconnectPolicy,
    pub tx_framing: Option<crate::framing::TxFramingConfig>,
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    pub rx_parser: Option<crate::framing::ParserConfig>,
    pub protocol: Option<crate::framing::ProtocolPreset>,
    pub rx_buffer_size: usize,
    pub max_buffered_bytes: usize,
    pub poll_interval_ms: u64,
}

impl PartialEq for ResolvedOpenSettings {
    fn eq(&self, other: &Self) -> bool {
        self.port == other.port
            && self.name == other.name
            && self.baud_rate == other.baud_rate
            && crate::serial::data_bits_to_str(self.data_bits)
                == crate::serial::data_bits_to_str(other.data_bits)
            && crate::serial::stop_bits_to_str(self.stop_bits)
                == crate::serial::stop_bits_to_str(other.stop_bits)
            && crate::serial::parity_to_str(self.parity)
                == crate::serial::parity_to_str(other.parity)
            && crate::serial::flow_control_to_str(self.flow_control)
                == crate::serial::flow_control_to_str(other.flow_control)
            && self.log_capacity == other.log_capacity
            && self.log_enabled == other.log_enabled
            && self.reconnect_policy == other.reconnect_policy
            && self.tx_framing == other.tx_framing
            && self.rx_framing == other.rx_framing
            && self.rx_parser == other.rx_parser
            && self.protocol == other.protocol
            && self.rx_buffer_size == other.rx_buffer_size
            && self.max_buffered_bytes == other.max_buffered_bytes
            && self.poll_interval_ms == other.poll_interval_ms
    }
}

impl ResolvedOpenSettings {
    /// Resolve `overlay` against `profile_defaults` (the selected profile's
    /// defaults) and the built-in defaults. Parsing failures (invalid data
    /// bits etc.) return a tool error.
    pub fn resolve(
        port: String,
        overlay: &OpenOverlay,
        profile_defaults: Option<&crate::profiles::ProfileDefaults>,
    ) -> Result<Self, String> {
        let builtin = crate::profiles::ProfileDefaults::default();
        let base = profile_defaults.unwrap_or(&builtin);

        // Connection name: explicit name, else the profile's name prefix
        // (expanded to `{prefix}-{short_port_name}`), else none.
        let name = match &overlay.name {
            Some(n) => Some(n.clone()),
            None => base.name.as_ref().map(|prefix| {
                let short = port.rsplit('/').next().unwrap_or(&port);
                format!("{prefix}-{short}")
            }),
        };

        let data_bits = overlay
            .data_bits
            .clone()
            .unwrap_or_else(|| base.data_bits.clone())
            .parse()?;
        let stop_bits = overlay
            .stop_bits
            .clone()
            .unwrap_or_else(|| base.stop_bits.clone())
            .parse()?;
        let parity = overlay
            .parity
            .clone()
            .unwrap_or_else(|| base.parity.clone())
            .parse()?;
        let flow_control = overlay
            .flow_control
            .clone()
            .unwrap_or_else(|| base.flow_control.clone())
            .parse()?;

        let rx_buffer_size = overlay.rx_buffer_size.unwrap_or(base.rx_buffer_size);
        let rx_buffer_size = validate_open_rx_buffer_size(rx_buffer_size)?;

        Ok(Self {
            port,
            name,
            baud_rate: overlay.baud_rate.unwrap_or(base.baud_rate),
            data_bits,
            stop_bits,
            parity,
            flow_control,
            log_capacity: overlay.log_capacity.unwrap_or(base.log_capacity),
            log_enabled: overlay.log_enabled.unwrap_or(base.log_enabled),
            reconnect_policy: overlay
                .reconnect_policy
                .clone()
                .unwrap_or_else(|| base.reconnect_policy.clone()),
            tx_framing: overlay
                .tx_framing
                .clone()
                .or_else(|| base.tx_framing.clone()),
            rx_framing: overlay
                .rx_framing
                .clone()
                .or_else(|| base.rx_framing.clone()),
            rx_parser: overlay.rx_parser.clone().or_else(|| base.rx_parser.clone()),
            protocol: overlay.protocol.or(base.protocol),
            rx_buffer_size,
            max_buffered_bytes: overlay
                .max_buffered_bytes
                .unwrap_or(base.max_buffered_bytes),
            poll_interval_ms: overlay.poll_interval_ms.unwrap_or(base.poll_interval_ms),
        })
    }

    /// The settings a profile alone would produce (no explicit overlay),
    /// used to detect whether explicit fields override the profile
    /// (`dirty`).
    pub fn from_profile(port: String, profile: &crate::profiles::Profile) -> Result<Self, String> {
        Self::resolve(port, &OpenOverlay::default(), Some(&profile.defaults))
    }

    /// Build the concrete `ConnectionConfig` for hardware open.
    pub fn into_connection_config(
        self,
        port_info: Option<crate::serial::PortInfo>,
    ) -> ConnectionConfig {
        ConnectionConfig {
            port: self.port,
            name: self.name,
            baud_rate: self.baud_rate,
            data_bits: self.data_bits,
            stop_bits: self.stop_bits,
            parity: self.parity,
            flow_control: self.flow_control,
            port_info,
            log_capacity: self.log_capacity,
            log_enabled: self.log_enabled,
            tx_framing: self.tx_framing,
            rx_framing: self.rx_framing,
            rx_parser: self.rx_parser,
            protocol: self.protocol,
            rx_buffer_size: self.rx_buffer_size,
            max_buffered_bytes: self.max_buffered_bytes,
            poll_interval_ms: self.poll_interval_ms,
        }
    }

    /// The effective settings as profile defaults (used for generated
    /// profiles, whose defaults equal the effective live open settings).
    pub fn as_profile_defaults(&self) -> crate::profiles::ProfileDefaults {
        crate::profiles::ProfileDefaults {
            baud_rate: self.baud_rate,
            data_bits: crate::serial::data_bits_to_str(self.data_bits),
            stop_bits: crate::serial::stop_bits_to_str(self.stop_bits),
            parity: crate::serial::parity_to_str(self.parity),
            flow_control: crate::serial::flow_control_to_str(self.flow_control),
            name: self.name.clone(),
            tx_framing: self.tx_framing.clone(),
            rx_framing: self.rx_framing.clone(),
            rx_parser: self.rx_parser.clone(),
            protocol: self.protocol,
            rx_buffer_size: self.rx_buffer_size,
            max_buffered_bytes: self.max_buffered_bytes,
            poll_interval_ms: self.poll_interval_ms,
            reconnect_policy: self.reconnect_policy.clone(),
            log_capacity: self.log_capacity,
            log_enabled: self.log_enabled,
        }
    }
}

/// Validate a resolved open `rx_buffer_size` (min 1, max 16 MiB ceiling).
fn validate_open_rx_buffer_size(size: usize) -> Result<usize, String> {
    use crate::limits::MAX_RX_BUFFER_SIZE;
    let size = require_min_or_err("open.rx_buffer_size", size, 1)?;
    clamp_or_err("open.rx_buffer_size", size, MAX_RX_BUFFER_SIZE)
}

pub fn parse_open_args(args: OpenArgs) -> Result<ConnectionConfig, String> {
    let overlay = OpenOverlay::from_open_args(&args);
    let port = args.port;
    let resolved = ResolvedOpenSettings::resolve(port, &overlay, None)?;
    Ok(resolved.into_connection_config(None))
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
            baud_rate: Some(115200),
            data_bits: Some("8".into()),
            stop_bits: Some("1".into()),
            parity: Some("none".into()),
            flow_control: Some("none".into()),
            log_capacity: Some(1024),
            log_enabled: Some(true),
            reconnect_policy: Some(Default::default()),
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: Some(crate::limits::DEFAULT_RX_BUFFER_SIZE),
            max_buffered_bytes: Some(32768),
            poll_interval_ms: Some(200),
            profile_mode: None,
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
            baud_rate: Some(9600),
            data_bits: Some("9".into()),
            stop_bits: Some("1".into()),
            parity: Some("none".into()),
            flow_control: Some("none".into()),
            log_capacity: Some(1024),
            log_enabled: Some(true),
            reconnect_policy: Some(Default::default()),
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: Some(crate::limits::DEFAULT_RX_BUFFER_SIZE),
            max_buffered_bytes: Some(32768),
            poll_interval_ms: Some(200),
            profile_mode: None,
        };
        let err = parse_open_args(args).unwrap_err();
        assert!(err.contains("data_bits"));
    }

    // ── Phase 3A: open-settings resolution precedence ─────────────────────

    #[test]
    fn omitted_open_fields_fall_back_to_builtin_defaults() {
        let args = OpenArgs {
            port: "/dev/ttyACM0".into(),
            name: None,
            baud_rate: None,
            data_bits: None,
            stop_bits: None,
            parity: None,
            flow_control: None,
            log_capacity: None,
            log_enabled: None,
            reconnect_policy: None,
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: None,
            max_buffered_bytes: None,
            poll_interval_ms: None,
            profile_mode: None,
        };
        let resolved = ResolvedOpenSettings::resolve(
            "/dev/ttyACM0".into(),
            &OpenOverlay::from_open_args(&args),
            None,
        )
        .unwrap();
        assert_eq!(resolved.baud_rate, 115200, "built-in 115200 fallback");
        assert_eq!(resolved.name, None);
        assert_eq!(resolved.log_capacity, 1024);
        assert_eq!(
            resolved.rx_buffer_size,
            crate::limits::DEFAULT_RX_BUFFER_SIZE
        );
        assert_eq!(resolved.max_buffered_bytes, 32768);
        assert_eq!(resolved.poll_interval_ms, 200);
        assert!(!resolved.reconnect_policy.enabled);
        assert_eq!(
            resolved.into_connection_config(None).baud_rate,
            115200,
            "config carries the resolved baud"
        );
    }

    #[test]
    fn explicit_open_field_overrides_profile_default() {
        let profile = crate::profiles::Profile {
            name: "p".into(),
            selector: Default::default(),
            defaults: crate::profiles::ProfileDefaults {
                baud_rate: 9600,
                rx_buffer_size: 8192,
                name: Some("console".into()),
                ..Default::default()
            },
            metadata: Default::default(),
            revisions: Vec::new(),
        };
        let args = OpenArgs {
            port: "/dev/ttyACM0".into(),
            baud_rate: Some(115200),
            name: None,
            data_bits: None,
            stop_bits: None,
            parity: None,
            flow_control: None,
            log_capacity: None,
            log_enabled: None,
            reconnect_policy: None,
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: None,
            max_buffered_bytes: None,
            poll_interval_ms: None,
            profile_mode: None,
        };
        let resolved = ResolvedOpenSettings::resolve(
            "/dev/ttyACM0".into(),
            &OpenOverlay::from_open_args(&args),
            Some(&profile.defaults),
        )
        .unwrap();
        assert_eq!(resolved.baud_rate, 115200, "explicit wins over profile");
        assert_eq!(resolved.rx_buffer_size, 8192, "omitted uses profile");
        assert_eq!(
            resolved.name.as_deref(),
            Some("console-ttyACM0"),
            "profile name prefix expanded"
        );
    }

    #[test]
    fn profile_only_settings_detect_dirty_overrides() {
        let profile = crate::profiles::Profile {
            name: "p".into(),
            selector: Default::default(),
            defaults: crate::profiles::ProfileDefaults {
                baud_rate: 9600,
                ..Default::default()
            },
            metadata: Default::default(),
            revisions: Vec::new(),
        };

        // Same effective settings as the profile → clean.
        let args = OpenArgs {
            port: "/dev/ttyACM0".into(),
            baud_rate: Some(9600),
            ..OpenArgs {
                port: "/dev/ttyACM0".into(),
                name: None,
                baud_rate: None,
                data_bits: None,
                stop_bits: None,
                parity: None,
                flow_control: None,
                log_capacity: None,
                log_enabled: None,
                reconnect_policy: None,
                tx_framing: None,
                rx_framing: None,
                rx_parser: None,
                protocol: None,
                rx_buffer_size: None,
                max_buffered_bytes: None,
                poll_interval_ms: None,
                profile_mode: None,
            }
        };
        let resolved = ResolvedOpenSettings::resolve(
            "/dev/ttyACM0".into(),
            &OpenOverlay::from_open_args(&args),
            Some(&profile.defaults),
        )
        .unwrap();
        let profile_only =
            ResolvedOpenSettings::from_profile("/dev/ttyACM0".into(), &profile).unwrap();
        assert_eq!(
            resolved, profile_only,
            "explicit value equal to profile is not dirty"
        );

        // Different baud → dirty.
        let args = OpenArgs {
            baud_rate: Some(19200),
            ..args
        };
        let resolved = ResolvedOpenSettings::resolve(
            "/dev/ttyACM0".into(),
            &OpenOverlay::from_open_args(&args),
            Some(&profile.defaults),
        )
        .unwrap();
        assert_ne!(resolved, profile_only, "explicit override differs → dirty");
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
        let resolved = validate_rx_request(&connections, &valid_args(&id), read_limits(), 256)
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
        let err = validate_rx_request(&connections, &a, read_limits(), 256)
            .await
            .unwrap_err();
        assert!(err.to_lowercase().contains("encoding"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_unknown_connection() {
        let connections = Arc::new(ConnectionManager::new());
        let err = validate_rx_request(&connections, &valid_args("nope"), read_limits(), 256)
            .await
            .unwrap_err();
        assert!(err.contains("Connection ID nope not found"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_buffered_below_min() {
        let (connections, id, _peer) = fake_conn().await;
        let a = valid_args(&id);
        let err = validate_rx_request(&connections, &a, read_limits(), 0)
            .await
            .unwrap_err();
        assert!(err.contains("read.max_buffered_bytes"), "got: {err}");
        assert!(err.contains("below minimum"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_buffered_above_max() {
        let (connections, id, _peer) = fake_conn().await;
        let a = valid_args(&id);
        let err = validate_rx_request(&connections, &a, read_limits(), MAX_READ_BYTES + 1)
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
        let err = validate_rx_request(&connections, &a, subscribe_limits, 256)
            .await
            .unwrap_err();
        assert_eq!(err, "subscribe.no_new_rx_timeout_ms must be > 0");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_oversized_timeout() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.timeout_ms = Some(MAX_TIMEOUT_MS + 1);
        let err = validate_rx_request(&connections, &a, read_limits(), 256)
            .await
            .unwrap_err();
        assert!(err.contains("read.timeout_ms"), "got: {err}");
        assert!(err.contains("exceeds maximum"), "got: {err}");
    }

    // ── Phase 5: private-cursor extraction ────────────────────────────────

    /// An already-cancelled request token routed through the private read
    /// path yields a STRUCTURED `cancelled` outcome (with offsets), not an
    /// ad-hoc error. This is the server-side behavior `capture_boot`
    /// produces for request cancellation during hold/settle.
    #[tokio::test]
    async fn cancelled_token_read_returns_structured_cancelled_outcome() {
        use crate::rx_session::RxSessionManager;
        use crate::serial::test_support::loopback_connection;
        use tokio::io::AsyncWriteExt;

        let connections = Arc::new(ConnectionManager::new());
        let (conn, mut peer) = loopback_connection("/dev/fake-cancel-read");
        let id = connections.insert(conn).await.unwrap();
        let connection = connections.get(&id).await.unwrap();

        let mgr = RxSessionManager::new(Arc::new(crate::buffer_budget::AtomicBudget::new(
            1 << 30,
            1 << 30,
        )));
        let session = mgr
            .get_or_create(Arc::clone(&connection), 4096)
            .await
            .unwrap();

        // Some bytes arrive and are consumed as history (no match), so the
        // cancelled outcome carries real offsets.
        peer.write_all(b"partial").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(session.ring().end_offset(), 7);

        let ct = tokio_util::sync::CancellationToken::new();
        ct.cancel(); // already cancelled, like a cancel-during-hold capture

        let matcher = crate::match_config::Matcher::new_literal(b"never".to_vec());
        let (outcome, _final) = read_from_private_cursor(
            &session,
            0,
            4096,
            Some(5000),
            &ct,
            None,
            None,
            matcher,
            None,
            Some(Arc::clone(&connection)),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome.meta.stop_reason.to_string(),
            "cancelled",
            "cancelled token must yield a structured cancelled outcome"
        );
        assert_eq!(
            outcome.bytes, b"partial",
            "history consumed before the cancel"
        );
        assert_eq!(
            outcome.from_offset,
            Some(0),
            "offsets carried in the outcome"
        );
        assert_eq!(outcome.next_offset, Some(7));

        session.shutdown_and_join().await;
    }

    /// The shared wrapper advances the shared cursor; the private form
    /// leaves it untouched. Both read the same bytes.
    #[tokio::test]
    async fn private_cursor_read_leaves_shared_cursor_unchanged() {
        use crate::rx_session::RxSessionManager;
        use crate::serial::test_support::loopback_connection;
        use tokio::io::AsyncWriteExt;

        let connections = Arc::new(ConnectionManager::new());
        let (conn, mut peer) = loopback_connection("/dev/fake-private-cursor");
        let id = connections.insert(conn).await.unwrap();
        let connection = connections.get(&id).await.unwrap();

        // Session with a real pump; feed bytes and wait for the ring.
        let mgr = RxSessionManager::new(Arc::new(crate::buffer_budget::AtomicBudget::new(
            1 << 30,
            1 << 30,
        )));
        let session = mgr
            .get_or_create(Arc::clone(&connection), 4096)
            .await
            .unwrap();
        peer.write_all(b"hello").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(session.ring().end_offset(), 5);

        let ct = tokio_util::sync::CancellationToken::new();

        // Private read from 0: gets the bytes, shared cursor untouched.
        let (outcome, final_cursor) = read_from_private_cursor(
            &session,
            0,
            4096,
            Some(500),
            &ct,
            None,
            None,
            None,
            None,
            Some(Arc::clone(&connection)),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.bytes, b"hello");
        assert_eq!(final_cursor, 5);
        assert_eq!(
            session.read_cursor(),
            0,
            "private read must not move the shared cursor"
        );

        // Shared wrapper: advances the shared cursor to the same offset.
        let outcome = read_bytes_from_ring(
            Arc::clone(&session),
            4096,
            Some(500),
            &ct,
            None,
            None,
            None,
            None,
            Some(Arc::clone(&connection)),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.bytes, b"hello");
        assert_eq!(
            session.read_cursor(),
            5,
            "shared wrapper must advance the cursor"
        );

        // And the shared cursor still reads forward from 5 (nothing new).
        let outcome = read_bytes_from_ring(
            Arc::clone(&session),
            4096,
            Some(100),
            &ct,
            None,
            None,
            None,
            None,
            Some(Arc::clone(&connection)),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.bytes.len(), 0);
        assert_eq!(outcome.meta.stop_reason.to_string(), "timeout");

        session.shutdown_and_join().await;
    }
}
