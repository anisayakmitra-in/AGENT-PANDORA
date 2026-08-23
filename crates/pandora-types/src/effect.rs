use crate::capability::{Capability, Operation};
use crate::execution_profile::ExecutionProfile;
use crate::ids::{
    ArtifactId, ExecutionId, GeneId, PermitId, PrincipalId, ReceiptId, RequestDigest, SessionId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

const REQUEST_PROTOCOL_VERSION: u16 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestError {
    InvalidId(crate::ids::IdError),
    EmptyField(&'static str),
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
        }
    }
}

impl std::error::Error for RequestError {}

impl From<crate::ids::IdError> for RequestError {
    fn from(error: crate::ids::IdError) -> Self {
        Self::InvalidId(error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SecretReference(String);

impl SecretReference {
    pub fn new(value: impl Into<String>) -> Result<Self, RequestError> {
        let value = value.into();
        validate_text("secret reference", &value)?;
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectTarget {
    Path {
        path: String,
    },
    Process {
        program: String,
    },
    Network {
        host: String,
        port: u16,
    },
    Provider {
        provider: String,
        credential: SecretReference,
    },
    Mcp {
        server: String,
        tool: String,
    },
    Package {
        package_id: String,
        version: String,
    },
}

impl EffectTarget {
    pub fn path(path: impl Into<String>) -> Self {
        Self::Path { path: path.into() }
    }

    pub fn process(program: impl Into<String>) -> Self {
        Self::Process {
            program: program.into(),
        }
    }

    pub fn network(host: impl Into<String>, port: u16) -> Self {
        Self::Network {
            host: host.into(),
            port,
        }
    }

    pub fn provider(provider: impl Into<String>, credential: SecretReference) -> Self {
        Self::Provider {
            provider: provider.into(),
            credential,
        }
    }

    pub fn mcp(server: impl Into<String>, tool: impl Into<String>) -> Self {
        Self::Mcp {
            server: server.into(),
            tool: tool.into(),
        }
    }

    pub fn package(package_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self::Package {
            package_id: package_id.into(),
            version: version.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceScope {
    None,
    Workspace { workspace_id: String },
    Path { root: String },
    Host { host: String },
}

impl ResourceScope {
    pub fn none() -> Self {
        Self::None
    }

    pub fn workspace(workspace_id: impl Into<String>) -> Self {
        Self::Workspace {
            workspace_id: workspace_id.into(),
        }
    }

    pub fn path(root: impl Into<String>) -> Self {
        Self::Path { root: root.into() }
    }

    pub fn host(host: impl Into<String>) -> Self {
        Self::Host { host: host.into() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationRequest {
    protocol_version: u16,
    execution_id: ExecutionId,
    session_id: SessionId,
    principal_id: PrincipalId,
    execution_profile: ExecutionProfile,
    gene_id: GeneId,
    artifact_id: Option<ArtifactId>,
    capability: Capability,
    operation: Operation,
    target: EffectTarget,
    resource_scope: ResourceScope,
    payload_digest: Option<RequestDigest>,
    request_digest: RequestDigest,
}

impl OperationRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        execution_id: ExecutionId,
        session_id: SessionId,
        principal_id: PrincipalId,
        execution_profile: ExecutionProfile,
        gene_id: GeneId,
        artifact_id: Option<ArtifactId>,
        capability: Capability,
        operation: Operation,
        target: EffectTarget,
        resource_scope: ResourceScope,
    ) -> Result<Self, RequestError> {
        validate_target(&target)?;
        validate_scope(&resource_scope)?;

        let mut request = Self {
            protocol_version: REQUEST_PROTOCOL_VERSION,
            execution_id,
            session_id,
            principal_id,
            execution_profile,
            gene_id,
            artifact_id,
            capability,
            operation,
            target,
            resource_scope,
            payload_digest: None,
            request_digest: RequestDigest::new("pending")?,
        };
        request.request_digest = request.calculate_digest()?;
        Ok(request)
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub fn execution_profile(&self) -> &ExecutionProfile {
        &self.execution_profile
    }

    pub fn gene_id(&self) -> &GeneId {
        &self.gene_id
    }

    pub fn artifact_id(&self) -> Option<&ArtifactId> {
        self.artifact_id.as_ref()
    }

    pub fn capability(&self) -> Capability {
        self.capability
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn target(&self) -> &EffectTarget {
        &self.target
    }

    pub fn resource_scope(&self) -> &ResourceScope {
        &self.resource_scope
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub fn with_payload_digest(mut self, payload: &[u8]) -> Result<Self, RequestError> {
        self.payload_digest = Some(digest_bytes("pandora-payload-v1", payload)?);
        self.request_digest = self.calculate_digest()?;
        Ok(self)
    }

    pub fn payload_digest(&self) -> Option<&RequestDigest> {
        self.payload_digest.as_ref()
    }

    pub fn payload_digest_matches(&self, payload: &[u8]) -> bool {
        self.payload_digest
            .as_ref()
            .zip(digest_bytes("pandora-payload-v1", payload).ok())
            .is_some_and(|(expected, actual)| expected.as_str() == actual.as_str())
    }

    fn calculate_digest(&self) -> Result<RequestDigest, RequestError> {
        let mut hasher = Sha256::new();
        hasher.update(self.canonical_json().as_bytes());
        let digest = hasher.finalize();
        let value = format!(
            "pandora-request-v2:sha256:{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        Ok(RequestDigest::new(value)?)
    }

    fn canonical_json(&self) -> String {
        let canonical = CanonicalOperationRequest {
            protocol_version: self.protocol_version,
            execution_id: self.execution_id.as_str(),
            session_id: self.session_id.as_str(),
            principal_id: self.principal_id.as_str(),
            execution_profile_digest: self.execution_profile.digest().as_str(),
            gene_id: self.gene_id.as_str(),
            artifact_id: self.artifact_id.as_ref().map(|id| id.as_str()),
            capability: self.capability.as_str(),
            operation: self.operation.as_str(),
            target: CanonicalTarget::from(&self.target),
            resource_scope: CanonicalScope::from(&self.resource_scope),
            payload_digest: self.payload_digest.as_ref().map(|digest| digest.as_str()),
        };
        serde_json::to_string(&canonical).expect("canonical effect request is serializable")
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const fn from_unix_seconds(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_unix_seconds(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectPermit {
    permit_id: PermitId,
    request_digest: RequestDigest,
    execution_id: ExecutionId,
    session_id: SessionId,
    principal_id: PrincipalId,
    policy_version: u32,
    nonce: u64,
    issued_at: Timestamp,
    expires_at: Timestamp,
}

impl EffectPermit {
    pub fn issue(
        permit_id: PermitId,
        request: &OperationRequest,
        policy_version: u32,
        nonce: u64,
        issued_at: Timestamp,
        expires_at: Timestamp,
    ) -> Self {
        Self {
            permit_id,
            request_digest: request.request_digest.clone(),
            execution_id: request.execution_id.clone(),
            session_id: request.session_id.clone(),
            principal_id: request.principal_id.clone(),
            policy_version,
            nonce,
            issued_at,
            expires_at,
        }
    }

    pub fn permit_id(&self) -> &PermitId {
        &self.permit_id
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    pub fn issued_at(&self) -> Timestamp {
        self.issued_at
    }

    pub fn expires_at(&self) -> Timestamp {
        self.expires_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectOutcome {
    Succeeded,
    Failed { code: String },
    Denied { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectReceipt {
    receipt_id: ReceiptId,
    permit_id: PermitId,
    request_digest: RequestDigest,
    completed_at: Timestamp,
    outcome: EffectOutcome,
}

impl EffectReceipt {
    pub fn new(
        receipt_id: ReceiptId,
        permit_id: PermitId,
        request_digest: RequestDigest,
        completed_at: Timestamp,
        outcome: EffectOutcome,
    ) -> Self {
        Self {
            receipt_id,
            permit_id,
            request_digest,
            completed_at,
            outcome,
        }
    }

    pub fn receipt_id(&self) -> &ReceiptId {
        &self.receipt_id
    }

    pub fn permit_id(&self) -> &PermitId {
        &self.permit_id
    }

    pub fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    pub fn completed_at(&self) -> Timestamp {
        self.completed_at
    }

    pub fn outcome(&self) -> &EffectOutcome {
        &self.outcome
    }
}

#[derive(Serialize)]
struct CanonicalOperationRequest<'a> {
    protocol_version: u16,
    execution_id: &'a str,
    session_id: &'a str,
    principal_id: &'a str,
    execution_profile_digest: &'a str,
    gene_id: &'a str,
    artifact_id: Option<&'a str>,
    capability: &'static str,
    operation: &'static str,
    target: CanonicalTarget<'a>,
    resource_scope: CanonicalScope<'a>,
    payload_digest: Option<&'a str>,
}

#[derive(Serialize)]
struct CanonicalTarget<'a> {
    kind: &'static str,
    value: &'a str,
    port: Option<u16>,
    secondary: Option<&'a str>,
}

impl<'a> From<&'a EffectTarget> for CanonicalTarget<'a> {
    fn from(target: &'a EffectTarget) -> Self {
        match target {
            EffectTarget::Path { path } => Self {
                kind: "path",
                value: path,
                port: None,
                secondary: None,
            },
            EffectTarget::Process { program } => Self {
                kind: "process",
                value: program,
                port: None,
                secondary: None,
            },
            EffectTarget::Network { host, port } => Self {
                kind: "network",
                value: host,
                port: Some(*port),
                secondary: None,
            },
            EffectTarget::Provider {
                provider,
                credential,
            } => Self {
                kind: "provider",
                value: provider,
                port: None,
                secondary: Some(credential.as_str()),
            },
            EffectTarget::Mcp { server, tool } => Self {
                kind: "mcp",
                value: server,
                port: None,
                secondary: Some(tool),
            },
            EffectTarget::Package {
                package_id,
                version,
            } => Self {
                kind: "package",
                value: package_id,
                port: None,
                secondary: Some(version),
            },
        }
    }
}

#[derive(Serialize)]
struct CanonicalScope<'a> {
    kind: &'static str,
    value: Option<&'a str>,
}

impl<'a> From<&'a ResourceScope> for CanonicalScope<'a> {
    fn from(scope: &'a ResourceScope) -> Self {
        match scope {
            ResourceScope::None => Self {
                kind: "none",
                value: None,
            },
            ResourceScope::Workspace { workspace_id } => Self {
                kind: "workspace",
                value: Some(workspace_id),
            },
            ResourceScope::Path { root } => Self {
                kind: "path",
                value: Some(root),
            },
            ResourceScope::Host { host } => Self {
                kind: "host",
                value: Some(host),
            },
        }
    }
}

fn validate_target(target: &EffectTarget) -> Result<(), RequestError> {
    match target {
        EffectTarget::Path { path } => validate_text("path", path),
        EffectTarget::Process { program } => validate_text("program", program),
        EffectTarget::Network { host, .. } => validate_text("host", host),
        EffectTarget::Provider {
            provider,
            credential,
        } => {
            validate_text("provider", provider)?;
            validate_text("credential reference", credential.as_str())
        }
        EffectTarget::Mcp { server, tool } => {
            validate_text("MCP server", server)?;
            validate_text("MCP tool", tool)
        }
        EffectTarget::Package {
            package_id,
            version,
        } => {
            validate_text("package ID", package_id)?;
            validate_text("package version", version)
        }
    }
}

fn validate_scope(scope: &ResourceScope) -> Result<(), RequestError> {
    match scope {
        ResourceScope::None => Ok(()),
        ResourceScope::Workspace { workspace_id } => validate_text("workspace ID", workspace_id),
        ResourceScope::Path { root } => validate_text("scope root", root),
        ResourceScope::Host { host } => validate_text("scope host", host),
    }
}

fn validate_text(field: &'static str, value: &str) -> Result<(), RequestError> {
    if value.trim().is_empty() {
        return Err(RequestError::EmptyField(field));
    }
    Ok(())
}

fn digest_bytes(prefix: &'static str, bytes: &[u8]) -> Result<RequestDigest, RequestError> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    RequestDigest::new(format!(
        "{prefix}:sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
    .map_err(RequestError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ArtifactId, ExecutionId, GeneId, PrincipalId, SessionId};
    use crate::{ExecutionProfile, ExecutionProfileBinding, ExecutionProfileBindingKind};

    fn profile(executor: &str) -> ExecutionProfile {
        ExecutionProfile::new(
            "2.0.0-alpha.6",
            "windows",
            "x86_64",
            1,
            r"C:\work\pandora",
            format!("sha256:{}", "2".repeat(64)),
            vec![
                ExecutionProfileBinding::new(
                    ExecutionProfileBindingKind::Executor,
                    executor,
                    Some("2.0.0-alpha.6"),
                    format!("sha256:{}", "1".repeat(64)),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn request(path: &str) -> OperationRequest {
        request_with_profile(path, profile("filesystem"))
    }

    fn request_with_profile(path: &str, profile: ExecutionProfile) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            profile,
            GeneId::new("workspace.read").unwrap(),
            Some(ArtifactId::new("artifact-1").unwrap()),
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path(path),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap()
    }

    #[test]
    fn equivalent_requests_have_the_same_digest() {
        assert_eq!(
            request("src/lib.rs").request_digest(),
            request("src/lib.rs").request_digest()
        );
    }

    #[test]
    fn changing_the_path_changes_the_digest() {
        assert_ne!(
            request("src/lib.rs").request_digest(),
            request("src/main.rs").request_digest()
        );
    }

    #[test]
    fn changing_only_the_execution_profile_changes_the_request_digest() {
        let filesystem = request_with_profile("src/lib.rs", profile("filesystem"));
        let process = request_with_profile("src/lib.rs", profile("process"));

        assert_ne!(filesystem.request_digest(), process.request_digest());
        assert_eq!(
            filesystem.execution_profile().digest(),
            profile("filesystem").digest()
        );
    }

    #[test]
    fn payload_digest_binds_the_exact_write_content() {
        let payload_request = request("src/lib.rs")
            .with_payload_digest(b"first patch")
            .unwrap();

        assert!(payload_request.payload_digest_matches(b"first patch"));
        assert!(!payload_request.payload_digest_matches(b"second patch"));
        assert_ne!(
            payload_request.request_digest(),
            request("src/lib.rs").request_digest()
        );
    }

    #[test]
    fn canonical_request_contains_secret_references_but_not_secret_values() {
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            profile("provider"),
            GeneId::new("provider.invoke").unwrap(),
            None,
            Capability::ProviderInvoke,
            Operation::Invoke,
            EffectTarget::provider("openai", SecretReference::new("credential-1").unwrap()),
            ResourceScope::none(),
        )
        .unwrap();

        let canonical = request.canonical_json();
        assert!(canonical.contains("credential-1"));
        assert!(!canonical.contains("sk-live-secret"));
    }
}
