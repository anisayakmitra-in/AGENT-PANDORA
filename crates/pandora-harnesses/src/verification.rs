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
const VERIFICATION_GUIDE: &str = "Verification contract audits runtime preconditions, postconditions, and API type invariants.\nGolden verification executes regression suites against deterministic baselines and reference fixtures.\nProperty verification checks state-machine transitions and algebraic properties via automated exploration.\nAll verification effects operate under governed read permits; test execution requires authorized sandbox permits.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationAction {
    Contract,
    Golden,
    Property,
    Guide,
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
}

impl VerificationGeneRole {
    const fn action(self) -> VerificationAction {
        match self {
            Self::Contract => VerificationAction::Contract,
            Self::Golden => VerificationAction::Golden,
            Self::Property => VerificationAction::Property,
            Self::Guide => VerificationAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Contract => "verification.contract",
            Self::Golden => "verification.golden",
            Self::Property => "verification.property",
            Self::Guide => "verification.guide",
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

    pub fn all() -> Vec<Box<dyn Gene>> {
        [
            VerificationGeneRole::Contract,
            VerificationGeneRole::Golden,
            VerificationGeneRole::Property,
            VerificationGeneRole::Guide,
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
        };
        targets
            .iter()
            .map(|marker| self.operation_request(&request, EffectTarget::path(*marker)))
            .collect()
    }
}

pub fn is_verification_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "verification.contract"
            | "verification.golden"
            | "verification.property"
            | "verification.guide"
    )
}

pub fn verification_static_output(gene_id: &GeneId) -> Option<&'static str> {
    match gene_id.as_str() {
        "verification.guide" => Some(VERIFICATION_GUIDE),
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
                _ => panic!("unknown gene"),
            };
            let request = match role {
                VerificationGeneRole::Contract => VerificationRequest::contract(test_context()),
                VerificationGeneRole::Golden => VerificationRequest::golden(test_context()),
                VerificationGeneRole::Property => VerificationRequest::property(test_context()),
                VerificationGeneRole::Guide => VerificationRequest::guide(test_context()),
            };
            let input = request.into_gene_input().unwrap();
            let operations = gene.plan(&input).unwrap();
            match role {
                VerificationGeneRole::Contract => assert_eq!(operations.len(), 8),
                VerificationGeneRole::Golden => assert_eq!(operations.len(), 8),
                VerificationGeneRole::Property => assert_eq!(operations.len(), 8),
                VerificationGeneRole::Guide => assert!(operations.is_empty()),
            }
        }
    }

    #[test]
    fn verification_guide_static_output_is_available() {
        let gene = VerificationGene::new(VerificationGeneRole::Guide).unwrap();
        assert!(verification_static_output(gene.manifest().id()).is_some());
    }
}
