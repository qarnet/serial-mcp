#![no_main]

use libfuzzer_sys::fuzz_target;
use serial_mcp_server::limits::*;
use serial_mcp_server::tools::helpers::{
    clamp_or_err, clamp_poll_interval_or_err, clamp_timeout_or_err, require_min_or_err,
};

fuzz_target!(|data: &[u8]| {
    if data.len() < 16 {
        return;
    }

    let value_usize = usize::from_le_bytes(data[0..8].try_into().unwrap());
    let max_usize = usize::from_le_bytes(data[8..16].try_into().unwrap());

    let value_u64 = u64::from_le_bytes(data[0..8].try_into().unwrap());
    let max_u64 = u64::from_le_bytes(data[8..16].try_into().unwrap());

    let _ = clamp_or_err("fuzz", value_usize, max_usize);
    let _ = require_min_or_err("fuzz", value_usize, max_usize);
    let _ = clamp_timeout_or_err("fuzz", value_u64, max_u64);
    let _ = clamp_poll_interval_or_err("fuzz", value_u64, max_u64);

    // With known constants
    let _ = clamp_or_err("read.max_bytes", value_usize, MAX_READ_BYTES);
    let _ = clamp_timeout_or_err("test", value_u64, MAX_TIMEOUT_MS);
});
