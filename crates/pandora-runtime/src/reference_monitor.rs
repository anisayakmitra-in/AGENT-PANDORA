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

    pub fn update_policy(&mut self, policy: PolicyContext) {
        let policy_version = policy.policy_version();
        self.parliament = Parliament::new(policy_version);
        self.policy = policy;
        self.policy_version = policy_version;
    }

    pub fn policy(&self) -> &PolicyContext {
        &self.policy
    }

    pub fn parliament(&self) -> &Parliament {
        &self.parliament
    }

    pub fn parliament_mut(&mut self) -> &mut Parliament {
        &mut self.parliament
    }

    pub fn with_decision_engine(
        mut self,
        decision_engine: Arc<crate::decision_engine::SinglePassLogitEngine>,
    ) -> Self {
        self.parliament = self.parliament.with_decision_engine(decision_engine);
        self
    }

    pub fn with_lats_engine(mut self, lats: Arc<crate::lats_engine::LatsEngine>) -> Self {
        self.parliament = self.parliament.with_lats_engine(lats);
        self
    }

    pub fn with_uncertainty_threshold(mut self, threshold: f64) -> Self {
        self.parliament = self.parliament.with_uncertainty_threshold(threshold);
        self
    }

    pub fn decision_engine(&self) -> Option<&Arc<crate::decision_engine::SinglePassLogitEngine>> {
        self.parliament.decision_engine()
    }

    pub fn lats_engine(&self) -> Option<&Arc<crate::lats_engine::LatsEngine>> {
        self.parliament.lats_engine()
    }

    pub fn uncertainty_threshold(&self) -> f64 {
        self.parliament.uncertainty_threshold()
    }

    pub fn should_escalate(&self, verdict: &pandora_types::DynamicVerdict) -> bool {
        self.parliament.should_escalate(verdict)
    }

    /// Evaluates tool safety reflexively in Parliament (System 1) with escalation to System 2 MCTS
    /// when margin < uncertainty_threshold OR normalized entropy > 0.666, and authorizes effect
    /// permit issuance if allowed.
    pub fn authorize_tool_safety(
        &self,
        request: OperationRequest,
        tool_name: &str,
        tool_args: &serde_json::Value,
        now: Timestamp,
    ) -> Result<EffectPermit, AuthorizationError> {
        let (permit, _) =
            self.authorize_tool_safety_with_verdict(request, tool_name, tool_args, now)?;
        Ok(permit)
    }

    /// Authorizes tool safety with full reflex verdict output (including escalation status,
    /// confidence margin, normalized entropy, reflex choice, and MCTS action if escalated).
    pub fn authorize_tool_safety_with_verdict(
        &self,
        request: OperationRequest,
        tool_name: &str,
        tool_args: &serde_json::Value,
        now: Timestamp,
    ) -> Result<(EffectPermit, crate::parliament::ToolSafetyReflexVerdict), AuthorizationError>
    {
        let reflex = self
            .parliament
            .evaluate_tool_safety_reflex(&request, &self.policy, tool_name, tool_args)
            .map_err(|e| AuthorizationError::Denied {
                reason: format!("tool safety evaluation failed: {e}"),
            })?;

        match reflex.decision {
            ParliamentDecision::Allow { .. } => {}
            ParliamentDecision::Deny { ref reason, .. } => {
                return Err(AuthorizationError::Denied {
                    reason: reason.clone(),
                });
            }
            ParliamentDecision::RequireApproval { ref reason, .. } => {
                return Err(AuthorizationError::ApprovalRequired {
                    reason: reason.clone(),
                });
            }
        }

        let permit = self.issue_permit(request, now)?;
        Ok((permit, reflex))
    }

    /// Evaluates tool safety into [`crate::parliament::ToolSafetyGateVerdict`] via Parliament.
    pub fn evaluate_tool_safety(
        &self,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Result<crate::parliament::ToolSafetyGateVerdict, crate::decision_engine::DecisionError>
    {
        self.parliament.evaluate_tool_safety(tool_name, tool_args)
    }

    /// Evaluates tool safety reflex into [`crate::parliament::ToolSafetyReflexVerdict`] via Parliament.
    pub fn evaluate_tool_safety_reflex(
        &self,
        request: &OperationRequest,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Result<crate::parliament::ToolSafetyReflexVerdict, crate::decision_engine::DecisionError>
    {
        self.parliament
            .evaluate_tool_safety_reflex(request, &self.policy, tool_name, tool_args)
    }

    /// Validates a ParliamentDecision generated by fast-path tool safety against this ReferenceMonitor.
    pub fn validate_tool_safety_decision(
        &self,
        request: &OperationRequest,
        decision: &ParliamentDecision,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Result<(), AuthorizationError> {
        let expected =
            self.parliament
                .decide_tool_safety(request, &self.policy, tool_name, tool_args);
        if expected != *decision {
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

    /// Authorizes a request and evaluates escalation against the dynamic verdict.
    pub fn authorize_with_safety_gate(
        &self,
        request: OperationRequest,
        decision: ParliamentDecision,
        verdict: &pandora_types::DynamicVerdict,
        now: Timestamp,
    ) -> Result<(EffectPermit, bool), AuthorizationError> {
        let permit = self.authorize(request, decision, now)?;
        let escalate = self.should_escalate(verdict);
        Ok((permit, escalate))
    }

    pub fn governance_mode(&self) -> pandora_types::GovernanceMode {
        self.policy.mode()
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
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
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

    #[test]
    fn test_reference_monitor_tool_safety_fast_path_reflex_evaluation() {
        let monitor = ReferenceMonitor::new(1, 300);
        let req = request("session-1");

        // Safe inspection tool reflex evaluation
        let verdict = monitor
            .evaluate_tool_safety_reflex(
                &req,
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
            )
            .expect("reflex evaluation succeeded");

        assert!(verdict.is_allow());
        assert!(!verdict.is_escalated());
        assert!(verdict.margin >= 0.15);
        assert!(verdict.normalized_entropy <= 0.666);

        // Authorize with verdict
        let (permit, reflex) = monitor
            .authorize_tool_safety_with_verdict(
                req.clone(),
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
                Timestamp::from_unix_seconds(10),
            )
            .expect("authorization succeeded");

        assert_eq!(permit.request_digest(), req.request_digest());
        assert!(reflex.is_allow());

        // Validate decision
        monitor
            .validate_tool_safety_decision(
                &req,
                &reflex.decision,
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
            )
            .expect("decision validation succeeded");
    }

    #[test]
    fn test_reference_monitor_confident_tool_execution_allow_without_escalation() {
        let monitor = ReferenceMonitor::new(1, 300);
        let req = request("session-1");

        let permit = monitor
            .authorize_tool_safety(
                req.clone(),
                "inspect_file",
                &serde_json::json!({"path": "Cargo.toml"}),
                Timestamp::from_unix_seconds(10),
            )
            .expect("direct tool safety authorization succeeded");

        assert_eq!(permit.request_digest(), req.request_digest());
    }

    #[test]
    fn test_reference_monitor_escalation_triggering_when_margin_below_threshold() {
        let monitor = ReferenceMonitor::new(1, 300).with_uncertainty_threshold(0.9999);
        let req = request("session-1");

        let verdict = monitor
            .evaluate_tool_safety_reflex(
                &req,
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
            )
            .expect("reflex evaluation succeeded");

        assert!(verdict.escalated);
    }

    #[test]
    fn test_reference_monitor_escalation_triggering_when_normalized_entropy_exceeds_0_666() {
        use crate::decision_engine::SinglePassLogitEngine;
        use std::sync::Arc;

        let engine = Arc::new(SinglePassLogitEngine::with_temperature(30.0).unwrap());
        let monitor = ReferenceMonitor::new(1, 300).with_decision_engine(engine);
        let req = request("session-1");

        let verdict = monitor
            .evaluate_tool_safety_reflex(
                &req,
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
            )
            .expect("reflex evaluation succeeded");

        assert!(verdict.escalated);
        assert!(verdict.normalized_entropy > 0.666);
    }

    #[test]
    fn test_reference_monitor_bounded_system2_mcts_tree_search_upon_escalation() {
        use std::sync::Arc;

        let driver = Arc::new(crate::decision_engine::TropicalDecisionDriver::calibrated());
        let lats = Arc::new(crate::lats_engine::LatsEngine::with_tropical_driver(driver));
        let monitor = ReferenceMonitor::new(1, 300)
            .with_lats_engine(lats)
            .with_uncertainty_threshold(0.9999);
        let req = request("session-1");

        let (permit, reflex) = monitor
            .authorize_tool_safety_with_verdict(
                req.clone(),
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
                Timestamp::from_unix_seconds(10),
            )
            .expect("escalated tool authorization succeeded");

        assert!(reflex.escalated);
        assert!(reflex.mcts_action.is_some());
        assert_eq!(permit.request_digest(), req.request_digest());
    }

    #[test]
    fn test_reference_monitor_reject_and_require_confirm() {
        let monitor = ReferenceMonitor::new(1, 300);
        let req = request("session-1");

        // Destructive command -> rejected
        let err = monitor
            .authorize_tool_safety(
                req.clone(),
                "run_command",
                &serde_json::json!({"command": "rm -rf /"}),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap_err();
        assert!(matches!(err, AuthorizationError::Denied { .. }));

        // Mutating command -> requires confirmation
        let err2 = monitor
            .authorize_tool_safety(
                req,
                "replace_file_content",
                &serde_json::json!({"path": "src/lib.rs"}),
                Timestamp::from_unix_seconds(10),
            )
            .unwrap_err();
        assert!(matches!(err2, AuthorizationError::ApprovalRequired { .. }));
    }
}
