//! Wire result construction for reads: `build_read_result` turns a
//! [`ReadOutcome`] (from the sibling `read_loop` module) into the MCP
//! `ReadResult` JSON with per-payload lossless encode-or-hex fallback.
//! The shared read timeout default comes from `helpers`.

use rmcp::Json;

use crate::codec::{self, Encoding};
use crate::tools::helpers::DEFAULT_READ_TIMEOUT_MS;
use crate::tools::read_loop::ReadOutcome;
use crate::tools::types::*;

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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::rx_metadata::RxStopMetadata;

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
}
