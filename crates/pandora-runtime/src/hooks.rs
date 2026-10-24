//! Ordered lifecycle guards evaluated inside the governed execution path.
//!
//! Hooks are declarative veto rules. They cannot mutate requests, execute code,
//! resolve approvals, or issue effect permits. Runtime events remain the
//! observation surface.

use pandora_types::{Capability, GeneId, OperationRequest};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

/// Errors encountered while loading or validating lifecycle hooks from disk.
#[derive(Debug)]
pub enum HooksLoadError {
    NotFound(PathBuf),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    Validation {
        path: PathBuf,
        message: &'static str,
    },
}

impl fmt::Display for HooksLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "lifecycle hooks file not found: {}", path.display()),
            Self::Io { path, source } => {
                write!(
                    f,
                    "failed to read lifecycle hooks file {}: {source}",
                    path.display()
                )
            }
            Self::Json { path, source } => {
                write!(
                    f,
                    "failed to parse lifecycle hooks file {}: {source}",
                    path.display()
                )
            }
            Self::Validation { path, message } => {
                write!(
                    f,
                    "invalid lifecycle hooks file {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl Error for HooksLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::NotFound(_) | Self::Validation { .. } => None,
        }
    }
}

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

    /// Validates that each configured hook has a non-empty identifier and reason.
    pub fn validate(&self) -> Result<(), &'static str> {
        for hook in &self.hooks {
            if hook.id.trim().is_empty() {
                return Err("lifecycle hook id must not be empty");
            }
            if hook.reason.trim().is_empty() {
                return Err("lifecycle hook reason must not be empty");
            }
        }
        Ok(())
    }

    /// Loads, deserializes, and validates lifecycle hooks from a JSON file on disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, HooksLoadError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(HooksLoadError::NotFound(path.to_path_buf()));
        }
        let content = std::fs::read_to_string(path).map_err(|source| HooksLoadError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let hooks = Self::from_json_str(&content).map_err(|source| HooksLoadError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        hooks
            .validate()
            .map_err(|message| HooksLoadError::Validation {
                path: path.to_path_buf(),
                message,
            })?;
        Ok(hooks)
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

    #[test]
    fn lifecycle_hooks_validation() {
        let valid = LifecycleHooks::new().with_hook(LifecycleHook::deny(
            "veto-net",
            HookPoint::BeforeAuthorization,
            HookSelector::Capability(Capability::NetworkConnect),
            "no networking allowed",
        ));
        assert!(valid.validate().is_ok());

        let empty_id = LifecycleHooks::new().with_hook(LifecycleHook::deny(
            "   ",
            HookPoint::BeforeAuthorization,
            HookSelector::Capability(Capability::NetworkConnect),
            "reason",
        ));
        assert_eq!(
            empty_id.validate().unwrap_err(),
            "lifecycle hook id must not be empty"
        );

        let empty_reason = LifecycleHooks::new().with_hook(LifecycleHook::deny(
            "valid-id",
            HookPoint::BeforeAuthorization,
            HookSelector::Capability(Capability::NetworkConnect),
            " \t ",
        ));
        assert_eq!(
            empty_reason.validate().unwrap_err(),
            "lifecycle hook reason must not be empty"
        );
    }

    #[test]
    fn lifecycle_hooks_from_file() {
        let temp_dir = crate::test_support::new_temp_dir("lifecycle-hooks-test").unwrap();
        let valid_file = temp_dir.join("valid_hooks.json");
        let valid_hooks = LifecycleHooks::new().with_hook(LifecycleHook::deny(
            "block-write",
            HookPoint::BeforeAuthorization,
            HookSelector::Capability(Capability::FilesystemWrite),
            "no writes",
        ));
        std::fs::write(&valid_file, valid_hooks.to_json_string().unwrap()).unwrap();

        let loaded = LifecycleHooks::from_file(&valid_file).unwrap();
        assert_eq!(loaded, valid_hooks);

        // NotFound error
        let missing = temp_dir.join("does_not_exist.json");
        match LifecycleHooks::from_file(&missing) {
            Err(HooksLoadError::NotFound(p)) => assert_eq!(p, missing),
            other => panic!("expected NotFound, got {other:?}"),
        }

        // Json parse error
        let invalid_json_file = temp_dir.join("invalid.json");
        std::fs::write(&invalid_json_file, "{ not valid json").unwrap();
        match LifecycleHooks::from_file(&invalid_json_file) {
            Err(HooksLoadError::Json { path, .. }) => assert_eq!(path, invalid_json_file),
            other => panic!("expected Json error, got {other:?}"),
        }

        // Validation error
        let invalid_hook_file = temp_dir.join("invalid_hook.json");
        let invalid_hooks = LifecycleHooks::new().with_hook(LifecycleHook::deny(
            "",
            HookPoint::BeforeAuthorization,
            HookSelector::Any,
            "no id",
        ));
        std::fs::write(&invalid_hook_file, invalid_hooks.to_json_string().unwrap()).unwrap();
        match LifecycleHooks::from_file(&invalid_hook_file) {
            Err(HooksLoadError::Validation { path, message }) => {
                assert_eq!(path, invalid_hook_file);
                assert_eq!(message, "lifecycle hook id must not be empty");
            }
            other => panic!("expected Validation error, got {other:?}"),
        }
    }
}
