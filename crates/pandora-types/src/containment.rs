use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;

pub const CONTAINMENT_EVIDENCE_VERSION: u16 = 1;
const MAX_IDENTITY_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContainmentContractError {
    InvalidIdentity(&'static str),
    EmptyControls(&'static str),
    DuplicateControl(&'static str),
    EmptyBoundaries,
    DuplicateBoundary(&'static str),
    InvalidPlatform(&'static str),
    EmptyExecutors,
    DuplicateExecutor(String),
}

impl fmt::Display for ContainmentContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity(field) => write!(formatter, "invalid {field}"),
            Self::EmptyControls(boundary) => {
                write!(formatter, "{boundary} containment needs a control")
            }
            Self::DuplicateControl(control) => {
                write!(formatter, "duplicate containment control {control}")
            }
            Self::EmptyBoundaries => write!(formatter, "containment evidence needs a boundary"),
            Self::DuplicateBoundary(boundary) => {
                write!(formatter, "duplicate {boundary} containment boundary")
            }
            Self::InvalidPlatform(field) => write!(formatter, "invalid platform {field}"),
            Self::EmptyExecutors => write!(formatter, "containment snapshot needs an executor"),
            Self::DuplicateExecutor(executor) => {
                write!(formatter, "duplicate containment executor {executor}")
            }
        }
    }
}

impl std::error::Error for ContainmentContractError {}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorWorkerClass {
    InProcess,
    ChildProcess,
    RemoteService,
}

impl ExecutorWorkerClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InProcess => "in_process",
            Self::ChildProcess => "child_process",
            Self::RemoteService => "remote_service",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExecutorIdentity {
    id: String,
    implementation_version: String,
    worker_class: ExecutorWorkerClass,
}

impl ExecutorIdentity {
    pub fn new(
        id: impl Into<String>,
        implementation_version: impl Into<String>,
        worker_class: ExecutorWorkerClass,
    ) -> Result<Self, ContainmentContractError> {
        let id = validate_code("executor ID", id.into(), true)
            .map_err(ContainmentContractError::InvalidIdentity)?;
        let implementation_version = validate_code(
            "executor implementation version",
            implementation_version.into(),
            false,
        )
        .map_err(ContainmentContractError::InvalidIdentity)?;
        Ok(Self {
            id,
            implementation_version,
            worker_class,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn implementation_version(&self) -> &str {
        &self.implementation_version
    }

    pub const fn worker_class(&self) -> ExecutorWorkerClass {
        self.worker_class
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainmentBoundaryKind {
    Workspace,
    Process,
    Network,
}

impl ContainmentBoundaryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Process => "process",
            Self::Network => "network",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainmentLevel {
    Enforced,
    Partial,
    Unavailable,
}

impl ContainmentLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enforced => "enforced",
            Self::Partial => "partial",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainmentControl {
    PermitRequestBinding,
    CanonicalWorkspaceRoot,
    NoFollowPathResolution,
    BoundedIo,
    AtomicFileCommit,
    FixedExecutable,
    ClearedEnvironment,
    TimeoutAndCancellation,
    TimeoutAndTermination,
    ManagedWorktreeRoot,
    ExactCommitVerification,
    PayloadDigestBinding,
    CredentialReference,
    DirectStdio,
    ProtocolFrameLimits,
    FreshFallbackProcess,
}

impl ContainmentControl {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PermitRequestBinding => "permit_request_binding",
            Self::CanonicalWorkspaceRoot => "canonical_workspace_root",
            Self::NoFollowPathResolution => "no_follow_path_resolution",
            Self::BoundedIo => "bounded_io",
            Self::AtomicFileCommit => "atomic_file_commit",
            Self::FixedExecutable => "fixed_executable",
            Self::ClearedEnvironment => "cleared_environment",
            Self::TimeoutAndCancellation => "timeout_and_cancellation",
            Self::TimeoutAndTermination => "timeout_and_termination",
            Self::ManagedWorktreeRoot => "managed_worktree_root",
            Self::ExactCommitVerification => "exact_commit_verification",
            Self::PayloadDigestBinding => "payload_digest_binding",
            Self::CredentialReference => "credential_reference",
            Self::DirectStdio => "direct_stdio",
            Self::ProtocolFrameLimits => "protocol_frame_limits",
            Self::FreshFallbackProcess => "fresh_fallback_process",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainmentLimitation {
    HostFilesystemNotSandboxed,
    HostProcessNotSandboxed,
    NetworkNotRestricted,
    RemoteServiceOutsideLocalBoundary,
    NativeServerNotSandboxed,
    RepositoryMetadataOutsideManagedRoot,
}

impl ContainmentLimitation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HostFilesystemNotSandboxed => "host_filesystem_not_sandboxed",
            Self::HostProcessNotSandboxed => "host_process_not_sandboxed",
            Self::NetworkNotRestricted => "network_not_restricted",
            Self::RemoteServiceOutsideLocalBoundary => "remote_service_outside_local_boundary",
            Self::NativeServerNotSandboxed => "native_server_not_sandboxed",
            Self::RepositoryMetadataOutsideManagedRoot => {
                "repository_metadata_outside_managed_root"
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContainmentBoundary {
    kind: ContainmentBoundaryKind,
    level: ContainmentLevel,
    controls: Vec<ContainmentControl>,
    limitation: Option<ContainmentLimitation>,
}

impl ContainmentBoundary {
    pub fn enforced(
        kind: ContainmentBoundaryKind,
        controls: Vec<ContainmentControl>,
    ) -> Result<Self, ContainmentContractError> {
        Self::with_controls(kind, ContainmentLevel::Enforced, controls, None)
    }

    pub fn partial(
        kind: ContainmentBoundaryKind,
        controls: Vec<ContainmentControl>,
        limitation: ContainmentLimitation,
    ) -> Result<Self, ContainmentContractError> {
        Self::with_controls(kind, ContainmentLevel::Partial, controls, Some(limitation))
    }

    pub const fn unavailable(
        kind: ContainmentBoundaryKind,
        limitation: ContainmentLimitation,
    ) -> Self {
        Self {
            kind,
            level: ContainmentLevel::Unavailable,
            controls: Vec::new(),
            limitation: Some(limitation),
        }
    }

    fn with_controls(
        kind: ContainmentBoundaryKind,
        level: ContainmentLevel,
        mut controls: Vec<ContainmentControl>,
        limitation: Option<ContainmentLimitation>,
    ) -> Result<Self, ContainmentContractError> {
        if controls.is_empty() {
            return Err(ContainmentContractError::EmptyControls(kind.as_str()));
        }
        controls.sort_unstable();
        if let Some(duplicate) = controls
            .windows(2)
            .find_map(|pair| (pair[0] == pair[1]).then_some(pair[0]))
        {
            return Err(ContainmentContractError::DuplicateControl(
                duplicate.as_str(),
            ));
        }
        Ok(Self {
            kind,
            level,
            controls,
            limitation,
        })
    }

    pub const fn kind(&self) -> ContainmentBoundaryKind {
        self.kind
    }

    pub const fn level(&self) -> ContainmentLevel {
        self.level
    }

    pub fn controls(&self) -> &[ContainmentControl] {
        &self.controls
    }

    pub const fn limitation(&self) -> Option<ContainmentLimitation> {
        self.limitation
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContainmentEvidence {
    #[serde(flatten)]
    identity: ExecutorIdentity,
    digest: String,
    boundaries: Vec<ContainmentBoundary>,
}

impl ContainmentEvidence {
    pub fn new(
        identity: ExecutorIdentity,
        mut boundaries: Vec<ContainmentBoundary>,
    ) -> Result<Self, ContainmentContractError> {
        if boundaries.is_empty() {
            return Err(ContainmentContractError::EmptyBoundaries);
        }
        boundaries.sort_by_key(ContainmentBoundary::kind);
        if let Some(duplicate) = boundaries
            .windows(2)
            .find_map(|pair| (pair[0].kind == pair[1].kind).then_some(pair[0].kind))
        {
            return Err(ContainmentContractError::DuplicateBoundary(
                duplicate.as_str(),
            ));
        }
        let digest = evidence_digest(&identity, &boundaries);
        Ok(Self {
            identity,
            digest,
            boundaries,
        })
    }

    pub fn identity(&self) -> &ExecutorIdentity {
        &self.identity
    }

    pub fn boundaries(&self) -> &[ContainmentBoundary] {
        &self.boundaries
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContainmentPlatform {
    os: String,
    architecture: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContainmentSnapshot {
    version: u16,
    authority: &'static str,
    platform: ContainmentPlatform,
    digest: String,
    executors: Vec<ContainmentEvidence>,
}

impl ContainmentSnapshot {
    pub fn new(
        platform_os: impl Into<String>,
        platform_architecture: impl Into<String>,
        mut executors: Vec<ContainmentEvidence>,
    ) -> Result<Self, ContainmentContractError> {
        let os = validate_code("OS", platform_os.into(), false)
            .map_err(ContainmentContractError::InvalidPlatform)?;
        let architecture = validate_code("architecture", platform_architecture.into(), false)
            .map_err(ContainmentContractError::InvalidPlatform)?;
        if executors.is_empty() {
            return Err(ContainmentContractError::EmptyExecutors);
        }
        executors.sort_by(|left, right| left.identity.id.cmp(&right.identity.id));
        if let Some(duplicate) = executors.windows(2).find_map(|pair| {
            (pair[0].identity.id == pair[1].identity.id).then(|| pair[0].identity.id.clone())
        }) {
            return Err(ContainmentContractError::DuplicateExecutor(duplicate));
        }
        let digest = snapshot_digest(&os, &architecture, &executors);
        Ok(Self {
            version: CONTAINMENT_EVIDENCE_VERSION,
            authority: "evidence_only",
            platform: ContainmentPlatform { os, architecture },
            digest,
            executors,
        })
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub fn platform_os(&self) -> &str {
        &self.platform.os
    }

    pub fn platform_architecture(&self) -> &str {
        &self.platform.architecture
    }

    pub fn executors(&self) -> &[ContainmentEvidence] {
        &self.executors
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

fn evidence_digest(identity: &ExecutorIdentity, boundaries: &[ContainmentBoundary]) -> String {
    let mut hasher = Sha256::new();
    digest_text(&mut hasher, "pandora-containment-evidence-v1");
    digest_text(&mut hasher, "identity");
    digest_text(&mut hasher, identity.id());
    digest_text(&mut hasher, identity.implementation_version());
    digest_text(&mut hasher, identity.worker_class().as_str());
    for boundary in boundaries {
        digest_text(&mut hasher, "boundary");
        digest_text(&mut hasher, boundary.kind().as_str());
        digest_text(&mut hasher, boundary.level().as_str());
        for control in boundary.controls() {
            digest_text(&mut hasher, "control");
            digest_text(&mut hasher, control.as_str());
        }
        digest_text(&mut hasher, "limitation");
        digest_text(
            &mut hasher,
            boundary
                .limitation()
                .map_or("none", ContainmentLimitation::as_str),
        );
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn snapshot_digest(os: &str, architecture: &str, executors: &[ContainmentEvidence]) -> String {
    let mut hasher = Sha256::new();
    digest_text(&mut hasher, "pandora-containment-snapshot-v1");
    digest_text(&mut hasher, os);
    digest_text(&mut hasher, architecture);
    for executor in executors {
        digest_text(&mut hasher, "executor");
        digest_text(&mut hasher, executor.digest());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn digest_text(hasher: &mut Sha256, value: &str) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn validate_code(
    field: &'static str,
    value: String,
    lower_case_only: bool,
) -> Result<String, &'static str> {
    let valid = !value.is_empty()
        && value.len() <= MAX_IDENTITY_BYTES
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || (!lower_case_only && byte.is_ascii_uppercase())
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b'+')
        });
    valid.then_some(value).ok_or(field)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(id: &str) -> ExecutorIdentity {
        ExecutorIdentity::new(id, "2.0.0-alpha.6", ExecutorWorkerClass::InProcess).unwrap()
    }

    #[test]
    fn evidence_digest_is_deterministic_across_input_order() {
        let process = ContainmentBoundary::partial(
            ContainmentBoundaryKind::Process,
            vec![
                ContainmentControl::BoundedIo,
                ContainmentControl::PermitRequestBinding,
            ],
            ContainmentLimitation::HostProcessNotSandboxed,
        )
        .unwrap();
        let network = ContainmentBoundary::unavailable(
            ContainmentBoundaryKind::Network,
            ContainmentLimitation::NetworkNotRestricted,
        );
        let first =
            ContainmentEvidence::new(identity("process"), vec![process.clone(), network.clone()])
                .unwrap();
        let reordered_process = ContainmentBoundary::partial(
            ContainmentBoundaryKind::Process,
            vec![
                ContainmentControl::PermitRequestBinding,
                ContainmentControl::BoundedIo,
            ],
            ContainmentLimitation::HostProcessNotSandboxed,
        )
        .unwrap();
        let second =
            ContainmentEvidence::new(identity("process"), vec![network, reordered_process])
                .unwrap();

        assert_eq!(first.digest(), second.digest());
        assert_eq!(
            first.digest(),
            "sha256:14d077b4920bdd613bae8c3051aef71ed56e0c2e85f55d62cb84ecf36674de76"
        );
    }

    #[test]
    fn snapshot_digest_is_deterministic_across_executor_order() {
        let filesystem = ContainmentEvidence::new(
            identity("filesystem"),
            vec![
                ContainmentBoundary::partial(
                    ContainmentBoundaryKind::Workspace,
                    vec![ContainmentControl::CanonicalWorkspaceRoot],
                    ContainmentLimitation::HostFilesystemNotSandboxed,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let process = ContainmentEvidence::new(
            identity("process"),
            vec![
                ContainmentBoundary::partial(
                    ContainmentBoundaryKind::Process,
                    vec![ContainmentControl::FixedExecutable],
                    ContainmentLimitation::HostProcessNotSandboxed,
                )
                .unwrap(),
            ],
        )
        .unwrap();

        let first = ContainmentSnapshot::new(
            "windows",
            "x86_64",
            vec![filesystem.clone(), process.clone()],
        )
        .unwrap();
        let second =
            ContainmentSnapshot::new("windows", "x86_64", vec![process, filesystem]).unwrap();

        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.executors()[0].identity().id(), "filesystem");
        assert_eq!(first.executors()[1].identity().id(), "process");
    }

    #[test]
    fn invalid_executor_identity_is_rejected() {
        assert_eq!(
            ExecutorIdentity::new("", "2.0.0", ExecutorWorkerClass::InProcess),
            Err(ContainmentContractError::InvalidIdentity("executor ID"))
        );
    }

    #[test]
    fn controlled_boundary_requires_at_least_one_control() {
        assert_eq!(
            ContainmentBoundary::partial(
                ContainmentBoundaryKind::Network,
                Vec::new(),
                ContainmentLimitation::NetworkNotRestricted,
            ),
            Err(ContainmentContractError::EmptyControls("network"))
        );
    }

    #[test]
    fn unavailable_boundary_cannot_claim_controls_by_construction() {
        let boundary = ContainmentBoundary::unavailable(
            ContainmentBoundaryKind::Network,
            ContainmentLimitation::NetworkNotRestricted,
        );
        assert!(boundary.controls().is_empty());
        assert_eq!(boundary.level(), ContainmentLevel::Unavailable);
        assert_eq!(
            boundary.limitation(),
            Some(ContainmentLimitation::NetworkNotRestricted)
        );
    }
}
