use pandora_types::harness::ManifestError;
use pandora_types::{GeneId, HarnessId, HarnessKind, HarnessManifest, MetaComposition};

pub const CORE_SOURCE_HARNESS_ID: &str = "core-source";
pub const CORE_SOURCE_HARNESS_VERSION: &str = "0.1.0";
pub const CODING_HARNESS_ID: &str = "coding-domain";
pub const CODING_HARNESS_VERSION: &str = "0.1.0";
pub const COORDINATION_META_HARNESS_ID: &str = "coordination-meta";
pub const COORDINATION_META_HARNESS_VERSION: &str = "0.1.0";

pub fn core_source_manifest() -> Result<HarnessManifest, ManifestError> {
    HarnessManifest::new(
        CORE_SOURCE_HARNESS_ID,
        CORE_SOURCE_HARNESS_VERSION,
        "Pandora Core",
        HarnessKind::Source,
        Some("pandora-runtime".to_owned()),
        Vec::new(),
    )
}

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

pub fn coordination_meta_manifest() -> Result<HarnessManifest, ManifestError> {
    let composition = MetaComposition::new(vec![HarnessId::new(CODING_HARNESS_ID)?], 8)?;
    HarnessManifest::new_meta(
        COORDINATION_META_HARNESS_ID,
        COORDINATION_META_HARNESS_VERSION,
        "Coordination Meta",
        composition,
        Vec::new(),
    )
}
