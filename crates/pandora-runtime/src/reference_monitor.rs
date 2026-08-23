use crate::approvals::ApprovalGrant;
use crate::parliament::Parliament;
use crate::permit_store::{PermitError, PermitStore};
use pandora_types::{
    EffectPermit, OperationRequest, ParliamentDecision, PermitId, PolicyContext, Timestamp,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ReferenceMonitor {
    parliament: Parliament,
    policy: PolicyContext,
    policy_version: u32,
    permit_ttl_seconds: u64,
    next_nonce: AtomicU64,
    store: Arc<PermitStore>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorizationError {
    RequestMismatch,
    DecisionMismatch,
    PolicyMismatch,
    Denied { reason: String },
    ApprovalRequired { reason: String },
    ApprovalEvidenceRequired,
    ApprovalNotRequired,
    ExpiryOverflow,
    NonceExhausted,
    InvalidPermitId,
    Store(PermitError),
}

impl ReferenceMonitor {
    pub fn new(policy_version: u32, permit_ttl_seconds: u64) -> Self {
        Self::new_with_policy(
            PolicyContext::new(
                policy_version,
                [
                    pandora_types::Capability::FilesystemRead,
                    pandora_types::Capability::ProviderInvoke,
                ],
                [
                    pandora_types::Operation::Write,
                    pandora_types::Operation::Execute,
                    pandora_types::Operation::Connect,
                    pandora_types::Operation::Install,
                ],
            ),
            permit_ttl_seconds,
        )
    }

    pub fn new_with_policy(policy: PolicyContext, permit_ttl_seconds: u64) -> Self {
        let policy_version = policy.policy_version();
        Self {
            parliament: Parliament::new(policy_version),
            policy,
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
        self.validate_decision(&request, &decision)?;

        match decision {
            ParliamentDecision::Allow { .. } => {}
            ParliamentDecision::Deny { reason, .. } => {
                return Err(AuthorizationError::Denied { reason });
            }
            ParliamentDecision::RequireApproval { reason, .. } => {
                return Err(AuthorizationError::ApprovalRequired { reason });
            }
        }

        self.issue_permit(request, now)
    }

    pub fn authorize_after_approval(
        &self,
        request: OperationRequest,
        decision: ParliamentDecision,
        _now: Timestamp,
    ) -> Result<EffectPermit, AuthorizationError> {
        if !decision.requires_approval() {
            return Err(AuthorizationError::ApprovalNotRequired);
        }
        self.validate_decision(&request, &decision)?;
        Err(AuthorizationError::ApprovalEvidenceRequired)
    }

    pub(crate) fn authorize_after_approval_with_grant(
        &self,
        request: OperationRequest,
        decision: ParliamentDecision,
        _grant: &ApprovalGrant,
        now: Timestamp,
    ) -> Result<EffectPermit, AuthorizationError> {
        if !decision.requires_approval() {
            return Err(AuthorizationError::ApprovalNotRequired);
        }
        self.validate_decision(&request, &decision)?;
        if !_grant.matches(&request, self.policy_version) {
            return Err(AuthorizationError::RequestMismatch);
        }
        self.issue_permit(request, now)
    }

    fn validate_decision(
        &self,
        request: &OperationRequest,
        decision: &ParliamentDecision,
    ) -> Result<(), AuthorizationError> {
        if self.parliament.decide(request, &self.policy) != *decision {
            return Err(AuthorizationError::DecisionMismatch);
        }
        if decision.request_digest() != request.request_digest() {
            return Err(AuthorizationError::RequestMismatch);
        }
        if decision.policy_version() != self.policy_version {
            return Err(AuthorizationError::PolicyMismatch);
        }
        Ok(())
    }

    fn issue_permit(
        &self,
        request: OperationRequest,
        now: Timestamp,
    ) -> Result<EffectPermit, AuthorizationError> {
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
    use crate::{ApprovalRequest, ApprovalStore};
    use std::path::PathBuf;

    fn request(session: &str) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new(session).unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
            GeneId::new("workspace.read").unwrap(),
            None,
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("src/lib.rs"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap()
    }

    fn consumed_approval(request: &OperationRequest) -> (ApprovalGrant, PathBuf) {
        let directory = crate::test_support::new_temp_dir("pandora-approval-grant").unwrap();
        let store = ApprovalStore::open(directory.join("approvals.sqlite3")).unwrap();
        store
            .create(
                ApprovalRequest::new(
                    "approval-1",
                    request.session_id().clone(),
                    request.execution_id().clone(),
                    request.principal_id().clone(),
                    request.gene_id().clone(),
                    request.request_digest().clone(),
                    "approve the exact requested operation",
                    1,
                    Timestamp::from_unix_seconds(100),
                )
                .unwrap(),
            )
            .unwrap();
        let approver = pandora_types::PrincipalId::new("approver-1").unwrap();
        store
            .resolve(
                "approval-1",
                request.principal_id(),
                &approver,
                true,
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let grant = store
            .consume_grant(
                "approval-1",
                request.principal_id(),
                request.session_id(),
                request.execution_id(),
                request.gene_id(),
                request.request_digest(),
                Timestamp::from_unix_seconds(11),
            )
            .unwrap();
        (grant, directory)
    }

    #[test]
    fn approval_required_does_not_issue_a_permit() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemWrite], [Operation::Write]);
        let monitor = ReferenceMonitor::new_with_policy(context.clone(), 60);
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
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
    fn caller_cannot_expand_authority_with_a_forged_policy_decision() {
        let monitor = ReferenceMonitor::new(1, 60);
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
            GeneId::new("workspace.write").unwrap(),
            None,
            Capability::FilesystemWrite,
            Operation::Write,
            EffectTarget::path("src/lib.rs"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap();
        let broad_policy = PolicyContext::new(1, [Capability::FilesystemWrite], []);
        let forged_decision = Parliament::new(1).decide(&request, &broad_policy);

        assert_eq!(
            monitor.authorize(request, forged_decision, Timestamp::from_unix_seconds(10),),
            Err(AuthorizationError::DecisionMismatch)
        );
    }

    #[test]
    fn approval_decision_can_issue_only_after_external_approval() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemWrite], [Operation::Write]);
        let monitor = ReferenceMonitor::new_with_policy(context.clone(), 60);
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
            GeneId::new("workspace.write").unwrap(),
            None,
            Capability::FilesystemWrite,
            Operation::Write,
            EffectTarget::path("src/lib.rs"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap();
        let decision = parliament.decide(&request, &context);

        assert_eq!(
            monitor.authorize_after_approval(
                request.clone(),
                decision.clone(),
                Timestamp::from_unix_seconds(10),
            ),
            Err(AuthorizationError::ApprovalEvidenceRequired)
        );
        let (grant, directory) = consumed_approval(&request);
        let permit = monitor
            .authorize_after_approval_with_grant(
                request.clone(),
                decision,
                &grant,
                Timestamp::from_unix_seconds(11),
            )
            .unwrap();
        assert_eq!(permit.request_digest(), request.request_digest());
        let other_request = OperationRequest::new(
            ExecutionId::new("execution-2").unwrap(),
            SessionId::new("session-2").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
            GeneId::new("workspace.write").unwrap(),
            None,
            Capability::FilesystemWrite,
            Operation::Write,
            EffectTarget::path("src/lib.rs"),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap();
        let other_decision = parliament.decide(&other_request, &context);
        assert_eq!(
            monitor.authorize_after_approval_with_grant(
                other_request,
                other_decision,
                &grant,
                Timestamp::from_unix_seconds(11),
            ),
            Err(AuthorizationError::RequestMismatch)
        );
        std::fs::remove_dir_all(directory).unwrap();

        let allowed = ParliamentDecision::allow(&request, &context, "already allowed");
        assert_eq!(
            monitor.authorize_after_approval(request, allowed, Timestamp::from_unix_seconds(10)),
            Err(AuthorizationError::ApprovalNotRequired)
        );
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
