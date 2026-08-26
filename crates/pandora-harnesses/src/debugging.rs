use crate::PlanningContext;
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};

const FAILURE_MARKERS: [&str; 8] = [
    "panic!",
    "unwrap()",
    "expect(",
    "traceback",
    "stack trace",
    "Exception",
    "ERROR",
    "failed",
];
const TEST_MARKERS: [&str; 8] = [
    "#[test]",
    "#[cfg(test)]",
    "assert!(",
    "assert_eq!(",
    "cargo test",
    "pytest",
    "jest",
    "go test",
];
const REGRESSION_MARKERS: [&str; 8] = [
    "regression",
    "reproduce",
    "repro",
    "minimal reproduction",
    "expected:",
    "actual:",
    "steps to reproduce",
    "git bisect",
];
const DIAGNOSTIC_MARKERS: [&str; 8] = [
    "debug",
    "backtrace",
    "stack trace",
    "assertion",
    "timeout",
    "flaky",
    "race",
    "deadlock",
];
const DEBUGGING_GUIDE: &str = "Debugging inventory maps the bounded workspace.\nFailure evidence searches fixed crash and error markers.\nTest evidence searches fixed test and assertion markers.\nRegression evidence searches reproduction and comparison markers.\nDiagnostic evidence searches fixed runtime symptom markers.\nAll filesystem effects require Pandora permits and receipts; process execution and code changes require separately governed capabilities.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DebuggingAction {
    Inventory,
    Failures,
    Tests,
    Regressions,
    Diagnostics,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DebuggingRequest {
    action: DebuggingAction,
    context: PlanningContext,
}

impl DebuggingRequest {
    pub fn inventory(context: PlanningContext) -> Self {
        Self::new(DebuggingAction::Inventory, context)
    }

    pub fn failures(context: PlanningContext) -> Self {
        Self::new(DebuggingAction::Failures, context)
    }

    pub fn tests(context: PlanningContext) -> Self {
        Self::new(DebuggingAction::Tests, context)
    }

    pub fn regressions(context: PlanningContext) -> Self {
        Self::new(DebuggingAction::Regressions, context)
    }

    pub fn diagnostics(context: PlanningContext) -> Self {
        Self::new(DebuggingAction::Diagnostics, context)
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::new(DebuggingAction::Guide, context)
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("debugging input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("debugging input must be valid JSON"))
    }

    const fn new(action: DebuggingAction, context: PlanningContext) -> Self {
        Self { action, context }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebuggingGeneRole {
    Inventory,
    Failures,
    Tests,
    Regressions,
    Diagnostics,
    Guide,
}

impl DebuggingGeneRole {
    const fn action(self) -> DebuggingAction {
        match self {
            Self::Inventory => DebuggingAction::Inventory,
            Self::Failures => DebuggingAction::Failures,
            Self::Tests => DebuggingAction::Tests,
            Self::Regressions => DebuggingAction::Regressions,
            Self::Diagnostics => DebuggingAction::Diagnostics,
            Self::Guide => DebuggingAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Inventory => "debugging.inventory",
            Self::Failures => "debugging.failures",
            Self::Tests => "debugging.tests",
            Self::Regressions => "debugging.regressions",
            Self::Diagnostics => "debugging.diagnostics",
            Self::Guide => "debugging.guide",
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Guide => None,
            _ => Some(Capability::FilesystemRead),
        }
    }
}

pub struct DebuggingGene {
    manifest: GeneManifest,
    role: DebuggingGeneRole,
}

impl DebuggingGene {
    pub fn new(role: DebuggingGeneRole) -> Result<Self, GeneError> {
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
            DebuggingGeneRole::Inventory,
            DebuggingGeneRole::Failures,
            DebuggingGeneRole::Tests,
            DebuggingGeneRole::Regressions,
            DebuggingGeneRole::Diagnostics,
            DebuggingGeneRole::Guide,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Debugging Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }

    fn operation_request(
        &self,
        request: &DebuggingRequest,
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

impl Gene for DebuggingGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = DebuggingRequest::parse(input)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "debugging input action does not match Gene",
            ));
        }
        let targets = match self.role {
            DebuggingGeneRole::Inventory => vec![EffectTarget::path(".")],
            DebuggingGeneRole::Failures => FAILURE_MARKERS
                .into_iter()
                .map(EffectTarget::path)
                .collect(),
            DebuggingGeneRole::Tests => TEST_MARKERS.into_iter().map(EffectTarget::path).collect(),
            DebuggingGeneRole::Regressions => REGRESSION_MARKERS
                .into_iter()
                .map(EffectTarget::path)
                .collect(),
            DebuggingGeneRole::Diagnostics => DIAGNOSTIC_MARKERS
                .into_iter()
                .map(EffectTarget::path)
                .collect(),
            DebuggingGeneRole::Guide => return Ok(Vec::new()),
        };

        targets
            .into_iter()
            .map(|target| self.operation_request(&request, target))
            .collect()
    }
}

pub fn is_debugging_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "debugging.inventory"
            | "debugging.failures"
            | "debugging.tests"
            | "debugging.regressions"
            | "debugging.diagnostics"
            | "debugging.guide"
    )
}

pub fn debugging_static_output(gene_id: &GeneId) -> Option<&'static str> {
    (gene_id.as_str() == "debugging.guide").then_some(DEBUGGING_GUIDE)
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
    fn failure_evidence_uses_only_fixed_read_searches() {
        let gene = DebuggingGene::new(DebuggingGeneRole::Failures).unwrap();
        let requests = gene
            .plan(
                &DebuggingRequest::failures(context())
                    .into_gene_input()
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(requests.len(), FAILURE_MARKERS.len());
        assert!(requests.iter().all(|request| {
            request.capability() == Capability::FilesystemRead
                && request.operation() == Operation::Read
                && request.resource_scope() == &ResourceScope::workspace("workspace-1")
        }));
        assert!(requests.iter().all(|request| {
            matches!(request.target(), EffectTarget::Path { path } if FAILURE_MARKERS.contains(&path.as_str()))
        }));
    }

    #[test]
    fn guide_is_pure_static_guidance() {
        let gene = DebuggingGene::new(DebuggingGeneRole::Guide).unwrap();
        let requests = gene
            .plan(
                &DebuggingRequest::guide(context())
                    .into_gene_input()
                    .unwrap(),
            )
            .unwrap();

        assert!(requests.is_empty());
        assert!(gene.manifest().capabilities().is_empty());
        assert!(debugging_static_output(gene.manifest().id()).is_some());
    }

    #[test]
    fn evidence_roles_have_distinct_ids_and_read_only_capabilities() {
        let roles = [
            DebuggingGeneRole::Inventory,
            DebuggingGeneRole::Failures,
            DebuggingGeneRole::Tests,
            DebuggingGeneRole::Regressions,
            DebuggingGeneRole::Diagnostics,
        ];
        let ids = roles
            .into_iter()
            .map(|role| DebuggingGene::new(role).unwrap().manifest.id().clone())
            .collect::<Vec<_>>();

        assert_eq!(ids.len(), 5);
        assert!(ids.windows(2).all(|pair| pair[0] != pair[1]));
        for role in roles {
            assert_eq!(
                DebuggingGene::new(role).unwrap().manifest().capabilities(),
                &[Capability::FilesystemRead]
            );
        }
    }
}
