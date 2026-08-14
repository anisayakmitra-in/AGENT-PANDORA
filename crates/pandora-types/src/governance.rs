use crate::capability::{Capability, Operation};
use crate::effect::OperationRequest;
use crate::ids::RequestDigest;
use std::collections::BTreeSet;

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
            [Capability::FilesystemRead],
            [
                Operation::Write,
                Operation::Execute,
                Operation::Connect,
                Operation::Invoke,
                Operation::Install,
            ],
        )
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
}
