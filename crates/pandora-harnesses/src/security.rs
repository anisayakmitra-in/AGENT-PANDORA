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
const DEEP_SCAN_MARKERS: [&str; 12] = [
    "SECURITY.md",
    "authentication",
    "authorization",
    "credential",
    "unsafe",
    "serde_json",
    "deserialize",
    "Command::new",
    "reqwest::",
    "trust boundary",
    "sandbox",
    "rate limit",
];
const DIFF_SCAN_MARKERS: [&str; 10] = [
    "git diff",
    "changed",
    "regression",
    "security",
    "authorization",
    "validation",
    "permission",
    "source",
    "sink",
    "test",
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
const ASSESSMENT_MARKERS: [&str; 24] = [
    "SECURITY.md",
    "Cargo.toml",
    "package.json",
    "unsafe",
    "Command::new",
    "std::process::Command",
    "reqwest::",
    "serde_json::from_str",
    "authentication",
    "authorization",
    "credential",
    "secret",
    "trust boundary",
    "sandbox",
    "isolation",
    "least privilege",
    "permission",
    "approval",
    "allowlist",
    "rate limit",
    "redact",
    "fail closed",
    "rollback",
    "CVE-",
];
const DISCOVERY_MARKERS: [&str; 8] = [
    "candidate",
    "source",
    "sink",
    "broken control",
    "untrusted",
    "authorization",
    "validation",
    "security test",
];
const ATTACK_PATH_MARKERS: [&str; 8] = [
    "source",
    "control",
    "sink",
    "reachability",
    "trust boundary",
    "privilege",
    "impact",
    "attack path",
];
const FIX_MARKERS: [&str; 6] = [
    "fix",
    "patch",
    "remediation",
    "mitigation",
    "regression test",
    "rollback",
];
const VERIFY_FIX_MARKERS: [&str; 7] = [
    "fixed",
    "regression",
    "negative control",
    "reproduction",
    "verification",
    "before/after",
    "holdout",
];
const WRITEUP_MARKERS: [&str; 7] = [
    "impact",
    "reproduction",
    "affected versions",
    "fix",
    "references",
    "PoC",
    "proof",
];
const TRACK_MARKERS: [&str; 8] = [
    "status",
    "owner",
    "severity",
    "finding",
    "accepted",
    "deferred",
    "closed",
    "fingerprint",
];
const SECURITY_GUIDE: &str = "Security Assessment performs one bounded fixed-marker evidence pass without claiming complete scanner coverage.\nSecurity Scan inventories fixed high-signal security markers without claiming complete scanner coverage.\nSecurity Deep Scan searches a broader fixed marker set without claiming complete scanner coverage.\nSecurity Diff Scan searches changed-code and regression terminology without reviewing a specific revision.\nSecurity Audit searches boundary-sensitive source markers and returns evidence paths.\nSecurity Dependencies searches dependency declarations without claiming advisory or vulnerability coverage.\nSecurity Threat Model searches local trust-boundary and isolation evidence.\nSecurity Discovery records candidate source, control, sink, and reachability terminology without asserting a finding.\nSecurity Triage searches existing finding and proof terminology without assigning a verdict.\nSecurity Attack Path searches source, control, sink, impact, and privilege evidence without proving exploitability.\nSecurity Validation searches tests and validation evidence without running a scanner.\nSecurity Fix searches remediation planning terminology without changing code.\nSecurity Verify Fix searches regression and negative-control evidence without certifying a fix.\nSecurity Writeup searches disclosure fields without generating a vulnerability report.\nSecurity Track searches finding lifecycle fields without creating or mutating a finding record.\nSecurity Hardening searches local defensive-control evidence and does not change code.\nSecurity Policy searches local authorization terminology without certifying compliance.\nAll filesystem effects require Pandora permits and receipts; process, network, package, and remediation actions require separately governed capabilities.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAction {
    Assessment,
    Audit,
    Scan,
    DeepScan,
    DiffScan,
    Dependencies,
    ThreatModel,
    Discovery,
    Triage,
    AttackPath,
    Validation,
    Fix,
    VerifyFix,
    Writeup,
    Track,
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
    pub fn assessment(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Assessment, context)
    }

    pub fn audit(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Audit, context)
    }

    pub fn scan(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Scan, context)
    }

    pub fn deep_scan(context: PlanningContext) -> Self {
        Self::new(SecurityAction::DeepScan, context)
    }

    pub fn diff_scan(context: PlanningContext) -> Self {
        Self::new(SecurityAction::DiffScan, context)
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

    pub fn discovery(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Discovery, context)
    }

    pub fn attack_path(context: PlanningContext) -> Self {
        Self::new(SecurityAction::AttackPath, context)
    }

    pub fn validation(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Validation, context)
    }

    pub fn fix(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Fix, context)
    }

    pub fn verify_fix(context: PlanningContext) -> Self {
        Self::new(SecurityAction::VerifyFix, context)
    }

    pub fn writeup(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Writeup, context)
    }

    pub fn track(context: PlanningContext) -> Self {
        Self::new(SecurityAction::Track, context)
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
    Assessment,
    Audit,
    Scan,
    DeepScan,
    DiffScan,
    Dependencies,
    ThreatModel,
    Discovery,
    Triage,
    AttackPath,
    Validation,
    Fix,
    VerifyFix,
    Writeup,
    Track,
    Hardening,
    Policy,
    Guide,
}

impl SecurityGeneRole {
    const fn action(self) -> SecurityAction {
        match self {
            Self::Assessment => SecurityAction::Assessment,
            Self::Audit => SecurityAction::Audit,
            Self::Scan => SecurityAction::Scan,
            Self::DeepScan => SecurityAction::DeepScan,
            Self::DiffScan => SecurityAction::DiffScan,
            Self::Dependencies => SecurityAction::Dependencies,
            Self::ThreatModel => SecurityAction::ThreatModel,
            Self::Discovery => SecurityAction::Discovery,
            Self::Triage => SecurityAction::Triage,
            Self::AttackPath => SecurityAction::AttackPath,
            Self::Validation => SecurityAction::Validation,
            Self::Fix => SecurityAction::Fix,
            Self::VerifyFix => SecurityAction::VerifyFix,
            Self::Writeup => SecurityAction::Writeup,
            Self::Track => SecurityAction::Track,
            Self::Hardening => SecurityAction::Hardening,
            Self::Policy => SecurityAction::Policy,
            Self::Guide => SecurityAction::Guide,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Assessment => "security.assess",
            Self::Audit => "security.audit",
            Self::Scan => "security.scan",
            Self::DeepScan => "security.deep-scan",
            Self::DiffScan => "security.diff-scan",
            Self::Dependencies => "security.dependencies",
            Self::ThreatModel => "security.threat-model",
            Self::Discovery => "security.discovery",
            Self::Triage => "security.triage",
            Self::AttackPath => "security.attack-path",
            Self::Validation => "security.validation",
            Self::Fix => "security.fix",
            Self::VerifyFix => "security.verify-fix",
            Self::Writeup => "security.writeup",
            Self::Track => "security.track",
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
            SecurityGeneRole::Assessment,
            SecurityGeneRole::Audit,
            SecurityGeneRole::Scan,
            SecurityGeneRole::DeepScan,
            SecurityGeneRole::DiffScan,
            SecurityGeneRole::Dependencies,
            SecurityGeneRole::ThreatModel,
            SecurityGeneRole::Discovery,
            SecurityGeneRole::Triage,
            SecurityGeneRole::AttackPath,
            SecurityGeneRole::Validation,
            SecurityGeneRole::Fix,
            SecurityGeneRole::VerifyFix,
            SecurityGeneRole::Writeup,
            SecurityGeneRole::Track,
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
            SecurityGeneRole::Assessment => &ASSESSMENT_MARKERS,
            SecurityGeneRole::Audit => &AUDIT_MARKERS,
            SecurityGeneRole::Scan => &SCAN_MARKERS,
            SecurityGeneRole::DeepScan => &DEEP_SCAN_MARKERS,
            SecurityGeneRole::DiffScan => &DIFF_SCAN_MARKERS,
            SecurityGeneRole::Dependencies => &DEPENDENCY_MARKERS,
            SecurityGeneRole::ThreatModel => &THREAT_MODEL_MARKERS,
            SecurityGeneRole::Discovery => &DISCOVERY_MARKERS,
            SecurityGeneRole::Triage => &TRIAGE_MARKERS,
            SecurityGeneRole::AttackPath => &ATTACK_PATH_MARKERS,
            SecurityGeneRole::Validation => &VALIDATION_MARKERS,
            SecurityGeneRole::Fix => &FIX_MARKERS,
            SecurityGeneRole::VerifyFix => &VERIFY_FIX_MARKERS,
            SecurityGeneRole::Writeup => &WRITEUP_MARKERS,
            SecurityGeneRole::Track => &TRACK_MARKERS,
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
        "security.assess"
            | "security.audit"
            | "security.scan"
            | "security.deep-scan"
            | "security.diff-scan"
            | "security.dependencies"
            | "security.threat-model"
            | "security.discovery"
            | "security.triage"
            | "security.attack-path"
            | "security.validation"
            | "security.fix"
            | "security.verify-fix"
            | "security.writeup"
            | "security.track"
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
            (SecurityGeneRole::DeepScan, &DEEP_SCAN_MARKERS[..]),
            (SecurityGeneRole::DiffScan, &DIFF_SCAN_MARKERS[..]),
            (SecurityGeneRole::ThreatModel, &THREAT_MODEL_MARKERS[..]),
            (SecurityGeneRole::Discovery, &DISCOVERY_MARKERS[..]),
            (SecurityGeneRole::Triage, &TRIAGE_MARKERS[..]),
            (SecurityGeneRole::AttackPath, &ATTACK_PATH_MARKERS[..]),
            (SecurityGeneRole::Validation, &VALIDATION_MARKERS[..]),
            (SecurityGeneRole::Fix, &FIX_MARKERS[..]),
            (SecurityGeneRole::VerifyFix, &VERIFY_FIX_MARKERS[..]),
            (SecurityGeneRole::Writeup, &WRITEUP_MARKERS[..]),
            (SecurityGeneRole::Track, &TRACK_MARKERS[..]),
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
