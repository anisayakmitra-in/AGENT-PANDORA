use pandora_types::{OperationRequest, ParliamentDecision, PolicyContext};

pub struct Parliament {
    policy_version: u32,
}

impl Parliament {
    pub const fn new(policy_version: u32) -> Self {
        Self { policy_version }
    }

    pub fn decide(
        &self,
        request: &OperationRequest,
        context: &PolicyContext,
    ) -> ParliamentDecision {
        if context.policy_version() != self.policy_version {
            return ParliamentDecision::deny(
                request,
                context,
                "policy version does not match the active Parliament",
            );
        }
        if !context.allows(request.capability()) {
            return ParliamentDecision::deny(
                request,
                context,
                "capability is not enabled by policy",
            );
        }
        if context.requires_approval(request.operation()) {
            return ParliamentDecision::require_approval(
                request,
                context,
                "operation requires explicit approval",
            );
        }
        ParliamentDecision::allow(request, context, "capability and operation are allowed")
    }
}

#[cfg(test)]
mod tests {
    use pandora_types::{
        Capability, EffectTarget, ExecutionId, GeneId, Operation, OperationRequest,
        ParliamentDecision, PolicyContext, PrincipalId, ResourceScope, SessionId,
    };

    use super::Parliament;

    fn request(
        capability: Capability,
        operation: Operation,
        target: EffectTarget,
    ) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            GeneId::new("test.gene").unwrap(),
            None,
            capability,
            operation,
            target,
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap()
    }

    #[test]
    fn read_only_workspace_inspection_is_allowed() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let request = request(
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("src/lib.rs"),
        );

        assert!(matches!(
            parliament.decide(&request, &context),
            ParliamentDecision::Allow { .. }
        ));
    }

    #[test]
    fn capability_not_in_policy_is_denied() {
        let parliament = Parliament::new(1);
        let request = request(
            Capability::ProviderInvoke,
            Operation::Invoke,
            EffectTarget::provider(
                "provider",
                pandora_types::SecretReference::new("credential").unwrap(),
            ),
        );

        assert!(matches!(
            parliament.decide(&request, &PolicyContext::default()),
            ParliamentDecision::Deny { .. }
        ));
    }

    #[test]
    fn writes_require_approval() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemWrite], [Operation::Write]);
        let request = request(
            Capability::FilesystemWrite,
            Operation::Write,
            EffectTarget::path("src/lib.rs"),
        );

        assert!(matches!(
            parliament.decide(&request, &context),
            ParliamentDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn process_execution_requires_approval() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::ProcessExecute], [Operation::Execute]);
        let request = request(
            Capability::ProcessExecute,
            Operation::Execute,
            EffectTarget::process("cargo"),
        );

        assert!(matches!(
            parliament.decide(&request, &context),
            ParliamentDecision::RequireApproval { .. }
        ));
    }
}
