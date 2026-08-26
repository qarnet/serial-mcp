//! Framed RX consumption for the ring-based read pipeline.
//!
//! [`consume_frames`] drives decoding, per-frame matching, sink dispatch, and
//! `max_frames` handling. The raw (no-framing) path remains in `read_loop`.

use crate::framing::{Frame, FrameDecodeError, FrameDecoder};
use crate::match_config::{MatchResult, Matcher};
use crate::rx_metadata::RxStopReason;
use crate::serial::{ConnectionState, SerialConnection};
use crate::stop_controller::RxStopController;
use tracing;

/// What the frame loop should do after a sink handled a frame.
pub enum SinkFlow {
    /// Keep processing frames.
    Continue,
    /// Stop processing with this reason.
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
    /// A runtime decode error occurred, such as a malformed SLIP escape or an
    /// invalid COBS code.
    DecodeError(FrameDecodeError),
}

/// Per-frame output action used by the read pipeline.
#[async_trait::async_trait]
pub trait RxFrameSink {
    /// Handle one decoded frame. `matched` / `match_index` come from the
    /// driver's per-frame matcher run. Return [`SinkFlow::Stop`] to halt
    /// processing.
    async fn on_frame(
        &mut self,
        frame: Frame,
        matched: bool,
        match_index: Option<usize>,
    ) -> SinkFlow;
}

/// Decode frames from `chunk`, run the per-frame matcher (window reset per
/// frame), dispatch each frame to `sink`, then check `max_frames`.
/// `frames_dropped` accumulates the per-frame drop count from the decoder
/// (currently only checksum mismatches with `validate: true`).
pub async fn consume_frames<S: RxFrameSink>(
    chunk: &[u8],
    decoder: &mut FrameDecoder,
    matcher: &mut Option<Matcher>,
    max_frames: Option<usize>,
    frames_seen: &mut usize,
    sink: &mut S,
    frames_dropped: &mut usize,
) -> FrameOutcome {
    let outcome = decoder.push(chunk);
    *frames_dropped += outcome.frames_dropped;
    let frames = outcome.frames;
    // Dispatch frames decoded before an error, then return that error. The read
    // result preserves frames decoded before the error.
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
    if let Some(e) = outcome.error {
        return FrameOutcome::DecodeError(e);
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
    /// Connected; proceed normally.
    Active,
    /// Disconnected or reconnecting with reconnect enabled; caller should
    /// pause, sleep, and continue. The silence timer has been reset.
    Reconnecting,
    /// Disconnected with reconnect disabled; caller should stop with
    /// `connection_closed`.
    Closed,
}

/// Evaluate the connection's disconnect/reconnect state. Resets the silence
/// timer when returning [`DisconnectState::Reconnecting`].
pub fn disconnect_state(conn: &SerialConnection, ctrl: &mut RxStopController) -> DisconnectState {
    let state = conn.state();
    match state {
        ConnectionState::Closed => return DisconnectState::Closed,
        ConnectionState::Disconnected | ConnectionState::Reconnecting => {
            let reconnect_enabled = conn.reconnect_policy.lock().expect("poisoned").enabled;
            if !reconnect_enabled {
                return DisconnectState::Closed;
            }
            ctrl.reset_silence_timer();
            return DisconnectState::Reconnecting;
        }
        _ => {}
    }
    DisconnectState::Active
}

/// Map a `FrameOutcome` from `consume_frames` into an optional
/// `RxStopOutcome`. Returns `None` for `FrameOutcome::Continue`. For
/// `MaxFrames`, `DecodeError`, and `SinkStop(reason)`, returns the
/// corresponding `RxStopOutcome`. For `DecodeError`, records the error
/// text in `frame_error_msg` and emits the `error!` log line.
///
/// Shared by `read_bytes_from_ring` (`read_loop.rs`) so the FrameOutcome dispatch
/// lives in one place.
pub(crate) fn frame_outcome_to_stop(
    outcome: FrameOutcome,
    ctrl: &crate::stop_controller::RxStopController,
    total_returned: usize,
    match_offset: Option<usize>,
    frame_error_msg: &mut Option<String>,
    conn_id: &str,
) -> Option<crate::stop_controller::RxStopOutcome> {
    match outcome {
        FrameOutcome::Continue => None,
        FrameOutcome::MaxFrames => Some(crate::stop_controller::RxStopOutcome {
            meta: crate::rx_metadata::RxStopMetadata::max_frames(
                ctrl.bytes_observed(),
                total_returned,
            ),
            matched: false,
            match_index: None,
        }),
        FrameOutcome::DecodeError(e) => {
            tracing::error!("RX framing decode error on {conn_id}: {e}");
            *frame_error_msg = Some(e.to_string());
            Some(ctrl.framing_error(e))
        }
        FrameOutcome::SinkStop(reason) => match reason {
            crate::rx_metadata::RxStopReason::MatchFound => {
                Some(crate::stop_controller::RxStopOutcome {
                    meta: crate::rx_metadata::RxStopMetadata::match_found(
                        ctrl.bytes_observed(),
                        total_returned,
                    ),
                    matched: true,
                    match_index: match_offset,
                })
            }
            other => {
                tracing::warn!(
                    "unexpected sink stop reason {other:?} on {conn_id}; treating as connection_closed"
                );
                Some(ctrl.connection_closed())
            }
        },
    }
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
        let mut dropped = 0;
        let out = consume_frames(
            b"a\nb\nc\n",
            &mut dec,
            &mut matcher,
            Some(2),
            &mut seen,
            &mut sink,
            &mut dropped,
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
        let mut dropped = 0;
        let out = consume_frames(
            b"a\nb\nc\n",
            &mut dec,
            &mut matcher,
            None,
            &mut seen,
            &mut sink,
            &mut dropped,
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
        let mut dropped = 0;
        let out = consume_frames(
            b"x\ny\n",
            &mut dec,
            &mut matcher,
            None,
            &mut seen,
            &mut sink,
            &mut dropped,
        )
        .await;
        assert!(matches!(out, FrameOutcome::Continue));
        assert_eq!(sink.frames.len(), 2);
    }

    #[tokio::test]
    async fn consume_frames_match_takes_priority_over_max_frames() {
        // SinkStop(MatchFound) wins over MaxFrames.
        let mut dec = line_decoder();
        let mut matcher = Matcher::new_literal(b"b".to_vec());
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: true,
        };
        let mut dropped = 0;
        let out = consume_frames(
            b"a\nb\nc\n",
            &mut dec,
            &mut matcher,
            Some(2),
            &mut seen,
            &mut sink,
            &mut dropped,
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
        // For read, collect post-match frames, so MaxFrames triggers after all 3.
        let mut dec = line_decoder();
        let mut matcher = Matcher::new_literal(b"b".to_vec());
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: false,
        };
        let mut dropped = 0;
        let out = consume_frames(
            b"a\nb\nc\n",
            &mut dec,
            &mut matcher,
            Some(2),
            &mut seen,
            &mut sink,
            &mut dropped,
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
        let mut dropped = 0;
        let _out = consume_frames(
            b"xA\nB\nAB\n",
            &mut dec,
            &mut matcher,
            None,
            &mut seen,
            &mut sink,
            &mut dropped,
        )
        .await;
        assert_eq!(sink.matches, vec![2], "only frame 2 (AB) should match");
        assert_eq!(seen, 3);
    }

    /// Checksum-mismatch frames are dropped and counted, not emitted.
    #[tokio::test]
    async fn consume_frames_accumulates_frames_dropped() {
        use crate::framing::{ParserConfig, ParserType};

        // Build a decoder with NMEA parser (validate=true).
        let rx_config = crate::framing::RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser_config = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();

        // Good sentence: $GPGLL,3751.65,N,12226.54,W*7E\r\n (correct checksum 0x7E)
        let good = b"$GPGLL,3751.65,N,12226.54,W*7E\r\n";
        // Bad checksum sentence: $GPGLL,3751.65,N,12226.54,W*00\r\n
        let bad = b"$GPGLL,3751.65,N,12226.54,W*00\r\n";
        let mut chunk = Vec::new();
        chunk.extend_from_slice(good);
        chunk.extend_from_slice(bad);
        chunk.extend_from_slice(good);

        let mut matcher = None;
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: false,
        };
        let mut dropped = 0;
        let out = consume_frames(
            &chunk,
            &mut dec,
            &mut matcher,
            None,
            &mut seen,
            &mut sink,
            &mut dropped,
        )
        .await;

        assert!(matches!(out, FrameOutcome::Continue));
        // Two good frames emitted with contiguous indices.
        assert_eq!(sink.frames.len(), 2, "expected 2 good frames");
        assert_eq!(sink.frames[0].index, 0);
        assert_eq!(sink.frames[1].index, 1);
        assert_eq!(dropped, 1, "expected 1 checksum drop");
    }

    /// Valid frames decoded before a stream-fatal error are dispatched first,
    /// then `DecodeError` is returned. Malformed bytes are not counted as
    /// `frames_dropped`; they are a stream-fatal error, not a per-frame drop.
    #[tokio::test]
    async fn consume_frames_decodes_frames_before_slip_decode_error() {
        // SLIP constants from RFC 1055 (private; use literal bytes).
        const END: u8 = 0xC0;
        const ESC: u8 = 0xDB;

        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Slip,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();

        // One valid SLIP frame (END OK END) followed by a malformed escape
        // (ESC 0xFF is an invalid escape code).
        let mut chunk = Vec::new();
        chunk.push(END);
        chunk.extend_from_slice(b"OK");
        chunk.push(END);
        chunk.push(ESC);
        chunk.push(0xFF);

        let mut matcher = None;
        let mut seen = 0;
        let mut sink = CollectSink {
            frames: vec![],
            matches: vec![],
            stop_on_match: false,
        };
        let mut dropped = 0;
        let out = consume_frames(
            &chunk,
            &mut dec,
            &mut matcher,
            None,
            &mut seen,
            &mut sink,
            &mut dropped,
        )
        .await;

        // The valid frame is dispatched before the error.
        assert_eq!(
            sink.frames.len(),
            1,
            "the valid SLIP frame should be emitted"
        );
        assert_eq!(sink.frames[0].data, b"OK");
        // Stream-fatal error, not a per-frame drop.
        assert_eq!(dropped, 0);
        assert!(matches!(out, FrameOutcome::DecodeError(_)));
        // The error carries text.
        if let FrameOutcome::DecodeError(ref e) = out {
            assert!(!e.to_string().is_empty(), "decode error should have text");
        } else {
            panic!("expected DecodeError");
        }
    }
}
