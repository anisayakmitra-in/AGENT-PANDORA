use crate::PlanningContext;
use crate::genes::validate_path;
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};

const DESIGN_TOKEN_MARKERS: [&str; 4] = [":root", "var(", "--color", "@theme"];
const ACCESSIBILITY_MARKERS: [&str; 4] = ["alt=", "aria-label", "aria-labelledby", "role="];
const DESIGN_GUIDE: &str = "Design inventory maps bounded workspace assets.\nToken evidence finds explicit local design-token markers.\nDesign inspection and comparison preserve exact source evidence.\nAccessibility evidence inventories common semantic markers without claiming standards conformance.\nAll filesystem effects require Pandora permits and receipts; rendering, browser automation, and image generation require separately governed capabilities.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignAction {
    Inventory,
    Tokens,
    Inspect,
    Compare,
    AccessibilityEvidence,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DesignRequest {
    action: DesignAction,
    context: PlanningContext,
    paths: Vec<String>,
}

impl DesignRequest {
    pub fn inventory(context: PlanningContext) -> Self {
        Self::empty(DesignAction::Inventory, context)
    }

    pub fn tokens(context: PlanningContext) -> Self {
        Self::empty(DesignAction::Tokens, context)
    }

    pub fn inspect(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::paths(DesignAction::Inspect, context, [path.into()])
    }

    pub fn compare(
        context: PlanningContext,
        left: impl Into<String>,
        right: impl Into<String>,
    ) -> Self {
        Self::paths(DesignAction::Compare, context, [left.into(), right.into()])
    }

    pub fn accessibility_evidence(context: PlanningContext) -> Self {
        Self::empty(DesignAction::AccessibilityEvidence, context)
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::empty(DesignAction::Guide, context)
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        validate_request(&self)?;
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("design input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("design input must be valid JSON"))
    }

    fn empty(action: DesignAction, context: PlanningContext) -> Self {
        Self {
            action,
            context,
            paths: Vec::new(),
        }
    }

    fn paths(
        action: DesignAction,
        context: PlanningContext,
        paths: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            action,
            context,
            paths: paths.into_iter().collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesignGeneRole {
    Inventory,
    Tokens,
    Inspect,
    Compare,
    AccessibilityEvidence,
    Guide,
}

impl DesignGeneRole {
    const fn action(self) -> DesignAction {
        match self {
            Self::Inventory => DesignAction::Inventory,
            Self::Tokens => DesignAction::Tokens,
            Self::Inspect => DesignAction::Inspect,
            Self::Compare => DesignAction::Compare,
            Self::AccessibilityEvidence => DesignAction::AccessibilityEvidence,
            Self::Guide => DesignAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Inventory => "design.inventory",
            Self::Tokens => "design.tokens",
            Self::Inspect => "design.inspect",
            Self::Compare => "design.compare",
            Self::AccessibilityEvidence => "accessibility.evidence",
            Self::Guide => "design.guide",
        }
    }

    const fn kind(self) -> GeneKind {
        match self {
            Self::Inspect => GeneKind::Tool,
            Self::Inventory
            | Self::Tokens
            | Self::Compare
            | Self::AccessibilityEvidence
            | Self::Guide => GeneKind::Workflow,
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Guide => None,
            _ => Some(Capability::FilesystemRead),
        }
    }
}

pub struct DesignGene {
    manifest: GeneManifest,
    role: DesignGeneRole,
}

impl DesignGene {
    pub fn new(role: DesignGeneRole) -> Result<Self, GeneError> {
        let manifest = GeneManifest::new(
            role.id(),
            "0.1.0",
            role.kind(),
            role.capability().into_iter().collect(),
        )?;
        Ok(Self { manifest, role })
    }

    pub fn all() -> Vec<Box<dyn Gene>> {
        [
            DesignGeneRole::Inventory,
            DesignGeneRole::Tokens,
            DesignGeneRole::Inspect,
            DesignGeneRole::Compare,
            DesignGeneRole::AccessibilityEvidence,
            DesignGeneRole::Guide,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Design Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }

    fn operation_request(
        &self,
        request: &DesignRequest,
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

impl Gene for DesignGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = DesignRequest::parse(input)?;
        validate_request(&request)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "design input action does not match Gene",
            ));
        }

        let targets = match self.role {
            DesignGeneRole::Inventory => vec![EffectTarget::path(".")],
            DesignGeneRole::Tokens => DESIGN_TOKEN_MARKERS
                .into_iter()
                .map(EffectTarget::path)
                .collect(),
            DesignGeneRole::Inspect | DesignGeneRole::Compare => request
                .paths
                .iter()
                .map(|path| EffectTarget::path(path.clone()))
                .collect(),
            DesignGeneRole::AccessibilityEvidence => ACCESSIBILITY_MARKERS
                .into_iter()
                .map(EffectTarget::path)
                .collect(),
            DesignGeneRole::Guide => return Ok(Vec::new()),
        };

        targets
            .into_iter()
            .map(|target| self.operation_request(&request, target))
            .collect()
    }
}

pub fn is_design_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "design.inventory"
            | "design.tokens"
            | "design.inspect"
            | "design.compare"
            | "accessibility.evidence"
            | "design.guide"
    )
}

pub fn design_static_output(gene_id: &GeneId) -> Option<&'static str> {
    (gene_id.as_str() == "design.guide").then_some(DESIGN_GUIDE)
}

fn validate_request(request: &DesignRequest) -> Result<(), GeneError> {
    match request.action {
        DesignAction::Inventory
        | DesignAction::Tokens
        | DesignAction::AccessibilityEvidence
        | DesignAction::Guide => {
            if !request.paths.is_empty() {
                return Err(GeneError::InvalidInput(
                    "paths are not valid for this design action",
                ));
            }
        }
        DesignAction::Inspect => validate_paths(request, 1)?,
        DesignAction::Compare => {
            validate_paths(request, 2)?;
            if request.paths.iter().any(|path| path.contains('|')) {
                return Err(GeneError::InvalidInput(
                    "design comparison paths cannot contain the delimiter",
                ));
            }
            if request.paths[0] == request.paths[1] {
                return Err(GeneError::InvalidInput(
                    "design comparison requires two distinct paths",
                ));
            }
        }
    }
    Ok(())
}

fn validate_paths(request: &DesignRequest, expected: usize) -> Result<(), GeneError> {
    if request.paths.len() != expected {
        return Err(GeneError::InvalidInput(
            "design action has an invalid path count",
        ));
    }
    for path in &request.paths {
        validate_path(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        ExecutionId, ExecutionProfile, ExecutionProfileBinding, ExecutionProfileBindingKind,
        PrincipalId, SessionId, WorkspaceId, hash_artifact,
    };

    fn context() -> PlanningContext {
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
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            WorkspaceId::new("workspace-1").unwrap(),
            profile,
        )
    }

    #[test]
    fn token_evidence_uses_only_fixed_markers() {
        let gene = DesignGene::new(DesignGeneRole::Tokens).unwrap();
        let requests = gene
            .plan(&DesignRequest::tokens(context()).into_gene_input().unwrap())
            .unwrap();
        let targets = requests
            .iter()
            .map(|request| match request.target() {
                EffectTarget::Path { path } => path.as_str(),
                _ => panic!("design token evidence must remain filesystem read-only"),
            })
            .collect::<Vec<_>>();

        assert_eq!(targets, DESIGN_TOKEN_MARKERS);
    }

    #[test]
    fn comparison_plans_two_bounded_reads() {
        let gene = DesignGene::new(DesignGeneRole::Compare).unwrap();
        let requests = gene
            .plan(
                &DesignRequest::compare(context(), "first.css", "second.css")
                    .into_gene_input()
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].target(), &EffectTarget::path("first.css"));
        assert_eq!(requests[1].target(), &EffectTarget::path("second.css"));
    }

    #[test]
    fn invalid_design_paths_fail_before_planning() {
        assert!(
            DesignRequest::inspect(context(), "../outside.css")
                .into_gene_input()
                .is_err()
        );
        assert!(
            DesignRequest::compare(context(), "same.css", "same.css")
                .into_gene_input()
                .is_err()
        );
    }

    #[test]
    fn guide_is_pure_static_guidance() {
        let gene = DesignGene::new(DesignGeneRole::Guide).unwrap();
        let requests = gene
            .plan(&DesignRequest::guide(context()).into_gene_input().unwrap())
            .unwrap();

        assert!(requests.is_empty());
        assert!(gene.manifest().capabilities().is_empty());
        assert!(design_static_output(gene.manifest().id()).is_some());
    }
}
