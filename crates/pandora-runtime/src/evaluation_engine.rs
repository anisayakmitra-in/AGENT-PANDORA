use pandora_types::{
    EffectOutcome, EvaluationKind, EvaluationRequest, EvaluationResult, EvaluationStatus,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const MAX_GOLDEN_CASES: usize = 256;
pub const MAX_GOLDEN_CASE_ID_BYTES: usize = 256;
pub const MAX_GOLDEN_EXPECTED_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_EVALUATION_TARGET_ID_BYTES: usize = 256;
pub const MAX_EVALUATION_TASK_BYTES: usize = 16 * 1024;
pub const MAX_HOLDOUT_CASES: usize = 256;
pub const MAX_HOLDOUT_CASE_ID_BYTES: usize = 256;
pub const MAX_HOLDOUT_OUTPUT_BYTES: usize = 64 * 1024;

/// A bounded execution result returned by a task-backed evaluation adapter.
///
/// Adapters own execution authority (for example, the governed controller or
/// agent loop). The evaluator only accepts the redacted evidence they return;
/// it never executes a target itself or treats the result as approval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationTaskResult {
    target: EvaluationTarget,
    task: String,
    request: EvaluationRequest,
}

impl EvaluationTaskResult {
    pub fn new(
        target: EvaluationTarget,
        task: impl Into<String>,
        request: EvaluationRequest,
    ) -> Result<Self, EvaluationError> {
        let task = validate_task(task.into())?;
        Ok(Self {
            target,
            task,
            request,
        })
    }

    pub fn target(&self) -> &EvaluationTarget {
        &self.target
    }

    pub fn task(&self) -> &str {
        &self.task
    }

    pub fn request(&self) -> &EvaluationRequest {
        &self.request
    }
}

/// Execution boundary for Prompt, Skill, Workflow, and WebAssembly Gene
/// evaluation cases. Implementations must route through an existing governed
/// adapter and return only redacted, bounded execution evidence.
pub trait EvaluationTaskAdapter {
    fn execute(
        &mut self,
        target: &EvaluationTarget,
        task: &str,
    ) -> Result<EvaluationTaskResult, EvaluationAdapterError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationAdapterError {
    Rejected,
    Failed,
    InvalidResult,
}

impl fmt::Display for EvaluationAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rejected => "evaluation target adapter rejected the task",
            Self::Failed => "evaluation target adapter failed the task",
            Self::InvalidResult => "evaluation target adapter returned invalid evidence",
        })
    }
}

impl std::error::Error for EvaluationAdapterError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationError {
    InvalidMarker,
    EmptyGoldenCaseId,
    GoldenCaseIdTooLong,
    GoldenExpectedOutputTooLong,
    EmptyTargetId,
    TargetIdTooLong,
    TargetIdControlCharacter,
    EmptyTask,
    TaskTooLong,
    TaskControlCharacter,
    AdapterRejected,
    AdapterFailed,
    InvalidAdapterResult,
    TooManyGoldenCases,
    DuplicateGoldenCase,
    EmptyHoldoutSet,
    HoldoutCaseIdTooLong,
    HoldoutOutputTooLong,
    TooManyHoldoutCases,
    DuplicateHoldoutCase,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EvaluationTargetKind {
    Prompt,
    Skill,
    Workflow,
    WasmGene,
}

impl EvaluationTargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Skill => "skill",
            Self::Workflow => "workflow",
            Self::WasmGene => "wasm_gene",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationTarget {
    kind: EvaluationTargetKind,
    id: String,
}

impl EvaluationTarget {
    pub fn new(kind: EvaluationTargetKind, id: impl Into<String>) -> Result<Self, EvaluationError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(EvaluationError::EmptyTargetId);
        }
        if id.len() > MAX_EVALUATION_TARGET_ID_BYTES {
            return Err(EvaluationError::TargetIdTooLong);
        }
        if id.chars().any(char::is_control) {
            return Err(EvaluationError::TargetIdControlCharacter);
        }
        Ok(Self {
            kind,
            id: id.trim().to_owned(),
        })
    }

    pub const fn kind(&self) -> EvaluationTargetKind {
        self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

/// A golden case whose output is produced by a governed target adapter at run
/// time rather than supplied as pre-recorded evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskBackedCase {
    id: String,
    target: EvaluationTarget,
    task: String,
    expected_output: String,
}

impl TaskBackedCase {
    pub fn new(
        id: impl Into<String>,
        target: EvaluationTarget,
        task: impl Into<String>,
        expected_output: impl Into<String>,
    ) -> Result<Self, EvaluationError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(EvaluationError::EmptyGoldenCaseId);
        }
        if id.len() > MAX_GOLDEN_CASE_ID_BYTES {
            return Err(EvaluationError::GoldenCaseIdTooLong);
        }
        let task = validate_task(task.into())?;
        let expected_output = expected_output.into();
        if expected_output.len() > MAX_GOLDEN_EXPECTED_OUTPUT_BYTES {
            return Err(EvaluationError::GoldenExpectedOutputTooLong);
        }
        Ok(Self {
            id: id.trim().to_owned(),
            target,
            task,
            expected_output,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn target(&self) -> &EvaluationTarget {
        &self.target
    }

    pub fn task(&self) -> &str {
        &self.task
    }

    pub fn expected_output(&self) -> &str {
        &self.expected_output
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoldenCase {
    id: String,
    evaluation: EvaluationRequest,
    expected_output: String,
    target: Option<EvaluationTarget>,
    task: Option<String>,
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
            target: None,
            task: None,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn evaluation(&self) -> &EvaluationRequest {
        &self.evaluation
    }

    pub fn with_target(mut self, target: EvaluationTarget) -> Self {
        self.target = Some(target);
        self
    }

    pub fn target(&self) -> Option<&EvaluationTarget> {
        self.target.as_ref()
    }

    pub fn with_task(mut self, task: impl Into<String>) -> Result<Self, EvaluationError> {
        let task = task.into();
        if task.trim().is_empty() {
            return Err(EvaluationError::EmptyTask);
        }
        if task.len() > MAX_EVALUATION_TASK_BYTES {
            return Err(EvaluationError::TaskTooLong);
        }
        if task.chars().any(char::is_control) {
            return Err(EvaluationError::TaskControlCharacter);
        }
        self.task = Some(task.trim().to_owned());
        Ok(self)
    }

    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutCase {
    id: String,
    evaluation: EvaluationRequest,
    expected_output: String,
    baseline_output: String,
}

impl HoldoutCase {
    pub fn new(
        id: impl Into<String>,
        evaluation: EvaluationRequest,
        expected_output: impl Into<String>,
        baseline_output: impl Into<String>,
    ) -> Result<Self, EvaluationError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(EvaluationError::EmptyGoldenCaseId);
        }
        if id.len() > MAX_HOLDOUT_CASE_ID_BYTES {
            return Err(EvaluationError::HoldoutCaseIdTooLong);
        }
        let expected_output = expected_output.into();
        let baseline_output = baseline_output.into();
        if expected_output.len() > MAX_HOLDOUT_OUTPUT_BYTES
            || baseline_output.len() > MAX_HOLDOUT_OUTPUT_BYTES
        {
            return Err(EvaluationError::HoldoutOutputTooLong);
        }
        if expected_output.trim().is_empty() || baseline_output.trim().is_empty() {
            return Err(EvaluationError::HoldoutOutputTooLong);
        }
        Ok(Self {
            id,
            evaluation,
            expected_output,
            baseline_output,
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
    target: Option<EvaluationTarget>,
    task: Option<String>,
}

impl GoldenCaseResult {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn result(&self) -> &EvaluationResult {
        &self.result
    }

    pub fn target(&self) -> Option<&EvaluationTarget> {
        self.target.as_ref()
    }

    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutCaseResult {
    id: String,
    trajectory: EvaluationResult,
    outcome: EvaluationResult,
    policy: EvaluationResult,
    regression: EvaluationResult,
}

impl HoldoutCaseResult {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn trajectory(&self) -> &EvaluationResult {
        &self.trajectory
    }

    pub fn outcome(&self) -> &EvaluationResult {
        &self.outcome
    }

    pub fn policy(&self) -> &EvaluationResult {
        &self.policy
    }

    pub fn regression(&self) -> &EvaluationResult {
        &self.regression
    }

    pub const fn passed(&self) -> bool {
        self.trajectory.passed()
            && self.outcome.passed()
            && self.policy.passed()
            && self.regression.passed()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldoutSetReport {
    total: usize,
    passed: usize,
    trajectory_score: u8,
    outcome_score: u8,
    holdout_passed: bool,
    policy_passed: bool,
    regression_passed: bool,
    digest: String,
    cases: Vec<HoldoutCaseResult>,
}

impl HoldoutSetReport {
    pub const fn total(&self) -> usize {
        self.total
    }

    pub const fn passed(&self) -> usize {
        self.passed
    }

    pub const fn failed(&self) -> usize {
        self.total.saturating_sub(self.passed)
    }

    pub const fn trajectory_score(&self) -> u8 {
        self.trajectory_score
    }

    pub const fn outcome_score(&self) -> u8 {
        self.outcome_score
    }

    pub const fn holdout_passed(&self) -> bool {
        self.holdout_passed
    }

    pub const fn policy_passed(&self) -> bool {
        self.policy_passed
    }

    pub const fn regression_passed(&self) -> bool {
        self.regression_passed
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn cases(&self) -> &[HoldoutCaseResult] {
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
                target: case.target.clone(),
                task: case.task.clone(),
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

    pub fn evaluate_task_backed_set<I, A>(
        &self,
        cases: I,
        adapter: &mut A,
    ) -> Result<GoldenSetReport, EvaluationError>
    where
        I: IntoIterator<Item = TaskBackedCase>,
        A: EvaluationTaskAdapter,
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

        let mut results = Vec::with_capacity(cases.len());
        for case in cases {
            let execution =
                adapter
                    .execute(&case.target, &case.task)
                    .map_err(|error| match error {
                        EvaluationAdapterError::Rejected => EvaluationError::AdapterRejected,
                        EvaluationAdapterError::Failed => EvaluationError::AdapterFailed,
                        EvaluationAdapterError::InvalidResult => {
                            EvaluationError::InvalidAdapterResult
                        }
                    })?;
            if execution.target != case.target || execution.task != case.task {
                return Err(EvaluationError::InvalidAdapterResult);
            }
            results.push(GoldenCaseResult {
                id: case.id,
                result: self.evaluate_outcome(&execution.request, &case.expected_output),
                target: Some(execution.target),
                task: Some(execution.task),
            });
        }
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

    pub fn evaluate_holdout_set<I>(&self, cases: I) -> Result<HoldoutSetReport, EvaluationError>
    where
        I: IntoIterator<Item = HoldoutCase>,
    {
        let mut cases = cases.into_iter().collect::<Vec<_>>();
        if cases.is_empty() {
            return Err(EvaluationError::EmptyHoldoutSet);
        }
        if cases.len() > MAX_HOLDOUT_CASES {
            return Err(EvaluationError::TooManyHoldoutCases);
        }
        cases.sort_by(|left, right| left.id.cmp(&right.id));
        let mut ids = BTreeSet::new();
        for case in &cases {
            if !ids.insert(case.id.clone()) {
                return Err(EvaluationError::DuplicateHoldoutCase);
            }
        }

        let results = cases
            .iter()
            .map(|case| HoldoutCaseResult {
                id: case.id.clone(),
                trajectory: self.evaluate_trajectory(&case.evaluation, 0),
                outcome: self.evaluate_outcome(&case.evaluation, &case.expected_output),
                policy: self.evaluate_policy(&case.evaluation),
                regression: self.evaluate_regression(&case.evaluation, &case.baseline_output),
            })
            .collect::<Vec<_>>();
        let passed = results.iter().filter(|case| case.passed()).count();
        let trajectory_score = average_score(&results, |case| case.trajectory.score());
        let outcome_score = average_score(&results, |case| case.outcome.score());
        let policy_passed = results.iter().all(|case| case.policy.passed());
        let regression_passed = results.iter().all(|case| case.regression.passed());
        let holdout_passed = results.iter().all(HoldoutCaseResult::passed);
        let digest = holdout_set_digest(&results);

        Ok(HoldoutSetReport {
            total: results.len(),
            passed,
            trajectory_score,
            outcome_score,
            holdout_passed,
            policy_passed,
            regression_passed,
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
    hasher.update(b"pandora.golden-set.v2");
    for case in cases {
        digest_text(&mut hasher, &case.id);
        if let Some(target) = case.target() {
            digest_text(&mut hasher, target.kind().as_str());
            digest_text(&mut hasher, target.id());
        } else {
            digest_text(&mut hasher, "legacy");
        }
        if let Some(task) = &case.task {
            digest_text(&mut hasher, task);
        } else {
            digest_text(&mut hasher, "no-task");
        }
        digest_text(&mut hasher, case.result.kind().as_str());
        digest_text(&mut hasher, case.result.status().as_str());
        hasher.update([case.result.score(), u8::from(case.result.advisory())]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn holdout_set_digest(cases: &[HoldoutCaseResult]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pandora.holdout-set.v1");
    for case in cases {
        digest_text(&mut hasher, &case.id);
        for result in [
            &case.trajectory,
            &case.outcome,
            &case.policy,
            &case.regression,
        ] {
            digest_text(&mut hasher, result.kind().as_str());
            digest_text(&mut hasher, result.status().as_str());
            hasher.update([result.score(), u8::from(result.advisory())]);
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn average_score(cases: &[HoldoutCaseResult], score: impl Fn(&HoldoutCaseResult) -> u8) -> u8 {
    let total = cases.iter().map(|case| u32::from(score(case))).sum::<u32>();
    u8::try_from(total / cases.len() as u32).unwrap_or(0)
}

fn validate_task(value: String) -> Result<String, EvaluationError> {
    if value.trim().is_empty() {
        return Err(EvaluationError::EmptyTask);
    }
    if value.len() > MAX_EVALUATION_TASK_BYTES {
        return Err(EvaluationError::TaskTooLong);
    }
    if value.chars().any(char::is_control) {
        return Err(EvaluationError::TaskControlCharacter);
    }
    Ok(value.trim().to_owned())
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
    fn targeted_golden_cases_are_typed_and_digest_bound() {
        let engine = EvaluationEngine::new();
        let prompt = EvaluationTarget::new(EvaluationTargetKind::Prompt, "prompt-1").unwrap();
        let workflow = EvaluationTarget::new(EvaluationTargetKind::Workflow, "workflow-1").unwrap();
        let first_case = GoldenCase::new("case-a", input("done"), "done")
            .unwrap()
            .with_target(prompt);
        let second_case = GoldenCase::new("case-a", input("done"), "done")
            .unwrap()
            .with_target(workflow);

        let first = engine.evaluate_golden_set([first_case]).unwrap();
        let second = engine.evaluate_golden_set([second_case]).unwrap();

        assert_eq!(
            first.cases()[0].target().unwrap().kind(),
            EvaluationTargetKind::Prompt
        );
        assert_eq!(first.cases()[0].target().unwrap().id(), "prompt-1");
        assert_ne!(first.digest(), second.digest());
    }

    struct RecordingAdapter {
        calls: Vec<(EvaluationTarget, String)>,
    }

    impl EvaluationTaskAdapter for RecordingAdapter {
        fn execute(
            &mut self,
            target: &EvaluationTarget,
            task: &str,
        ) -> Result<EvaluationTaskResult, EvaluationAdapterError> {
            self.calls.push((target.clone(), task.to_owned()));
            EvaluationTaskResult::new(target.clone(), task, input("done"))
                .map_err(|_| EvaluationAdapterError::InvalidResult)
        }
    }

    #[test]
    fn task_backed_cases_execute_through_the_adapter_and_bind_evidence() {
        let cases = [
            (EvaluationTargetKind::Prompt, "prompt-1"),
            (EvaluationTargetKind::Skill, "skill-1"),
            (EvaluationTargetKind::Workflow, "workflow-1"),
            (EvaluationTargetKind::WasmGene, "gene-1"),
        ]
        .into_iter()
        .map(|(kind, id)| {
            TaskBackedCase::new(
                format!("case-{}", kind.as_str()),
                EvaluationTarget::new(kind, id).unwrap(),
                format!("run {}", kind.as_str()),
                "done",
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
        let mut adapter = RecordingAdapter { calls: Vec::new() };

        let report = EvaluationEngine::new()
            .evaluate_task_backed_set(cases, &mut adapter)
            .unwrap();

        assert_eq!(report.total(), 4);
        assert_eq!(report.passed(), 4);
        assert_eq!(report.failed(), 0);
        assert_eq!(adapter.calls.len(), 4);
        assert_eq!(
            adapter
                .calls
                .iter()
                .map(|(target, _)| target.kind())
                .collect::<Vec<_>>(),
            vec![
                EvaluationTargetKind::Prompt,
                EvaluationTargetKind::Skill,
                EvaluationTargetKind::WasmGene,
                EvaluationTargetKind::Workflow,
            ]
        );
        assert!(report.cases().iter().all(|case| {
            case.target().is_some() && case.task().is_some() && case.result().passed()
        }));
    }

    struct MismatchedAdapter;

    impl EvaluationTaskAdapter for MismatchedAdapter {
        fn execute(
            &mut self,
            _target: &EvaluationTarget,
            task: &str,
        ) -> Result<EvaluationTaskResult, EvaluationAdapterError> {
            EvaluationTaskResult::new(
                EvaluationTarget::new(EvaluationTargetKind::Prompt, "wrong-target").unwrap(),
                task,
                input("done"),
            )
            .map_err(|_| EvaluationAdapterError::InvalidResult)
        }
    }

    #[test]
    fn task_backed_cases_reject_adapter_target_substitution() {
        let case = TaskBackedCase::new(
            "case-a",
            EvaluationTarget::new(EvaluationTargetKind::Workflow, "workflow-1").unwrap(),
            "run workflow",
            "done",
        )
        .unwrap();

        assert_eq!(
            EvaluationEngine::new().evaluate_task_backed_set([case], &mut MismatchedAdapter,),
            Err(EvaluationError::InvalidAdapterResult)
        );
    }

    #[test]
    fn evaluation_targets_reject_unbounded_or_controlled_ids() {
        assert_eq!(
            EvaluationTarget::new(EvaluationTargetKind::Skill, "  "),
            Err(EvaluationError::EmptyTargetId)
        );
        assert_eq!(
            EvaluationTarget::new(
                EvaluationTargetKind::Skill,
                "x".repeat(MAX_EVALUATION_TARGET_ID_BYTES + 1)
            ),
            Err(EvaluationError::TargetIdTooLong)
        );
        assert_eq!(
            EvaluationTarget::new(EvaluationTargetKind::Skill, "skill\n-1"),
            Err(EvaluationError::TargetIdControlCharacter)
        );
        assert_eq!(
            GoldenCase::new("case-a", input("done"), "done")
                .unwrap()
                .with_task("  "),
            Err(EvaluationError::EmptyTask)
        );
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

    #[test]
    fn holdout_set_combines_trajectory_outcome_policy_and_regression_evidence() {
        let engine = EvaluationEngine::new();
        let report = engine
            .evaluate_holdout_set([
                HoldoutCase::new("case-b", input("done"), "done", "done").unwrap(),
                HoldoutCase::new("case-a", input("wrong"), "done", "wrong").unwrap(),
            ])
            .unwrap();

        assert_eq!(report.total(), 2);
        assert_eq!(report.passed(), 1);
        assert_eq!(report.failed(), 1);
        assert_eq!(report.trajectory_score(), 100);
        assert_eq!(report.outcome_score(), 50);
        assert!(report.policy_passed());
        assert!(report.regression_passed());
        assert!(!report.holdout_passed());
        assert_eq!(report.cases()[0].id(), "case-a");
    }

    #[test]
    fn holdout_set_is_order_independent_and_rejects_vacuous_or_duplicate_evidence() {
        let engine = EvaluationEngine::new();
        let first = engine
            .evaluate_holdout_set([
                HoldoutCase::new("case-b", input("done"), "done", "done").unwrap(),
                HoldoutCase::new("case-a", input("done"), "done", "done").unwrap(),
            ])
            .unwrap();
        let second = engine
            .evaluate_holdout_set([
                HoldoutCase::new("case-a", input("done"), "done", "done").unwrap(),
                HoldoutCase::new("case-b", input("done"), "done", "done").unwrap(),
            ])
            .unwrap();
        assert_eq!(first.digest(), second.digest());
        assert_eq!(
            engine.evaluate_holdout_set(Vec::<HoldoutCase>::new()),
            Err(EvaluationError::EmptyHoldoutSet)
        );
        assert_eq!(
            engine.evaluate_holdout_set([
                HoldoutCase::new("same", input("done"), "done", "done").unwrap(),
                HoldoutCase::new("same", input("done"), "done", "done").unwrap(),
            ]),
            Err(EvaluationError::DuplicateHoldoutCase)
        );
    }
}
