#![no_main]

use libfuzzer_sys::fuzz_target;
use pandora_types::OrchestrationPlan;

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<OrchestrationPlan>(data);
});
