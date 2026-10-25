use crate::capability::{Capability, Operation};
use crate::effect::OperationRequest;
use crate::ids::RequestDigest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GovernanceMode {
    Autonomous,
    Supervised,
    ReadOnly,
}

impl GovernanceMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "autonomous" | "auto" | "2" | "full" => Some(Self::Autonomous),
            "supervised" | "super" | "1" | "safe" | "default" => Some(Self::Supervised),
            "read-only" | "readonly" | "ro" | "0" | "read" => Some(Self::ReadOnly),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Autonomous => "autonomous",
            Self::Supervised => "supervised",
            Self::ReadOnly => "read-only",
        }
    }

    pub const fn level(self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::Supervised => 1,
            Self::Autonomous => 2,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyContext {
    policy_version: u32,
    allowed_capabilities: BTreeSet<Capability>,
    approval_operations: BTreeSet<Operation>,
}

impl PolicyContext {
    pub fn new(
        policy_version: u32,
        allowed_capabilities: impl IntoIterator<Item = Capability>,
        approval_operations: impl IntoIterator<Item = Operation>,
    ) -> Self {
        Self {
            policy_version,
            allowed_capabilities: allowed_capabilities.into_iter().collect(),
            approval_operations: approval_operations.into_iter().collect(),
        }
    }

    pub fn read_only_workspace() -> Self {
        Self::new(
            1,
            [Capability::FilesystemRead, Capability::ProviderInvoke],
            [
                Operation::Write,
                Operation::Execute,
                Operation::Connect,
                Operation::Install,
            ],
        )
    }

    pub fn supervised_workspace() -> Self {
        Self::new(
            1,
            [
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
                Capability::ProcessExecute,
                Capability::NetworkConnect,
                Capability::ProviderInvoke,
                Capability::WasmExecute,
                Capability::McpInvoke,
            ],
            [Operation::Write, Operation::Execute, Operation::Connect],
        )
    }

    pub fn autonomous_workspace() -> Self {
        Self::new(
            1,
            [
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
                Capability::ProcessExecute,
                Capability::NetworkConnect,
                Capability::ProviderInvoke,
                Capability::WasmExecute,
                Capability::McpInvoke,
            ],
            [],
        )
    }

    pub fn for_mode(mode: GovernanceMode) -> Self {
        match mode {
            GovernanceMode::ReadOnly => Self::read_only_workspace(),
            GovernanceMode::Supervised => Self::supervised_workspace(),
            GovernanceMode::Autonomous => Self::autonomous_workspace(),
        }
    }

    pub fn mode(&self) -> GovernanceMode {
        if self.approval_operations.is_empty() {
            GovernanceMode::Autonomous
        } else if self.allows(Capability::FilesystemWrite) {
            GovernanceMode::Supervised
        } else {
            GovernanceMode::ReadOnly
        }
    }

    pub fn allowed_capabilities(&self) -> &BTreeSet<Capability> {
        &self.allowed_capabilities
    }

    pub fn approval_operations(&self) -> &BTreeSet<Operation> {
        &self.approval_operations
    }

    pub fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub fn allows(&self, capability: Capability) -> bool {
        self.allowed_capabilities.contains(&capability)
    }

    pub fn requires_approval(&self, operation: Operation) -> bool {
        self.approval_operations.contains(&operation)
    }
}

impl Default for PolicyContext {
    fn default() -> Self {
        Self::new(1, [], [])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParliamentDecision {
    Allow {
        request_digest: RequestDigest,
        policy_version: u32,
        reason: String,
    },
    Deny {
        request_digest: RequestDigest,
        policy_version: u32,
        reason: String,
    },
    RequireApproval {
        request_digest: RequestDigest,
        policy_version: u32,
        reason: String,
    },
}

impl ParliamentDecision {
    pub fn allow(
        request: &OperationRequest,
        context: &PolicyContext,
        reason: impl Into<String>,
    ) -> Self {
        Self::Allow {
            request_digest: request.request_digest().clone(),
            policy_version: context.policy_version(),
            reason: reason.into(),
        }
    }

    pub fn deny(
        request: &OperationRequest,
        context: &PolicyContext,
        reason: impl Into<String>,
    ) -> Self {
        Self::Deny {
            request_digest: request.request_digest().clone(),
            policy_version: context.policy_version(),
            reason: reason.into(),
        }
    }

    pub fn require_approval(
        request: &OperationRequest,
        context: &PolicyContext,
        reason: impl Into<String>,
    ) -> Self {
        Self::RequireApproval {
            request_digest: request.request_digest().clone(),
            policy_version: context.policy_version(),
            reason: reason.into(),
        }
    }

    pub fn request_digest(&self) -> &RequestDigest {
        match self {
            Self::Allow { request_digest, .. }
            | Self::Deny { request_digest, .. }
            | Self::RequireApproval { request_digest, .. } => request_digest,
        }
    }

    pub fn policy_version(&self) -> u32 {
        match self {
            Self::Allow { policy_version, .. }
            | Self::Deny { policy_version, .. }
            | Self::RequireApproval { policy_version, .. } => *policy_version,
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Allow { reason, .. }
            | Self::Deny { reason, .. }
            | Self::RequireApproval { reason, .. } => reason,
        }
    }

    pub fn requires_approval(&self) -> bool {
        matches!(self, Self::RequireApproval { .. })
    }
}

#[cfg(test)]
mod tests {
    use crate::events::{EventContext, EventPayload, EventType, RuntimeEvent};
    use crate::harness::{HarnessKind, HarnessManifest};
    use crate::{EventId, GeneId, RequestDigest, SecretReference, TenantId, WorkspaceId};

    #[test]
    fn source_harness_requires_one_constitutional_service() {
        let result = HarnessManifest::new(
            "system.memory",
            "1.0.0",
            "Memory",
            HarnessKind::Source,
            None,
            Vec::new(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn domain_harness_does_not_require_a_constitutional_service() {
        let result = HarnessManifest::new(
            "coding",
            "1.0.0",
            "Coding",
            HarnessKind::Domain,
            None,
            vec![GeneId::new("workspace.read").unwrap()],
        );

        assert!(result.is_ok());
    }

    #[test]
    fn runtime_event_serialization_is_stable_and_redacted() {
        let event = RuntimeEvent::new(
            EventId::new("event-1").unwrap(),
            EventType::ProviderCall,
            EventContext::new(
                TenantId::new("tenant-1").unwrap(),
                WorkspaceId::new("workspace-1").unwrap(),
            ),
            EventPayload::ProviderCall {
                provider: "openai".to_owned(),
                credential: SecretReference::new("credential-1").unwrap(),
                request_digest: RequestDigest::new("pandora-request-v1:sha256:test").unwrap(),
            },
        );

        let first = serde_json::to_string(&event).unwrap();
        let second = serde_json::to_string(&event).unwrap();

        assert_eq!(first, second);
        assert!(first.contains("\"protocol_version\":1"));
        assert!(first.contains("credential-1"));
        assert!(!first.contains("sk-live-secret"));
    }

    #[test]
    fn governance_mode_parsing_and_levels() {
        use super::{GovernanceMode, PolicyContext};
        use crate::capability::{Capability, Operation};

        assert_eq!(
            GovernanceMode::parse("autonomous"),
            Some(GovernanceMode::Autonomous)
        );
        assert_eq!(
            GovernanceMode::parse("auto"),
            Some(GovernanceMode::Autonomous)
        );
        assert_eq!(GovernanceMode::parse("2"), Some(GovernanceMode::Autonomous));
        assert_eq!(
            GovernanceMode::parse("full"),
            Some(GovernanceMode::Autonomous)
        );

        assert_eq!(
            GovernanceMode::parse("supervised"),
            Some(GovernanceMode::Supervised)
        );
        assert_eq!(
            GovernanceMode::parse("super"),
            Some(GovernanceMode::Supervised)
        );
        assert_eq!(GovernanceMode::parse("1"), Some(GovernanceMode::Supervised));
        assert_eq!(
            GovernanceMode::parse("safe"),
            Some(GovernanceMode::Supervised)
        );
        assert_eq!(
            GovernanceMode::parse("default"),
            Some(GovernanceMode::Supervised)
        );

        assert_eq!(
            GovernanceMode::parse("read-only"),
            Some(GovernanceMode::ReadOnly)
        );
        assert_eq!(
            GovernanceMode::parse("readonly"),
            Some(GovernanceMode::ReadOnly)
        );
        assert_eq!(GovernanceMode::parse("ro"), Some(GovernanceMode::ReadOnly));
        assert_eq!(GovernanceMode::parse("0"), Some(GovernanceMode::ReadOnly));
        assert_eq!(
            GovernanceMode::parse("read"),
            Some(GovernanceMode::ReadOnly)
        );

        assert_eq!(GovernanceMode::parse("invalid"), None);

        assert_eq!(GovernanceMode::ReadOnly.level(), 0);
        assert_eq!(GovernanceMode::Supervised.level(), 1);
        assert_eq!(GovernanceMode::Autonomous.level(), 2);

        assert_eq!(GovernanceMode::ReadOnly.as_str(), "read-only");
        assert_eq!(GovernanceMode::Supervised.as_str(), "supervised");
        assert_eq!(GovernanceMode::Autonomous.as_str(), "autonomous");

        // Verify policy contexts
        let ro = PolicyContext::for_mode(GovernanceMode::ReadOnly);
        assert_eq!(ro.mode(), GovernanceMode::ReadOnly);
        assert!(ro.allows(Capability::FilesystemRead));
        assert!(!ro.allows(Capability::FilesystemWrite));
        assert!(ro.requires_approval(Operation::Write));

        let sup = PolicyContext::for_mode(GovernanceMode::Supervised);
        assert_eq!(sup.mode(), GovernanceMode::Supervised);
        assert!(sup.allows(Capability::FilesystemRead));
        assert!(sup.allows(Capability::FilesystemWrite));
        assert!(sup.requires_approval(Operation::Write));
        assert!(sup.requires_approval(Operation::Execute));

        let auto = PolicyContext::for_mode(GovernanceMode::Autonomous);
        assert_eq!(auto.mode(), GovernanceMode::Autonomous);
        assert!(auto.allows(Capability::FilesystemRead));
        assert!(auto.allows(Capability::FilesystemWrite));
        assert!(auto.approval_operations().is_empty());
        assert!(!auto.requires_approval(Operation::Write));
        assert!(!auto.requires_approval(Operation::Execute));
    }
}
