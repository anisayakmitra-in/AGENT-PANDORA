#![no_main]

use libfuzzer_sys::fuzz_target;
use pandora_runtime::subagent_store::validate_effect_receipt_for_fuzzing;

fuzz_target!(|data: &[u8]| {
    let _ = validate_effect_receipt_for_fuzzing(data);
});
