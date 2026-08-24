use crate::PlanningContext;
use crate::genes::{validate_path, validate_query};
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};

const DEPLOYMENT_MARKERS: [&str; 4] = ["FROM ", "services:", "apiVersion:", "workflow_dispatch:"];
const OPERATIONS_GUIDE: &str = "Operations inventory maps the bounded workspace.\nOperations search locates explicit local evidence.\nConfiguration inspection and comparison preserve exact source material.\nDeployment evidence finds fixed container, Compose, Kubernetes, and workflow markers without claiming deployability.\nAll filesystem effects require Pandora permits and receipts; process, network, and infrastructure mutations require separately governed capabilities.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationsAction {
    Inventory,
    Search,
    InspectConfig,
    CompareConfig,
    DeploymentEvidence,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationsRequest {
    action: OperationsAction,
    context: PlanningContext,
    query: Option<String>,
    paths: Vec<String>,
}

impl OperationsRequest {
    pub fn inventory(context: PlanningContext) -> Self {
        Self::empty(OperationsAction::Inventory, context)
    }

    pub fn search(context: PlanningContext, query: impl Into<String>) -> Self {
        Self {
            action: OperationsAction::Search,
            context,
            query: Some(query.into()),
            paths: Vec::new(),
        }
    }

    pub fn inspect_config(context: PlanningContext, path: impl Into<String>) -> Self {
        Self::paths(OperationsAction::InspectConfig, context, [path.into()])
    }

    pub fn compare_config(
        context: PlanningContext,
        left: impl Into<String>,
        right: impl Into<String>,
    ) -> Self {
        Self::paths(
            OperationsAction::CompareConfig,
            context,
            [left.into(), right.into()],
        )
    }

    pub fn deployment_evidence(context: PlanningContext) -> Self {
        Self::empty(OperationsAction::DeploymentEvidence, context)
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::empty(OperationsAction::Guide, context)
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        validate_request(&self)?;
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("operations input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("operations input must be valid JSON"))
    }

    fn empty(action: OperationsAction, context: PlanningContext) -> Self {
        Self {
            action,
            context,
            query: None,
            paths: Vec::new(),
        }
    }

    fn paths(
        action: OperationsAction,
        context: PlanningContext,
        paths: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            action,
            context,
            query: None,
            paths: paths.into_iter().collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationsGeneRole {
    Inventory,
    Search,
    ConfigInspect,
    ConfigCompare,
    DeploymentEvidence,
    Guide,
}

impl OperationsGeneRole {
    const fn action(self) -> OperationsAction {
        match self {
            Self::Inventory => OperationsAction::Inventory,
            Self::Search => OperationsAction::Search,
            Self::ConfigInspect => OperationsAction::InspectConfig,
            Self::ConfigCompare => OperationsAction::CompareConfig,
            Self::DeploymentEvidence => OperationsAction::DeploymentEvidence,
            Self::Guide => OperationsAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Inventory => "operations.inventory",
            Self::Search => "operations.search",
            Self::ConfigInspect => "config.inspect",
            Self::ConfigCompare => "config.compare",
            Self::DeploymentEvidence => "deployment.evidence",
            Self::Guide => "operations.guide",
        }
    }

    const fn kind(self) -> GeneKind {
        match self {
            Self::Search | Self::ConfigInspect => GeneKind::Tool,
            Self::Inventory | Self::ConfigCompare | Self::DeploymentEvidence | Self::Guide => {
                GeneKind::Workflow
            }
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Guide => None,
            _ => Some(Capability::FilesystemRead),
        }
    }
}

pub struct OperationsGene {
    manifest: GeneManifest,
    role: OperationsGeneRole,
}

impl OperationsGene {
    pub fn new(role: OperationsGeneRole) -> Result<Self, GeneError> {
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
            OperationsGeneRole::Inventory,
            OperationsGeneRole::Search,
            OperationsGeneRole::ConfigInspect,
            OperationsGeneRole::ConfigCompare,
            OperationsGeneRole::DeploymentEvidence,
            OperationsGeneRole::Guide,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Operations Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }

    fn operation_request(
        &self,
        request: &OperationsRequest,
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

impl Gene for OperationsGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = OperationsRequest::parse(input)?;
        validate_request(&request)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "operations input action does not match Gene",
            ));
        }

        let targets = match self.role {
            OperationsGeneRole::Inventory => vec![EffectTarget::path(".")],
            OperationsGeneRole::Search => vec![EffectTarget::path(
                request
                    .query
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("query is required"))?,
            )],
            OperationsGeneRole::ConfigInspect | OperationsGeneRole::ConfigCompare => request
                .paths
                .iter()
                .map(|path| EffectTarget::path(path.clone()))
                .collect(),
            OperationsGeneRole::DeploymentEvidence => DEPLOYMENT_MARKERS
                .into_iter()
                .map(EffectTarget::path)
                .collect(),
            OperationsGeneRole::Guide => return Ok(Vec::new()),
        };

        targets
            .into_iter()
            .map(|target| self.operation_request(&request, target))
            .collect()
    }
}

pub fn is_operations_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "operations.inventory"
            | "operations.search"
            | "config.inspect"
            | "config.compare"
            | "deployment.evidence"
            | "operations.guide"
    )
}

pub fn operations_static_output(gene_id: &GeneId) -> Option<&'static str> {
    (gene_id.as_str() == "operations.guide").then_some(OPERATIONS_GUIDE)
}

fn validate_request(request: &OperationsRequest) -> Result<(), GeneError> {
    match request.action {
        OperationsAction::Inventory
        | OperationsAction::DeploymentEvidence
        | OperationsAction::Guide => {
            if request.query.is_some() || !request.paths.is_empty() {
                return Err(GeneError::InvalidInput(
                    "query and paths are not valid for this operations action",
                ));
            }
        }
        OperationsAction::Search => {
            validate_query(
                request
                    .query
                    .as_deref()
                    .ok_or(GeneError::InvalidInput("query is required"))?,
            )?;
            if !request.paths.is_empty() {
                return Err(GeneError::InvalidInput(
                    "paths are not valid for operations search",
                ));
            }
        }
        OperationsAction::InspectConfig => validate_paths(request, 1)?,
        OperationsAction::CompareConfig => {
            validate_paths(request, 2)?;
            if request.paths.iter().any(|path| path.contains('|')) {
                return Err(GeneError::InvalidInput(
                    "configuration comparison paths cannot contain the delimiter",
                ));
            }
            if request.paths[0] == request.paths[1] {
                return Err(GeneError::InvalidInput(
                    "configuration comparison requires two distinct paths",
                ));
            }
        }
    }
    Ok(())
}

fn validate_paths(request: &OperationsRequest, expected: usize) -> Result<(), GeneError> {
    if request.query.is_some() || request.paths.len() != expected {
        return Err(GeneError::InvalidInput(
            "operations action has an invalid path count",
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
    fn deployment_evidence_uses_only_fixed_markers() {
        let gene = OperationsGene::new(OperationsGeneRole::DeploymentEvidence).unwrap();
        let requests = gene
            .plan(
                &OperationsRequest::deployment_evidence(context())
                    .into_gene_input()
                    .unwrap(),
            )
            .unwrap();
        let targets = requests
            .iter()
            .map(|request| match request.target() {
                EffectTarget::Path { path } => path.as_str(),
                _ => panic!("deployment evidence must remain filesystem read-only"),
            })
            .collect::<Vec<_>>();

        assert_eq!(targets, DEPLOYMENT_MARKERS);
        assert!(requests.iter().all(|request| {
            request.capability() == Capability::FilesystemRead
                && request.operation() == Operation::Read
        }));
    }

    #[test]
    fn configuration_comparison_plans_two_bounded_reads() {
        let gene = OperationsGene::new(OperationsGeneRole::ConfigCompare).unwrap();
        let requests = gene
            .plan(
                &OperationsRequest::compare_config(context(), "first.toml", "second.toml")
                    .into_gene_input()
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].target(), &EffectTarget::path("first.toml"));
        assert_eq!(requests[1].target(), &EffectTarget::path("second.toml"));
    }

    #[test]
    fn invalid_operations_inputs_fail_before_planning() {
        assert!(
            OperationsRequest::inspect_config(context(), "../outside.toml")
                .into_gene_input()
                .is_err()
        );
        assert!(
            OperationsRequest::compare_config(context(), "same.toml", "same.toml")
                .into_gene_input()
                .is_err()
        );
        assert!(
            OperationsRequest::search(context(), "")
                .into_gene_input()
                .is_err()
        );
    }

    #[test]
    fn guide_is_pure_static_guidance() {
        let gene = OperationsGene::new(OperationsGeneRole::Guide).unwrap();
        let requests = gene
            .plan(
                &OperationsRequest::guide(context())
                    .into_gene_input()
                    .unwrap(),
            )
            .unwrap();

        assert!(requests.is_empty());
        assert!(gene.manifest().capabilities().is_empty());
        assert!(operations_static_output(gene.manifest().id()).is_some());
    }
}
