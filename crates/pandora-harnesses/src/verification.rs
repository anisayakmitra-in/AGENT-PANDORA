use crate::PlanningContext;
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};

const CONTRACT_MARKERS: [&str; 8] = [
    "assert!",
    "ensure!",
    "require!",
    "contract",
    "invariant",
    "precondition",
    "postcondition",
    "Result<",
];
const GOLDEN_MARKERS: [&str; 8] = [
    "golden",
    "fixture",
    "baseline",
    "expected",
    "snapshot",
    "reference",
    "diff",
    "oracle",
];
const PROPERTY_MARKERS: [&str; 8] = [
    "proptest",
    "quickcheck",
    "fuzz",
    "arbitrary",
    "strategy",
    "transition",
    "state_machine",
    "property",
];
const VISUAL_MARKERS: [&str; 8] = [
    "screenshot",
    "snapshot",
    "layout",
    "dom",
    "viewport",
    "render",
    "visual",
    "canvas",
];
const VERIFICATION_GUIDE: &str = "Verification contract audits runtime preconditions, postconditions, and API type invariants.\nGolden verification executes regression suites against deterministic baselines and reference fixtures.\nProperty verification checks state-machine transitions and algebraic properties via automated exploration.\nAll verification effects operate under governed read permits; test execution requires authorized sandbox permits.";
const VISUAL_GUIDE: &str = "Visual verification executes automated headless browser flows via mock Chrome DevTools Protocol (CDP).\nSupports DOM element selection, visibility assertions, text verification, keyboard/mouse interaction, and synthetic PNG screenshot rasterization.\nAll visual effects operate with zero external browser binaries.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationAction {
    Contract,
    Golden,
    Property,
    Guide,
    Visual,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationRequest {
    action: VerificationAction,
    context: PlanningContext,
}

impl VerificationRequest {
    pub fn contract(context: PlanningContext) -> Self {
        Self::new(VerificationAction::Contract, context)
    }

    pub fn golden(context: PlanningContext) -> Self {
        Self::new(VerificationAction::Golden, context)
    }

    pub fn property(context: PlanningContext) -> Self {
        Self::new(VerificationAction::Property, context)
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::new(VerificationAction::Guide, context)
    }

    pub fn visual(context: PlanningContext) -> Self {
        Self::new(VerificationAction::Visual, context)
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("verification input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("verification input must be valid JSON"))
    }

    const fn new(action: VerificationAction, context: PlanningContext) -> Self {
        Self { action, context }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationGeneRole {
    Contract,
    Golden,
    Property,
    Guide,
    Visual,
}

impl VerificationGeneRole {
    const fn action(self) -> VerificationAction {
        match self {
            Self::Contract => VerificationAction::Contract,
            Self::Golden => VerificationAction::Golden,
            Self::Property => VerificationAction::Property,
            Self::Guide => VerificationAction::Guide,
            Self::Visual => VerificationAction::Visual,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Contract => "verification.contract",
            Self::Golden => "verification.golden",
            Self::Property => "verification.property",
            Self::Guide => "verification.guide",
            Self::Visual => "verification.visual",
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Guide => None,
            _ => Some(Capability::FilesystemRead),
        }
    }
}

pub struct VerificationGene {
    manifest: GeneManifest,
    role: VerificationGeneRole,
}

impl VerificationGene {
    pub fn new(role: VerificationGeneRole) -> Result<Self, GeneError> {
        let manifest = GeneManifest::new(
            role.id(),
            "0.1.0",
            GeneKind::Workflow,
            role.capability().into_iter().collect(),
        )?;
        Ok(Self { manifest, role })
    }

    pub fn visual() -> Result<Self, GeneError> {
        Self::new(VerificationGeneRole::Visual)
    }

    pub fn all() -> Vec<Box<dyn Gene>> {
        [
            VerificationGeneRole::Contract,
            VerificationGeneRole::Golden,
            VerificationGeneRole::Property,
            VerificationGeneRole::Guide,
            VerificationGeneRole::Visual,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Verification Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }

    fn operation_request(
        &self,
        request: &VerificationRequest,
        target: EffectTarget,
    ) -> Result<OperationRequest, GeneError> {
        Ok(OperationRequest::new(
            request.context.execution_id().clone(),
            request.context.session_id().clone(),
            request.context.principal_id().clone(),
            request.context.execution_profile().clone(),
            self.manifest.id().clone(),
            request.context.artifact_id().cloned(),
            Capability::FilesystemRead,
            Operation::Read,
            target,
            ResourceScope::workspace(request.context.workspace_id().as_str()),
        )?)
    }
}

impl Gene for VerificationGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = VerificationRequest::parse(input)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "verification input action does not match Gene",
            ));
        }
        let targets: &[&str] = match self.role {
            VerificationGeneRole::Contract => &CONTRACT_MARKERS,
            VerificationGeneRole::Golden => &GOLDEN_MARKERS,
            VerificationGeneRole::Property => &PROPERTY_MARKERS,
            VerificationGeneRole::Guide => return Ok(Vec::new()),
            VerificationGeneRole::Visual => &VISUAL_MARKERS,
        };
        targets
            .iter()
            .map(|marker| self.operation_request(&request, EffectTarget::path(*marker)))
            .collect()
    }
}

pub struct VisualVerificationGene {
    inner: VerificationGene,
}

impl VisualVerificationGene {
    pub fn new() -> Result<Self, GeneError> {
        Ok(Self {
            inner: VerificationGene::new(VerificationGeneRole::Visual)?,
        })
    }
}

impl Gene for VisualVerificationGene {
    fn manifest(&self) -> &GeneManifest {
        self.inner.manifest()
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        self.inner.plan(input)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VisualViewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VisualStep {
    #[serde(rename = "type")]
    pub step_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub expected_hash: Option<String>,
    #[serde(default)]
    pub expected_text: Option<String>,
    #[serde(default)]
    pub expected_value: Option<serde_json::Value>,
    #[serde(default)]
    pub tolerance_percent: Option<f32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScreenshotArtifact {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
    pub base64_png: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VisualVerificationReport {
    pub passed: bool,
    pub steps_total: usize,
    pub steps_passed: usize,
    pub assertions_passed: usize,
    pub assertions_failed: usize,
    pub screenshots: Vec<ScreenshotArtifact>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VisualVerificationSpec {
    pub url: Option<String>,
    pub html: Option<String>,
    pub viewport: Option<VisualViewport>,
    pub steps: Vec<VisualStep>,
}

/// Asserts that two screenshot SHA-256 hashes match.
pub fn assert_screenshot_hash(actual_hash: &str, expected_hash: &str) -> Result<(), String> {
    if actual_hash.eq_ignore_ascii_case(expected_hash) {
        Ok(())
    } else {
        Err(format!(
            "screenshot hash mismatch: expected {expected_hash}, got {actual_hash}"
        ))
    }
}

/// Asserts pixel tolerance between two RGBA byte buffers.
/// Computes difference ratio and verifies it is <= tolerance_percent.
pub fn assert_pixel_tolerance(
    actual: &[u8],
    expected: &[u8],
    tolerance_percent: f32,
) -> Result<f32, String> {
    if actual.len() != expected.len() {
        return Err(format!(
            "pixel buffer size mismatch: actual {} bytes, expected {} bytes",
            actual.len(),
            expected.len()
        ));
    }
    if actual.is_empty() {
        return Ok(0.0);
    }

    let total_pixels = actual.len() / 4;
    let mut mismatched_pixels = 0usize;

    for i in (0..actual.len()).step_by(4) {
        if actual[i..i + 4] != expected[i..i + 4] {
            mismatched_pixels += 1;
        }
    }

    let diff_ratio = (mismatched_pixels as f32 / total_pixels as f32) * 100.0;
    if diff_ratio <= tolerance_percent {
        Ok(diff_ratio)
    } else {
        Err(format!(
            "pixel mismatch ratio {diff_ratio:.2}% exceeds threshold {tolerance_percent:.2}% ({mismatched_pixels}/{total_pixels} pixels differ)"
        ))
    }
}

/// Asserts that element text matches expected string.
pub fn assert_text_matches(actual: &str, expected: &str, exact: bool) -> Result<(), String> {
    let matches = if exact {
        actual == expected
    } else {
        actual.contains(expected)
    };
    if matches {
        Ok(())
    } else {
        Err(format!(
            "text assertion failed: expected '{expected}', got '{actual}'"
        ))
    }
}

/// Asserts element visibility state.
pub fn assert_element_visibility(is_visible: bool, expected: bool) -> Result<(), String> {
    if is_visible == expected {
        Ok(())
    } else {
        Err(format!(
            "visibility assertion failed: expected {expected}, got {is_visible}"
        ))
    }
}

pub fn is_verification_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "verification.contract"
            | "verification.golden"
            | "verification.property"
            | "verification.guide"
            | "verification.visual"
    )
}

pub fn verification_static_output(gene_id: &GeneId) -> Option<&'static str> {
    match gene_id.as_str() {
        "verification.guide" => Some(VERIFICATION_GUIDE),
        "verification.visual" => Some(VISUAL_GUIDE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genes::PlanningContext;
    use pandora_types::{
        ArtifactId, ExecutionId, ExecutionProfile, ExecutionProfileBinding,
        ExecutionProfileBindingKind, PrincipalId, SessionId, WorkspaceId, hash_artifact,
    };

    fn test_context() -> PlanningContext {
        let profile = ExecutionProfile::new(
            env!("CARGO_PKG_VERSION"),
            "windows",
            "x86_64",
            1,
            "workspace-1",
            hash_artifact(b"containment"),
            vec![
                ExecutionProfileBinding::new(
                    ExecutionProfileBindingKind::Executor,
                    "filesystem",
                    Some(env!("CARGO_PKG_VERSION")),
                    hash_artifact(b"filesystem"),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        PlanningContext::new(
            ExecutionId::new("exec-verification").unwrap(),
            SessionId::new("session-verification").unwrap(),
            PrincipalId::new("principal-verification").unwrap(),
            WorkspaceId::new("workspace-verification").unwrap(),
            profile,
        )
        .with_artifact(ArtifactId::new("artifact-verification").unwrap())
    }

    #[test]
    fn verification_genes_plan_requests_within_boundaries() {
        for gene in VerificationGene::all() {
            let role = match gene.manifest().id().as_str() {
                "verification.contract" => VerificationGeneRole::Contract,
                "verification.golden" => VerificationGeneRole::Golden,
                "verification.property" => VerificationGeneRole::Property,
                "verification.guide" => VerificationGeneRole::Guide,
                "verification.visual" => VerificationGeneRole::Visual,
                _ => panic!("unknown gene"),
            };
            let request = match role {
                VerificationGeneRole::Contract => VerificationRequest::contract(test_context()),
                VerificationGeneRole::Golden => VerificationRequest::golden(test_context()),
                VerificationGeneRole::Property => VerificationRequest::property(test_context()),
                VerificationGeneRole::Guide => VerificationRequest::guide(test_context()),
                VerificationGeneRole::Visual => VerificationRequest::visual(test_context()),
            };
            let input = request.into_gene_input().unwrap();
            let operations = gene.plan(&input).unwrap();
            match role {
                VerificationGeneRole::Contract => assert_eq!(operations.len(), 8),
                VerificationGeneRole::Golden => assert_eq!(operations.len(), 8),
                VerificationGeneRole::Property => assert_eq!(operations.len(), 8),
                VerificationGeneRole::Guide => assert!(operations.is_empty()),
                VerificationGeneRole::Visual => assert_eq!(operations.len(), 8),
            }
        }
    }

    #[test]
    fn verification_guide_static_output_is_available() {
        let gene = VerificationGene::new(VerificationGeneRole::Guide).unwrap();
        assert!(verification_static_output(gene.manifest().id()).is_some());
        let visual_gene = VerificationGene::new(VerificationGeneRole::Visual).unwrap();
        assert!(verification_static_output(visual_gene.manifest().id()).is_some());
    }

    #[test]
    fn visual_verification_assertions_work() {
        assert!(assert_screenshot_hash("abcdef1234", "ABCDEF1234").is_ok());
        assert!(assert_screenshot_hash("abcdef1234", "wrong").is_err());

        assert!(assert_text_matches("Welcome to Pandora", "Pandora", false).is_ok());
        assert!(assert_text_matches("Welcome", "welcome", true).is_err());

        assert!(assert_element_visibility(true, true).is_ok());
        assert!(assert_element_visibility(false, true).is_err());

        let buf1 = vec![255, 0, 0, 255, 0, 255, 0, 255]; // 2 pixels
        let buf2 = vec![255, 0, 0, 255, 0, 0, 255, 255]; // 1 pixel differs (50%)
        assert!(assert_pixel_tolerance(&buf1, &buf2, 60.0).is_ok());
        assert!(assert_pixel_tolerance(&buf1, &buf2, 40.0).is_err());
    }
}
