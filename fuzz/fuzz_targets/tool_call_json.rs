#![no_main]

use libfuzzer_sys::fuzz_target;
use serial_mcp::tools::types::*;

fuzz_target!(|data: &[u8]| {
    let s = match std::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };

    let _ = serde_json::from_str::<OpenArgs>(s);
    let _ = serde_json::from_str::<CloseArgs>(s);
    let _ = serde_json::from_str::<WriteArgs>(s);
    let _ = serde_json::from_str::<ReadArgs>(s);
    let _ = serde_json::from_str::<FlushArgs>(s);
    let _ = serde_json::from_str::<SetDtrRtsArgs>(s);
    let _ = serde_json::from_str::<SendBreakArgs>(s);
    let _ = serde_json::from_str::<SubscribeArgs>(s);
    let _ = serde_json::from_str::<UnsubscribeArgs>(s);
    let _ = serde_json::from_str::<SetFlowControlArgs>(s);
});
