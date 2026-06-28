//! Shared checksum primitives for protocol parsers/presets.
//!
//! Pure functions, no I/O. Future home for LRC (Modbus ASCII),
//! CRC-16 (Modbus RTU), and FCS-16 (HDLC) when those protocols land.
//!
//! The `Checksum` trait and `XorChecksum` have no in-tree consumer yet;
//! they are infrastructure consumed by NMEA in P2. Allow dead_code until
//! the first consumer lands.

#![allow(dead_code)]

/// A checksum algorithm over a byte slice.
///
/// Implementations compute a checksum value and (optionally) validate a
/// received checksum against recomputed bytes. The framing layer carries the
/// raw frame bytes; checksum validation is a parser/preset concern that
/// surfaces failures through the existing `FramingError` stop reason.
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
}
