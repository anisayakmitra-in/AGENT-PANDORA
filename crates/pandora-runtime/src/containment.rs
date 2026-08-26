use pandora_types::{
    ContainmentBoundary, ContainmentBoundaryKind, ContainmentContractError, ContainmentControl,
    ContainmentEvidence, ContainmentLimitation, ContainmentSnapshot, ExecutorIdentity,
    ExecutorWorkerClass,
};

pub fn shipped_executor_containment() -> Result<ContainmentSnapshot, ContainmentContractError> {
    ContainmentSnapshot::new(
        std::env::consts::OS,
        std::env::consts::ARCH,
        vec![
            filesystem_evidence()?,
            worktree_evidence()?,
            mcp_evidence()?,
            process_evidence()?,
            provider_evidence()?,
            wasm_evidence()?,
        ],
    )
}

fn filesystem_evidence() -> Result<ContainmentEvidence, ContainmentContractError> {
    ContainmentEvidence::new(
        identity("filesystem", ExecutorWorkerClass::InProcess)?,
        vec![partial(
            ContainmentBoundaryKind::Workspace,
            vec![
                ContainmentControl::PermitRequestBinding,
                ContainmentControl::CanonicalWorkspaceRoot,
                ContainmentControl::NoFollowPathResolution,
                ContainmentControl::BoundedIo,
                ContainmentControl::AtomicFileCommit,
            ],
            ContainmentLimitation::HostFilesystemNotSandboxed,
        )?],
    )
}

fn process_evidence() -> Result<ContainmentEvidence, ContainmentContractError> {
    ContainmentEvidence::new(
        identity("process", ExecutorWorkerClass::ChildProcess)?,
        vec![
            partial(
                ContainmentBoundaryKind::Workspace,
                vec![
                    ContainmentControl::PermitRequestBinding,
                    ContainmentControl::CanonicalWorkspaceRoot,
                ],
                ContainmentLimitation::HostFilesystemNotSandboxed,
            )?,
            partial(
                ContainmentBoundaryKind::Process,
                vec![
                    ContainmentControl::PermitRequestBinding,
                    ContainmentControl::FixedExecutable,
                    ContainmentControl::ClearedEnvironment,
                    ContainmentControl::BoundedIo,
                    ContainmentControl::TimeoutAndCancellation,
                    ContainmentControl::ProcessTreeTermination,
                ],
                ContainmentLimitation::HostProcessNotSandboxed,
            )?,
            unavailable(
                ContainmentBoundaryKind::Network,
                ContainmentLimitation::NetworkNotRestricted,
            ),
        ],
    )
}

fn worktree_evidence() -> Result<ContainmentEvidence, ContainmentContractError> {
    ContainmentEvidence::new(
        identity("git_worktree", ExecutorWorkerClass::ChildProcess)?,
        vec![
            partial(
                ContainmentBoundaryKind::Workspace,
                vec![
                    ContainmentControl::PermitRequestBinding,
                    ContainmentControl::CanonicalWorkspaceRoot,
                    ContainmentControl::ManagedWorktreeRoot,
                    ContainmentControl::ExactCommitVerification,
                ],
                ContainmentLimitation::RepositoryMetadataOutsideManagedRoot,
            )?,
            partial(
                ContainmentBoundaryKind::Process,
                vec![
                    ContainmentControl::PermitRequestBinding,
                    ContainmentControl::FixedExecutable,
                ],
                ContainmentLimitation::HostProcessNotSandboxed,
            )?,
            unavailable(
                ContainmentBoundaryKind::Network,
                ContainmentLimitation::NetworkNotRestricted,
            ),
        ],
    )
}

fn mcp_evidence() -> Result<ContainmentEvidence, ContainmentContractError> {
    ContainmentEvidence::new(
        identity("mcp_stdio", ExecutorWorkerClass::ChildProcess)?,
        vec![
            unavailable(
                ContainmentBoundaryKind::Workspace,
                ContainmentLimitation::HostFilesystemNotSandboxed,
            ),
            partial(
                ContainmentBoundaryKind::Process,
                vec![
                    ContainmentControl::PermitRequestBinding,
                    ContainmentControl::FixedExecutable,
                    ContainmentControl::ClearedEnvironment,
                    ContainmentControl::BoundedIo,
                    ContainmentControl::TimeoutAndTermination,
                    ContainmentControl::ProcessTreeTermination,
                    ContainmentControl::DirectStdio,
                    ContainmentControl::ProtocolFrameLimits,
                    ContainmentControl::FreshFallbackProcess,
                ],
                ContainmentLimitation::NativeServerNotSandboxed,
            )?,
            unavailable(
                ContainmentBoundaryKind::Network,
                ContainmentLimitation::NetworkNotRestricted,
            ),
        ],
    )
}

fn provider_evidence() -> Result<ContainmentEvidence, ContainmentContractError> {
    ContainmentEvidence::new(
        identity("provider", ExecutorWorkerClass::RemoteService)?,
        vec![partial(
            ContainmentBoundaryKind::Network,
            vec![
                ContainmentControl::PermitRequestBinding,
                ContainmentControl::PayloadDigestBinding,
                ContainmentControl::CredentialReference,
            ],
            ContainmentLimitation::RemoteServiceOutsideLocalBoundary,
        )?],
    )
}

fn wasm_evidence() -> Result<ContainmentEvidence, ContainmentContractError> {
    ContainmentEvidence::new(
        identity("wasm", ExecutorWorkerClass::InProcess)?,
        vec![partial(
            ContainmentBoundaryKind::Process,
            vec![
                ContainmentControl::PermitRequestBinding,
                ContainmentControl::PayloadDigestBinding,
                ContainmentControl::BoundedIo,
            ],
            ContainmentLimitation::HostProcessNotSandboxed,
        )?],
    )
}

fn identity(
    id: &str,
    worker_class: ExecutorWorkerClass,
) -> Result<ExecutorIdentity, ContainmentContractError> {
    ExecutorIdentity::new(id, env!("CARGO_PKG_VERSION"), worker_class)
}

fn partial(
    kind: ContainmentBoundaryKind,
    controls: Vec<ContainmentControl>,
    limitation: ContainmentLimitation,
) -> Result<ContainmentBoundary, ContainmentContractError> {
    ContainmentBoundary::partial(kind, controls, limitation)
}

fn unavailable(
    kind: ContainmentBoundaryKind,
    limitation: ContainmentLimitation,
) -> ContainmentBoundary {
    ContainmentBoundary::unavailable(kind, limitation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        ContainmentBoundaryKind, ContainmentLevel, ContainmentLimitation, ExecutorWorkerClass,
    };

    #[test]
    fn shipped_snapshot_names_every_effect_executor_without_claiming_a_sandbox() {
        let snapshot = shipped_executor_containment().unwrap();
        let ids = snapshot
            .executors()
            .iter()
            .map(|evidence| evidence.identity().id())
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            [
                "filesystem",
                "git_worktree",
                "mcp_stdio",
                "process",
                "provider",
                "wasm"
            ]
        );
        assert!(snapshot.digest().starts_with("sha256:"));
        assert!(snapshot.executors().iter().all(|evidence| {
            evidence
                .boundaries()
                .iter()
                .all(|boundary| boundary.level() != ContainmentLevel::Enforced)
        }));
    }

    #[test]
    fn process_and_mcp_report_unrestricted_network_and_host_process_limits() {
        let snapshot = shipped_executor_containment().unwrap();
        let process = executor(&snapshot, "process");
        let mcp = executor(&snapshot, "mcp_stdio");

        assert_eq!(
            process.identity().worker_class(),
            ExecutorWorkerClass::ChildProcess
        );
        assert_boundary(
            process,
            ContainmentBoundaryKind::Process,
            ContainmentLevel::Partial,
            ContainmentLimitation::HostProcessNotSandboxed,
        );
        assert_boundary(
            process,
            ContainmentBoundaryKind::Network,
            ContainmentLevel::Unavailable,
            ContainmentLimitation::NetworkNotRestricted,
        );
        assert!(
            process
                .boundaries()
                .iter()
                .find(|boundary| boundary.kind() == ContainmentBoundaryKind::Process)
                .unwrap()
                .controls()
                .contains(&ContainmentControl::ProcessTreeTermination)
        );
        assert_boundary(
            mcp,
            ContainmentBoundaryKind::Workspace,
            ContainmentLevel::Unavailable,
            ContainmentLimitation::HostFilesystemNotSandboxed,
        );
        assert_boundary(
            mcp,
            ContainmentBoundaryKind::Process,
            ContainmentLevel::Partial,
            ContainmentLimitation::NativeServerNotSandboxed,
        );
        assert!(
            mcp.boundaries()
                .iter()
                .find(|boundary| boundary.kind() == ContainmentBoundaryKind::Process)
                .unwrap()
                .controls()
                .contains(&ContainmentControl::ProcessTreeTermination)
        );
        assert_boundary(
            mcp,
            ContainmentBoundaryKind::Network,
            ContainmentLevel::Unavailable,
            ContainmentLimitation::NetworkNotRestricted,
        );
    }

    #[test]
    fn provider_reports_remote_boundary_instead_of_local_network_isolation() {
        let snapshot = shipped_executor_containment().unwrap();
        let provider = executor(&snapshot, "provider");

        assert_eq!(
            provider.identity().worker_class(),
            ExecutorWorkerClass::RemoteService
        );
        assert_boundary(
            provider,
            ContainmentBoundaryKind::Network,
            ContainmentLevel::Partial,
            ContainmentLimitation::RemoteServiceOutsideLocalBoundary,
        );
    }

    fn executor<'a>(
        snapshot: &'a pandora_types::ContainmentSnapshot,
        id: &str,
    ) -> &'a pandora_types::ContainmentEvidence {
        snapshot
            .executors()
            .iter()
            .find(|evidence| evidence.identity().id() == id)
            .unwrap()
    }

    fn assert_boundary(
        evidence: &pandora_types::ContainmentEvidence,
        kind: ContainmentBoundaryKind,
        level: ContainmentLevel,
        limitation: ContainmentLimitation,
    ) {
        let boundary = evidence
            .boundaries()
            .iter()
            .find(|boundary| boundary.kind() == kind)
            .unwrap();
        assert_eq!(boundary.level(), level);
        assert_eq!(boundary.limitation(), Some(limitation));
    }
}
