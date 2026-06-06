//! Shared stop-decision controller for all RX operations.
//!
//! PLAN 5 introduces a single `RxStopController` that evaluates stop conditions
//! for both `read` and `subscribe`. Same inputs must yield the same stop reason
//! regardless of which tool calls in.
//!
//! The controller is *stateless* regarding event sourcing — it does not own
//! channels, loops, or notification logic. It only tracks counters and deadlines
//! and answers: "should the operation continue, and if not, why?"
//!
//! ## Normal vs Error stop reasons
//!
//! Normal stops (successful outcomes):
//! - `match_found`  — pattern matched in the byte stream
//! - `timeout`      — total wall-clock budget elapsed
//! - `max_buffered_bytes` — buffer budget reached
//! - `data_complete` — settle phase completed (read without matcher)
//!
//! Error stops (something went wrong):
//! - `connection_closed` — underlying serial port closed
//! - `cancelled`        — MCP client cancelled the request
//! - `read_error`       — I/O error on the serial port
//! - `channel_closed`   — RX pump channel closed (internal)
//! - `peer_disconnected` — MCP peer went away during streaming
//!
//! Error stops return `Err(String)` from `read` and `wait_for`
//! tool handlers. `subscribe` logs them as the stop reason in the
//! final notification. Normal stops always produce `Ok` results.

use std::time::Instant;

use crate::match_config::MatchResult;
use crate::rx_metadata::{RxStopMetadata, RxStopReason};

/// Decision returned by [`RxStopController`] after evaluating the current state.
#[derive(Debug)]
pub enum RxStopDecision {
    /// No stop condition has triggered; continue collecting data.
    Continue,
    /// A stop condition was met. Contains the outcome metadata.
    Stop(RxStopOutcome),
}

/// Outcome produced when a stop condition triggers.
///
/// Carries the stop metadata plus optional match information.
#[derive(Debug)]
pub struct RxStopOutcome {
    pub meta: RxStopMetadata,
    pub matched: bool,
    pub match_index: Option<usize>,
}

/// Shared stop-decision controller for all RX operations.
///
/// Both `read` and `subscribe` call into the same controller instance for
/// the duration of one operation. The controller tracks:
/// - deadline (from `timeout_ms`)
/// - byte counters (observed and returned)
/// - max buffer limit
/// - match state
pub struct RxStopController {
    deadline: Option<Instant>,
    max_bytes: usize,
    bytes_observed: usize,
    bytes_returned: usize,
    matched: bool,
    match_index: Option<usize>,
}

impl RxStopController {
    /// Create a new controller.
    ///
    /// `start` is the instant the operation began (for deadline computation).
    /// `timeout_ms` is the optional total wall-clock budget in milliseconds.
    /// `max_bytes` is the buffer budget limit.
    pub fn new(start: Instant, timeout_ms: Option<u64>, max_bytes: usize) -> Self {
        let deadline = timeout_ms.map(|ms| start + std::time::Duration::from_millis(ms));
        Self {
            deadline,
            max_bytes,
            bytes_observed: 0,
            bytes_returned: 0,
            matched: false,
            match_index: None,
        }
    }

    /// Check whether the total timeout deadline has passed.
    ///
    /// Returns `Stop(RxStopOutcome)` with `RxStopReason::Timeout` if the
    /// deadline has been reached, otherwise `Continue`.
    pub fn check_timeout(&self) -> RxStopDecision {
        if let Some(dl) = self.deadline {
            if Instant::now() >= dl {
                return RxStopDecision::Stop(RxStopOutcome {
                    meta: RxStopMetadata::timeout(self.bytes_observed)
                        .with_bytes(self.bytes_observed, self.bytes_returned),
                    matched: self.matched,
                    match_index: self.match_index,
                });
            }
        }
        RxStopDecision::Continue
    }

    /// Record incoming data and evaluate stop conditions.
    ///
    /// Call this after receiving a data chunk. The caller provides:
    /// - `chunk_len`: how many bytes arrived in this chunk
    /// - `buffered_len`: total bytes currently in the accumulation buffer
    /// - `match_result`: `Some(result)` if a matcher is active, `None` otherwise
    ///
    /// The controller updates its counters and checks:
    /// 1. Whether the matcher found a pattern (→ `MatchFound`)
    /// 2. Whether `max_buffered_bytes` was reached (→ `MaxBufferedBytes`)
    ///
    /// Returns `Continue` if no stop condition triggered.
    pub fn push_data(
        &mut self,
        chunk_len: usize,
        buffered_len: usize,
        match_result: Option<MatchResult>,
    ) -> RxStopDecision {
        self.bytes_observed += chunk_len;
        self.bytes_returned = buffered_len;

        if let Some(MatchResult::Found(idx)) = match_result {
            self.matched = true;
            self.match_index = Some(idx);
            return RxStopDecision::Stop(RxStopOutcome {
                meta: RxStopMetadata::match_found(self.bytes_observed, self.bytes_returned),
                matched: self.matched,
                match_index: self.match_index,
            });
        }

        if buffered_len >= self.max_bytes {
            let meta = RxStopMetadata::max_buffered_bytes(self.bytes_observed, self.bytes_returned);
            return RxStopDecision::Stop(RxStopOutcome {
                meta,
                matched: self.matched,
                match_index: self.match_index,
            });
        }

        RxStopDecision::Continue
    }

    /// Record a data chunk without checking stop conditions (for settle phase
    /// or streaming data notifications where the caller decides when to stop).
    ///
    /// Only updates byte counters; does not evaluate matcher or max_buffered_bytes.
    /// The caller must call `check_timeout` or `check_max_buffered_bytes` next.
    pub fn record_data(&mut self, chunk_len: usize, buffered_len: usize) {
        self.bytes_observed += chunk_len;
        self.bytes_returned = buffered_len;
    }

    /// Check whether the buffer has reached `max_buffered_bytes`.
    ///
    /// Useful for settle-phase evaluation where data was recorded separately
    /// from the stop check.
    pub fn check_max_buffered_bytes(&self) -> RxStopDecision {
        if self.bytes_returned >= self.max_bytes && self.bytes_observed > 0 {
            RxStopDecision::Stop(RxStopOutcome {
                meta: RxStopMetadata::max_buffered_bytes(self.bytes_observed, self.bytes_returned),
                matched: self.matched,
                match_index: self.match_index,
            })
        } else {
            RxStopDecision::Continue
        }
    }

    /// Produce the stop outcome for a connection-closed event.
    pub fn connection_closed(&self) -> RxStopOutcome {
        RxStopOutcome {
            meta: RxStopMetadata::connection_closed(self.bytes_observed, self.bytes_returned),
            matched: self.matched,
            match_index: self.match_index,
        }
    }

    /// Produce the stop outcome for a channel-closed event (pump exited).
    pub fn channel_closed(&self) -> RxStopOutcome {
        RxStopOutcome {
            meta: RxStopMetadata::channel_closed()
                .with_bytes(self.bytes_observed, self.bytes_returned),
            matched: self.matched,
            match_index: self.match_index,
        }
    }

    /// Produce the stop outcome for a client-initiated cancellation.
    pub fn cancelled(&self) -> RxStopOutcome {
        RxStopOutcome {
            meta: RxStopMetadata::cancelled().with_bytes(self.bytes_observed, self.bytes_returned),
            matched: self.matched,
            match_index: self.match_index,
        }
    }

    /// Produce the stop outcome for a serial-port read error.
    pub fn read_error(&self) -> RxStopOutcome {
        RxStopOutcome {
            meta: RxStopMetadata::read_error().with_bytes(self.bytes_observed, self.bytes_returned),
            matched: self.matched,
            match_index: self.match_index,
        }
    }

    /// Produce the stop outcome for a peer disconnected during streaming.
    pub fn peer_disconnected(&self) -> RxStopOutcome {
        RxStopOutcome {
            meta: RxStopMetadata::peer_disconnected(self.bytes_observed),
            matched: self.matched,
            match_index: self.match_index,
        }
    }

    /// Build a `DataComplete` outcome for the settle phase of a non-matcher read.
    ///
    /// This is only called by the read path when no matcher is active and the
    /// settle phase ends naturally.
    pub fn data_complete(&self) -> RxStopOutcome {
        RxStopOutcome {
            meta: RxStopMetadata::data_complete(self.bytes_observed, self.bytes_returned),
            matched: false,
            match_index: None,
        }
    }

    /// Current `bytes_observed` counter.
    pub fn bytes_observed(&self) -> usize {
        self.bytes_observed
    }

    /// Current `bytes_returned` counter.
    pub fn bytes_returned(&self) -> usize {
        self.bytes_returned
    }

    /// Whether a match was found.
    pub fn matched(&self) -> bool {
        self.matched
    }

    /// The match index, if found.
    pub fn match_index(&self) -> Option<usize> {
        self.match_index
    }

    /// The configured deadline, if any.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// The configured max_bytes limit.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

/// Classify a stop reason as "normal" (not an error).
///
/// Normal stop reasons are returned as successful results from `read` and
/// as informational notifications from `subscribe`. Error stop reasons
/// are returned as `Err` from `read`/`wait_for` and surfaced differently
/// in `subscribe` final notifications.
pub fn is_normal_stop(reason: RxStopReason) -> bool {
    matches!(
        reason,
        RxStopReason::DataComplete
            | RxStopReason::Timeout
            | RxStopReason::MaxBufferedBytes
            | RxStopReason::MatchFound
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_stops_at_deadline() {
        let start = Instant::now();
        let ctrl = RxStopController::new(start, Some(0), 1024);
        // Deadline already passed (0ms timeout).
        let decision = ctrl.check_timeout();
        assert!(matches!(decision, RxStopDecision::Stop(_)));
        if let RxStopDecision::Stop(outcome) = decision {
            assert_eq!(outcome.meta.stop_reason, RxStopReason::Timeout);
            assert!(!outcome.matched);
            assert!(outcome.match_index.is_none());
        }
    }

    #[test]
    fn continue_before_deadline() {
        let start = Instant::now();
        let ctrl = RxStopController::new(start, Some(5000), 1024);
        assert!(matches!(ctrl.check_timeout(), RxStopDecision::Continue));
    }

    #[test]
    fn no_timeout_without_deadline() {
        let start = Instant::now();
        let ctrl = RxStopController::new(start, None, 1024);
        assert!(matches!(ctrl.check_timeout(), RxStopDecision::Continue));
    }

    #[test]
    fn match_found_stops_immediately() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 1024);
        let decision = ctrl.push_data(5, 5, Some(MatchResult::Found(2)));
        assert!(matches!(decision, RxStopDecision::Stop(_)));
        if let RxStopDecision::Stop(outcome) = decision {
            assert_eq!(outcome.meta.stop_reason, RxStopReason::MatchFound);
            assert!(outcome.matched);
            assert_eq!(outcome.match_index, Some(2));
            assert_eq!(outcome.meta.bytes_observed, 5);
            assert_eq!(outcome.meta.bytes_returned, 5);
        }
    }

    #[test]
    fn no_match_continues() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 1024);
        let decision = ctrl.push_data(5, 5, Some(MatchResult::NoMatch));
        assert!(matches!(decision, RxStopDecision::Continue));
        assert_eq!(ctrl.bytes_observed(), 5);
    }

    #[test]
    fn max_buffered_bytes_stops() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 10);
        // push_data with buffered_len == max_bytes should stop.
        let decision = ctrl.push_data(10, 10, None);
        assert!(matches!(decision, RxStopDecision::Stop(_)));
        if let RxStopDecision::Stop(outcome) = decision {
            assert_eq!(outcome.meta.stop_reason, RxStopReason::MaxBufferedBytes);
        }
    }

    #[test]
    fn match_found_takes_priority_over_max_bytes() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 5);
        // Both match and max_bytes — match should win.
        let decision = ctrl.push_data(10, 10, Some(MatchResult::Found(3)));
        assert!(matches!(decision, RxStopDecision::Stop(_)));
        if let RxStopDecision::Stop(outcome) = decision {
            assert_eq!(outcome.meta.stop_reason, RxStopReason::MatchFound);
        }
    }

    #[test]
    fn connection_closed_outcome() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 1024);
        ctrl.record_data(100, 80);
        let outcome = ctrl.connection_closed();
        assert_eq!(outcome.meta.stop_reason, RxStopReason::ConnectionClosed);
        assert_eq!(outcome.meta.bytes_observed, 100);
        assert_eq!(outcome.meta.bytes_returned, 80);
    }

    #[test]
    fn channel_closed_outcome() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 1024);
        ctrl.record_data(50, 50);
        let outcome = ctrl.channel_closed();
        assert_eq!(outcome.meta.stop_reason, RxStopReason::ChannelClosed);
        assert_eq!(outcome.meta.bytes_observed, 50);
    }

    #[test]
    fn cancelled_outcome() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 1024);
        ctrl.record_data(30, 30);
        let outcome = ctrl.cancelled();
        assert_eq!(outcome.meta.stop_reason, RxStopReason::Cancelled);
        assert_eq!(outcome.meta.bytes_observed, 30);
    }

    #[test]
    fn read_error_outcome() {
        let start = Instant::now();
        let ctrl = RxStopController::new(start, None, 1024);
        let outcome = ctrl.read_error();
        assert_eq!(outcome.meta.stop_reason, RxStopReason::ReadError);
        assert!(!outcome.matched);
    }

    #[test]
    fn peer_disconnected_outcome() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 1024);
        ctrl.record_data(200, 200);
        let outcome = ctrl.peer_disconnected();
        assert_eq!(outcome.meta.stop_reason, RxStopReason::PeerDisconnected);
        assert_eq!(outcome.meta.bytes_observed, 200);
    }

    #[test]
    fn data_complete_outcome() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 1024);
        ctrl.record_data(42, 42);
        let outcome = ctrl.data_complete();
        assert_eq!(outcome.meta.stop_reason, RxStopReason::DataComplete);
        assert!(!outcome.matched);
    }

    #[test]
    fn is_normal_stop_classifies_correctly() {
        assert!(is_normal_stop(RxStopReason::DataComplete));
        assert!(is_normal_stop(RxStopReason::Timeout));
        assert!(is_normal_stop(RxStopReason::MaxBufferedBytes));
        assert!(is_normal_stop(RxStopReason::MatchFound));

        assert!(!is_normal_stop(RxStopReason::ConnectionClosed));
        assert!(!is_normal_stop(RxStopReason::Cancelled));
        assert!(!is_normal_stop(RxStopReason::ReadError));
        assert!(!is_normal_stop(RxStopReason::ChannelClosed));
        assert!(!is_normal_stop(RxStopReason::PeerDisconnected));
        assert!(!is_normal_stop(RxStopReason::BudgetExhausted));
    }

    #[test]
    fn push_data_without_matcher_accumulates() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 1024);
        // No matcher: pass None
        let decision = ctrl.push_data(10, 10, None);
        assert!(matches!(decision, RxStopDecision::Continue));
        assert_eq!(ctrl.bytes_observed(), 10);

        let decision = ctrl.push_data(20, 30, None);
        assert!(matches!(decision, RxStopDecision::Continue));
        assert_eq!(ctrl.bytes_observed(), 30);
        assert_eq!(ctrl.bytes_returned(), 30);
    }

    #[test]
    fn timeout_preserves_match_state_from_earlier_data() {
        let start = Instant::now();
        // 0ms timeout means deadline has already passed.
        let mut ctrl = RxStopController::new(start, Some(0), 1024);
        // Record some data but don't find match yet (this simulates
        // a scenario where data arrived before timeout was checked).
        ctrl.record_data(5, 5);
        // Now check timeout — should include bytes info.
        let decision = ctrl.check_timeout();
        if let RxStopDecision::Stop(outcome) = decision {
            assert_eq!(outcome.meta.bytes_observed, 5);
            assert_eq!(outcome.meta.bytes_returned, 5);
            assert_eq!(outcome.meta.stop_reason, RxStopReason::Timeout);
        } else {
            panic!("expected Stop decision for expired deadline");
        }
    }

    #[test]
    fn check_max_buffered_bytes_after_record_data() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 10);
        ctrl.record_data(10, 10);
        let decision = ctrl.check_max_buffered_bytes();
        assert!(matches!(decision, RxStopDecision::Stop(_)));
        if let RxStopDecision::Stop(outcome) = decision {
            assert_eq!(outcome.meta.stop_reason, RxStopReason::MaxBufferedBytes);
        }
    }

    #[test]
    fn record_data_does_not_trigger_stops() {
        let start = Instant::now();
        let mut ctrl = RxStopController::new(start, None, 5);
        ctrl.record_data(100, 100);
        // record_data just updates counters; caller must check separately.
        assert_eq!(ctrl.bytes_observed(), 100);
        assert_eq!(ctrl.bytes_returned(), 100);
        // But check_max_buffered_bytes should now stop.
        let decision = ctrl.check_max_buffered_bytes();
        assert!(matches!(decision, RxStopDecision::Stop(_)));
    }
}
