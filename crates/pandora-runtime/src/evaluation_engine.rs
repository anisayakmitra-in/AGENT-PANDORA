use pandora_types::{
    EffectOutcome, EvaluationKind, EvaluationRequest, EvaluationResult, EvaluationStatus,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationError {
    InvalidMarker,
}

pub struct EvaluationEngine;

impl EvaluationEngine {
    pub const fn new() -> Self {
        Self
    }

    pub fn evaluate_trajectory(
        &self,
        input: &EvaluationRequest,
        max_failed_receipts: usize,
    ) -> EvaluationResult {
        let failures = input
            .receipts()
            .iter()
            .filter(|receipt| {
                matches!(
                    receipt.outcome(),
                    EffectOutcome::Failed { .. } | EffectOutcome::Denied { .. }
                )
            })
            .count();
        if failures <= max_failed_receipts {
            result(
                EvaluationKind::Trajectory,
                EvaluationStatus::Passed,
                100,
                "trajectory stayed within the failure budget",
                false,
            )
        } else {
            result(
                EvaluationKind::Trajectory,
                EvaluationStatus::Failed,
                0,
                "trajectory exceeded the failure budget",
                false,
            )
        }
    }

    pub fn evaluate_outcome(
        &self,
        input: &EvaluationRequest,
        expected_output: &str,
    ) -> EvaluationResult {
        let passed = input.redacted_output() == expected_output;
        result(
            EvaluationKind::Outcome,
            if passed {
                EvaluationStatus::Passed
            } else {
                EvaluationStatus::Failed
            },
            if passed { 100 } else { 0 },
            if passed {
                "redacted output matches the expected outcome"
            } else {
                "redacted output does not match the expected outcome"
            },
            false,
        )
    }

    pub fn evaluate_policy(&self, input: &EvaluationRequest) -> EvaluationResult {
        let passed = input.policy_violations().is_empty();
        result(
            EvaluationKind::Policy,
            if passed {
                EvaluationStatus::Passed
            } else {
                EvaluationStatus::Failed
            },
            if passed { 100 } else { 0 },
            if passed {
                "no policy violations were recorded"
            } else {
                "one or more policy violations were recorded"
            },
            false,
        )
    }

    pub fn require_human_review(
        &self,
        _input: &EvaluationRequest,
        reason: &str,
    ) -> EvaluationResult {
        result(
            EvaluationKind::Human,
            EvaluationStatus::HumanReviewRequired,
            0,
            reason,
            false,
        )
    }

    pub fn evaluate_regression(
        &self,
        input: &EvaluationRequest,
        baseline_output: &str,
    ) -> EvaluationResult {
        let passed = input.redacted_output() == baseline_output;
        result(
            EvaluationKind::Regression,
            if passed {
                EvaluationStatus::Passed
            } else {
                EvaluationStatus::Failed
            },
            if passed { 100 } else { 0 },
            if passed {
                "redacted output matches the regression baseline"
            } else {
                "redacted output differs from the regression baseline"
            },
            false,
        )
    }

    pub fn evaluate_adversarial(
        &self,
        input: &EvaluationRequest,
        forbidden_markers: &[&str],
    ) -> Result<EvaluationResult, EvaluationError> {
        if forbidden_markers
            .iter()
            .any(|marker| marker.trim().is_empty())
        {
            return Err(EvaluationError::InvalidMarker);
        }
        let found = forbidden_markers
            .iter()
            .any(|marker| input.redacted_output().contains(marker));
        Ok(result(
            EvaluationKind::Adversarial,
            if found {
                EvaluationStatus::Failed
            } else {
                EvaluationStatus::Passed
            },
            if found { 0 } else { 100 },
            if found {
                "a forbidden marker was found"
            } else {
                "no forbidden marker was found"
            },
            false,
        ))
    }

    pub fn advisory_judgement(
        &self,
        _input: &EvaluationRequest,
        passed: bool,
        score: u8,
        reason: &str,
    ) -> EvaluationResult {
        result(
            EvaluationKind::Outcome,
            if passed {
                EvaluationStatus::Passed
            } else {
                EvaluationStatus::Failed
            },
            score,
            reason,
            true,
        )
    }
}

impl Default for EvaluationEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn result(
    kind: EvaluationKind,
    status: EvaluationStatus,
    score: u8,
    reason: &str,
    advisory: bool,
) -> EvaluationResult {
    EvaluationResult::new(kind, status, score, reason, advisory)
        .expect("built-in evaluation result is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        EffectReceipt, ExecutionId, PermitId, ReceiptId, RequestDigest, Timestamp,
    };

    fn input(output: &str) -> EvaluationRequest {
        EvaluationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            vec![EffectReceipt::new(
                ReceiptId::new("receipt-1").unwrap(),
                PermitId::new("permit-1").unwrap(),
                RequestDigest::new("request-1").unwrap(),
                Timestamp::from_unix_seconds(2),
                EffectOutcome::Succeeded,
            )],
            output,
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn trajectory_and_outcome_are_distinct_evaluations() {
        let engine = EvaluationEngine::new();
        let request = input("done");

        let trajectory = engine.evaluate_trajectory(&request, 0);
        let outcome = engine.evaluate_outcome(&request, "done");

        assert_eq!(trajectory.kind(), EvaluationKind::Trajectory);
        assert_eq!(outcome.kind(), EvaluationKind::Outcome);
        assert_eq!(trajectory.status(), EvaluationStatus::Passed);
        assert_eq!(outcome.status(), EvaluationStatus::Passed);
    }

    #[test]
    fn policy_violations_fail_without_becoming_authority() {
        let request = input("done").with_policy_violations(vec!["undeclared tool".to_owned()]);
        let result = EvaluationEngine::new().evaluate_policy(&request);

        assert_eq!(result.kind(), EvaluationKind::Policy);
        assert_eq!(result.status(), EvaluationStatus::Failed);
        assert!(!result.can_authorize_permit());
    }

    #[test]
    fn human_review_is_explicit_and_blocking() {
        let result =
            EvaluationEngine::new().require_human_review(&input("needs review"), "write operation");

        assert_eq!(result.kind(), EvaluationKind::Human);
        assert_eq!(result.status(), EvaluationStatus::HumanReviewRequired);
    }

    #[test]
    fn regression_and_adversarial_checks_are_separate() {
        let engine = EvaluationEngine::new();
        let request = input("safe result");

        assert_eq!(
            engine.evaluate_regression(&request, "safe result").kind(),
            EvaluationKind::Regression
        );
        assert_eq!(
            engine
                .evaluate_adversarial(&request, &["unsafe"])
                .unwrap()
                .kind(),
            EvaluationKind::Adversarial
        );
    }

    #[test]
    fn advisory_judgement_cannot_approve_execution() {
        let result =
            EvaluationEngine::new().advisory_judgement(&input("done"), true, 90, "looks good");

        assert!(result.advisory());
        assert!(result.passed());
        assert!(!result.can_authorize_permit());
    }
}
