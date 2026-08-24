//! TX framing codecs and byte helpers.
//!
//! Contains `TxFramingMode::encode`, SLIP (RFC 1055) and COBS stuffing,
//! length-prefix/delimiter/blank-frame helpers used by the decoder, and
//! `hex_upper`.

use crate::checksums::xor_checksum;
use crate::codec;
use crate::util::find_subsequence;

use super::config::{Endianness, TxFramingMode, TxLineEnding};

impl TxFramingMode {
    /// Apply this TX framing mode and return bytes for the UART.
    pub fn encode(&self, payload: &[u8]) -> Result<Vec<u8>, String> {
        match self {
            TxFramingMode::Line { ending } => {
                let mut framed = payload.to_vec();
                match ending {
                    TxLineEnding::Lf => framed.push(b'\n'),
                    TxLineEnding::Cr => framed.push(b'\r'),
                    TxLineEnding::Crlf => framed.extend_from_slice(b"\r\n"),
                }
                Ok(framed)
            }
            TxFramingMode::Delimiter {
                delimiter,
                delimiter_encoding,
            } => {
                let delim_bytes = codec::decode((*delimiter_encoding).into(), delimiter)
                    .map_err(|e| format!("Invalid TX delimiter encoding: {e}"))?;
                if delim_bytes.is_empty() {
                    return Err("TX delimiter must not be empty".into());
                }
                let mut framed = payload.to_vec();
                framed.extend_from_slice(&delim_bytes);
                Ok(framed)
            }
            TxFramingMode::LengthPrefixed {
                prefix_size,
                endianness,
            } => {
                if !matches!(prefix_size, 1 | 2 | 4) {
                    return Err("TX prefix_size must be 1, 2, or 4".into());
                }
                let len = payload.len();
                match prefix_size {
                    1 => {
                        if len > 255 {
                            return Err(format!(
                                "TX payload length {len} exceeds maximum 255 for prefix_size=1"
                            ));
                        }
                        let mut framed = Vec::with_capacity(1 + len);
                        framed.push(len as u8);
                        framed.extend_from_slice(payload);
                        Ok(framed)
                    }
                    2 => {
                        if len > 65535 {
                            return Err(format!(
                                "TX payload length {len} exceeds maximum 65535 for prefix_size=2"
                            ));
                        }
                        let mut framed = Vec::with_capacity(2 + len);
                        let len_bytes = match endianness {
                            Endianness::Big => (len as u16).to_be_bytes(),
                            Endianness::Little => (len as u16).to_le_bytes(),
                        };
                        framed.extend_from_slice(&len_bytes);
                        framed.extend_from_slice(payload);
                        Ok(framed)
                    }
                    4 => {
                        let mut framed = Vec::with_capacity(4 + len);
                        let len_bytes = match endianness {
                            Endianness::Big => (len as u32).to_be_bytes(),
                            Endianness::Little => (len as u32).to_le_bytes(),
                        };
                        framed.extend_from_slice(&len_bytes);
                        framed.extend_from_slice(payload);
                        Ok(framed)
                    }
                    _ => unreachable!("prefix_size validated above"),
                }
            }
            TxFramingMode::StartEnd {
                start,
                end,
                marker_encoding,
            } => {
                if start.is_empty() {
                    return Err("TX start markers must not be empty".into());
                }
                let start_bytes = codec::decode((*marker_encoding).into(), &start[0])
                    .map_err(|e| format!("Invalid TX start marker encoding: {e}"))?;
                let end_bytes = codec::decode((*marker_encoding).into(), end)
                    .map_err(|e| format!("Invalid TX end marker encoding: {e}"))?;
                if start_bytes.is_empty() || end_bytes.is_empty() {
                    return Err("TX start and end markers must not be empty".into());
                }
                let mut framed =
                    Vec::with_capacity(start_bytes.len() + payload.len() + end_bytes.len());
                framed.extend_from_slice(&start_bytes);
                framed.extend_from_slice(payload);
                framed.extend_from_slice(&end_bytes);
                Ok(framed)
            }
            TxFramingMode::Slip => {
                let mut framed = vec![SLIP_END];
                framed.extend_from_slice(&slip_stuff(payload));
                framed.push(SLIP_END);
                Ok(framed)
            }
            TxFramingMode::Cobs => {
                let mut framed = vec![0x00];
                framed.extend_from_slice(&cobs_stuff(payload));
                framed.push(0x00);
                Ok(framed)
            }
            TxFramingMode::Nmea => {
                // Reject embedded terminators and non-printable bytes.
                if payload.iter().any(|&b| b == b'\r' || b == b'\n') {
                    return Err("TX NMEA payload must not contain \\r or \\n".into());
                }
                if payload.iter().any(|&b| !(0x20..=0x7E).contains(&b)) {
                    return Err("TX NMEA payload must be printable ASCII".into());
                }
                // Determine leading character: preserve $/! if present, else '$'.
                let lead_byte = if payload.first() == Some(&b'$') || payload.first() == Some(&b'!')
                {
                    payload[0]
                } else {
                    b'$'
                };
                let body_start = if payload.first() == Some(&b'$') || payload.first() == Some(&b'!')
                {
                    1
                } else {
                    0
                };
                let body = &payload[body_start..];

                // Check for existing trailing *HH.
                let existing_checksum = (body.len() >= 3
                    && body[body.len() - 3] == b'*'
                    && body[body.len() - 2].is_ascii_hexdigit()
                    && body[body.len() - 1].is_ascii_hexdigit())
                .then(|| {
                    let hex_str = std::str::from_utf8(&body[body.len() - 2..])
                        .map_err(|_| "TX NMEA invalid UTF-8 in checksum".to_string())?;
                    u8::from_str_radix(hex_str, 16)
                        .map_err(|_| "TX NMEA invalid hex in checksum".to_string())
                });
                // Derive checksum input after releasing the borrow of `body`.
                let existing_val = match existing_checksum {
                    Some(Ok(v)) => Some(v),
                    Some(Err(e)) => return Err(e),
                    None => None,
                };
                let body_to_checksum = match existing_val {
                    Some(_) => &body[..body.len() - 3],
                    None => body,
                };
                let computed = xor_checksum(body_to_checksum);
                if let Some(received) = existing_val {
                    if computed != received {
                        return Err(format!(
                            "TX NMEA checksum mismatch: payload declares {received:02X}, computed {computed:02X}"
                        ));
                    }
                }
                let cs_hex = format!("{computed:02X}");
                let additional = if existing_val.is_some() { 0 } else { 3 };
                let mut framed = Vec::with_capacity(1 + body.len() + additional + 2);
                framed.push(lead_byte);
                framed.extend_from_slice(body);
                if existing_val.is_none() {
                    framed.push(b'*');
                    framed.extend_from_slice(cs_hex.as_bytes());
                }
                framed.extend_from_slice(b"\r\n");
                Ok(framed)
            }
        }
    }
}

/// Return whether frame data is empty or contains only ASCII whitespace
/// (space, `\t`, `\r`, `\n`, `\x0b`, or `\x0c`) for `skip_empty`.
pub(crate) fn is_blank_frame(data: &[u8]) -> bool {
    data.iter().all(|&b| b.is_ascii_whitespace())
}

/// Extract the next frame when `delim` appears in `buf`.
///
/// Always drains the delimiter. Includes it in the returned frame only when
/// `include_terminators` is `true`.
pub(crate) fn extract_delimited(
    buf: &mut Vec<u8>,
    delim: &[u8],
    include_terminators: bool,
) -> Option<Vec<u8>> {
    let pos = find_subsequence(buf, delim)?;
    let fb = if include_terminators {
        buf[..pos + delim.len()].to_vec()
    } else {
        buf[..pos].to_vec()
    };
    buf.drain(..pos + delim.len());
    Some(fb)
}

/// Format a byte slice as uppercase hex with no separator (e.g. "4F1A").
pub(crate) fn hex_upper(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

// SLIP (RFC 1055) constants and codec.

pub(crate) const SLIP_END: u8 = 0xC0;
pub(crate) const SLIP_ESC: u8 = 0xDB;
pub(crate) const SLIP_ESC_END: u8 = 0xDC;
pub(crate) const SLIP_ESC_ESC: u8 = 0xDD;

/// Byte-stuff a payload for SLIP TX framing. Replaces `END` (0xC0) with
/// `ESC ESC_END` and `ESC` (0xDB) with `ESC ESC_ESC`. All other bytes pass
/// through unchanged. The caller wraps the result in `END` markers.
pub(crate) fn slip_stuff(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + payload.len() / 10);
    for &b in payload {
        match b {
            SLIP_END => {
                out.push(SLIP_ESC);
                out.push(SLIP_ESC_END);
            }
            SLIP_ESC => {
                out.push(SLIP_ESC);
                out.push(SLIP_ESC_ESC);
            }
            _ => out.push(b),
        }
    }
    out
}

/// COBS-encode a payload for TX framing using plain COBS (delimiter 0x00).
/// The encoded block contains no 0x00 bytes. The caller wraps the result as
/// `[0x00][block][0x00]` (SLIP-parity framing).
///
/// Plain COBS treats the payload as ending with a phantom trailing 0x00.
/// Codes 0x01-0xFE mean "the next 0x00 is `code` bytes ahead"; the receiver
/// inserts a 0x00 after `code-1` data bytes. Code 0xFF means "254 non-zero
/// data bytes follow, no implicit zero" (continuation for runs > 253).
fn cobs_stuff(payload: &[u8]) -> Vec<u8> {
    // Worst-case overhead: ceil(payload_len / 254) + 1 code bytes.
    let mut out: Vec<u8> = Vec::with_capacity(payload.len() + payload.len() / 254 + 2);
    // `code_index` tracks the placeholder position for the current block's
    // code byte; `code` counts bytes since the last zero (the distance to the
    // next zero, including the zero itself).
    let mut code_index: usize = 0;
    out.push(0x00); // placeholder for the first code byte
    let mut code: u8 = 1;

    for &b in payload {
        if b == 0x00 {
            // End of a run: write the code at the placeholder, start a new
            // block with a fresh placeholder.
            out[code_index] = code;
            code = 1;
            code_index = out.len();
            out.push(0x00); // placeholder for the next code
        } else {
            out.push(b);
            code = code.saturating_add(1);
            // 254 non-zero bytes with no zero: emit a continuation block
            // (code 0xFF = "254 data bytes, no implicit zero") and start a
            // new block. This keeps code values in 0x01-0xFF and bounds the
            // overhead.
            if code == 0xFF {
                out[code_index] = 0xFF;
                code = 1;
                code_index = out.len();
                out.push(0x00); // placeholder for the next code
            }
        }
    }
    // Final code for the phantom trailing zero.
    out[code_index] = code;
    out
}

/// Read a length prefix from the given bytes.
/// `prefix_size` must be 1, 2, or 4 (validated at construction).
/// Returns 0 for invalid sizes as a safe fallback.
pub(crate) fn read_length_prefix(bytes: &[u8], prefix_size: u8, endianness: Endianness) -> usize {
    match prefix_size {
        1 => bytes[0] as usize,
        2 => {
            let arr: [u8; 2] = bytes[..2].try_into().unwrap_or([0; 2]);
            match endianness {
                Endianness::Big => u16::from_be_bytes(arr) as usize,
                Endianness::Little => u16::from_le_bytes(arr) as usize,
            }
        }
        4 => {
            let arr: [u8; 4] = bytes[..4].try_into().unwrap_or([0; 4]);
            match endianness {
                Endianness::Big => u32::from_be_bytes(arr) as usize,
                Endianness::Little => u32::from_le_bytes(arr) as usize,
            }
        }
        _ => {
            // Invalid prefix_size; FrameDecoder::new() rejects sizes other
            // than 1/2/4.
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::config::{
        preset_rx_framing, preset_rx_parser, LineEnding, ProtocolPreset, RxFramingConfig,
        RxFramingMode, TxFramingConfig,
    };
    use crate::framing::decoder::{FrameDecoder, ParsedFrame};
    use crate::match_config::PatternEncoding;

    // SLIP (RFC 1055) tests.

    #[test]
    fn slip_stuff_replaces_end_and_esc() {
        let payload = &[SLIP_END, SLIP_ESC, 0x41];
        let stuffed = slip_stuff(payload);
        assert_eq!(
            stuffed,
            &[SLIP_ESC, SLIP_ESC_END, SLIP_ESC, SLIP_ESC_ESC, 0x41]
        );
    }

    #[test]
    fn tx_slip_encodes_end_end() {
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(b"hi").unwrap();
        assert_eq!(framed, &[SLIP_END, b'h', b'i', SLIP_END]);
    }

    #[test]
    fn tx_slip_stuffs_payload_with_end() {
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(&[SLIP_END]).unwrap();
        assert_eq!(framed, &[SLIP_END, SLIP_ESC, SLIP_ESC_END, SLIP_END]);
    }

    #[test]
    fn tx_slip_stuffs_payload_with_esc() {
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(&[SLIP_ESC]).unwrap();
        assert_eq!(framed, &[SLIP_END, SLIP_ESC, SLIP_ESC_ESC, SLIP_END]);
    }

    // TX framing tests.

    #[test]
    fn tx_line_lf() {
        let mode = TxFramingMode::Line {
            ending: TxLineEnding::Lf,
        };
        let framed = mode.encode(b"AT+CGMI").unwrap();
        assert_eq!(framed, b"AT+CGMI\n");
    }

    #[test]
    fn tx_line_cr() {
        let mode = TxFramingMode::Line {
            ending: TxLineEnding::Cr,
        };
        let framed = mode.encode(b"AT+CGMI").unwrap();
        assert_eq!(framed, b"AT+CGMI\r");
    }

    #[test]
    fn tx_line_crlf() {
        let mode = TxFramingMode::Line {
            ending: TxLineEnding::Crlf,
        };
        let framed = mode.encode(b"AT+CGMI").unwrap();
        assert_eq!(framed, b"AT+CGMI\r\n");
    }

    #[test]
    fn tx_delimiter_utf8() {
        let mode = TxFramingMode::Delimiter {
            delimiter: "|".into(),
            delimiter_encoding: PatternEncoding::Utf8,
        };
        let framed = mode.encode(b"data").unwrap();
        assert_eq!(framed, b"data|");
    }

    #[test]
    fn tx_delimiter_empty_rejected() {
        let mode = TxFramingMode::Delimiter {
            delimiter: "".into(),
            delimiter_encoding: PatternEncoding::Utf8,
        };
        match mode.encode(b"data") {
            Ok(_) => panic!("empty TX delimiter should be rejected"),
            Err(err) => assert!(err.contains("TX delimiter must not be empty"), "got: {err}"),
        }
    }

    #[test]
    fn tx_length_prefixed_u8() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 1,
            endianness: Endianness::Big,
        };
        let framed = mode.encode(b"hello").unwrap();
        assert_eq!(framed, b"\x05hello");
    }

    #[test]
    fn tx_length_prefixed_u16_big() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Big,
        };
        let framed = mode.encode(b"hello").unwrap();
        assert_eq!(&framed[..2], &[0x00, 0x05]);
        assert_eq!(&framed[2..], b"hello");
    }

    #[test]
    fn tx_length_prefixed_u16_little() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Little,
        };
        let framed = mode.encode(b"hello").unwrap();
        assert_eq!(&framed[..2], &[0x05, 0x00]);
        assert_eq!(&framed[2..], b"hello");
    }

    #[test]
    fn tx_length_prefixed_u32() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 4,
            endianness: Endianness::Big,
        };
        let framed = mode.encode(b"hello").unwrap();
        assert_eq!(&framed[..4], &[0x00, 0x00, 0x00, 0x05]);
        assert_eq!(&framed[4..], b"hello");
    }

    #[test]
    fn tx_length_prefixed_invalid_prefix_size() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 3,
            endianness: Endianness::Big,
        };
        match mode.encode(b"data") {
            Ok(_) => panic!("prefix_size=3 should be rejected"),
            Err(err) => assert!(
                err.contains("TX prefix_size must be 1, 2, or 4"),
                "got: {err}"
            ),
        }
    }

    #[test]
    fn tx_length_prefixed_u8_overflow() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 1,
            endianness: Endianness::Big,
        };
        let payload = vec![0u8; 300];
        match mode.encode(&payload) {
            Ok(_) => panic!("payload too large for prefix_size=1 should be rejected"),
            Err(err) => assert!(err.contains("exceeds maximum 255"), "got: {err}"),
        }
    }

    #[test]
    fn tx_length_prefixed_u16_overflow() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Big,
        };
        let payload = vec![0u8; 65536];
        match mode.encode(&payload) {
            Ok(_) => panic!("payload too large for prefix_size=2 should be rejected"),
            Err(err) => assert!(err.contains("exceeds maximum 65535"), "got: {err}"),
        }
    }

    #[test]
    fn tx_start_end() {
        let mode = TxFramingMode::StartEnd {
            start: vec!["<".into()],
            end: ">".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        let framed = mode.encode(b"data").unwrap();
        assert_eq!(framed, b"<data>");
    }

    #[test]
    fn tx_start_end_empty_markers_rejected() {
        let mode = TxFramingMode::StartEnd {
            start: vec!["".into()],
            end: ">".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        match mode.encode(b"data") {
            Ok(_) => panic!("empty markers should be rejected"),
            Err(err) => assert!(
                err.contains("TX start and end markers must not be empty"),
                "got: {err}"
            ),
        }
    }

    // TX-to-RX round-trip tests.

    #[test]
    fn roundtrip_line_lf() {
        let tx = TxFramingMode::Line {
            ending: TxLineEnding::Lf,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Lf,
            },
            ..Default::default()
        };
        let payload = b"hello world";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_line_cr() {
        let tx = TxFramingMode::Line {
            ending: TxLineEnding::Cr,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Cr,
            },
            ..Default::default()
        };
        let payload = b"hello world";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_line_crlf() {
        let tx = TxFramingMode::Line {
            ending: TxLineEnding::Crlf,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Crlf,
            },
            ..Default::default()
        };
        let payload = b"hello world";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_delimiter() {
        let tx = TxFramingMode::Delimiter {
            delimiter: "|".into(),
            delimiter_encoding: PatternEncoding::Utf8,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let payload = b"data";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_length_prefixed_u8() {
        let tx = TxFramingMode::LengthPrefixed {
            prefix_size: 1,
            endianness: Endianness::Big,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let payload = b"binary data";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_length_prefixed_u16_be() {
        let tx = TxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Big,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let payload = b"binary data";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_start_end() {
        let tx = TxFramingMode::StartEnd {
            start: vec!["STX".into()],
            end: "ETX".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["STX".into()],
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let payload = b"data";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    // TX framing JSON deserialization tests.

    #[test]
    fn tx_framing_line_crlf_deserialize() {
        let json = serde_json::json!({
            "type": "line",
            "ending": "crlf"
        });
        let cfg: TxFramingConfig = serde_json::from_value(json).unwrap();
        assert!(matches!(
            cfg.mode,
            TxFramingMode::Line {
                ending: TxLineEnding::Crlf
            }
        ));
    }

    #[test]
    fn tx_framing_delimiter_deserialize() {
        let json = serde_json::json!({
            "type": "delimiter",
            "delimiter": "|",
            "delimiter_encoding": "utf8"
        });
        let cfg: TxFramingConfig = serde_json::from_value(json).unwrap();
        assert!(matches!(cfg.mode, TxFramingMode::Delimiter { .. }));
    }

    #[test]
    fn cobs_stuff_preserves_payload_without_delimiter() {
        // Encode a payload containing the 0x00 delimiter byte; the encoded block
        // must not contain the 0x00 delimiter byte.
        let stuffed = cobs_stuff(&[0x00, 0x41, 0x00]);
        assert!(
            !stuffed.contains(&0x00),
            "encoded block must not contain delimiter"
        );
    }

    #[test]
    fn tx_cobs_encodes_block_then_delimiter() {
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"hi").unwrap();
        // Format: [delim] [stuffed] [delim]. Body between delimiters
        // must contain no 0x00.
        assert_eq!(framed.first(), Some(&0x00));
        assert_eq!(framed.last(), Some(&0x00));
        let body = &framed[1..framed.len() - 1];
        assert!(!body.contains(&0x00), "body must not contain delimiter");
        assert!(
            !body.is_empty(),
            "body must not be empty for non-empty payload"
        );
    }

    #[test]
    fn tx_cobs_stuffs_payload_with_delimiter_byte() {
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&[0x00]).unwrap();
        // Payload [0x00]: two code blocks of 0x01 (distance to next 0x00 is 1).
        // Stuffed = [0x01, 0x01]. Framed = [0x00, 0x01, 0x01, 0x00].
        assert_eq!(framed, &[0x00, 0x01, 0x01, 0x00]);
    }

    #[test]
    fn start_end_tx_uses_first_marker() {
        let mode = TxFramingMode::StartEnd {
            start: vec!["<".into(), ">".into()],
            end: "|".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        let framed = mode.encode(b"hi").unwrap();
        // Uses start[0] = "<"
        assert_eq!(framed, b"<hi|");
    }

    // NMEA TX auto-checksum tests.

    #[test]
    fn tx_nmea_appends_checksum_and_terminators() {
        let payload = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
        let cs = xor_checksum(payload);
        let framed = TxFramingMode::Nmea.encode(payload).unwrap();
        let expected =
            format!("${}*{cs:02X}\r\n", std::str::from_utf8(payload).unwrap(),).into_bytes();
        assert_eq!(framed, expected);
    }

    #[test]
    fn tx_nmea_validates_existing_correct_checksum() {
        let payload = format!(
            "$GPGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*{:02X}",
            xor_checksum(b"GPGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,"),
        )
        .into_bytes();
        let framed = TxFramingMode::Nmea.encode(&payload).unwrap();
        // Payload already starts with $ and has correct checksum; pass it through.
        assert_eq!(framed.len(), payload.len() + 2);
        assert!(framed.starts_with(b"$"));
        assert!(framed.ends_with(b"\r\n"));
        // No double checksum.
        let inner = &framed[..framed.len() - 2];
        assert_eq!(inner, payload);
    }

    #[test]
    fn tx_nmea_rejects_existing_wrong_checksum() {
        let body = b"GPGGA,123519";
        let correct_cs = xor_checksum(body);
        let wrong_cs = correct_cs ^ 0xFF;
        let payload = format!("$GPGA,123519*{wrong_cs:02X}",).into_bytes();
        let err = TxFramingMode::Nmea.encode(&payload).unwrap_err();
        assert!(
            err.contains("TX NMEA checksum mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn tx_nmea_keeps_bang_lead_for_ais() {
        let payload = b"!AIVDM,1,1,,A,15M_@`P00P<8:HltHUV@vThPB80,4*";
        let framed = TxFramingMode::Nmea.encode(payload).unwrap();
        assert!(framed.starts_with(b"!"));
        // Should not double the leading ! (no !! prefix).
        assert!(!framed.starts_with(b"!!"));
        // Should end with *XX\r\n where XX is computed over body after !
        let body = b"AIVDM,1,1,,A,15M_@`P00P<8:HltHUV@vThPB80,4*";
        let cs = xor_checksum(body);
        let expected_end = format!("*{cs:02X}\r\n").into_bytes();
        assert!(framed.ends_with(&expected_end));
    }

    #[test]
    fn tx_nmea_rejects_embedded_crlf() {
        let err = TxFramingMode::Nmea.encode(b"GPGGA\r\n").unwrap_err();
        assert!(
            err.contains("must not contain \\r or \\n"),
            "unexpected error: {err}"
        );

        let err = TxFramingMode::Nmea.encode(b"\nGPGGA").unwrap_err();
        assert!(
            err.contains("must not contain \\r or \\n"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn tx_nmea_rejects_non_ascii_payload() {
        let err = TxFramingMode::Nmea.encode(&[0x80]).unwrap_err();
        assert!(
            err.contains("must be printable ASCII"),
            "unexpected error: {err}"
        );

        let err = TxFramingMode::Nmea.encode(&[0x1F]).unwrap_err();
        assert!(
            err.contains("must be printable ASCII"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn tx_nmea_roundtrip_validates_on_rx() {
        let payload = b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
        let framed = TxFramingMode::Nmea.encode(payload).unwrap();

        let rx_config = preset_rx_framing(ProtocolPreset::Nmea0183);
        let parser_config = preset_rx_parser(ProtocolPreset::Nmea0183);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        let result = dec.push(&framed);
        assert!(
            result.error.is_none(),
            "unexpected decode error: {:?}",
            result.error
        );
        assert_eq!(
            result.frames.len(),
            1,
            "expected 1 frame, got {:?}",
            result.frames
        );
        let parsed = result.frames.first().unwrap();
        match &parsed.parsed {
            Some(ParsedFrame::Nmea { checksum_valid, .. }) => {
                assert_eq!(*checksum_valid, Some(true));
            }
            other => panic!("expected Nmea with checksum_valid: Some(true), got {other:?}"),
        }
    }
}
