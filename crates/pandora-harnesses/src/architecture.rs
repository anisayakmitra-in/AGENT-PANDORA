use crate::PlanningContext;
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};

const BOUNDARY_MARKERS: [&str; 8] = [
    "pub(crate)",
    "pub ",
    "export ",
    "internal",
    "private",
    "boundary",
    "contract",
    "interface",
];
const INVARIANT_MARKERS: [&str; 8] = [
    "layer",
    "invariant",
    "dependency",
    "cyclic",
    "forbidden",
    "violation",
    "strict",
    "isolated",
];
const ARCHITECTURE_GUIDE: &str = "Architecture inventory maps subsystem modules, structural layers, and dependency hierarchies.\nBoundary analysis verifies encapsulation, API surface exposure, and interface invariants.\nInvariant analysis audits layered boundaries and prevents circular or forbidden coupling.\nAll filesystem effects require Pandora permits and receipts; structural refactorings must preserve interface compatibility.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureAction {
    Inventory,
    Boundaries,
    Invariants,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureRequest {
    action: ArchitectureAction,
    context: PlanningContext,
}

impl ArchitectureRequest {
    pub fn inventory(context: PlanningContext) -> Self {
        Self::new(ArchitectureAction::Inventory, context)
    }

    pub fn boundaries(context: PlanningContext) -> Self {
        Self::new(ArchitectureAction::Boundaries, context)
    }

    pub fn invariants(context: PlanningContext) -> Self {
        Self::new(ArchitectureAction::Invariants, context)
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::new(ArchitectureAction::Guide, context)
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("architecture input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("architecture input must be valid JSON"))
    }

    const fn new(action: ArchitectureAction, context: PlanningContext) -> Self {
        Self { action, context }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchitectureGeneRole {
    Inventory,
    Boundaries,
    Invariants,
    Guide,
}

impl ArchitectureGeneRole {
    const fn action(self) -> ArchitectureAction {
        match self {
            Self::Inventory => ArchitectureAction::Inventory,
            Self::Boundaries => ArchitectureAction::Boundaries,
            Self::Invariants => ArchitectureAction::Invariants,
            Self::Guide => ArchitectureAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Inventory => "architecture.inventory",
            Self::Boundaries => "architecture.boundaries",
            Self::Invariants => "architecture.invariants",
            Self::Guide => "architecture.guide",
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Guide => None,
            _ => Some(Capability::FilesystemRead),
        }
    }
}

pub struct ArchitectureGene {
    manifest: GeneManifest,
    role: ArchitectureGeneRole,
}

impl ArchitectureGene {
    pub fn new(role: ArchitectureGeneRole) -> Result<Self, GeneError> {
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
            ArchitectureGeneRole::Inventory,
            ArchitectureGeneRole::Boundaries,
            ArchitectureGeneRole::Invariants,
            ArchitectureGeneRole::Guide,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Architecture Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }

    fn operation_request(
        &self,
        request: &ArchitectureRequest,
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

impl Gene for ArchitectureGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = ArchitectureRequest::parse(input)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "architecture input action does not match Gene",
            ));
        }
        let targets: &[&str] = match self.role {
            ArchitectureGeneRole::Inventory => {
                return Ok(vec![
                    self.operation_request(&request, EffectTarget::path("."))?,
                ]);
            }
            ArchitectureGeneRole::Boundaries => &BOUNDARY_MARKERS,
            ArchitectureGeneRole::Invariants => &INVARIANT_MARKERS,
            ArchitectureGeneRole::Guide => return Ok(Vec::new()),
        };
        targets
            .iter()
            .map(|marker| self.operation_request(&request, EffectTarget::path(*marker)))
            .collect()
    }
}

pub fn is_architecture_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "architecture.inventory"
            | "architecture.boundaries"
            | "architecture.invariants"
            | "architecture.guide"
    )
}

pub fn architecture_static_output(gene_id: &GeneId) -> Option<&'static str> {
    match gene_id.as_str() {
        "architecture.guide" => Some(ARCHITECTURE_GUIDE),
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
            ExecutionId::new("exec-architecture").unwrap(),
            SessionId::new("session-architecture").unwrap(),
            PrincipalId::new("principal-architecture").unwrap(),
            WorkspaceId::new("workspace-architecture").unwrap(),
            profile,
        )
        .with_artifact(ArtifactId::new("artifact-architecture").unwrap())
    }

    #[test]
    fn architecture_genes_plan_requests_within_boundaries() {
        for gene in ArchitectureGene::all() {
            let role = match gene.manifest().id().as_str() {
                "architecture.inventory" => ArchitectureGeneRole::Inventory,
                "architecture.boundaries" => ArchitectureGeneRole::Boundaries,
                "architecture.invariants" => ArchitectureGeneRole::Invariants,
                "architecture.guide" => ArchitectureGeneRole::Guide,
                _ => panic!("unknown gene"),
            };
            let request = match role {
                ArchitectureGeneRole::Inventory => ArchitectureRequest::inventory(test_context()),
                ArchitectureGeneRole::Boundaries => ArchitectureRequest::boundaries(test_context()),
                ArchitectureGeneRole::Invariants => ArchitectureRequest::invariants(test_context()),
                ArchitectureGeneRole::Guide => ArchitectureRequest::guide(test_context()),
            };
            let input = request.into_gene_input().unwrap();
            let operations = gene.plan(&input).unwrap();
            match role {
                ArchitectureGeneRole::Inventory => assert_eq!(operations.len(), 1),
                ArchitectureGeneRole::Boundaries => assert_eq!(operations.len(), 8),
                ArchitectureGeneRole::Invariants => assert_eq!(operations.len(), 8),
                ArchitectureGeneRole::Guide => assert!(operations.is_empty()),
            }
        }
    }

    #[test]
    fn architecture_guide_static_output_is_available() {
        let gene = ArchitectureGene::new(ArchitectureGeneRole::Guide).unwrap();
        assert!(architecture_static_output(gene.manifest().id()).is_some());
    }
}
