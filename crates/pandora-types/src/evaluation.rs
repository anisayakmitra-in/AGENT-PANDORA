use crate::effect::{EffectReceipt, Timestamp};
use crate::ids::{ExecutionId, SessionId};
use std::collections::BTreeSet;
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
    EmptyResults,
    DuplicateKind(EvaluationKind),
    InvalidScore,
}

impl fmt::Display for EvaluationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::EmptyResults => formatter.write_str("evaluation receipt requires a result"),
            Self::DuplicateKind(kind) => {
                write!(formatter, "evaluation receipt repeats {}", kind.as_str())
            }
            Self::InvalidScore => formatter.write_str("evaluation score exceeds 100"),
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
    terminal_failure: Option<String>,
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
            terminal_failure: None,
        })
    }

    pub fn with_policy_violations(mut self, violations: Vec<String>) -> Self {
        self.policy_violations = violations;
        self
    }

    pub fn with_terminal_failure(
        mut self,
        failure: impl Into<String>,
    ) -> Result<Self, EvaluationContractError> {
        self.terminal_failure = Some(validate_text("terminal failure", failure.into(), 4096)?);
        Ok(self)
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

    pub fn terminal_failure(&self) -> Option<&str> {
        self.terminal_failure.as_deref()
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
        if score > 100 {
            return Err(EvaluationContractError::InvalidScore);
        }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationReceipt {
    session_id: SessionId,
    execution_id: ExecutionId,
    evaluated_at: Timestamp,
    results: Vec<EvaluationResult>,
}

impl EvaluationReceipt {
    pub fn new(
        session_id: SessionId,
        execution_id: ExecutionId,
        evaluated_at: Timestamp,
        results: Vec<EvaluationResult>,
    ) -> Result<Self, EvaluationContractError> {
        if results.is_empty() {
            return Err(EvaluationContractError::EmptyResults);
        }
        let mut kinds = BTreeSet::new();
        for result in &results {
            if !kinds.insert(result.kind()) {
                return Err(EvaluationContractError::DuplicateKind(result.kind()));
            }
        }
        Ok(Self {
            session_id,
            execution_id,
            evaluated_at,
            results,
        })
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub const fn evaluated_at(&self) -> Timestamp {
        self.evaluated_at
    }

    pub fn results(&self) -> &[EvaluationResult] {
        &self.results
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluation_receipt_requires_unique_results() {
        let execution_id = ExecutionId::new("execution-1").unwrap();
        let session_id = crate::SessionId::new("session-1").unwrap();
        let result = EvaluationResult::new(
            EvaluationKind::Trajectory,
            EvaluationStatus::Passed,
            100,
            "trajectory passed",
            false,
        )
        .unwrap();

        assert!(matches!(
            EvaluationReceipt::new(
                session_id.clone(),
                execution_id.clone(),
                Timestamp::from_unix_seconds(1),
                Vec::new(),
            ),
            Err(EvaluationContractError::EmptyResults)
        ));
        assert!(matches!(
            EvaluationReceipt::new(
                session_id,
                execution_id,
                Timestamp::from_unix_seconds(1),
                vec![result.clone(), result],
            ),
            Err(EvaluationContractError::DuplicateKind(
                EvaluationKind::Trajectory
            ))
        ));
    }

    #[test]
    fn evaluation_score_is_bounded_to_one_hundred() {
        assert!(matches!(
            EvaluationResult::new(
                EvaluationKind::Outcome,
                EvaluationStatus::Passed,
                101,
                "invalid score",
                false,
            ),
            Err(EvaluationContractError::InvalidScore)
        ));
    }
}
