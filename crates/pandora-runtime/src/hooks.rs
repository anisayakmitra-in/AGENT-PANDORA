//! Ordered lifecycle guards evaluated inside the governed execution path.
//!
//! Hooks are declarative veto rules. They cannot mutate requests, execute code,
//! resolve approvals, or issue effect permits. Runtime events remain the
//! observation surface.

use pandora_types::{Capability, GeneId, OperationRequest};
use serde::{Deserialize, Serialize};

/// A closed point where a declarative hook may reduce authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    BeforeAuthorization,
}

/// A bounded selector evaluated against an immutable operation request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum HookSelector {
    Any,
    Capability(Capability),
    Gene(GeneId),
}

impl HookSelector {
    fn matches(&self, request: &OperationRequest) -> bool {
        match self {
            Self::Any => true,
            Self::Capability(capability) => request.capability() == *capability,
            Self::Gene(gene_id) => request.gene_id() == gene_id,
        }
    }
}

/// A declarative lifecycle veto rule.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LifecycleHook {
    id: String,
    point: HookPoint,
    selector: HookSelector,
    reason: String,
}

impl LifecycleHook {
    /// Creates a rule that denies the first matching request at `point`.
    pub fn deny(
        id: impl Into<String>,
        point: HookPoint,
        selector: HookSelector,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            point,
            selector,
            reason: reason.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn point(&self) -> HookPoint {
        self.point
    }

    pub fn selector(&self) -> &HookSelector {
        &self.selector
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HookDecision {
    Continue,
    Deny { hook_id: String, reason: String },
}

/// An immutable, registration-ordered set of lifecycle rules.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LifecycleHooks {
    hooks: Vec<LifecycleHook>,
}

impl LifecycleHooks {
    /// Creates an empty hook set that permits normal policy evaluation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a rule. The first matching denial wins.
    pub fn with_hook(mut self, hook: LifecycleHook) -> Self {
        self.hooks.push(hook);
        self
    }

    pub fn hooks(&self) -> &[LifecycleHook] {
        &self.hooks
    }

    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn from_json_str(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub(crate) fn evaluate(&self, point: HookPoint, request: &OperationRequest) -> HookDecision {
        self.hooks
            .iter()
            .find(|hook| hook.point == point && hook.selector.matches(request))
            .map_or(HookDecision::Continue, |hook| HookDecision::Deny {
                hook_id: hook.id.clone(),
                reason: hook.reason.clone(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        Capability, ExecutionId, GeneId, Operation, OperationRequest, PrincipalId, ResourceScope,
        SessionId,
    };

    fn test_request(capability: Capability) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("exec-1").unwrap(),
            SessionId::new("sess-1").unwrap(),
            PrincipalId::new("princ-1").unwrap(),
            crate::test_support::execution_profile("hooks"),
            GeneId::new("workspace.read").unwrap(),
            None,
            capability,
            Operation::Read,
            pandora_types::EffectTarget::path("test.txt"),
            ResourceScope::workspace("default"),
        )
        .unwrap()
    }

    #[test]
    fn lifecycle_hooks_serialization_round_trip() {
        let hooks = LifecycleHooks::new()
            .with_hook(LifecycleHook::deny(
                "block-write",
                HookPoint::BeforeAuthorization,
                HookSelector::Capability(Capability::FilesystemWrite),
                "writes blocked by policy",
            ))
            .with_hook(LifecycleHook::deny(
                "block-any",
                HookPoint::BeforeAuthorization,
                HookSelector::Any,
                "global veto",
            ));

        let json = hooks.to_json_string().unwrap();
        let deserialized = LifecycleHooks::from_json_str(&json).unwrap();
        assert_eq!(hooks, deserialized);
        assert_eq!(deserialized.len(), 2);
    }

    #[test]
    fn lifecycle_hooks_evaluate_veto() {
        let hooks = LifecycleHooks::new().with_hook(LifecycleHook::deny(
            "block-write",
            HookPoint::BeforeAuthorization,
            HookSelector::Capability(Capability::FilesystemWrite),
            "writes forbidden",
        ));

        let write_req = test_request(Capability::FilesystemWrite);
        let read_req = test_request(Capability::FilesystemRead);

        assert_eq!(
            hooks.evaluate(HookPoint::BeforeAuthorization, &write_req),
            HookDecision::Deny {
                hook_id: "block-write".to_owned(),
                reason: "writes forbidden".to_owned(),
            }
        );
        assert_eq!(
            hooks.evaluate(HookPoint::BeforeAuthorization, &read_req),
            HookDecision::Continue
        );
    }
}
