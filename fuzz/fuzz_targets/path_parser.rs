#![no_main]

use libfuzzer_sys::fuzz_target;
use pandora_runtime::executors::validate_workspace_relative_path;

fuzz_target!(|data: &[u8]| {
    if let Ok(value) = std::str::from_utf8(data) {
        let _ = validate_workspace_relative_path(value);
    }
});
