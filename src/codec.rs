//! Data encoding/decoding between MCP tool strings and raw bytes.
//!
//! MCP tool arguments carry serial payloads as strings tagged with an
//! `encoding` field. This module converts in both directions and reports
//! a typed error per failure mode.

use std::fmt;
use std::str::FromStr;

use base64::{engine::general_purpose, Engine as _};
use thiserror::Error;

/// Wire-format used to represent serial bytes inside an MCP tool string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Raw UTF-8 text. Invalid UTF-8 bytes are replaced with the Unicode
    /// replacement character during encoding.
    Utf8,
    /// Human-readable text: like UTF-8, but ANSI/VT100 escape sequences are
    /// stripped during encoding. The ideal mode for reading firmware shell
    /// output, log lines, and other human-facing text that may include
    /// terminal control codes for colour, cursor movement, or screen clear.
    Text,
    /// Lowercase hex pairs. Decoder accepts upper or lower case and ignores spaces.
    Hex,
    /// Standard Base64. Decoder also accepts URL-safe / no-padding input.
    Base64,
}

impl FromStr for Encoding {
    type Err = CodecError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "utf8" | "utf-8" => Ok(Encoding::Utf8),
            "text" => Ok(Encoding::Text),
            "hex" => Ok(Encoding::Hex),
            "base64" | "b64" => Ok(Encoding::Base64),
            _ => Err(CodecError::UnknownEncoding(s.to_string())),
        }
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Encoding::Utf8 => "utf8",
            Encoding::Text => "text",
            Encoding::Hex => "hex",
            Encoding::Base64 => "base64",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("Unknown encoding: {0}")]
    UnknownEncoding(String),

    #[error("Invalid UTF-8: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),

    #[error("Invalid hex: {0}")]
    InvalidHex(#[from] hex::FromHexError),

    #[error("Invalid base64: {0}")]
    InvalidBase64(#[from] base64::DecodeError),

    #[error("Hex string must have even length")]
    HexOddLength,
}

/// Decode a tool-supplied string into raw bytes.
pub fn decode(encoding: Encoding, input: &str) -> Result<Vec<u8>, CodecError> {
    match encoding {
        Encoding::Utf8 | Encoding::Text => Ok(input.as_bytes().to_vec()),
        Encoding::Hex => decode_hex(input),
        Encoding::Base64 => decode_base64(input),
    }
}

/// Encode raw bytes into a string suitable for an MCP tool response.
///
/// When `encoding` is [`Encoding::Utf8`] or [`Encoding::Text`], invalid UTF-8
/// byte sequences are replaced with the Unicode replacement character
/// (`\u{FFFD}`) instead of returning an error. When `encoding` is
/// [`Encoding::Text`], ANSI/VT100 escape sequences are also stripped.
pub fn encode(encoding: Encoding, bytes: &[u8]) -> Result<String, CodecError> {
    match encoding {
        Encoding::Utf8 => Ok(String::from_utf8_lossy(bytes).into_owned()),
        Encoding::Text => {
            let cleaned = strip_ansi_bytes(bytes);
            Ok(String::from_utf8_lossy(&cleaned).into_owned())
        }
        Encoding::Hex => Ok(encode_hex_spaced(bytes)),
        Encoding::Base64 => Ok(general_purpose::STANDARD.encode(bytes)),
    }
}

fn decode_hex(input: &str) -> Result<Vec<u8>, CodecError> {
    let stripped = input.trim().replace(' ', "");
    if !stripped.len().is_multiple_of(2) {
        return Err(CodecError::HexOddLength);
    }
    Ok(hex::decode(&stripped)?)
}

fn decode_base64(input: &str) -> Result<Vec<u8>, CodecError> {
    let trimmed = input.trim();
    general_purpose::STANDARD
        .decode(trimmed)
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(trimmed))
        .map_err(CodecError::InvalidBase64)
}

fn encode_hex_spaced(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Strip ANSI/VT100 escape sequences from raw bytes.
///
/// Recognises:
/// - CSI sequences: `ESC [` followed by parameter bytes (`0x30`–`0x3F`),
///   intermediate bytes (`0x20`–`0x2F`), and a final byte (`0x40`–`0x7E`).
/// - Two-byte sequences: `ESC` followed by a single byte in `0x30`–`0x5F`
///   (e.g. `ESC M`, `ESC 7`, `ESC 8`).
/// - OSC sequences: `ESC ]` ... `BEL` or `ESC ]` ... `ESC \`.
///
/// Non-ANSI bytes are passed through unchanged.
pub fn strip_ansi_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1B && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'[' {
                i += 2;
                while i < bytes.len() && !(bytes[i] >= 0x40 && bytes[i] <= 0x7E) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            } else if next == b']' {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1B && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            } else if (0x30..=0x5F).contains(&next) {
                i += 2;
                continue;
            }
            out.push(0x1B);
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Strip ANSI/VT100 escape sequences from a UTF-8 string.
///
/// See [`strip_ansi_bytes`] for the byte-level version that operates
/// before UTF-8 decoding.
pub fn strip_ansi(s: &str) -> String {
    let cleaned = strip_ansi_bytes(s.as_bytes());
    String::from_utf8(cleaned)
        .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_from_str_accepts_aliases() {
        assert_eq!("utf8".parse::<Encoding>().unwrap(), Encoding::Utf8);
        assert_eq!("UTF-8".parse::<Encoding>().unwrap(), Encoding::Utf8);
        assert_eq!("text".parse::<Encoding>().unwrap(), Encoding::Text);
        assert_eq!("TEXT".parse::<Encoding>().unwrap(), Encoding::Text);
        assert_eq!("Hex".parse::<Encoding>().unwrap(), Encoding::Hex);
        assert_eq!("b64".parse::<Encoding>().unwrap(), Encoding::Base64);
    }

    #[test]
    fn encoding_from_str_rejects_unknown() {
        assert!("rot13".parse::<Encoding>().is_err());
    }

    #[test]
    fn utf8_roundtrip() {
        let bytes = decode(Encoding::Utf8, "Hello, 世界!").unwrap();
        assert_eq!(encode(Encoding::Utf8, &bytes).unwrap(), "Hello, 世界!");
    }

    #[test]
    fn utf8_encode_replaces_invalid_bytes_with_replacement_char() {
        let result = encode(Encoding::Utf8, &[0xFF, 0xFE]).unwrap();
        assert_eq!(result, "\u{FFFD}\u{FFFD}");
    }

    #[test]
    fn utf8_encode_ascii_does_not_corrupt() {
        let data = b"[14341] Hello from RTT! Counter: 14\r\n";
        let result = encode(Encoding::Utf8, data).unwrap();
        assert_eq!(result, "[14341] Hello from RTT! Counter: 14\r\n");
    }

    #[test]
    fn utf8_encode_mixed_valid_and_invalid() {
        let data: &[u8] = b"Hello \xFF world";
        let result = encode(Encoding::Utf8, data).unwrap();
        assert!(result.contains('\u{FFFD}'));
        assert!(result.starts_with("Hello "));
        assert!(result.ends_with(" world"));
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(decode(Encoding::Hex, "48656c6c6f").unwrap(), b"Hello");
        assert_eq!(decode(Encoding::Hex, "48 65 6c 6c 6f").unwrap(), b"Hello");
        assert_eq!(decode(Encoding::Hex, "48656C6C6F").unwrap(), b"Hello");
        assert_eq!(encode(Encoding::Hex, b"Hello").unwrap(), "48 65 6c 6c 6f");
    }

    #[test]
    fn hex_odd_length_rejected() {
        assert!(matches!(
            decode(Encoding::Hex, "48656c6c6"),
            Err(CodecError::HexOddLength)
        ));
    }

    #[test]
    fn hex_invalid_chars_rejected() {
        assert!(matches!(
            decode(Encoding::Hex, "48656cXY"),
            Err(CodecError::InvalidHex(_))
        ));
    }

    #[test]
    fn base64_roundtrip_and_padding_variants() {
        assert_eq!(
            decode(Encoding::Base64, "SGVsbG8gV29ybGQ=").unwrap(),
            b"Hello World"
        );
        assert_eq!(
            decode(Encoding::Base64, "SGVsbG8gV29ybGQ").unwrap(),
            b"Hello World"
        );
        assert_eq!(
            encode(Encoding::Base64, b"Hello World").unwrap(),
            "SGVsbG8gV29ybGQ="
        );
    }

    #[test]
    fn binary_roundtrips_via_hex_and_base64() {
        let data: &[u8] = b"Hello, World! 123 \x00\xFF";
        let hex = encode(Encoding::Hex, data).unwrap();
        assert_eq!(decode(Encoding::Hex, &hex).unwrap(), data);
        let b64 = encode(Encoding::Base64, data).unwrap();
        assert_eq!(decode(Encoding::Base64, &b64).unwrap(), data);
    }

    #[test]
    fn text_encode_strips_ansi_csi_color() {
        let input = b"\x1b[32m<inf> rtt_feedback: Started\x1b[0m\r\n";
        let result = encode(Encoding::Text, input).unwrap();
        assert_eq!(result, "<inf> rtt_feedback: Started\r\n");
    }

    #[test]
    fn text_encode_strips_ansi_cursor_and_clear() {
        let input = b"\x1b[8D\x1b[J[00:00:00.291,442] Hello\r\n";
        let result = encode(Encoding::Text, input).unwrap();
        assert_eq!(result, "[00:00:00.291,442] Hello\r\n");
    }

    #[test]
    fn text_encode_strips_ansi_osc() {
        let input = b"\x1b]0;window title\x07prompt> ";
        let result = encode(Encoding::Text, input).unwrap();
        assert_eq!(result, "prompt> ");
    }

    #[test]
    fn text_encode_strips_ansi_osc_with_st() {
        let input = b"\x1b]0;window title\x1b\\prompt> ";
        let result = encode(Encoding::Text, input).unwrap();
        assert_eq!(result, "prompt> ");
    }

    #[test]
    fn text_encode_strips_two_byte_escapes() {
        let input = b"\x1bMsave\x1b7cursor";
        let result = encode(Encoding::Text, input).unwrap();
        assert_eq!(result, "savecursor");
    }

    #[test]
    fn text_encode_preserves_normal_text() {
        let input = b"[14341] Hello from RTT! Counter: 14\r\n";
        let result = encode(Encoding::Text, input).unwrap();
        assert_eq!(result, "[14341] Hello from RTT! Counter: 14\r\n");
    }

    #[test]
    fn text_encode_uses_lossy_utf8() {
        let data: &[u8] = b"Hello \xFF world";
        let result = encode(Encoding::Text, data).unwrap();
        assert!(result.contains('\u{FFFD}'));
        assert!(result.starts_with("Hello "));
        assert!(result.ends_with(" world"));
    }

    #[test]
    fn text_decode_like_utf8() {
        let decoded = decode(Encoding::Text, "hello text").unwrap();
        assert_eq!(decoded, b"hello text");
    }

    #[test]
    fn strip_ansi_zephyr_shell_output() {
        let input = b"\x1b[8D\x1b[J[00:00:00.291,442] \x1b[0m<inf> udc_nrf: Initialized\x1b[0m\r\n\x1b[1;32m\x1b[m\x1b[J[00:00:00.291,625] \x1b[0m<inf> rtt_feedback: RTT feedback firmware started.\x1b[0m\r\n";
        let result = strip_ansi_bytes(input);
        let text = String::from_utf8_lossy(&result);
        assert_eq!(
            text,
            "[00:00:00.291,442] <inf> udc_nrf: Initialized\r\n[00:00:00.291,625] <inf> rtt_feedback: RTT feedback firmware started.\r\n"
        );
    }
}
