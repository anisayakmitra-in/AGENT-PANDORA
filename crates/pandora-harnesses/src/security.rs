use crate::PlanningContext;
use pandora_types::gene::{GeneError, GeneKind};
use pandora_types::{
    Capability, EffectTarget, Gene, GeneId, GeneInput, GeneManifest, Operation, OperationRequest,
    ResourceScope,
};
use serde::{Deserialize, Serialize};

const AUDIT_MARKERS: [&str; 6] = [
    "unsafe",
    "Command::new",
    "std::process::Command",
    "reqwest::",
    "serde_json::from_str",
    "secret",
];
const DEPENDENCY_MARKERS: [&str; 4] = [
    "[dependencies]",
    "[dev-dependencies]",
    "dependencies:",
    "\"dependencies\"",
];
const POLICY_MARKERS: [&str; 4] = ["SECURITY", "permission", "approval", "credential"];
const SECURITY_GUIDE: &str = "Security Audit searches fixed high-signal boundary markers and returns evidence paths.\nSecurity Dependencies searches dependency declarations without claiming vulnerability coverage.\nSecurity Policy searches local policy and authorization terminology without certifying compliance.\nAll filesystem effects require Pandora permits and receipts; process, network, package, and remediation actions require separate governed capabilities.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAction {
    Audit,
    Dependencies,
    Policy,
    Guide,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityRequest {
    action: SecurityAction,
    context: PlanningContext,
}

impl SecurityRequest {
    pub fn audit(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Audit, context)
    }

    pub fn dependencies(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Dependencies, context)
    }

    pub fn policy(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Policy, context)
    }

    pub fn guide(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Guide, context)
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("security input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("security input must be valid JSON"))
    }

    const fn new(action: SecurityAction, context: PlanningContext) -> Self {
        Self { action, context }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityGeneRole {
    Audit,
    Dependencies,
    Policy,
    Guide,
}

impl SecurityGeneRole {
    const fn action(self) -> SecurityAction {
        match self {
            Self::Audit => SecurityAction::Audit,
            Self::Dependencies => SecurityAction::Dependencies,
            Self::Policy => SecurityAction::Policy,
            Self::Guide => SecurityAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Audit => "security.audit",
            Self::Dependencies => "security.dependencies",
            Self::Policy => "security.policy",
            Self::Guide => "security.guide",
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Guide => None,
            Self::Audit | Self::Dependencies | Self::Policy => Some(Capability::FilesystemRead),
        }
    }
}

pub struct SecurityGene {
    manifest: GeneManifest,
    role: SecurityGeneRole,
}

impl SecurityGene {
    pub fn new(role: SecurityGeneRole) -> Result<Self, GeneError> {
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
            SecurityGeneRole::Audit,
            SecurityGeneRole::Dependencies,
            SecurityGeneRole::Policy,
            SecurityGeneRole::Guide,
        ]
        .into_iter()
        .map(|role| {
            Box::new(Self::new(role).expect("built-in Security Gene is valid")) as Box<dyn Gene>
        })
        .collect()
    }

    fn operation_request(
        &self,
        request: &SecurityRequest,
        marker: &str,
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
            EffectTarget::path(marker),
            ResourceScope::workspace(request.context.workspace_id().as_str()),
        )?)
    }
}

impl Gene for SecurityGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = SecurityRequest::parse(input)?;
        if request.action != self.role.action() {
            return Err(GeneError::InvalidInput(
                "security input action does not match Gene",
            ));
        }
        let markers: &[&str] = match self.role {
            SecurityGeneRole::Audit => &AUDIT_MARKERS,
            SecurityGeneRole::Dependencies => &DEPENDENCY_MARKERS,
            SecurityGeneRole::Policy => &POLICY_MARKERS,
            SecurityGeneRole::Guide => return Ok(Vec::new()),
        };
        markers
            .iter()
            .map(|marker| self.operation_request(&request, marker))
            .collect()
    }
}

pub fn is_security_gene(gene_id: &GeneId) -> bool {
    matches!(
        gene_id.as_str(),
        "security.audit" | "security.dependencies" | "security.policy" | "security.guide"
    )
}

pub fn security_static_output(gene_id: &GeneId) -> Option<&'static str> {
    (gene_id.as_str() == "security.guide").then_some(SECURITY_GUIDE)
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
    fn audit_plans_only_fixed_read_searches() {
        let gene = SecurityGene::new(SecurityGeneRole::Audit).unwrap();
        let requests = gene
            .plan(&SecurityRequest::audit(context()).into_gene_input().unwrap())
            .unwrap();

        assert_eq!(requests.len(), AUDIT_MARKERS.len());
        assert!(requests.iter().all(|request| {
            request.capability() == Capability::FilesystemRead
                && request.operation() == Operation::Read
                && request.resource_scope() == &ResourceScope::workspace("workspace-1")
        }));
        assert!(requests.iter().all(|request| {
            matches!(request.target(), EffectTarget::Path { path } if AUDIT_MARKERS.contains(&path.as_str()))
        }));
    }

    #[test]
    fn guide_is_pure_static_guidance() {
        let gene = SecurityGene::new(SecurityGeneRole::Guide).unwrap();
        let requests = gene
            .plan(&SecurityRequest::guide(context()).into_gene_input().unwrap())
            .unwrap();

        assert!(requests.is_empty());
        assert!(gene.manifest().capabilities().is_empty());
        assert!(security_static_output(gene.manifest().id()).is_some());
    }
}
