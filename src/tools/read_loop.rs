//! Ring-based read driving shared by `read`, `transact`, and `capture_boot`:
//! the `ReadOutcome` result shape, the private `ReadFrameSink`, the
//! private-cursor read core (`read_from_private_cursor`, also used by
//! `capture_boot`), and the shared-cursor wrapper `read_bytes_from_ring`.
//! Wire result construction into `ReadResult` lives in the sibling
//! `result_builders` module.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{model::ProgressToken, service::Peer, RoleServer};

use crate::match_config::{shape_match_context, MatchResult, Matcher};
use crate::rx_metadata::RxStopMetadata;
use crate::rx_session::RxSession;
use crate::serial::SerialConnection;
use crate::stop_controller::{RxStopController, RxStopDecision};
use crate::tools::helpers::DEFAULT_READ_TIMEOUT_MS;
use crate::tools::rx_consume::{
    consume_frames, disconnect_state, frame_outcome_to_stop, DisconnectState, RxFrameSink, SinkFlow,
};

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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::serial::ConnectionManager;

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
