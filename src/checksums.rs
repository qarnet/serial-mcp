//! Shared checksum primitives for protocol parsers/presets.
//!
//! Pure functions, no I/O. Future home for CRC-16 (Modbus RTU)
//! and FCS-16 (HDLC) when those protocols land.
//!
//! The `Checksum` trait, `XorChecksum`, and `Lrc` are consumed by the
//! `NmeaParser` and `ModbusAsciiParser` in `src/framing.rs` for NMEA-0183
//! `*XX` XOR validation and Modbus ASCII LRC validation.

/// A checksum algorithm over a byte slice.
///
/// Implementations compute a checksum value and (optionally) validate a
/// received checksum against recomputed bytes. The framing layer carries the
/// raw frame bytes; checksum validation is a parser/preset concern that
/// surfaces failures through the existing `FramingError` stop reason.
#[allow(dead_code)] // width and validate are trait metadata reserved for a future generic
                    // checksum-length-aware caller; parsers currently call compute() directly.
pub(crate) trait Checksum: Send + Sync {
    /// Width of the checksum value in bytes (e.g. 1 for XOR/LRC, 2 for CRC-16).
    fn width(&self) -> usize;
    /// Compute the checksum over `bytes`, returning the value as a byte slice
    /// in the algorithm's natural byte order (caller compares byte-for-byte
    /// against the received checksum bytes).
    fn compute(&self, bytes: &[u8]) -> Vec<u8>;
    /// Validate `received` (the checksum bytes lifted from the frame) against
    /// a recomputed checksum over `bytes`. Default impl recomputes and
    /// compares; algorithms with special comparison rules override.
    fn validate(&self, bytes: &[u8], received: &[u8]) -> bool {
        let computed = self.compute(bytes);
        computed == received
    }
}

/// NMEA-0183 `*XX` XOR checksum: XOR of all bytes in the slice.
pub(crate) struct XorChecksum;

impl Checksum for XorChecksum {
    fn width(&self) -> usize {
        1
    }
    fn compute(&self, bytes: &[u8]) -> Vec<u8> {
        let mut acc: u8 = 0;
        for &b in bytes {
            acc ^= b;
        }
        vec![acc]
    }
}

/// Modbus ASCII LRC (Longitudinal Redundancy Check): the two's complement of
/// the sum of all bytes, as a single byte. Transmitted as 2 hex chars in the
/// frame. Used by Modbus ASCII mode.
pub(crate) struct Lrc;

impl Checksum for Lrc {
    fn width(&self) -> usize {
        1
    }
    fn compute(&self, bytes: &[u8]) -> Vec<u8> {
        let sum: u8 = bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        // Two's complement: (0x00 - sum) & 0xFF == (!sum).wrapping_add(1) == 0x100 - sum
        let lrc = sum.wrapping_neg();
        vec![lrc]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_checksum_empty_returns_zero() {
        let cs = XorChecksum.compute(b"");
        assert_eq!(cs, vec![0x00]);
    }

    #[test]
    fn xor_checksum_known_nmea_sentence() {
        // NMEA-0183 $GPGLL sentence body between $ and *:
        // "GPGLL,3751.65,N,12226.54,W" → XOR checksum 0x7E
        let body = b"GPGLL,3751.65,N,12226.54,W";
        let cs = XorChecksum.compute(body);
        assert_eq!(cs, vec![0x7E]);
    }

    #[test]
    fn xor_checksum_validate_matches_compute() {
        let data = b"GPGLL,3751.65,N,12226.54,W";
        let cs = XorChecksum.compute(data);
        assert!(XorChecksum.validate(data, &cs));
        // Corrupted checksum byte
        assert!(!XorChecksum.validate(data, &[0x00]));
    }

    #[test]
    fn xor_checksum_width_is_one() {
        assert_eq!(XorChecksum.width(), 1);
    }

    #[test]
    fn lrc_empty_returns_zero() {
        let cs = Lrc.compute(b"");
        assert_eq!(cs, vec![0x00]);
    }

    #[test]
    fn lrc_known_modbus_request() {
        // Modbus spec worked example: read holding registers
        // address=1, function=3, start=0, qty=1 → [0x01, 0x03, 0x00, 0x00, 0x00, 0x01]
        // sum = 0x01+0x03+0x00+0x00+0x00+0x01 = 0x05
        // LRC = two's complement of 0x05 = 0xFB
        let cs = Lrc.compute(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x01]);
        assert_eq!(cs, vec![0xFB]);
    }

    #[test]
    fn lrc_validate_matches_compute() {
        let data = b"test";
        let cs = Lrc.compute(data);
        assert!(Lrc.validate(data, &cs));
        // Corrupted checksum byte
        assert!(!Lrc.validate(data, &[0x00]));
    }

    #[test]
    fn lrc_width_is_one() {
        assert_eq!(Lrc.width(), 1);
    }

    #[test]
    fn lrc_wraps_on_overflow() {
        // sum = 0xFF + 0x02 = 0x101, wraps to 0x01
        // LRC = wrapping_neg(0x01) = 0xFF
        let cs = Lrc.compute(&[0xFF, 0x02]);
        assert_eq!(cs, vec![0xFF]);
    }
}
