use crate::effect::{EffectReceipt, Timestamp};
use crate::ids::ExecutionId;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EvaluationKind {
    Trajectory,
    Outcome,
    Policy,
    Human,
    Regression,
    Adversarial,
}

impl EvaluationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trajectory => "trajectory",
            Self::Outcome => "outcome",
            Self::Policy => "policy",
            Self::Human => "human",
            Self::Regression => "regression",
            Self::Adversarial => "adversarial",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EvaluationStatus {
    Passed,
    Failed,
    HumanReviewRequired,
}

impl EvaluationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::HumanReviewRequired => "human_review_required",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationContractError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
}

impl fmt::Display for EvaluationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
        }
    }
}

impl std::error::Error for EvaluationContractError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationRequest {
    execution_id: ExecutionId,
    receipts: Vec<EffectReceipt>,
    redacted_output: String,
    policy_violations: Vec<String>,
}

impl EvaluationRequest {
    pub fn new(
        execution_id: ExecutionId,
        receipts: Vec<EffectReceipt>,
        redacted_output: impl Into<String>,
        policy_violations: Vec<String>,
    ) -> Result<Self, EvaluationContractError> {
        let redacted_output = validate_text("redacted output", redacted_output.into(), 65_536)?;
        let policy_violations = policy_violations
            .into_iter()
            .map(|violation| validate_text("policy violation", violation, 4096))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            execution_id,
            receipts,
            redacted_output,
            policy_violations,
        })
    }

    pub fn with_policy_violations(mut self, violations: Vec<String>) -> Self {
        self.policy_violations = violations;
        self
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn receipts(&self) -> &[EffectReceipt] {
        &self.receipts
    }

    pub fn redacted_output(&self) -> &str {
        &self.redacted_output
    }

    pub fn policy_violations(&self) -> &[String] {
        &self.policy_violations
    }

    pub fn observed_at(&self) -> Option<Timestamp> {
        self.receipts.iter().map(EffectReceipt::completed_at).max()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationResult {
    kind: EvaluationKind,
    status: EvaluationStatus,
    score: u8,
    reason: String,
    advisory: bool,
}

impl EvaluationResult {
    pub fn new(
        kind: EvaluationKind,
        status: EvaluationStatus,
        score: u8,
        reason: impl Into<String>,
        advisory: bool,
    ) -> Result<Self, EvaluationContractError> {
        Ok(Self {
            kind,
            status,
            score,
            reason: validate_text("evaluation reason", reason.into(), 4096)?,
            advisory,
        })
    }

    pub const fn kind(&self) -> EvaluationKind {
        self.kind
    }

    pub const fn status(&self) -> EvaluationStatus {
        self.status
    }

    pub const fn score(&self) -> u8 {
        self.score
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub const fn advisory(&self) -> bool {
        self.advisory
    }

    pub const fn passed(&self) -> bool {
        matches!(self.status, EvaluationStatus::Passed)
    }

    pub const fn can_authorize_permit(&self) -> bool {
        false
    }
}

fn validate_text(
    field: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, EvaluationContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(EvaluationContractError::EmptyField(field));
    }
    if value.len() > max_bytes {
        return Err(EvaluationContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(EvaluationContractError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}
