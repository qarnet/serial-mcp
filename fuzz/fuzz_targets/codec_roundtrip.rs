#![no_main]

use libfuzzer_sys::fuzz_target;
use serial_mcp_server::codec::{decode, encode, Encoding};

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
});
