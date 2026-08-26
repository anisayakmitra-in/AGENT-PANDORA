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
const SCAN_MARKERS: [&str; 8] = [
    "SECURITY.md",
    "unsafe",
    "Command::new",
    "reqwest::",
    "serde_json::from_str",
    "authentication",
    "authorization",
    "credential",
];
const DEPENDENCY_MARKERS: [&str; 4] = [
    "[dependencies]",
    "[dev-dependencies]",
    "dependencies:",
    "\"dependencies\"",
];
const THREAT_MODEL_MARKERS: [&str; 8] = [
    "trust boundary",
    "trust_boundary",
    "attacker",
    "threat model",
    "sandbox",
    "isolation",
    "capability",
    "least privilege",
];
const TRIAGE_MARKERS: [&str; 8] = [
    "finding",
    "vulnerability",
    "CVE-",
    "GHSA-",
    "severity",
    "exploit",
    "reachability",
    "proof gap",
];
const VALIDATION_MARKERS: [&str; 8] = [
    "cargo test",
    "cargo audit",
    "security scan",
    "regression",
    "assert!",
    "deny",
    "validation",
    "holdout",
];
const HARDENING_MARKERS: [&str; 8] = [
    "allowlist",
    "denylist",
    "rate limit",
    "redact",
    "fail closed",
    "permission",
    "approval",
    "rollback",
];
const POLICY_MARKERS: [&str; 4] = ["SECURITY", "permission", "approval", "credential"];
const SECURITY_GUIDE: &str = "Security Scan inventories fixed high-signal security markers without claiming complete scanner coverage.\nSecurity Audit searches boundary-sensitive source markers and returns evidence paths.\nSecurity Dependencies searches dependency declarations without claiming advisory or vulnerability coverage.\nSecurity Threat Model searches local trust-boundary and isolation evidence.\nSecurity Triage searches existing finding and proof terminology without assigning a verdict.\nSecurity Validation searches tests and validation evidence without running a scanner.\nSecurity Hardening searches local defensive-control evidence and does not change code.\nSecurity Policy searches local authorization terminology without certifying compliance.\nAll filesystem effects require Pandora permits and receipts; process, network, package, and remediation actions require separately governed capabilities.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAction {
    Audit,
    Scan,
    Dependencies,
    ThreatModel,
    Triage,
    Validation,
    Hardening,
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

    pub fn scan(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Scan, context)
    }

    pub fn dependencies(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Dependencies, context)
    }

    pub fn threat_model(context: PlanningContext) -> Self {
        Self::new(SecurityAction::ThreatModel, context)
    }

    pub fn triage(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Triage, context)
    }

    pub fn validation(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Validation, context)
    }

    pub fn hardening(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Hardening, context)
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
    Scan,
    Dependencies,
    ThreatModel,
    Triage,
    Validation,
    Hardening,
    Policy,
    Guide,
}

impl SecurityGeneRole {
    const fn action(self) -> SecurityAction {
        match self {
            Self::Audit => SecurityAction::Audit,
            Self::Scan => SecurityAction::Scan,
            Self::Dependencies => SecurityAction::Dependencies,
            Self::ThreatModel => SecurityAction::ThreatModel,
            Self::Triage => SecurityAction::Triage,
            Self::Validation => SecurityAction::Validation,
            Self::Hardening => SecurityAction::Hardening,
            Self::Policy => SecurityAction::Policy,
            Self::Guide => SecurityAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Audit => "security.audit",
            Self::Scan => "security.scan",
            Self::Dependencies => "security.dependencies",
            Self::ThreatModel => "security.threat-model",
            Self::Triage => "security.triage",
            Self::Validation => "security.validation",
            Self::Hardening => "security.hardening",
            Self::Policy => "security.policy",
            Self::Guide => "security.guide",
        }
    }

    const fn capability(self) -> Option<Capability> {
        match self {
            Self::Guide => None,
            _ => Some(Capability::FilesystemRead),
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
            SecurityGeneRole::Scan,
            SecurityGeneRole::Dependencies,
            SecurityGeneRole::ThreatModel,
            SecurityGeneRole::Triage,
            SecurityGeneRole::Validation,
            SecurityGeneRole::Hardening,
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
            SecurityGeneRole::Scan => &SCAN_MARKERS,
            SecurityGeneRole::Dependencies => &DEPENDENCY_MARKERS,
            SecurityGeneRole::ThreatModel => &THREAT_MODEL_MARKERS,
            SecurityGeneRole::Triage => &TRIAGE_MARKERS,
            SecurityGeneRole::Validation => &VALIDATION_MARKERS,
            SecurityGeneRole::Hardening => &HARDENING_MARKERS,
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
        "security.audit"
            | "security.scan"
            | "security.dependencies"
            | "security.threat-model"
            | "security.triage"
            | "security.validation"
            | "security.hardening"
            | "security.policy"
            | "security.guide"
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

    #[test]
    fn assessment_roles_are_read_only_and_have_distinct_ids() {
        let roles = [
            (SecurityGeneRole::Scan, &SCAN_MARKERS[..]),
            (SecurityGeneRole::ThreatModel, &THREAT_MODEL_MARKERS[..]),
            (SecurityGeneRole::Triage, &TRIAGE_MARKERS[..]),
            (SecurityGeneRole::Validation, &VALIDATION_MARKERS[..]),
            (SecurityGeneRole::Hardening, &HARDENING_MARKERS[..]),
        ];

        for (role, markers) in roles {
            let gene = SecurityGene::new(role).unwrap();
            let requests = gene
                .plan(
                    &SecurityRequest::new(role.action(), context())
                        .into_gene_input()
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(requests.len(), markers.len());
            assert!(requests.iter().all(|request| {
                request.capability() == Capability::FilesystemRead
                    && request.operation() == Operation::Read
            }));
            assert!(is_security_gene(gene.manifest().id()));
        }
    }
}
