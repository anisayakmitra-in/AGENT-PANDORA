use pandora_types::harness::ManifestError;
use pandora_types::{GeneId, HarnessId, HarnessKind, HarnessManifest, MetaComposition};

pub const CORE_SOURCE_HARNESS_ID: &str = "core-source";
pub const CORE_SOURCE_HARNESS_VERSION: &str = "0.1.0";
pub const CODING_HARNESS_ID: &str = "coding-domain";
pub const CODING_HARNESS_VERSION: &str = "0.1.0";
pub const RESEARCH_HARNESS_ID: &str = "research-domain";
pub const RESEARCH_HARNESS_VERSION: &str = "0.1.0";
pub const DESIGN_HARNESS_ID: &str = "design-domain";
pub const DESIGN_HARNESS_VERSION: &str = "0.1.0";
pub const OPERATIONS_HARNESS_ID: &str = "operations-domain";
pub const OPERATIONS_HARNESS_VERSION: &str = "0.1.0";
pub const SECURITY_HARNESS_ID: &str = "security-domain";
pub const SECURITY_HARNESS_VERSION: &str = "0.1.0";
pub const DEBUGGING_HARNESS_ID: &str = "debugging-domain";
pub const DEBUGGING_HARNESS_VERSION: &str = "0.1.0";
pub const DATA_HARNESS_ID: &str = "data-domain";
pub const DATA_HARNESS_VERSION: &str = "0.1.0";
pub const COORDINATION_META_HARNESS_ID: &str = "coordination-meta";
pub const COORDINATION_META_HARNESS_VERSION: &str = "0.4.0";

pub fn core_source_manifest() -> Result<HarnessManifest, ManifestError> {
    HarnessManifest::new_source(
        CORE_SOURCE_HARNESS_ID,
        CORE_SOURCE_HARNESS_VERSION,
        "Pandora Core",
        "pandora-runtime",
        env!("CARGO_PKG_VERSION"),
        Vec::new(),
    )
}

pub fn coding_manifest() -> Result<HarnessManifest, ManifestError> {
    let genes = [
        "workspace.read",
        "workspace.search",
        "patch.apply",
        "verification.run",
        "tests.run",
        "format.check",
        "lint.check",
        "build.check",
        "change.review",
        "daedalus.audit",
        "argus.review",
        "ariadne.debt",
        "hephaestus.measure",
        "athena.guide",
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

pub fn research_manifest() -> Result<HarnessManifest, ManifestError> {
    let genes = [
        "evidence.inventory",
        "evidence.search",
        "source.read",
        "source.compare",
        "citation.inventory",
        "research.guide",
    ]
    .into_iter()
    .map(|id| GeneId::new(id).expect("built-in Gene ID is valid"))
    .collect();
    HarnessManifest::new(
        RESEARCH_HARNESS_ID,
        RESEARCH_HARNESS_VERSION,
        "Research Domain",
        HarnessKind::Domain,
        None,
        genes,
    )
}

pub fn design_manifest() -> Result<HarnessManifest, ManifestError> {
    let genes = [
        "design.inventory",
        "design.tokens",
        "design.inspect",
        "design.compare",
        "accessibility.evidence",
        "design.guide",
    ]
    .into_iter()
    .map(|id| GeneId::new(id).expect("built-in Gene ID is valid"))
    .collect();
    HarnessManifest::new(
        DESIGN_HARNESS_ID,
        DESIGN_HARNESS_VERSION,
        "Design Domain",
        HarnessKind::Domain,
        None,
        genes,
    )
}

pub fn operations_manifest() -> Result<HarnessManifest, ManifestError> {
    let genes = [
        "operations.inventory",
        "operations.search",
        "config.inspect",
        "config.compare",
        "deployment.evidence",
        "operations.guide",
    ]
    .into_iter()
    .map(|id| GeneId::new(id).expect("built-in Gene ID is valid"))
    .collect();
    HarnessManifest::new(
        OPERATIONS_HARNESS_ID,
        OPERATIONS_HARNESS_VERSION,
        "Operations Domain",
        HarnessKind::Domain,
        None,
        genes,
    )
}

pub fn security_manifest() -> Result<HarnessManifest, ManifestError> {
    let genes = [
        "security.assess",
        "security.audit",
        "security.scan",
        "security.deep-scan",
        "security.diff-scan",
        "security.dependencies",
        "security.threat-model",
        "security.discovery",
        "security.triage",
        "security.attack-path",
        "security.validation",
        "security.fix",
        "security.verify-fix",
        "security.writeup",
        "security.track",
        "security.hardening",
        "security.policy",
        "security.guide",
    ]
    .into_iter()
    .map(|id| GeneId::new(id).expect("built-in Gene ID is valid"))
    .collect();
    HarnessManifest::new(
        SECURITY_HARNESS_ID,
        SECURITY_HARNESS_VERSION,
        "Security Domain",
        HarnessKind::Domain,
        None,
        genes,
    )
}

pub fn debugging_manifest() -> Result<HarnessManifest, ManifestError> {
    let genes = [
        "debugging.inventory",
        "debugging.failures",
        "debugging.tests",
        "debugging.regressions",
        "debugging.diagnostics",
        "debugging.guide",
    ]
    .into_iter()
    .map(|id| GeneId::new(id).expect("built-in Gene ID is valid"))
    .collect();
    HarnessManifest::new(
        DEBUGGING_HARNESS_ID,
        DEBUGGING_HARNESS_VERSION,
        "Debugging Domain",
        HarnessKind::Domain,
        None,
        genes,
    )
}

pub fn data_manifest() -> Result<HarnessManifest, ManifestError> {
    let genes = [
        "data.inventory",
        "data.schema",
        "data.quality",
        "data.lineage",
        "data.analysis",
        "data.guide",
    ]
    .into_iter()
    .map(|id| GeneId::new(id).expect("built-in Gene ID is valid"))
    .collect();
    HarnessManifest::new(
        DATA_HARNESS_ID,
        DATA_HARNESS_VERSION,
        "Data Domain",
        HarnessKind::Domain,
        None,
        genes,
    )
}

pub fn coordination_meta_manifest() -> Result<HarnessManifest, ManifestError> {
    let composition = MetaComposition::new(
        vec![
            HarnessId::new(CODING_HARNESS_ID)?,
            HarnessId::new(RESEARCH_HARNESS_ID)?,
            HarnessId::new(DESIGN_HARNESS_ID)?,
            HarnessId::new(OPERATIONS_HARNESS_ID)?,
            HarnessId::new(SECURITY_HARNESS_ID)?,
            HarnessId::new(DEBUGGING_HARNESS_ID)?,
            HarnessId::new(DATA_HARNESS_ID)?,
        ],
        8,
    )?;
    HarnessManifest::new_meta(
        COORDINATION_META_HARNESS_ID,
        COORDINATION_META_HARNESS_VERSION,
        "Coordination Meta",
        composition,
    )
}
