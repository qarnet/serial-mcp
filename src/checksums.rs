//! Shared checksum primitives for protocol parsers/presets.
//!
//! Pure functions, no I/O. Two free functions compute single-byte checksums:
//!
//! - [`xor_checksum`]: NMEA-0183 `*XX` XOR of all bytes.
//! - [`lrc`]: Modbus ASCII LRC (Longitudinal Redundancy Check) —
//!   two's complement of the sum of all bytes.
//!
//! A trait abstraction will return when multi-byte checksums land
//! (CRC-16 for Modbus RTU, FCS-16 for HDLC) — tracked in FEATURES.md.

/// NMEA-0183 `*XX` XOR checksum: XOR of all bytes in the slice.
pub(crate) fn xor_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, &b| acc ^ b)
}

/// Modbus ASCII LRC (Longitudinal Redundancy Check): the two's complement of
/// the sum of all bytes, as a single byte. Transmitted as 2 hex chars in the
/// frame. Used by Modbus ASCII mode.
pub(crate) fn lrc(bytes: &[u8]) -> u8 {
    let sum: u8 = bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    sum.wrapping_neg()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_checksum_empty_returns_zero() {
        assert_eq!(xor_checksum(b""), 0x00);
    }

    #[test]
    fn xor_checksum_known_nmea_sentence() {
        // NMEA-0183 $GPGLL sentence body between $ and *:
        // "GPGLL,3751.65,N,12226.54,W" → XOR checksum 0x7E
        assert_eq!(xor_checksum(b"GPGLL,3751.65,N,12226.54,W"), 0x7E);
    }

    #[test]
    fn xor_checksum_single_byte() {
        assert_eq!(xor_checksum(&[0xAB]), 0xAB);
    }

    #[test]
    fn xor_checksum_identity() {
        // XOR of two identical values is 0
        assert_eq!(xor_checksum(&[0x5A, 0x5A]), 0x00);
    }

    #[test]
    fn lrc_empty_returns_zero() {
        assert_eq!(lrc(b""), 0x00);
    }

    #[test]
    fn lrc_known_modbus_request() {
        // Modbus spec worked example: read holding registers
        // address=1, function=3, start=0, qty=1 → [0x01, 0x03, 0x00, 0x00, 0x00, 0x01]
        // sum = 0x01+0x03+0x00+0x00+0x00+0x01 = 0x05
        // LRC = two's complement of 0x05 = 0xFB
        assert_eq!(lrc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x01]), 0xFB);
    }

    #[test]
    fn lrc_wraps_on_overflow() {
        // sum = 0xFF + 0x02 = 0x101, wraps to 0x01
        // LRC = wrapping_neg(0x01) = 0xFF
        assert_eq!(lrc(&[0xFF, 0x02]), 0xFF);
    }

    #[test]
    fn lrc_all_zeros() {
        assert_eq!(lrc(&[0x00, 0x00, 0x00]), 0x00);
    }
}
