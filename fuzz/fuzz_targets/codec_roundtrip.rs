#![no_main]

use libfuzzer_sys::fuzz_target;
use serial_mcp::codec::{decode, encode, Encoding};

fuzz_target!(|data: &[u8]| {
    // Hex roundtrip
    if let Ok(hex_str) = encode(Encoding::Hex, data) {
        if let Ok(decoded) = decode(Encoding::Hex, &hex_str) {
            assert_eq!(decoded, data, "hex roundtrip mismatch");
        }
    }

    // Base64 roundtrip
    if let Ok(b64_str) = encode(Encoding::Base64, data) {
        if let Ok(decoded) = decode(Encoding::Base64, &b64_str) {
            assert_eq!(decoded, data, "base64 roundtrip mismatch");
        }
    }

    // UTF-8: if valid, must roundtrip; if invalid, encode must error
    match std::str::from_utf8(data) {
        Ok(valid) => {
            let encoded = encode(Encoding::Utf8, data).unwrap();
            assert_eq!(encoded, valid);
        }
        Err(_) => {
            assert!(encode(Encoding::Utf8, data).is_err());
        }
    }

    // COBS roundtrip (plain COBS, delimiter 0x00)
    {
        use serial_mcp::framing;
        let mode = framing::TxFramingMode::Cobs;
        if let Ok(framed) = mode.encode(data) {
            let cfg = framing::RxFramingConfig {
                mode: framing::RxFramingMode::Cobs,
                ..Default::default()
            };
            if let Ok(mut dec) = framing::FrameDecoder::new(&cfg, None) {
                let outcome = dec.push(&framed);
                assert!(
                    outcome.error.is_none(),
                    "valid COBS decode produced an error: {:?}",
                    outcome.error
                );
                assert!(!outcome.frames.is_empty(), "COBS decode produced no frames");
                let mut reconstructed = Vec::new();
                for f in &outcome.frames {
                    reconstructed.extend_from_slice(&f.data);
                }
                assert_eq!(reconstructed, data, "COBS roundtrip mismatch");
            }
        }
    }
});
