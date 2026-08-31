#![no_main]

use libfuzzer_sys::fuzz_target;
use pandora_runtime::mcp::validate_mcp_response_frame_for_fuzzing;

fuzz_target!(|data: &[u8]| {
    let mut frame = Vec::with_capacity(data.len().saturating_add(1));
    frame.extend_from_slice(data);
    if !frame.ends_with(b"\n") {
        frame.push(b'\n');
    }
    let _ = validate_mcp_response_frame_for_fuzzing(&frame);
});
