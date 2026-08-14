use crate::permit_store::{PermitError, PermitStore};
use pandora_types::{EffectPermit, OperationRequest, ParliamentDecision, PermitId, Timestamp};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ReferenceMonitor {
    policy_version: u32,
    permit_ttl_seconds: u64,
    next_nonce: AtomicU64,
    store: Arc<PermitStore>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorizationError {
    RequestMismatch,
    PolicyMismatch,
    Denied { reason: String },
    ApprovalRequired { reason: String },
    ExpiryOverflow,
    NonceExhausted,
    InvalidPermitId,
    Store(PermitError),
}

impl ReferenceMonitor {
    pub fn new(policy_version: u32, permit_ttl_seconds: u64) -> Self {
        Self {
            policy_version,
            permit_ttl_seconds,
            next_nonce: AtomicU64::new(1),
            store: Arc::new(PermitStore::new()),
        }
    }

    pub fn authorize(
        &self,
        request: OperationRequest,
        decision: ParliamentDecision,
        now: Timestamp,
    ) -> Result<EffectPermit, AuthorizationError> {
        if decision.request_digest() != request.request_digest() {
            return Err(AuthorizationError::RequestMismatch);
        }
        if decision.policy_version() != self.policy_version {
            return Err(AuthorizationError::PolicyMismatch);
        }

        match decision {
            ParliamentDecision::Allow { .. } => {}
            ParliamentDecision::Deny { reason, .. } => {
                return Err(AuthorizationError::Denied { reason });
            }
            ParliamentDecision::RequireApproval { reason, .. } => {
                return Err(AuthorizationError::ApprovalRequired { reason });
            }
        }

        let nonce = self
            .next_nonce
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_add(1)
            })
            .map_err(|_| AuthorizationError::NonceExhausted)?;
        let expires_at = now
            .as_unix_seconds()
            .checked_add(self.permit_ttl_seconds)
            .map(Timestamp::from_unix_seconds)
            .ok_or(AuthorizationError::ExpiryOverflow)?;
        let permit_id = PermitId::new(format!("permit-{nonce}"))
            .map_err(|_| AuthorizationError::InvalidPermitId)?;
        let permit = EffectPermit::issue(
            permit_id,
            &request,
            self.policy_version,
            nonce,
            now,
            expires_at,
        );
        self.store
            .register(permit.clone())
            .map_err(AuthorizationError::Store)?;
        Ok(permit)
    }

    pub fn store(&self) -> Arc<PermitStore> {
        Arc::clone(&self.store)
    }
}

#[cfg(test)]
mod tests {
    use pandora_types::{
        Capability, EffectTarget, ExecutionId, GeneId, Operation, OperationRequest, PolicyContext,
        PrincipalId, ResourceScope, SessionId, Timestamp,
    };

    use super::*;
    use crate::parliament::Parliament;

    fn request(session: &str) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new(session).unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            GeneId::new("workspace.read").unwrap(),
            None,
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("src/lib.rs"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap()
    }

    #[test]
    fn approval_required_does_not_issue_a_permit() {
        let parliament = Parliament::new(1);
        let monitor = ReferenceMonitor::new(1, 60);
        let context = PolicyContext::new(1, [Capability::FilesystemWrite], [Operation::Write]);
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            GeneId::new("workspace.write").unwrap(),
            None,
            Capability::FilesystemWrite,
            Operation::Write,
            EffectTarget::path("src/lib.rs"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap();
        let decision = parliament.decide(&request, &context);

        assert!(matches!(
            monitor.authorize(request, decision, Timestamp::from_unix_seconds(10)),
            Err(AuthorizationError::ApprovalRequired { .. })
        ));
    }

    #[test]
    fn an_allowed_request_gets_a_registered_permit() {
        let parliament = Parliament::new(1);
        let monitor = ReferenceMonitor::new(1, 60);
        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let request = request("session-1");
        let decision = parliament.decide(&request, &context);

        let permit = monitor
            .authorize(request.clone(), decision, Timestamp::from_unix_seconds(10))
            .unwrap();
        assert_eq!(permit.request_digest(), request.request_digest());
    }
}
