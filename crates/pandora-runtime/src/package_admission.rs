use pandora_types::{
    Capability, GeneExecutionMode, Operation, OperationRequest, PackageKind, PackageManifest,
    SkillManifest,
};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageAdmissionBoundary {
    HarnessRegistry,
    ConstitutionalSource,
    DataOnly,
    ProviderConfiguration,
    SkillEngine,
}

impl PackageAdmissionBoundary {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HarnessRegistry => "harness_registry",
            Self::ConstitutionalSource => "constitutional_source",
            Self::DataOnly => "data_only",
            Self::ProviderConfiguration => "provider_configuration",
            Self::SkillEngine => "skill_engine",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageAdmissionRule {
    kind: PackageKind,
    boundary: PackageAdmissionBoundary,
    executable_artifact: bool,
}

impl PackageAdmissionRule {
    pub const fn kind(self) -> PackageKind {
        self.kind
    }

    pub const fn boundary(self) -> PackageAdmissionBoundary {
        self.boundary
    }

    pub const fn executable_artifact(self) -> bool {
        self.executable_artifact
    }

    pub const fn grants_runtime_authority(self) -> bool {
        false
    }

    pub const fn allows_harness_registry(self) -> bool {
        matches!(self.boundary, PackageAdmissionBoundary::HarnessRegistry)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PackageAdmissionError {
    InvalidResource,
    MissingGeneContract,
    GeneIdentityMismatch,
    UndeclaredGeneCapability,
    InvalidGeneOperation,
    WrongBoundary {
        kind: PackageKind,
        required: PackageAdmissionBoundary,
    },
}

impl fmt::Display for PackageAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidResource => formatter.write_str("skill resource is invalid"),
            Self::MissingGeneContract => {
                formatter.write_str("Gene package does not declare an effect contract")
            }
            Self::GeneIdentityMismatch => {
                formatter.write_str("Gene request identity does not match its package")
            }
            Self::UndeclaredGeneCapability => {
                formatter.write_str("Gene request uses an undeclared capability")
            }
            Self::InvalidGeneOperation => {
                formatter.write_str("Gene request operation does not match its declared capability")
            }
            Self::WrongBoundary { kind, required } => write!(
                formatter,
                "{} packages require the {} admission boundary",
                kind.as_str(),
                required.as_str()
            ),
        }
    }
}

impl std::error::Error for PackageAdmissionError {}

pub struct PackageAdmission;

impl PackageAdmission {
    pub const fn rule_for(kind: PackageKind) -> PackageAdmissionRule {
        let (boundary, executable_artifact) = match kind {
            PackageKind::Gene => (PackageAdmissionBoundary::HarnessRegistry, true),
            PackageKind::DomainHarness | PackageKind::MetaHarness => {
                (PackageAdmissionBoundary::HarnessRegistry, false)
            }
            PackageKind::SourceHarness => (PackageAdmissionBoundary::ConstitutionalSource, false),
            PackageKind::Package => (PackageAdmissionBoundary::DataOnly, false),
            PackageKind::Provider => (PackageAdmissionBoundary::ProviderConfiguration, false),
            PackageKind::Skill => (PackageAdmissionBoundary::SkillEngine, false),
        };
        PackageAdmissionRule {
            kind,
            boundary,
            executable_artifact,
        }
    }

    pub fn validate_harness_registry_kind(
        kind: PackageKind,
    ) -> Result<PackageAdmissionRule, PackageAdmissionError> {
        let rule = Self::rule_for(kind);
        if !rule.allows_harness_registry() {
            return Err(PackageAdmissionError::WrongBoundary {
                kind,
                required: rule.boundary(),
            });
        }
        Ok(rule)
    }

    pub fn validate_skill(manifest: &SkillManifest) -> Result<(), PackageAdmissionError> {
        if manifest.resources().iter().any(|resource| {
            resource.is_empty()
                || resource.chars().any(char::is_control)
                || resource == "."
                || resource == ".."
                || resource.contains(['/', '\\'])
        }) {
            return Err(PackageAdmissionError::InvalidResource);
        }
        Ok(())
    }

    pub fn validate_gene_request(
        manifest: &PackageManifest,
        request: &OperationRequest,
    ) -> Result<(), PackageAdmissionError> {
        if manifest.kind() != PackageKind::Gene {
            return Err(PackageAdmissionError::WrongBoundary {
                kind: manifest.kind(),
                required: PackageAdmissionBoundary::HarnessRegistry,
            });
        }
        let contract = manifest
            .gene_contract()
            .ok_or(PackageAdmissionError::MissingGeneContract)?;
        if request.gene_id().as_str() != manifest.id().as_str() {
            return Err(PackageAdmissionError::GeneIdentityMismatch);
        }
        if !contract.declares(request.capability()) {
            return Err(PackageAdmissionError::UndeclaredGeneCapability);
        }
        let operation_matches = match request.capability() {
            Capability::FilesystemRead => request.operation() == Operation::Read,
            Capability::FilesystemWrite => request.operation() == Operation::Write,
            Capability::ProcessExecute | Capability::WasmExecute => {
                request.operation() == Operation::Execute
            }
            Capability::NetworkConnect => request.operation() == Operation::Connect,
            Capability::ProviderInvoke | Capability::McpInvoke => {
                request.operation() == Operation::Invoke
            }
            Capability::PackageInstall => request.operation() == Operation::Install,
        };
        let mode_matches = match contract.execution() {
            GeneExecutionMode::StaticGuidance => false,
            GeneExecutionMode::BoundedRead => {
                request.capability() == Capability::FilesystemRead
                    && request.operation() == Operation::Read
            }
            GeneExecutionMode::EffectRequest => true,
        };
        if !operation_matches || !mode_matches {
            return Err(PackageAdmissionError::InvalidGeneOperation);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::{FilesystemExecutor, WorkspaceRoot};
    use crate::{ApprovalRequest, ApprovalStore, AuthorizationError, Parliament, ReferenceMonitor};
    use pandora_types::{
        EffectOutcome, EffectTarget, ExecutionId, GeneId, GenePackageContract,
        PackageCompatibility, ParliamentDecision, PolicyContext, PrincipalId, ResourceScope,
        SessionId, Timestamp, TrustEvidence, hash_artifact,
    };
    use std::fs;

    fn gene_manifest(
        id: &str,
        execution: GeneExecutionMode,
        capabilities: Vec<Capability>,
        approval_required: bool,
    ) -> PackageManifest {
        PackageManifest::new(
            id,
            "1.0.0",
            PackageKind::Gene,
            "example",
            hash_artifact(b"gene"),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "MIT",
            TrustEvidence::unsigned(),
        )
        .unwrap()
        .with_gene_contract(
            GenePackageContract::new(execution, capabilities, approval_required).unwrap(),
        )
        .unwrap()
    }

    fn request(id: &str, capability: Capability, operation: Operation) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
            GeneId::new(id).unwrap(),
            None,
            capability,
            operation,
            EffectTarget::path("README.md"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap()
    }

    #[test]
    fn resource_labels_cannot_be_paths() {
        let manifest = SkillManifest::new(
            "alpha",
            "0.1.0",
            "Alpha",
            "A skill",
            None,
            vec!["../secrets".to_owned()],
        )
        .unwrap();

        assert_eq!(
            PackageAdmission::validate_skill(&manifest),
            Err(PackageAdmissionError::InvalidResource)
        );
    }

    #[test]
    fn package_kinds_have_distinct_fail_closed_boundaries() {
        for (kind, boundary) in [
            (
                PackageKind::SourceHarness,
                PackageAdmissionBoundary::ConstitutionalSource,
            ),
            (PackageKind::Package, PackageAdmissionBoundary::DataOnly),
            (
                PackageKind::Provider,
                PackageAdmissionBoundary::ProviderConfiguration,
            ),
            (PackageKind::Skill, PackageAdmissionBoundary::SkillEngine),
        ] {
            let rule = PackageAdmission::rule_for(kind);
            assert_eq!(rule.kind(), kind);
            assert_eq!(rule.boundary(), boundary);
            assert!(!rule.executable_artifact());
            assert!(!rule.grants_runtime_authority());
            assert_eq!(
                PackageAdmission::validate_harness_registry_kind(kind),
                Err(PackageAdmissionError::WrongBoundary {
                    kind,
                    required: boundary,
                })
            );
        }
    }

    #[test]
    fn harness_registry_rules_preserve_wasm_and_profile_boundaries() {
        let gene = PackageAdmission::validate_harness_registry_kind(PackageKind::Gene).unwrap();
        assert!(gene.executable_artifact());
        assert!(!gene.grants_runtime_authority());

        for kind in [PackageKind::DomainHarness, PackageKind::MetaHarness] {
            let profile = PackageAdmission::validate_harness_registry_kind(kind).unwrap();
            assert!(!profile.executable_artifact());
            assert!(!profile.grants_runtime_authority());
        }
    }

    #[test]
    fn gene_requests_must_match_signed_identity_capability_and_operation() {
        let manifest = gene_manifest(
            "example/patch-proposal",
            GeneExecutionMode::EffectRequest,
            vec![Capability::FilesystemWrite],
            true,
        );

        assert!(
            PackageAdmission::validate_gene_request(
                &manifest,
                &request(
                    "example/patch-proposal",
                    Capability::FilesystemWrite,
                    Operation::Write,
                ),
            )
            .is_ok()
        );
        assert_eq!(
            PackageAdmission::validate_gene_request(
                &manifest,
                &request(
                    "example/patch-proposal",
                    Capability::ProcessExecute,
                    Operation::Execute,
                ),
            ),
            Err(PackageAdmissionError::UndeclaredGeneCapability)
        );
        assert_eq!(
            PackageAdmission::validate_gene_request(
                &manifest,
                &request(
                    "example/patch-proposal",
                    Capability::FilesystemWrite,
                    Operation::Read,
                ),
            ),
            Err(PackageAdmissionError::InvalidGeneOperation)
        );
        assert_eq!(
            PackageAdmission::validate_gene_request(
                &manifest,
                &request(
                    "example/other-gene",
                    Capability::FilesystemWrite,
                    Operation::Write,
                ),
            ),
            Err(PackageAdmissionError::GeneIdentityMismatch)
        );
    }

    #[test]
    fn static_guidance_cannot_propose_an_effect() {
        let manifest = gene_manifest(
            "example/static-guide",
            GeneExecutionMode::StaticGuidance,
            Vec::new(),
            false,
        );

        assert_eq!(
            PackageAdmission::validate_gene_request(
                &manifest,
                &request(
                    "example/static-guide",
                    Capability::FilesystemRead,
                    Operation::Read,
                ),
            ),
            Err(PackageAdmissionError::UndeclaredGeneCapability)
        );
    }

    #[test]
    fn effect_request_requires_policy_approval_permit_and_receipt() {
        let directory = crate::test_support::new_temp_dir("pandora-gene-contract-effect").unwrap();
        let workspace = directory.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let root = WorkspaceRoot::new(&workspace).unwrap();
        let target = root.path("gene-pack-output.txt").unwrap();
        let content = b"approved through the governed path";
        let manifest = gene_manifest(
            "example/patch-proposal",
            GeneExecutionMode::EffectRequest,
            vec![Capability::FilesystemWrite],
            true,
        );
        let request = OperationRequest::new(
            ExecutionId::new("execution-effect-1").unwrap(),
            SessionId::new("session-effect-1").unwrap(),
            PrincipalId::new("principal-effect-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
            GeneId::new("example/patch-proposal").unwrap(),
            None,
            Capability::FilesystemWrite,
            Operation::Write,
            EffectTarget::path("gene-pack-output.txt"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap()
        .with_payload_digest(content)
        .unwrap();
        PackageAdmission::validate_gene_request(&manifest, &request).unwrap();

        let denied_policy = PolicyContext::new(1, [], [Operation::Write]);
        let denied_decision = Parliament::new(1).decide(&request, &denied_policy);
        assert!(matches!(denied_decision, ParliamentDecision::Deny { .. }));
        let denied_monitor = ReferenceMonitor::new_with_policy(denied_policy, 60);
        assert!(matches!(
            denied_monitor.authorize(
                request.clone(),
                denied_decision,
                Timestamp::from_unix_seconds(10),
            ),
            Err(AuthorizationError::Denied { .. })
        ));
        assert!(!target.absolute().exists());

        let governed_policy =
            PolicyContext::new(1, [Capability::FilesystemWrite], [Operation::Write]);
        let decision = Parliament::new(1).decide(&request, &governed_policy);
        assert!(matches!(
            decision,
            ParliamentDecision::RequireApproval { .. }
        ));
        let monitor = ReferenceMonitor::new_with_policy(governed_policy, 60);
        assert!(matches!(
            monitor.authorize(
                request.clone(),
                decision.clone(),
                Timestamp::from_unix_seconds(10),
            ),
            Err(AuthorizationError::ApprovalRequired { .. })
        ));
        assert!(!target.absolute().exists());

        let approvals = ApprovalStore::open(directory.join("approvals.sqlite3")).unwrap();
        approvals
            .create(
                ApprovalRequest::new(
                    "approval-effect-1",
                    request.session_id().clone(),
                    request.execution_id().clone(),
                    request.principal_id().clone(),
                    request.gene_id().clone(),
                    request.request_digest().clone(),
                    "approve the exact Gene pack write",
                    1,
                    Timestamp::from_unix_seconds(100),
                )
                .unwrap(),
            )
            .unwrap();
        approvals
            .resolve(
                "approval-effect-1",
                request.principal_id(),
                &PrincipalId::new("approver-1").unwrap(),
                true,
                Timestamp::from_unix_seconds(11),
            )
            .unwrap();
        let grant = approvals
            .consume_grant(
                "approval-effect-1",
                request.principal_id(),
                request.session_id(),
                request.execution_id(),
                request.gene_id(),
                request.request_digest(),
                Timestamp::from_unix_seconds(12),
            )
            .unwrap();
        let permit = monitor
            .authorize_after_approval_with_grant(
                request.clone(),
                decision,
                &grant,
                Timestamp::from_unix_seconds(12),
            )
            .unwrap();
        let consumed = monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(13))
            .unwrap();
        let result = FilesystemExecutor::for_workspace(root).write_patch(
            &consumed,
            &target,
            content,
            Timestamp::from_unix_seconds(13),
        );

        assert!(result.result().is_ok());
        assert_eq!(result.receipt().outcome(), &EffectOutcome::Succeeded);
        assert_eq!(fs::read(target.absolute()).unwrap(), content);
        drop(approvals);
        fs::remove_dir_all(directory).unwrap();
    }
}
