//! Ordered lifecycle guards evaluated inside the governed execution path.
//!
//! Hooks are declarative veto rules. They cannot mutate requests, execute code,
//! resolve approvals, or issue effect permits. Runtime events remain the
//! observation surface.

use pandora_types::{Capability, GeneId, OperationRequest};

/// A closed point where a declarative hook may reduce authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookPoint {
    BeforeAuthorization,
}

/// A bounded selector evaluated against an immutable operation request.
#[derive(Clone, Debug, Eq, PartialEq)]
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
#[derive(Clone, Debug, Eq, PartialEq)]
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HookDecision {
    Continue,
    Deny { hook_id: String, reason: String },
}

/// An immutable, registration-ordered set of lifecycle rules.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
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
