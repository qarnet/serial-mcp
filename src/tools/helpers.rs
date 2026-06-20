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
use crate::match_config::{shape_match_context, Matcher};
use crate::rx_metadata::RxStopMetadata;
use crate::rx_session::RxEvent;
use crate::serial::{
    ConnectionConfig, ConnectionManager, DataBits, FlowControl, Parity, SerialConnection, StopBits,
};
use crate::stop_controller::{RxStopController, RxStopDecision};
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
    /// Decoded frames, empty when framing was not configured.
    pub frames: Vec<crate::framing::Frame>,
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
    framing: Option<crate::framing::FramingConfig>,
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
        Some(cfg) => Some(crate::framing::FrameDecoder::new(cfg)?),
        None => None,
    };
    let mut collected_frames: Vec<crate::framing::Frame> = Vec::new();
    let make_outcome = |frames: Vec<crate::framing::Frame>,
                        bytes: Vec<u8>,
                        elapsed_ms: u64,
                        meta: RxStopMetadata,
                        matched: bool,
                        match_index: Option<usize>| {
        let outcome = ReadOutcome {
            bytes,
            elapsed_ms,
            meta,
            matched,
            match_index,
            frames,
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
            frames: outcome.frames,
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

    loop {
        // Pause timeouts while the connection is disconnected or reconnecting.
        if let Some(ref conn) = conn {
            let state = conn.state();
            if state == crate::serial::ConnectionState::Disconnected
                || state == crate::serial::ConnectionState::Reconnecting
            {
                ctrl.reset_silence_timer();
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            }
        }

        if let RxStopDecision::Stop(outcome) = ctrl.check_timeout() {
            return Ok(make_outcome(
                finalize_frames(&mut decoder, &mut collected_frames),
                accumulated,
                read_start.elapsed().as_millis() as u64,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
            ));
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            return Ok(make_outcome(
                finalize_frames(&mut decoder, &mut collected_frames),
                accumulated,
                read_start.elapsed().as_millis() as u64,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
            ));
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        let remaining_ms = remaining.as_millis() as u64;
        let wait = remaining_ms.saturating_sub(1).clamp(1, 250);

        let event = tokio::select! {
            _ = ct.cancelled() => {
                let outcome = ctrl.cancelled();
                return Ok(make_outcome(finalize_frames(&mut decoder, &mut collected_frames), accumulated, read_start.elapsed().as_millis() as u64, outcome.meta, outcome.matched, outcome.match_index));
            }
            msg = tokio::time::timeout(Duration::from_millis(wait), event_rx.recv()) => match msg {
                Ok(Some(e)) => e,
                Ok(None) => {
                    let outcome = ctrl.channel_closed();
                    return Ok(make_outcome(finalize_frames(&mut decoder, &mut collected_frames), accumulated, read_start.elapsed().as_millis() as u64, outcome.meta, outcome.matched, outcome.match_index));
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

                // Feed to frame decoder (full chunk, not just take portion).
                if let Some(ref mut dec) = decoder {
                    let new_frames = dec.push(&chunk);
                    collected_frames.extend(new_frames);
                    if let Some(limit) = max_frames {
                        if collected_frames.len() >= limit {
                            let bytes_observed = ctrl.bytes_observed();
                            let bytes_returned = accumulated.len();
                            let meta = RxStopMetadata::max_frames(bytes_observed, bytes_returned);
                            return Ok(make_outcome(
                                finalize_frames(&mut decoder, &mut collected_frames),
                                accumulated,
                                read_start.elapsed().as_millis() as u64,
                                meta,
                                false,
                                None,
                            ));
                        }
                    }
                }

                let match_result = matcher.as_mut().map(|m| m.push(&chunk[..take]));
                let buffered_len = accumulated.len();

                if let RxStopDecision::Stop(outcome) =
                    ctrl.push_data(chunk_len, buffered_len, match_result)
                {
                    return Ok(make_outcome(
                        finalize_frames(&mut decoder, &mut collected_frames),
                        accumulated,
                        read_start.elapsed().as_millis() as u64,
                        outcome.meta,
                        outcome.matched,
                        outcome.match_index,
                    ));
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
                return Ok(make_outcome(
                    finalize_frames(&mut decoder, &mut collected_frames),
                    accumulated,
                    read_start.elapsed().as_millis() as u64,
                    outcome.meta,
                    outcome.matched,
                    outcome.match_index,
                ));
            }
            RxEvent::Error(msg) => {
                return Err(log_tool_err("read", "Data reading failed", msg));
            }
        }
    }

    // Settle phase: gather burst after first byte (only when no matcher
    // and no framing). With a matcher or framing, the loop above continues
    // until match/frames/timeout/max/closed.
    let _start_len = accumulated.len();
    while accumulated.len() < max_bytes {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        let settle = remaining.min(SETTLE_MS);
        if settle == 0 {
            break;
        }

        if let RxStopDecision::Stop(outcome) = ctrl.check_timeout() {
            return Ok(make_outcome(
                finalize_frames(&mut decoder, &mut collected_frames),
                accumulated,
                read_start.elapsed().as_millis() as u64,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
            ));
        }
        if let RxStopDecision::Stop(outcome) = ctrl.check_silence_timeout() {
            return Ok(make_outcome(
                finalize_frames(&mut decoder, &mut collected_frames),
                accumulated,
                read_start.elapsed().as_millis() as u64,
                outcome.meta,
                outcome.matched,
                outcome.match_index,
            ));
        }

        let event = tokio::select! {
            _ = ct.cancelled() => {
                let outcome = ctrl.cancelled();
                return Ok(make_outcome(finalize_frames(&mut decoder, &mut collected_frames), accumulated, read_start.elapsed().as_millis() as u64, outcome.meta, outcome.matched, outcome.match_index));
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

                // Feed to frame decoder.
                if let Some(ref mut dec) = decoder {
                    let new_frames = dec.push(&chunk);
                    collected_frames.extend(new_frames);
                    if let Some(limit) = max_frames {
                        if collected_frames.len() >= limit {
                            let bytes_observed = ctrl.bytes_observed();
                            let bytes_returned = accumulated.len();
                            let meta = RxStopMetadata::max_frames(bytes_observed, bytes_returned);
                            return Ok(make_outcome(
                                finalize_frames(&mut decoder, &mut collected_frames),
                                accumulated,
                                read_start.elapsed().as_millis() as u64,
                                meta,
                                false,
                                None,
                            ));
                        }
                    }
                }

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

    // Flush any partial frame remaining in the decoder.
    if let Some(ref mut dec) = decoder {
        if let Some(partial) = dec.flush_partial() {
            collected_frames.push(partial);
        }
    }

    // After settle phase, determine the stop reason.
    // If we filled the buffer during settle, it's MaxBufferedBytes;
    // otherwise it's DataComplete.
    if let RxStopDecision::Stop(outcome) = ctrl.check_max_buffered_bytes() {
        return Ok(make_outcome(
            finalize_frames(&mut decoder, &mut collected_frames),
            accumulated,
            read_start.elapsed().as_millis() as u64,
            outcome.meta,
            outcome.matched,
            outcome.match_index,
        ));
    }
    let outcome = ctrl.data_complete();
    Ok(make_outcome(
        finalize_frames(&mut decoder, &mut collected_frames),
        accumulated,
        read_start.elapsed().as_millis() as u64,
        outcome.meta,
        outcome.matched,
        outcome.match_index,
    ))
}

pub(crate) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
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
    let data = codec::encode(encoding, &outcome.bytes)
        .map_err(|e| format!("Data encoding failed - {e}"))?;

    let frames = if outcome.frames.is_empty() {
        None
    } else {
        let encoded_frames: Result<Vec<FrameResult>, String> = outcome
            .frames
            .iter()
            .map(|f| {
                let fdata = codec::encode(encoding, &f.data)
                    .map_err(|e| format!("Frame data encoding failed - {e}"))?;
                Ok(FrameResult {
                    data: fdata,
                    encoding: encoding.to_string(),
                    frame_index: f.index,
                    frame_type: f.frame_type.clone(),
                    parsed: f.parsed.as_ref().map(convert_parsed_frame),
                })
            })
            .collect();
        Some(encoded_frames?)
    };

    Ok(Json(ReadResult {
        connection_id,
        name,
        bytes_read,
        encoding: encoding.to_string(),
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
        frames,
    }))
}

/// Convert a `ParsedFrame` (from the framing module) to a `ParsedFrameResult`
/// (the API type in tools/types). The two enums have identical variants.
fn convert_parsed_frame(parsed: &crate::framing::ParsedFrame) -> ParsedFrameResult {
    match parsed {
        crate::framing::ParsedFrame::AtCommand {
            response_type,
            command,
            status,
            fields,
        } => ParsedFrameResult::AtCommand {
            response_type: response_type.clone(),
            command: command.clone(),
            status: status.clone(),
            fields: fields.clone(),
        },
        crate::framing::ParsedFrame::Json(v) => ParsedFrameResult::Json(v.clone()),
        crate::framing::ParsedFrame::ShellPrompt {
            prompt,
            prompt_type,
        } => ParsedFrameResult::ShellPrompt {
            prompt: prompt.clone(),
            prompt_type: prompt_type.clone(),
        },
        crate::framing::ParsedFrame::Raw => ParsedFrameResult::Raw,
    }
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
        data_bits: parse_data_bits(&args.data_bits)?,
        stop_bits: parse_stop_bits(&args.stop_bits)?,
        parity: parse_parity(&args.parity)?,
        flow_control: parse_flow_control(&args.flow_control)?,
        port_info: None,
        log_capacity: args.log_capacity,
        log_enabled: args.log_enabled,
    })
}

pub fn parse_data_bits(raw: &str) -> Result<DataBits, String> {
    match raw {
        "5" => Ok(DataBits::Five),
        "6" => Ok(DataBits::Six),
        "7" => Ok(DataBits::Seven),
        "8" => Ok(DataBits::Eight),
        other => Err(format!("Invalid data_bits {other:?} (expected 5/6/7/8)")),
    }
}

pub fn parse_stop_bits(raw: &str) -> Result<StopBits, String> {
    match raw {
        "1" => Ok(StopBits::One),
        "2" => Ok(StopBits::Two),
        other => Err(format!("Invalid stop_bits {other:?} (expected 1/2)")),
    }
}

pub fn parse_parity(raw: &str) -> Result<Parity, String> {
    match raw.to_lowercase().as_str() {
        "none" => Ok(Parity::None),
        "odd" => Ok(Parity::Odd),
        "even" => Ok(Parity::Even),
        other => Err(format!("Invalid parity {other:?} (expected none/odd/even)")),
    }
}

pub fn parse_flow_control(raw: &str) -> Result<FlowControl, String> {
    match raw.to_lowercase().as_str() {
        "none" => Ok(FlowControl::None),
        "software" => Ok(FlowControl::Software),
        "hardware" => Ok(FlowControl::Hardware),
        other => Err(format!(
            "Invalid flow_control {other:?} (expected none/software/hardware)"
        )),
    }
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
        };
        let err = parse_open_args(args).unwrap_err();
        assert!(err.contains("data_bits"));
    }

    #[test]
    fn open_args_reject_invalid_parity() {
        assert!(parse_parity("weird").is_err());
        assert!(parse_parity("none").is_ok());
        assert!(parse_parity("Even").is_ok());
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
            frames: vec![],
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
            frames: vec![],
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
            frames: vec![],
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
            frames: vec![],
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
            frames: vec![],
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
    fn find_subslice_locates_pattern() {
        assert_eq!(find_subslice(b"hello OK> world", b"OK>"), Some(6));
        assert_eq!(find_subslice(b"OK>at-start", b"OK>"), Some(0));
        assert_eq!(find_subslice(b"trailing OK>", b"OK>"), Some(9));
    }

    #[test]
    fn find_subslice_missing_returns_none() {
        assert_eq!(find_subslice(b"hello world", b"OK>"), None);
        assert_eq!(find_subslice(b"", b"x"), None);
    }

    #[test]
    fn find_subslice_empty_needle_returns_none() {
        assert_eq!(find_subslice(b"hello", b""), None);
    }

    #[test]
    fn find_subslice_needle_longer_than_haystack() {
        assert_eq!(find_subslice(b"hi", b"hello"), None);
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
        let framing = Some(crate::framing::FramingConfig {
            mode: crate::framing::FramingMode::Delimiter {
                delimiter: "".into(),
                delimiter_encoding: crate::match_config::PatternEncoding::Utf8,
            },
            ..Default::default()
        });
        let result =
            read_bytes_via_session(rx, 128, None, &ct, None, None, None, None, None, framing).await;
        match result {
            Ok(_) => panic!("empty delimiter should be rejected"),
            Err(err) => assert!(err.contains("Delimiter must not be empty"), "got: {err}"),
        }
    }

    #[tokio::test]
    async fn read_via_session_rejects_invalid_prefix_size() {
        let rx = make_closed_rx();
        let ct = tokio_util::sync::CancellationToken::new();
        let framing = Some(crate::framing::FramingConfig {
            mode: crate::framing::FramingMode::LengthPrefixed {
                prefix_size: 3,
                endianness: crate::framing::Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        });
        let result =
            read_bytes_via_session(rx, 128, None, &ct, None, None, None, None, None, framing).await;
        match result {
            Ok(_) => panic!("prefix_size=3 should be rejected"),
            Err(err) => assert!(err.contains("prefix_size must be 1, 2, or 4"), "got: {err}"),
        }
    }

    #[tokio::test]
    async fn read_via_session_rejects_empty_markers() {
        let rx = make_closed_rx();
        let ct = tokio_util::sync::CancellationToken::new();
        let framing = Some(crate::framing::FramingConfig {
            mode: crate::framing::FramingMode::StartEnd {
                start: "".into(),
                end: "X".into(),
                marker_encoding: crate::match_config::PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        });
        let result =
            read_bytes_via_session(rx, 128, None, &ct, None, None, None, None, None, framing).await;
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
        let framing = Some(crate::framing::FramingConfig {
            mode: crate::framing::FramingMode::Line,
            parser: Some(crate::framing::ParserConfig {
                parser_type: crate::framing::ParserType::ShellPrompt,
                custom_prompt: Some("[invalid".to_string()),
            }),
            ..Default::default()
        });
        let result =
            read_bytes_via_session(rx, 128, None, &ct, None, None, None, None, None, framing).await;
        match result {
            Ok(_) => panic!("invalid regex should be rejected"),
            Err(err) => assert!(err.contains("Invalid prompt regex"), "got: {err}"),
        }
    }
}
