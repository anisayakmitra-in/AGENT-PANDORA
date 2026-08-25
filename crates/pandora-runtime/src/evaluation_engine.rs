use pandora_types::{
    EffectOutcome, EvaluationKind, EvaluationRequest, EvaluationResult, EvaluationStatus,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const MAX_GOLDEN_CASES: usize = 256;
pub const MAX_GOLDEN_CASE_ID_BYTES: usize = 256;
pub const MAX_GOLDEN_EXPECTED_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationError {
    InvalidMarker,
    EmptyGoldenCaseId,
    GoldenCaseIdTooLong,
    GoldenExpectedOutputTooLong,
    TooManyGoldenCases,
    DuplicateGoldenCase,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoldenCase {
    id: String,
    evaluation: EvaluationRequest,
    expected_output: String,
}

impl GoldenCase {
    pub fn new(
        id: impl Into<String>,
        evaluation: EvaluationRequest,
        expected_output: impl Into<String>,
    ) -> Result<Self, EvaluationError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(EvaluationError::EmptyGoldenCaseId);
        }
        if id.len() > MAX_GOLDEN_CASE_ID_BYTES {
            return Err(EvaluationError::GoldenCaseIdTooLong);
        }
        let expected_output = expected_output.into();
        if expected_output.len() > MAX_GOLDEN_EXPECTED_OUTPUT_BYTES {
            return Err(EvaluationError::GoldenExpectedOutputTooLong);
        }
        Ok(Self {
            id,
            evaluation,
            expected_output,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn evaluation(&self) -> &EvaluationRequest {
        &self.evaluation
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoldenCaseResult {
    id: String,
    result: EvaluationResult,
}

impl GoldenCaseResult {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn result(&self) -> &EvaluationResult {
        &self.result
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoldenSetReport {
    total: usize,
    passed: usize,
    failed: usize,
    digest: String,
    cases: Vec<GoldenCaseResult>,
}

impl GoldenSetReport {
    pub const fn total(&self) -> usize {
        self.total
    }

    pub const fn passed(&self) -> usize {
        self.passed
    }

    pub const fn failed(&self) -> usize {
        self.failed
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn cases(&self) -> &[GoldenCaseResult] {
        &self.cases
    }
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
        if failures <= max_failed_receipts && input.terminal_failure().is_none() {
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

    pub fn evaluate_golden_set<I>(&self, cases: I) -> Result<GoldenSetReport, EvaluationError>
    where
        I: IntoIterator<Item = GoldenCase>,
    {
        let mut cases = cases.into_iter().collect::<Vec<_>>();
        if cases.len() > MAX_GOLDEN_CASES {
            return Err(EvaluationError::TooManyGoldenCases);
        }
        cases.sort_by(|left, right| left.id.cmp(&right.id));
        let mut ids = BTreeSet::new();
        for case in &cases {
            if !ids.insert(case.id.clone()) {
                return Err(EvaluationError::DuplicateGoldenCase);
            }
        }

        let results = cases
            .iter()
            .map(|case| GoldenCaseResult {
                id: case.id.clone(),
                result: self.evaluate_outcome(&case.evaluation, &case.expected_output),
            })
            .collect::<Vec<_>>();
        let passed = results.iter().filter(|case| case.result.passed()).count();
        let failed = results.len().saturating_sub(passed);
        let digest = golden_set_digest(&results);

        Ok(GoldenSetReport {
            total: results.len(),
            passed,
            failed,
            digest,
            cases: results,
        })
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

fn golden_set_digest(cases: &[GoldenCaseResult]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pandora.golden-set.v1");
    for case in cases {
        digest_text(&mut hasher, &case.id);
        digest_text(&mut hasher, case.result.kind().as_str());
        digest_text(&mut hasher, case.result.status().as_str());
        hasher.update([case.result.score(), u8::from(case.result.advisory())]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn digest_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
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
    fn terminal_execution_failure_fails_trajectory_without_a_receipt() {
        let request = input("failed")
            .with_terminal_failure("executor_failed")
            .unwrap();

        let result = EvaluationEngine::new().evaluate_trajectory(&request, 0);

        assert_eq!(result.status(), EvaluationStatus::Failed);
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

    #[test]
    fn golden_set_is_bounded_deterministic_and_does_not_expose_expected_output() {
        let engine = EvaluationEngine::new();
        let first = engine
            .evaluate_golden_set([
                GoldenCase::new("case-b", input("done"), "done").unwrap(),
                GoldenCase::new("case-a", input("wrong"), "done").unwrap(),
            ])
            .unwrap();
        let second = engine
            .evaluate_golden_set([
                GoldenCase::new("case-a", input("wrong"), "done").unwrap(),
                GoldenCase::new("case-b", input("done"), "done").unwrap(),
            ])
            .unwrap();

        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.total(), 2);
        assert_eq!(first.passed(), 1);
        assert_eq!(first.failed(), 1);
        assert!(first.cases().iter().all(|case| case.id() != "done"));
    }

    #[test]
    fn golden_set_rejects_duplicate_ids_and_oversized_collections() {
        let engine = EvaluationEngine::new();
        assert_eq!(
            engine.evaluate_golden_set([
                GoldenCase::new("same", input("done"), "done").unwrap(),
                GoldenCase::new("same", input("done"), "done").unwrap(),
            ]),
            Err(EvaluationError::DuplicateGoldenCase)
        );

        let cases = (0..=MAX_GOLDEN_CASES)
            .map(|index| GoldenCase::new(format!("case-{index}"), input("done"), "done").unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            engine.evaluate_golden_set(cases),
            Err(EvaluationError::TooManyGoldenCases)
        );
    }
}
