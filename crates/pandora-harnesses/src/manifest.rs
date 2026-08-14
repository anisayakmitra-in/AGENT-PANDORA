use pandora_types::harness::ManifestError;
use pandora_types::{GeneId, HarnessKind, HarnessManifest};

pub const CODING_HARNESS_ID: &str = "coding-domain";
pub const CODING_HARNESS_VERSION: &str = "0.1.0";

pub fn coding_manifest() -> Result<HarnessManifest, ManifestError> {
    let genes = [
        "workspace.read",
        "workspace.search",
        "patch.apply",
        "verification.run",
        "change.review",
    ]
    .into_iter()
    .map(|id| GeneId::new(id).expect("built-in Gene ID is valid"))
    .collect();
    HarnessManifest::new(
        CODING_HARNESS_ID,
        CODING_HARNESS_VERSION,
        "Coding Domain",
        HarnessKind::Domain,
        None,
        genes,
    )
}
