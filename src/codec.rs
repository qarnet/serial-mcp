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
    /// Raw UTF-8 text. Bytes that are not valid UTF-8 cannot be encoded.
    Utf8,
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
        Encoding::Utf8 => Ok(input.as_bytes().to_vec()),
        Encoding::Hex => decode_hex(input),
        Encoding::Base64 => decode_base64(input),
    }
}

/// Encode raw bytes into a string suitable for an MCP tool response.
///
/// When `encoding` is [`Encoding::Utf8`], invalid UTF-8 byte sequences are
/// replaced with the Unicode replacement character (`\u{FFFD}`) instead of
/// returning an error. This allows firmware log output that contains
/// occasional binary framing bytes or corrupted multi-byte sequences to
/// remain readable without forcing the user to switch to hex encoding.
pub fn encode(encoding: Encoding, bytes: &[u8]) -> Result<String, CodecError> {
    match encoding {
        Encoding::Utf8 => Ok(String::from_utf8_lossy(bytes).into_owned()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_from_str_accepts_aliases() {
        assert_eq!("utf8".parse::<Encoding>().unwrap(), Encoding::Utf8);
        assert_eq!("UTF-8".parse::<Encoding>().unwrap(), Encoding::Utf8);
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
}
