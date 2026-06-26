//! Shared RX frame-consumption logic for `read` and `subscribe`.
//!
//! The two tools share frame decoding + per-frame matching + `max_frames`, but
//! differ in the per-frame ACTION (read collects frames; subscribe emits
//! notifications). [`RxFrameSink`] captures that action; [`consume_frames`]
//! drives it. The raw (no-framing) path is intentionally NOT shared — read and
//! subscribe differ there semantically (scan extent).

use crate::framing::{Frame, FrameDecodeError, FrameDecoder};
use crate::match_config::{MatchResult, Matcher};
use crate::rx_metadata::RxStopReason;
use crate::serial::{ConnectionState, SerialConnection};
use crate::stop_controller::RxStopController;

/// What the frame loop should do after a sink handled a frame.
pub enum SinkFlow {
    /// Keep processing frames.
    Continue,
    /// Stop processing with this reason (e.g. subscribe stops at its match, or a
    /// peer disconnected while emitting).
    Stop(RxStopReason),
}

/// Result of consuming all frames decoded from one chunk.
pub enum FrameOutcome {
    /// No stop condition; keep the outer loop running.
    Continue,
    /// `max_frames` reached (checked after all frames in the chunk).
    MaxFrames,
    /// The sink returned [`SinkFlow::Stop`].
    SinkStop(RxStopReason),
    /// A runtime decode error occurred (e.g. SLIP malformed escape).
    DecodeError(FrameDecodeError),
}

/// Per-frame output action. `read` collects frames; `subscribe` emits
/// notifications.
#[async_trait::async_trait]
pub trait RxFrameSink {
    /// Handle one decoded frame. `matched` / `match_index` come from the
    /// driver's per-frame matcher run. Return [`SinkFlow::Stop`] to halt
    /// processing (subscribe stops at its match; read returns `Continue`).
    async fn on_frame(
        &mut self,
        frame: Frame,
        matched: bool,
        match_index: Option<usize>,
    ) -> SinkFlow;
}

/// Decode frames from `chunk`, run the per-frame matcher (window reset per
/// frame), dispatch each frame to `sink`, then check `max_frames`.
pub async fn consume_frames<S: RxFrameSink>(
    chunk: &[u8],
    decoder: &mut FrameDecoder,
    matcher: &mut Option<Matcher>,
    max_frames: Option<usize>,
    frames_seen: &mut usize,
    sink: &mut S,
) -> FrameOutcome {
    let frames = match decoder.push(chunk) {
        Ok(f) => f,
        Err(e) => return FrameOutcome::DecodeError(e),
    };
    for frame in frames {
        *frames_seen += 1;
        let match_index = match matcher.as_mut() {
            Some(m) => {
                m.reset_window();
                match m.push(&frame.data) {
                    MatchResult::Found(idx) => Some(idx),
                    _ => None,
                }
            }
            None => None,
        };
        if let SinkFlow::Stop(reason) = sink
            .on_frame(frame, match_index.is_some(), match_index)
            .await
        {
            return FrameOutcome::SinkStop(reason);
        }
    }
    if let Some(limit) = max_frames {
        if *frames_seen >= limit {
            return FrameOutcome::MaxFrames;
        }
    }
    FrameOutcome::Continue
}

/// Connection liveness for the RX loop's pause check.
pub enum DisconnectState {
    /// Connected — proceed normally.
    Active,
    /// Disconnected/reconnecting with reconnect enabled — caller should pause
    /// (sleep) and continue. The silence timer has been reset.
    Reconnecting,
    /// Disconnected with reconnect disabled — caller should stop with
    /// `connection_closed`.
    Closed,
}

/// Evaluate the connection's disconnect/reconnect state. Resets the silence
/// timer when returning [`DisconnectState::Reconnecting`].
pub fn disconnect_state(conn: &SerialConnection, ctrl: &mut RxStopController) -> DisconnectState {
    let state = conn.state();
    if state == ConnectionState::Disconnected || state == ConnectionState::Reconnecting {
        let reconnect_enabled = conn.reconnect_policy.lock().expect("poisoned").enabled;
        if !reconnect_enabled {
            return DisconnectState::Closed;
        }
        ctrl.reset_silence_timer();
        return DisconnectState::Reconnecting;
    }
    DisconnectState::Active
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::{LineEnding, RxFramingConfig, RxFramingMode};

    struct CollectSink {
        frames: Vec<Frame>,
        matches: Vec<usize>,
        stop_on_match: bool,
    }

    #[async_trait::async_trait]
    impl RxFrameSink for CollectSink {
        async fn on_frame(&mut self, frame: Frame, matched: bool, _mi: Option<usize>) -> SinkFlow {
            if matched {
                self.matches.push(frame.index);
                if self.stop_on_match {
                    self.frames.push(frame);
                    return SinkFlow::Stop(RxStopReason::MatchFound);
                }
            }
            self.frames.push(frame);
            SinkFlow::Continue
        }
    }

    fn line_decoder() -> FrameDecoder {
        FrameDecoder::new(
            &RxFramingConfig {
                mode: RxFramingMode::Line {
                    ending: LineEnding::Auto,
                },
                ..Default::default()
            },
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn consume_frames_processes_all_then_reports_max_frames() {
        let mut dec = line_decoder();
        let mut matcher = None;
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: false,
        };
        let out = consume_frames(
            b"a\nb\nc\n",
            &mut dec,
            &mut matcher,
            Some(2),
            &mut seen,
            &mut sink,
        )
        .await;
        assert!(matches!(out, FrameOutcome::MaxFrames));
        // All 3 frames processed before the post-chunk max_frames check.
        assert_eq!(sink.frames.len(), 3);
        assert_eq!(seen, 3);
    }

    #[tokio::test]
    async fn consume_frames_sink_stop_halts_processing() {
        let mut dec = line_decoder();
        let mut matcher = Matcher::new_literal(b"b".to_vec());
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: true,
        };
        let out = consume_frames(
            b"a\nb\nc\n",
            &mut dec,
            &mut matcher,
            None,
            &mut seen,
            &mut sink,
        )
        .await;
        assert!(matches!(
            out,
            FrameOutcome::SinkStop(RxStopReason::MatchFound)
        ));
        // Stopped at "b"; "c" never processed.
        assert_eq!(sink.frames.len(), 2);
        assert_eq!(sink.matches, vec![1]);
    }

    #[tokio::test]
    async fn consume_frames_no_match_no_limit_continues() {
        let mut dec = line_decoder();
        let mut matcher = None;
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: false,
        };
        let out = consume_frames(
            b"x\ny\n",
            &mut dec,
            &mut matcher,
            None,
            &mut seen,
            &mut sink,
        )
        .await;
        assert!(matches!(out, FrameOutcome::Continue));
        assert_eq!(sink.frames.len(), 2);
    }

    #[tokio::test]
    async fn consume_frames_match_takes_priority_over_max_frames_subscribe_semantics() {
        // subscribe: SinkStop(MatchFound) wins over MaxFrames.
        let mut dec = line_decoder();
        let mut matcher = Matcher::new_literal(b"b".to_vec());
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: true,
        };
        let out = consume_frames(
            b"a\nb\nc\n",
            &mut dec,
            &mut matcher,
            Some(2),
            &mut seen,
            &mut sink,
        )
        .await;
        assert!(matches!(
            out,
            FrameOutcome::SinkStop(RxStopReason::MatchFound)
        ));
        assert_eq!(seen, 2); // "a", "b" processed; "c" not (stopped at match)
    }

    #[tokio::test]
    async fn consume_frames_match_takes_priority_over_max_frames_read_semantics() {
        // read: collect post-match frames, so MaxFrames triggers after all 3.
        let mut dec = line_decoder();
        let mut matcher = Matcher::new_literal(b"b".to_vec());
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: false,
        };
        let out = consume_frames(
            b"a\nb\nc\n",
            &mut dec,
            &mut matcher,
            Some(2),
            &mut seen,
            &mut sink,
        )
        .await;
        assert!(matches!(out, FrameOutcome::MaxFrames));
        assert_eq!(seen, 3); // all 3 processed; match recorded as side-effect
        assert_eq!(sink.matches, vec![1]);
    }

    #[tokio::test]
    async fn consume_frames_resets_matcher_window_per_frame() {
        // "xA" + "B" across two frames would match "AB" if the matcher
        // carried state between frames. Verify only frame 2 ("AB\n") matches.
        let mut dec = line_decoder();
        let mut matcher = Matcher::new_literal(b"AB".to_vec());
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: false,
        };
        let _out = consume_frames(
            b"xA\nB\nAB\n",
            &mut dec,
            &mut matcher,
            None,
            &mut seen,
            &mut sink,
        )
        .await;
        assert_eq!(sink.matches, vec![2], "only frame 2 (AB) should match");
        assert_eq!(seen, 3);
    }
}
