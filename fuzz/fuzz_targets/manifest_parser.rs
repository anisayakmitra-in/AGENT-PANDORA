#![no_main]

use libfuzzer_sys::fuzz_target;
use pandora_types::PackageManifest;

fuzz_target!(|data: &[u8]| {
    if let Ok(manifest) = serde_json::from_slice::<PackageManifest>(data) {
        let _ = manifest.validate();
    }
});
