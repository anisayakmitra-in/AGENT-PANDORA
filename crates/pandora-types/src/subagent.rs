use crate::ids::{ExecutionId, HarnessId, IdError, SessionId};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_SUBAGENT_TASK_BYTES: usize = 64 * 1024;
pub const MAX_SUBAGENT_RESULT_BYTES: usize = 1024 * 1024;
pub const MAX_SUBAGENT_TURNS: u32 = 64;
pub const MAX_SUBAGENT_TOOL_CALLS: u32 = 128;
pub const MAX_SUBAGENT_TOKENS: u32 = 2_000_000;
pub const MAX_SUBAGENT_DURATION_SECONDS: u64 = 86_400;
pub const MAX_SUBAGENT_DELEGATION_DEPTH: u8 = 8;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Preparing,
    Queued,
    Running,
    ApprovalRequired,
    Completed,
    Failed,
    Interrupted,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentWorktreeState {
    Pending,
    Ready,
    Preserved,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubagentContractError {
    InvalidTurnBudget,
    InvalidToolBudget,
    InvalidTokenBudget,
    InvalidDurationBudget,
    InvalidDelegationDepth,
    InvalidResultBudget,
    InvalidCommit,
    EmptyField(&'static str),
    FieldTooLarge(&'static str),
    ControlCharacter(&'static str),
    InvalidIdentifier(IdError),
}

impl fmt::Display for SubagentContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTurnBudget => formatter.write_str("turn budget is invalid"),
            Self::InvalidToolBudget => formatter.write_str("tool budget is invalid"),
            Self::InvalidTokenBudget => formatter.write_str("token budget is invalid"),
            Self::InvalidDurationBudget => formatter.write_str("duration budget is invalid"),
            Self::InvalidDelegationDepth => formatter.write_str("delegation depth is invalid"),
            Self::InvalidResultBudget => formatter.write_str("result budget is invalid"),
            Self::InvalidCommit => {
                formatter.write_str("commit must be 40 or 64 hexadecimal characters")
            }
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLarge(field) => write!(formatter, "{field} exceeds its size limit"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::InvalidIdentifier(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SubagentContractError {}

impl From<IdError> for SubagentContractError {
    fn from(error: IdError) -> Self {
        Self::InvalidIdentifier(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubagentBudgets {
    max_turns: u32,
    max_tool_calls: u32,
    max_tokens: u32,
    max_duration_seconds: u64,
    max_delegation_depth: u8,
    max_result_bytes: usize,
}

impl SubagentBudgets {
    pub fn new(
        max_turns: u32,
        max_tool_calls: u32,
        max_tokens: u32,
        max_duration_seconds: u64,
        max_delegation_depth: u8,
        max_result_bytes: usize,
    ) -> Result<Self, SubagentContractError> {
        if !(1..=MAX_SUBAGENT_TURNS).contains(&max_turns) {
            return Err(SubagentContractError::InvalidTurnBudget);
        }
        if !(1..=MAX_SUBAGENT_TOOL_CALLS).contains(&max_tool_calls) {
            return Err(SubagentContractError::InvalidToolBudget);
        }
        if !(1..=MAX_SUBAGENT_TOKENS).contains(&max_tokens) {
            return Err(SubagentContractError::InvalidTokenBudget);
        }
        if !(1..=MAX_SUBAGENT_DURATION_SECONDS).contains(&max_duration_seconds) {
            return Err(SubagentContractError::InvalidDurationBudget);
        }
        if max_delegation_depth > MAX_SUBAGENT_DELEGATION_DEPTH {
            return Err(SubagentContractError::InvalidDelegationDepth);
        }
        if !(1..=MAX_SUBAGENT_RESULT_BYTES).contains(&max_result_bytes) {
            return Err(SubagentContractError::InvalidResultBudget);
        }
        Ok(Self {
            max_turns,
            max_tool_calls,
            max_tokens,
            max_duration_seconds,
            max_delegation_depth,
            max_result_bytes,
        })
    }

    pub const fn max_turns(&self) -> u32 {
        self.max_turns
    }

    pub const fn max_tool_calls(&self) -> u32 {
        self.max_tool_calls
    }

    pub const fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    pub const fn max_duration_seconds(&self) -> u64 {
        self.max_duration_seconds
    }

    pub const fn max_delegation_depth(&self) -> u8 {
        self.max_delegation_depth
    }

    pub const fn max_result_bytes(&self) -> usize {
        self.max_result_bytes
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubagentBudgetsWire {
    max_turns: u32,
    max_tool_calls: u32,
    max_tokens: u32,
    max_duration_seconds: u64,
    max_delegation_depth: u8,
    max_result_bytes: usize,
}

impl<'de> Deserialize<'de> for SubagentBudgets {
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let wire = SubagentBudgetsWire::deserialize(deserializer)?;
        Self::new(
            wire.max_turns,
            wire.max_tool_calls,
            wire.max_tokens,
            wire.max_duration_seconds,
            wire.max_delegation_depth,
            wire.max_result_bytes,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubagentHarnessBinding {
    harness_id: HarnessId,
    version: String,
}

impl SubagentHarnessBinding {
    pub fn new(
        harness_id: HarnessId,
        version: impl Into<String>,
    ) -> Result<Self, SubagentContractError> {
        Ok(Self {
            harness_id,
            version: validate_text("harness version", version.into())?,
        })
    }

    pub fn harness_id(&self) -> &HarnessId {
        &self.harness_id
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubagentHarnessBindingWire {
    harness_id: String,
    version: String,
}

impl<'de> Deserialize<'de> for SubagentHarnessBinding {
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let wire = SubagentHarnessBindingWire::deserialize(deserializer)?;
        let harness_id = HarnessId::new(wire.harness_id).map_err(serde::de::Error::custom)?;
        Self::new(harness_id, wire.version).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SubagentRequest {
    parent_session_id: SessionId,
    parent_execution_id: ExecutionId,
    delegation_depth: u8,
    exact_commit: String,
    task: String,
    budgets: SubagentBudgets,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harness: Option<SubagentHarnessBinding>,
}

impl SubagentRequest {
    pub fn new(
        parent_session_id: SessionId,
        parent_execution_id: ExecutionId,
        delegation_depth: u8,
        exact_commit: impl Into<String>,
        task: impl Into<String>,
        budgets: SubagentBudgets,
    ) -> Result<Self, SubagentContractError> {
        if delegation_depth > MAX_SUBAGENT_DELEGATION_DEPTH {
            return Err(SubagentContractError::InvalidDelegationDepth);
        }
        let exact_commit = exact_commit.into();
        if !is_exact_commit(&exact_commit) {
            return Err(SubagentContractError::InvalidCommit);
        }
        Ok(Self {
            parent_session_id,
            parent_execution_id,
            delegation_depth,
            exact_commit,
            task: validate_task(task.into())?,
            budgets,
            provider_profile: None,
            harness: None,
        })
    }

    pub fn with_provider_profile(
        mut self,
        provider_profile: impl Into<String>,
    ) -> Result<Self, SubagentContractError> {
        self.provider_profile = Some(validate_text("provider profile", provider_profile.into())?);
        Ok(self)
    }

    pub fn with_harness(mut self, harness: SubagentHarnessBinding) -> Self {
        self.harness = Some(harness);
        self
    }

    pub fn parent_session_id(&self) -> &SessionId {
        &self.parent_session_id
    }

    pub fn parent_execution_id(&self) -> &ExecutionId {
        &self.parent_execution_id
    }

    pub const fn delegation_depth(&self) -> u8 {
        self.delegation_depth
    }

    pub fn exact_commit(&self) -> &str {
        &self.exact_commit
    }

    pub fn task(&self) -> &str {
        &self.task
    }

    pub fn budgets(&self) -> &SubagentBudgets {
        &self.budgets
    }

    pub fn provider_profile(&self) -> Option<&str> {
        self.provider_profile.as_deref()
    }

    pub fn harness(&self) -> Option<&SubagentHarnessBinding> {
        self.harness.as_ref()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubagentRequestWire {
    parent_session_id: String,
    parent_execution_id: String,
    delegation_depth: u8,
    exact_commit: String,
    task: String,
    budgets: SubagentBudgets,
    provider_profile: Option<String>,
    harness: Option<SubagentHarnessBinding>,
}

impl<'de> Deserialize<'de> for SubagentRequest {
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let wire = SubagentRequestWire::deserialize(deserializer)?;
        let parent_session_id = SessionId::new(wire.parent_session_id)
            .map_err(SubagentContractError::from)
            .map_err(serde::de::Error::custom)?;
        let parent_execution_id = ExecutionId::new(wire.parent_execution_id)
            .map_err(SubagentContractError::from)
            .map_err(serde::de::Error::custom)?;
        let request = Self::new(
            parent_session_id,
            parent_execution_id,
            wire.delegation_depth,
            wire.exact_commit,
            wire.task,
            wire.budgets,
        )
        .map_err(serde::de::Error::custom)?;
        let request = match wire.provider_profile {
            Some(provider_profile) => request
                .with_provider_profile(provider_profile)
                .map_err(serde::de::Error::custom)?,
            None => request,
        };
        Ok(match wire.harness {
            Some(harness) => request.with_harness(harness),
            None => request,
        })
    }
}

fn is_exact_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_task(value: String) -> Result<String, SubagentContractError> {
    if value.trim().is_empty() {
        return Err(SubagentContractError::EmptyField("task"));
    }
    if value.len() > MAX_SUBAGENT_TASK_BYTES {
        return Err(SubagentContractError::FieldTooLarge("task"));
    }
    if value.chars().any(char::is_control) {
        return Err(SubagentContractError::ControlCharacter("task"));
    }
    Ok(value)
}

fn validate_text(field: &'static str, value: String) -> Result<String, SubagentContractError> {
    if value.trim().is_empty() {
        return Err(SubagentContractError::EmptyField(field));
    }
    if value.chars().any(char::is_control) {
        return Err(SubagentContractError::ControlCharacter(field));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExecutionId, HarnessId, SessionId, SubagentId};
    use serde_json::json;

    #[test]
    fn request_has_a_stable_bounded_wire_contract() {
        let request = SubagentRequest::new(
            SessionId::new("session-parent").unwrap(),
            ExecutionId::new("execution-parent").unwrap(),
            0,
            "a".repeat(40),
            "review the exact checkout",
            SubagentBudgets::new(8, 16, 50_000, 900, 2, 65_536).unwrap(),
        )
        .unwrap()
        .with_provider_profile("coding")
        .unwrap()
        .with_harness(
            SubagentHarnessBinding::new(HarnessId::new("coding-domain").unwrap(), "2.0.0-alpha.6")
                .unwrap(),
        );

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["parent_session_id"], "session-parent");
        assert_eq!(value["parent_execution_id"], "execution-parent");
        assert_eq!(value["exact_commit"], "a".repeat(40));
        assert!(value.get("workspace").is_none());
        assert!(value.get("approval_id").is_none());
        assert_eq!(
            serde_json::from_value::<SubagentRequest>(value).unwrap(),
            request
        );
    }

    #[test]
    fn budgets_and_task_fail_closed() {
        assert_eq!(
            SubagentBudgets::new(0, 1, 1, 1, 0, 1),
            Err(SubagentContractError::InvalidTurnBudget)
        );
        assert_eq!(
            SubagentBudgets::new(1, 129, 1, 1, 0, 1),
            Err(SubagentContractError::InvalidToolBudget)
        );
        assert!(matches!(
            SubagentRequest::new(
                SessionId::new("session-parent").unwrap(),
                ExecutionId::new("execution-parent").unwrap(),
                0,
                "not-a-commit",
                "task",
                SubagentBudgets::new(1, 1, 1, 1, 0, 1).unwrap(),
            ),
            Err(SubagentContractError::InvalidCommit)
        ));
    }

    #[test]
    fn request_rejects_forbidden_task_payload_fields() {
        for field in [
            "workspace",
            "permit",
            "approval_id",
            "credential",
            "command",
        ] {
            let mut value = serde_json::to_value(
                SubagentRequest::new(
                    SessionId::new("session-parent").unwrap(),
                    ExecutionId::new("execution-parent").unwrap(),
                    0,
                    "a".repeat(40),
                    "task",
                    SubagentBudgets::new(1, 1, 1, 1, 0, 1).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
            value[field] = json!("forbidden");

            assert!(
                serde_json::from_value::<SubagentRequest>(value).is_err(),
                "forbidden field {field} was accepted"
            );
        }
    }

    #[test]
    fn subagent_id_deserialization_rejects_empty_values() {
        assert!(serde_json::from_value::<SubagentId>(json!("")).is_err());
    }

    #[test]
    fn subagent_id_deserialization_rejects_overlong_values() {
        assert!(serde_json::from_value::<SubagentId>(json!("a".repeat(257))).is_err());
    }
}
