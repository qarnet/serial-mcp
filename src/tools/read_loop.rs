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
use crate::stop_controller::{RxStopController, RxStopDecision, RxStopOutcome};
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
// Ring-based read (private and shared cursor paths)
// ------------------------------------------------------------------

/// Advance a caller-owned cursor by `consumed` bytes from `base`, clamped to
/// the ring's live edge. Saturating by design; other branches keep their own
/// wrapping-plus-clamp formulas.
fn advance_private_cursor(base: u64, consumed: u64, ring: &crate::rx_ring::RxRing) -> u64 {
    base.saturating_add(consumed).min(ring.end_offset())
}

/// Owns all mutable state accumulated across a private read: raw bytes
/// consumed from the ring, returned payload bytes, decoded frames, and frame
/// accounting. Consumed only on return paths.
struct ReadAccumulator {
    /// Raw bytes consumed from the ring (drives cursor advancement and
    /// offsets). Returned bytes may differ after match-context shaping.
    consumed_offset: u64,
    returned_bytes: Vec<u8>,
    frames: Vec<crate::framing::Frame>,
    frames_seen: usize,
    frames_dropped: usize,
    frame_error: Option<String>,
}

impl ReadAccumulator {
    fn new(max_bytes: usize) -> Self {
        Self {
            consumed_offset: 0,
            returned_bytes: Vec::with_capacity(max_bytes),
            frames: Vec::new(),
            frames_seen: 0,
            frames_dropped: 0,
            frame_error: None,
        }
    }

    /// Consume the accumulator into a generic `ReadOutcome` plus the final
    /// private cursor. Offsets derive from the ring's current start/end, the
    /// ORIGINAL private cursor, and raw `consumed_offset`.
    /// `payload_override` changes only the returned bytes (framed match data,
    /// shaped live-match context) — never the offsets.
    #[allow(clippy::too_many_arguments)]
    fn into_outcome(
        self,
        ring: &crate::rx_ring::RxRing,
        original_cursor: u64,
        max_bytes: usize,
        elapsed_ms: u64,
        meta: RxStopMetadata,
        matched: bool,
        match_index: Option<usize>,
        match_frame_index: Option<usize>,
        payload_override: Option<Vec<u8>>,
        final_cursor: u64,
    ) -> (ReadOutcome, u64) {
        let start_off = ring.start_offset();
        let end_off = ring.end_offset();
        let clamped_from = original_cursor.max(start_off).min(end_off);
        let bytes_lost = start_off.saturating_sub(original_cursor);
        let used = self.consumed_offset.min(max_bytes as u64);
        let next_off = clamped_from + used;
        let from_off = if self.returned_bytes.is_empty() && self.consumed_offset == 0 {
            None
        } else {
            Some(clamped_from)
        };
        let next_off_out = if self.returned_bytes.is_empty() && self.consumed_offset == 0 {
            None
        } else {
            Some(next_off)
        };
        let buffered_remaining = end_off.saturating_sub(next_off);
        (
            ReadOutcome {
                bytes: payload_override.unwrap_or(self.returned_bytes),
                elapsed_ms,
                meta,
                matched,
                match_index,
                match_frame_index,
                frames: self.frames,
                frames_dropped: self.frames_dropped,
                error: self.frame_error,
                from_offset: from_off,
                next_offset: next_off_out,
                bytes_lost,
                buffered_remaining,
                start_offset: ring.start_offset(),
                end_offset: ring.end_offset(),
            },
            final_cursor,
        )
    }
}

/// Central completion descriptor for a private read. Every ordinary return
/// path builds one and funnels it through [`complete_read`]. The payload
/// override changes only the returned bytes (framed match data, shaped
/// live-match context) — never the consumed offsets.
struct ReadCompletion {
    meta: RxStopMetadata,
    matched: bool,
    match_index: Option<usize>,
    match_frame_index: Option<usize>,
    payload_override: Option<Vec<u8>>,
    final_cursor: u64,
}

/// Consume the accumulator into the final outcome via
/// [`ReadAccumulator::into_outcome`], computing elapsed time here so every
/// ordinary return path shares one call site.
fn complete_read(
    acc: ReadAccumulator,
    ring: &crate::rx_ring::RxRing,
    original_cursor: u64,
    max_bytes: usize,
    read_start: Instant,
    completion: ReadCompletion,
) -> (ReadOutcome, u64) {
    acc.into_outcome(
        ring,
        original_cursor,
        max_bytes,
        read_start.elapsed().as_millis() as u64,
        completion.meta,
        completion.matched,
        completion.match_index,
        completion.match_frame_index,
        completion.payload_override,
        completion.final_cursor,
    )
}

/// What processing one framed chunk produced. `Match` carries the first
/// framed match's payload and indexes; `Stop` carries the translated stop
/// outcome (max_frames, runtime decode error, sink stop) plus the sink's
/// match frame index (always `None` there — a match would have returned
/// earlier).
enum FramedChunkDecision {
    /// No stop condition; keep the outer loop running.
    Continue,
    /// The first framed match: payload plus match byte/frame indexes.
    Match {
        data: Vec<u8>,
        match_index: Option<usize>,
        match_frame_index: Option<usize>,
    },
    /// A stop outcome from `frame_outcome_to_stop`, with the sink's match
    /// frame index preserved.
    Stop {
        outcome: RxStopOutcome,
        match_frame_index: Option<usize>,
    },
}

/// Decode one chunk through the shared `consume_frames` pipeline, preserving
/// decoded frames before a fatal error, the first framed match, and the
/// `frame_outcome_to_stop` translation with its `frame_error` text. Shared by
/// the initial buffered-bytes path and the live wait loop.
async fn process_framed_chunk(
    chunk: &[u8],
    decoder: &mut crate::framing::FrameDecoder,
    matcher: &mut Option<Matcher>,
    max_frames: Option<usize>,
    acc: &mut ReadAccumulator,
    ctrl: &RxStopController,
    conn_id: &str,
) -> FramedChunkDecision {
    let mut sink = ReadFrameSink::new(&mut acc.frames);
    let outcome = consume_frames(
        chunk,
        decoder,
        matcher,
        max_frames,
        &mut acc.frames_seen,
        &mut sink,
        &mut acc.frames_dropped,
    )
    .await;
    let ReadFrameSink {
        match_data,
        match_index,
        match_frame_index,
        ..
    } = sink;
    if let Some(data) = match_data {
        return FramedChunkDecision::Match {
            data,
            match_index,
            match_frame_index,
        };
    }
    if let Some(stop) = frame_outcome_to_stop(
        outcome,
        ctrl,
        acc.returned_bytes.len(),
        match_index,
        &mut acc.frame_error,
        conn_id,
    ) {
        return FramedChunkDecision::Stop {
            outcome: stop,
            match_frame_index,
        };
    }
    FramedChunkDecision::Continue
}

/// What woke the read loop from its ring wait.
enum WaitWake {
    /// The request cancellation token fired.
    Cancelled,
    /// New data arrived on the ring at/after the clocked cursor.
    Data,
    /// Adaptive poll fired so the loop can re-check timeouts/stop conditions.
    Poll,
}

/// Wait for ring data, cancellation, or a short adaptive poll. Preserves the
/// deadline source (`ctrl.deadline()`, falling back to `effective_timeout_ms`
/// from the read start), the 1–250 ms adaptive poll, and cancellation
/// precedence through the `tokio::select!` branch order.
async fn wait_for_ring_data(
    ct: &tokio_util::sync::CancellationToken,
    ring: &crate::rx_ring::RxRing,
    clocked_cursor: u64,
    ctrl: &RxStopController,
    read_start: Instant,
    effective_timeout_ms: u64,
) -> WaitWake {
    let deadline = ctrl
        .deadline()
        .unwrap_or_else(|| read_start + Duration::from_millis(effective_timeout_ms));
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .as_millis() as u64;
    let poll_ms = remaining.saturating_sub(1).clamp(1, 250); // adaptive: 1-250ms
    tokio::select! {
        _ = ct.cancelled() => WaitWake::Cancelled,
        _ = ring.wait_for_data(clocked_cursor) => WaitWake::Data,
        _ = tokio::time::sleep(std::time::Duration::from_millis(poll_ms)) => WaitWake::Poll,
    }
}

/// What the initial buffered-slice phase decided.
enum InitialDecision {
    /// Proceed to the wait/process loop.
    Continue,
    /// An ordinary completion (cat path, historical match, initial framed
    /// match/stop): complete through the driver.
    Completion(ReadCompletion),
    /// The special historical shaped-context result: a fully built outcome
    /// with snapshot-derived offsets plus its final cursor. Returned without
    /// consuming the accumulator.
    HistoricalContextMatch((ReadOutcome, u64)),
}

/// What processing one live chunk decided. The completion is boxed to keep
/// the variant small (clippy `large_enum_variant`).
enum LiveChunkDecision {
    /// Keep looping from the chunk's next offset.
    Continue { next_clocked_cursor: u64 },
    /// Stop with this completion (framed/raw match, framed stop, or
    /// max-bytes drained).
    Completion(Box<ReadCompletion>),
}

/// Owns all mutable state for one private read: ring reference, cursor math,
/// timeout/controller state, matcher/decoder, connection, and the
/// accumulator. Built once per call by [`read_from_private_cursor`];
/// [`ReadDriver::run_wait`] consumes it.
struct ReadDriver<'a> {
    ring: &'a crate::rx_ring::RxRing,
    original_cursor: u64,
    max_bytes: usize,
    read_start: Instant,
    effective_timeout_ms: u64,
    ct: &'a tokio_util::sync::CancellationToken,
    ctrl: RxStopController,
    matcher: Option<Matcher>,
    decoder: Option<crate::framing::FrameDecoder>,
    max_frames: Option<usize>,
    conn: Option<Arc<SerialConnection>>,
    conn_id: String,
    framing: Option<crate::framing::RxFramingConfig>,
    parser: Option<crate::framing::ParserConfig>,
    acc: ReadAccumulator,
}

impl<'a> ReadDriver<'a> {
    /// Consume the driver and produce the final outcome for `completion`.
    fn complete(self, completion: ReadCompletion) -> (ReadOutcome, u64) {
        complete_read(
            self.acc,
            self.ring,
            self.original_cursor,
            self.max_bytes,
            self.read_start,
            completion,
        )
    }

    /// Process the initial buffered slice: build the frame decoder, run the
    /// raw cat path, the historical match path (including the special
    /// snapshot-derived shaped-context result), and the initial framed
    /// bytes. Returns `Continue` to enter the wait loop, an ordinary
    /// completion, or the special historical context result.
    async fn process_initial(
        &mut self,
        initial_slice: &crate::rx_ring::RingSlice,
    ) -> Result<InitialDecision, String> {
        // Frame decoder construction errors are tool errors.
        self.decoder = match self.framing.as_ref() {
            Some(cfg) => Some(crate::framing::FrameDecoder::new(
                cfg,
                self.parser.as_ref(),
            )?),
            None => None,
        };
        let max_bytes = self.max_bytes;
        let context_amount = self.matcher.as_ref().and_then(|m| m.context_amount());
        let needle_len = self.matcher.as_ref().and_then(|m| m.needle_len());

        // Immediate raw cat path — no match, bytes available, no framing.
        let has_immediate_data = !initial_slice.bytes.is_empty() && self.matcher.is_none();
        if has_immediate_data && self.decoder.is_none() {
            let take = initial_slice.bytes.len().min(max_bytes);
            self.acc.returned_bytes = initial_slice.bytes[..take].to_vec();
            let consumed = self.acc.returned_bytes.len() as u64;
            self.acc.consumed_offset = consumed;
            let meta = RxStopMetadata::drained(
                self.original_cursor + consumed,
                consumed as usize,
                consumed as usize,
            );
            let final_cursor =
                advance_private_cursor(initial_slice.from_offset, consumed, self.ring);
            return Ok(InitialDecision::Completion(ReadCompletion {
                meta,
                matched: false,
                match_index: None,
                match_frame_index: None,
                payload_override: None,
                final_cursor,
            }));
        }

        // Match-check history first if a matcher is present.
        if self.matcher.is_some() && !initial_slice.bytes.is_empty() {
            let take = initial_slice.bytes.len().min(max_bytes);
            let hist = &initial_slice.bytes[..take];
            // Bounded push: same matcher-owned window policy as read's live
            // path. The initial slice is at most `max_bytes`, so no
            // truncation occurs here and the history match stays exact.
            let match_result = self
                .matcher
                .as_mut()
                .map(|m| m.push_bounded(hist, max_bytes));
            if let Some(MatchResult::Found(idx)) = match_result {
                let match_end = idx + needle_len.unwrap_or(0);
                let consumed = match_end as u64;
                self.acc.returned_bytes = hist[..match_end].to_vec();
                let meta = RxStopMetadata::match_found(consumed as usize, consumed as usize);
                // Historical literal-context match: offsets stay on the
                // initial slice snapshot (from_offset/bytes_lost), not the
                // live ring.
                if let Some(context) = context_amount {
                    let shaped =
                        shape_match_context(hist, idx, needle_len.unwrap_or(0), Some(context));
                    let shaped_consumed = shaped.data.len() as u64;
                    let final_cursor =
                        advance_private_cursor(initial_slice.from_offset, consumed, self.ring);
                    return Ok(InitialDecision::HistoricalContextMatch((
                        ReadOutcome {
                            bytes: shaped.data,
                            elapsed_ms: self.read_start.elapsed().as_millis() as u64,
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
                            buffered_remaining: self
                                .ring
                                .end_offset()
                                .saturating_sub(initial_slice.from_offset + consumed),
                            start_offset: self.ring.start_offset(),
                            end_offset: self.ring.end_offset(),
                        },
                        final_cursor,
                    )));
                }
                let final_cursor =
                    advance_private_cursor(initial_slice.from_offset, consumed, self.ring);
                return Ok(InitialDecision::Completion(ReadCompletion {
                    meta,
                    matched: true,
                    match_index: Some(idx),
                    match_frame_index: None,
                    payload_override: None,
                    final_cursor,
                }));
            }
            // Not found in history — consume what we read from the ring for
            // the result so far.
            self.acc.consumed_offset = take as u64;
            self.acc.returned_bytes = hist.to_vec();
            self.ctrl.notify_data_received();
            self.ctrl.push_data(take, take, Some(MatchResult::NoMatch));
        }

        // Initial framed bytes.
        if self.decoder.is_some() && !initial_slice.bytes.is_empty() {
            let take = initial_slice.bytes.len().min(max_bytes);
            let chunk = &initial_slice.bytes[..take];
            self.acc.consumed_offset += take as u64;
            self.acc.returned_bytes.extend_from_slice(chunk);

            if let Some(ref mut dec) = self.decoder {
                match process_framed_chunk(
                    chunk,
                    dec,
                    &mut self.matcher,
                    self.max_frames,
                    &mut self.acc,
                    &self.ctrl,
                    &self.conn_id,
                )
                .await
                {
                    FramedChunkDecision::Continue => {}
                    FramedChunkDecision::Match {
                        data,
                        match_index,
                        match_frame_index,
                    } => {
                        let meta = RxStopMetadata::match_found(
                            self.ctrl.bytes_observed(),
                            self.acc.returned_bytes.len(),
                        );
                        let final_cursor = initial_slice
                            .from_offset
                            .wrapping_add(self.acc.consumed_offset)
                            .min(self.ring.end_offset());
                        return Ok(InitialDecision::Completion(ReadCompletion {
                            meta,
                            matched: true,
                            match_index,
                            match_frame_index,
                            payload_override: Some(data),
                            final_cursor,
                        }));
                    }
                    FramedChunkDecision::Stop {
                        outcome,
                        match_frame_index,
                    } => {
                        let final_cursor = initial_slice
                            .from_offset
                            .wrapping_add(self.acc.consumed_offset)
                            .min(self.ring.end_offset());
                        return Ok(InitialDecision::Completion(ReadCompletion {
                            meta: outcome.meta,
                            matched: outcome.matched,
                            match_index: outcome.match_index,
                            match_frame_index,
                            payload_override: None,
                            final_cursor,
                        }));
                    }
                }
            }
        }

        Ok(InitialDecision::Continue)
    }

    /// Process one live slice: accumulate bytes, feed the framed pipeline or
    /// the raw matcher with context shaping, then decide whether to continue
    /// from the slice's next offset or complete.
    async fn process_live_chunk(&mut self, slice: &crate::rx_ring::RingSlice) -> LiveChunkDecision {
        let max_bytes = self.max_bytes;
        self.ctrl.notify_data_received();
        let take = slice
            .bytes
            .len()
            .min(max_bytes.saturating_sub(self.acc.returned_bytes.len()));
        let chunk = &slice.bytes[..take];
        self.acc.returned_bytes.extend_from_slice(chunk);
        self.acc.consumed_offset = self
            .acc
            .consumed_offset
            .wrapping_add(take as u64)
            .min(max_bytes as u64);

        // Feed to frame decoder if active.
        if let Some(ref mut dec) = self.decoder {
            match process_framed_chunk(
                chunk,
                dec,
                &mut self.matcher,
                self.max_frames,
                &mut self.acc,
                &self.ctrl,
                &self.conn_id,
            )
            .await
            {
                FramedChunkDecision::Continue => {}
                FramedChunkDecision::Match {
                    data,
                    match_index,
                    match_frame_index,
                } => {
                    let meta = RxStopMetadata::match_found(
                        self.ctrl.bytes_observed(),
                        self.acc.returned_bytes.len(),
                    );
                    let final_cursor = slice
                        .from_offset
                        .wrapping_add(take as u64)
                        .min(self.ring.end_offset());
                    return LiveChunkDecision::Completion(Box::new(ReadCompletion {
                        meta,
                        matched: true,
                        match_index,
                        match_frame_index,
                        payload_override: Some(data),
                        final_cursor,
                    }));
                }
                FramedChunkDecision::Stop {
                    outcome,
                    match_frame_index,
                } => {
                    let final_cursor = slice
                        .from_offset
                        .wrapping_add(take as u64)
                        .min(self.ring.end_offset());
                    return LiveChunkDecision::Completion(Box::new(ReadCompletion {
                        meta: outcome.meta,
                        matched: outcome.matched,
                        match_index: outcome.match_index,
                        match_frame_index,
                        payload_override: None,
                        final_cursor,
                    }));
                }
            }
        }

        // Raw matcher path (no framing).
        if self.decoder.is_none() {
            // Bounded push: same matcher-owned window policy as the
            // initial-history path and read path.
            let match_result = self
                .matcher
                .as_mut()
                .map(|m| m.push_bounded(chunk, max_bytes));
            let buffered_len = self.acc.returned_bytes.len();
            let data_count = chunk.len();
            if let RxStopDecision::Stop(outcome) =
                self.ctrl.push_data(data_count, buffered_len, match_result)
            {
                // Live matches apply matcher-owned context shaping (same
                // policy as read path. Only the returned payload and the
                // relative match_index change — cursor consumption and the
                // stream offsets stay based on the consumed bytes.
                let (match_bytes, match_index) = match outcome.match_index {
                    Some(idx) => match self
                        .matcher
                        .as_ref()
                        .and_then(|m| m.shape_literal_match_context(idx))
                    {
                        Some(shaped) => (shaped.data, Some(shaped.match_index)),
                        None => (self.acc.returned_bytes.clone(), Some(idx)),
                    },
                    None => (self.acc.returned_bytes.clone(), None),
                };
                let final_cursor = slice
                    .from_offset
                    .wrapping_add(take as u64)
                    .min(self.ring.end_offset());
                return LiveChunkDecision::Completion(Box::new(ReadCompletion {
                    meta: outcome.meta,
                    matched: outcome.matched,
                    match_index,
                    match_frame_index: None,
                    payload_override: Some(match_bytes),
                    final_cursor,
                }));
            }
        }

        // max_bytes reached -> drained
        if self.acc.returned_bytes.len() >= max_bytes {
            let meta = RxStopMetadata::drained(
                self.original_cursor.wrapping_add(self.acc.consumed_offset),
                self.acc.returned_bytes.len(),
                self.acc.returned_bytes.len(),
            );
            let final_cursor =
                advance_private_cursor(self.original_cursor, self.acc.consumed_offset, self.ring);
            return LiveChunkDecision::Completion(Box::new(ReadCompletion {
                meta,
                matched: false,
                match_index: None,
                match_frame_index: None,
                payload_override: None,
                final_cursor,
            }));
        }

        LiveChunkDecision::Continue {
            next_clocked_cursor: slice.next_offset,
        }
    }

    /// Drive the wait/process loop until a stop condition fires: pause during
    /// disconnect/reconnect, check timeout/silence, wait for data/cancel/poll,
    /// handle spurious wakeups, and delegate non-empty slices to
    /// [`ReadDriver::process_live_chunk`]. Consumes the driver; the final
    /// completion consumes the accumulator.
    async fn run_wait(mut self, mut clocked_cursor: u64) -> Result<(ReadOutcome, u64), String> {
        loop {
            // Pause timeouts while connection is disconnected/reconnecting.
            if let Some(ref conn) = self.conn {
                match disconnect_state(conn, &mut self.ctrl) {
                    DisconnectState::Closed => {
                        let outcome = self.ctrl.connection_closed();
                        let final_cursor = self
                            .original_cursor
                            .wrapping_add(self.acc.consumed_offset)
                            .min(self.ring.end_offset());
                        return Ok(self.complete(ReadCompletion {
                            meta: outcome.meta,
                            matched: outcome.matched,
                            match_index: outcome.match_index,
                            match_frame_index: None,
                            payload_override: None,
                            final_cursor,
                        }));
                    }
                    DisconnectState::Reconnecting => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        continue;
                    }
                    DisconnectState::Active => {}
                }
            }

            if let RxStopDecision::Stop(outcome) = self.ctrl.check_timeout() {
                let final_cursor = advance_private_cursor(
                    self.original_cursor,
                    self.acc.consumed_offset,
                    self.ring,
                );
                return Ok(self.complete(ReadCompletion {
                    meta: outcome.meta,
                    matched: outcome.matched,
                    match_index: outcome.match_index,
                    match_frame_index: None,
                    payload_override: None,
                    final_cursor,
                }));
            }
            if let RxStopDecision::Stop(outcome) = self.ctrl.check_silence_timeout() {
                let final_cursor = advance_private_cursor(
                    self.original_cursor,
                    self.acc.consumed_offset,
                    self.ring,
                );
                return Ok(self.complete(ReadCompletion {
                    meta: outcome.meta,
                    matched: outcome.matched,
                    match_index: outcome.match_index,
                    match_frame_index: None,
                    payload_override: None,
                    final_cursor,
                }));
            }

            // Wait for more data on the ring, or a short poll to check timeouts.
            match wait_for_ring_data(
                self.ct,
                self.ring,
                clocked_cursor,
                &self.ctrl,
                self.read_start,
                self.effective_timeout_ms,
            )
            .await
            {
                WaitWake::Cancelled => {
                    let outcome = self.ctrl.cancelled();
                    let final_cursor = self
                        .original_cursor
                        .wrapping_add(self.acc.consumed_offset)
                        .min(self.ring.end_offset());
                    return Ok(self.complete(ReadCompletion {
                        meta: outcome.meta,
                        matched: outcome.matched,
                        match_index: outcome.match_index,
                        match_frame_index: None,
                        payload_override: None,
                        final_cursor,
                    }));
                }
                WaitWake::Data => {}
                WaitWake::Poll => {
                    // Poll wakeup: loop back to check timeouts and stop conditions.
                    continue;
                }
            }

            // New data arrived — read from ring at the clocked cursor.
            let slice = self.ring.read_from(
                clocked_cursor,
                self.max_bytes.saturating_sub(self.acc.returned_bytes.len()),
            );
            if slice.bytes.is_empty() {
                continue; // spurious wakeup
            }

            match self.process_live_chunk(&slice).await {
                LiveChunkDecision::Continue {
                    next_clocked_cursor,
                } => {
                    clocked_cursor = next_clocked_cursor;
                }
                LiveChunkDecision::Completion(completion) => {
                    return Ok(self.complete(*completion));
                }
            }
        }
    }
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
    matcher: Option<Matcher>,
    no_new_rx_timeout_ms: Option<u64>,
    conn: Option<Arc<SerialConnection>>,
    framing: Option<crate::framing::RxFramingConfig>,
    parser: Option<crate::framing::ParserConfig>,
) -> Result<(ReadOutcome, u64), String> {
    // Phase 1: resolve timing, ring, initial slice, and driver state.
    let effective_timeout_ms = timeout_ms.unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    let read_start = Instant::now();
    let ring = session.ring();
    let initial_slice = ring.read_from(initial_cursor, max_bytes);
    let max_frames = framing.as_ref().and_then(|f| f.max_frames);
    let conn_id = session.connection_id().to_string();

    let mut driver = ReadDriver {
        ring,
        original_cursor: initial_cursor,
        max_bytes,
        read_start,
        effective_timeout_ms,
        ct,
        ctrl: RxStopController::new(read_start, timeout_ms, max_bytes, no_new_rx_timeout_ms),
        matcher,
        decoder: None,
        max_frames,
        conn,
        conn_id,
        framing,
        parser,
        acc: ReadAccumulator::new(max_bytes),
    };

    // Phases 2–4: raw cat path, historical match path, initial framed bytes.
    match driver.process_initial(&initial_slice).await? {
        InitialDecision::Continue => {}
        InitialDecision::Completion(completion) => return Ok(driver.complete(completion)),
        InitialDecision::HistoricalContextMatch((outcome, final_cursor)) => {
            return Ok((outcome, final_cursor));
        }
    }

    // Phase 5: wait/process loop.
    driver.run_wait(initial_slice.next_offset).await
}

/// Drive a `read` from the ring buffer with the SHARED read cursor: reads
/// the current cursor, delegates to [`read_from_private_cursor`], and
/// applies the returned final cursor. `read`/`transact` call this wrapper.
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

    // ── Private/shared cursor behavior ────────────────────────────────────

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

        let mgr = RxSessionManager::new(
            Arc::new(crate::buffer_budget::AtomicBudget::new(1 << 30, 1 << 30)),
            Arc::new(crate::resource_events::ResourceEventHub::new(64)),
        );
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
        let mgr = RxSessionManager::new(
            Arc::new(crate::buffer_budget::AtomicBudget::new(1 << 30, 1 << 30)),
            Arc::new(crate::resource_events::ResourceEventHub::new(64)),
        );
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

    /// `complete_read` (and the `into_outcome` it drives) advances
    /// `next_offset` by raw `consumed_offset` even when the returned payload
    /// is overridden with a different length (framed match data, shaped
    /// live-match context). Offsets are consumption-based, not
    /// payload-length-based.
    #[test]
    fn into_outcome_offsets_follow_consumed_bytes_not_payload_length() {
        let ring = crate::rx_ring::RxRing::new(64);
        ring.append(&[0u8; 60]); // end_offset 60, start_offset 0

        let mut acc = ReadAccumulator::new(4096);
        acc.consumed_offset = 10;
        acc.returned_bytes = vec![0u8; 40];

        let (outcome, final_cursor) = complete_read(
            acc,
            &ring,
            0,
            4096,
            Instant::now(),
            ReadCompletion {
                meta: RxStopMetadata::match_found(40, 5),
                matched: true,
                match_index: Some(0),
                match_frame_index: None,
                payload_override: Some(vec![0u8; 5]),
                final_cursor: 10,
            },
        );

        assert_eq!(outcome.bytes.len(), 5, "override replaces the payload");
        assert_eq!(
            outcome.next_offset,
            Some(10),
            "offset advances by consumed bytes, not payload length"
        );
        assert_eq!(outcome.from_offset, Some(0));
        assert_eq!(outcome.bytes_lost, 0);
        assert_eq!(outcome.buffered_remaining, 50);
        assert_eq!(final_cursor, 10);
    }
}
