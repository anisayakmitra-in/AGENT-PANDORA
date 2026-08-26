use crate::adaptive_engine::{AdaptationResult, AdaptiveEngine, AdaptiveError};
use crate::evaluation_engine::EvaluationEngine;
use crate::run_loop::{RunLoop, RunLoopError};
use crate::self_healing::{SelfHealingEngine, SelfHealingError};
use crate::strategies::StrategyError;
use crate::strategies::reflexion::ReflexionStrategy;
use pandora_types::{
    AdaptationPolicy, AdaptationRequest, EvaluationContractError, EvaluationReceipt,
    EvaluationRequest, EvaluationResult, IterationOutcome, LoopDecision, ReflexionArtifact,
    RunLoopSnapshot, Timestamp, Usage,
};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodingFeedbackError {
    ExecutionMismatch,
    RunLoop(RunLoopError),
    Strategy(StrategyError),
    Adaptation(AdaptiveError),
    SelfHealing(SelfHealingError),
    Evaluation(EvaluationContractError),
}

impl fmt::Display for CodingFeedbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExecutionMismatch => {
                formatter.write_str("coding feedback evidence belongs to different executions")
            }
            Self::RunLoop(error) => error.fmt(formatter),
            Self::Strategy(error) => error.fmt(formatter),
            Self::Adaptation(error) => error.fmt(formatter),
            Self::SelfHealing(error) => error.fmt(formatter),
            Self::Evaluation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CodingFeedbackError {}

impl From<RunLoopError> for CodingFeedbackError {
    fn from(error: RunLoopError) -> Self {
        Self::RunLoop(error)
    }
}

impl From<StrategyError> for CodingFeedbackError {
    fn from(error: StrategyError) -> Self {
        Self::Strategy(error)
    }
}

impl From<AdaptiveError> for CodingFeedbackError {
    fn from(error: AdaptiveError) -> Self {
        Self::Adaptation(error)
    }
}

impl From<SelfHealingError> for CodingFeedbackError {
    fn from(error: SelfHealingError) -> Self {
        Self::SelfHealing(error)
    }
}

impl From<EvaluationContractError> for CodingFeedbackError {
    fn from(error: EvaluationContractError) -> Self {
        Self::Evaluation(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodingFeedbackInput {
    evaluation: EvaluationRequest,
    expected_output: String,
    adaptation: AdaptationRequest,
    usage: Usage,
    retryable: bool,
}

impl CodingFeedbackInput {
    pub fn new(
        evaluation: EvaluationRequest,
        expected_output: impl Into<String>,
        adaptation: AdaptationRequest,
        usage: Usage,
        retryable: bool,
    ) -> Result<Self, CodingFeedbackError> {
        if evaluation.execution_id() != adaptation.execution_id() {
            return Err(CodingFeedbackError::ExecutionMismatch);
        }
        Ok(Self {
            evaluation,
            expected_output: expected_output.into(),
            adaptation,
            usage,
            retryable,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodingFeedbackResult {
    evaluation_receipt: EvaluationReceipt,
    reflexion: Option<ReflexionArtifact>,
    adaptation: Option<AdaptationResult>,
    decision: LoopDecision,
    snapshot: RunLoopSnapshot,
}

impl CodingFeedbackResult {
    pub fn evaluations(&self) -> &[EvaluationResult] {
        self.evaluation_receipt.results()
    }

    pub fn evaluation_receipt(&self) -> &EvaluationReceipt {
        &self.evaluation_receipt
    }

    pub fn reflexion(&self) -> Option<&ReflexionArtifact> {
        self.reflexion.as_ref()
    }

    pub fn adaptation(&self) -> Option<&AdaptationResult> {
        self.adaptation.as_ref()
    }

    pub const fn decision(&self) -> LoopDecision {
        self.decision
    }

    pub fn snapshot(&self) -> &RunLoopSnapshot {
        &self.snapshot
    }
}

pub struct CodingFeedbackLoop {
    run_loop: RunLoop,
    evaluation: EvaluationEngine,
    reflexion: ReflexionStrategy,
    adaptive: AdaptiveEngine,
    healing: SelfHealingEngine,
}

impl CodingFeedbackLoop {
    pub fn new(
        run_loop: RunLoop,
        adaptation_policy: AdaptationPolicy,
        max_failure_signals: usize,
    ) -> Result<Self, CodingFeedbackError> {
        Ok(Self {
            run_loop,
            evaluation: EvaluationEngine::new(),
            reflexion: ReflexionStrategy::new(max_failure_signals)?,
            adaptive: AdaptiveEngine::new(adaptation_policy.clone()),
            healing: SelfHealingEngine::new(adaptation_policy),
        })
    }

    pub fn record_iteration(
        &mut self,
        input: CodingFeedbackInput,
        now: Timestamp,
    ) -> Result<CodingFeedbackResult, CodingFeedbackError> {
        let trajectory = self.evaluation.evaluate_trajectory(&input.evaluation, 0);
        let outcome = self
            .evaluation
            .evaluate_outcome(&input.evaluation, &input.expected_output);
        let policy = self.evaluation.evaluate_policy(&input.evaluation);
        let policy_passed = policy.passed();
        let evaluations = vec![trajectory, outcome, policy];
        let evaluation_receipt = EvaluationReceipt::new(
            input.adaptation.session_id().clone(),
            input.evaluation.execution_id().clone(),
            now,
            evaluations,
        )?;
        let passed = evaluation_receipt
            .results()
            .iter()
            .all(EvaluationResult::passed);
        let reflexion = if passed {
            None
        } else {
            let failure_signals = evaluation_receipt
                .results()
                .iter()
                .filter(|result| !result.passed())
                .take(self.reflexion.max_failure_signals())
                .map(|result| format!("{}: {}", result.kind().as_str(), result.reason()))
                .collect();
            Some(self.reflexion.distill(
                input.evaluation.execution_id().clone(),
                "coding iteration did not satisfy verification",
                failure_signals,
                "address the failed evaluation evidence and rerun verification before completion",
                now,
            )?)
        };
        let decision = self.run_loop.record_iteration(IterationOutcome::new(
            input.usage,
            passed,
            passed,
            !passed,
            !passed && policy_passed && input.retryable,
        ))?;
        let adaptation = if decision == LoopDecision::Retry {
            if self.healing.can_handle(&input.adaptation) {
                Some(self.healing.recover(&input.adaptation, now)?)
            } else {
                Some(self.adaptive.select(&input.adaptation, now)?)
            }
        } else {
            None
        };

        Ok(CodingFeedbackResult {
            evaluation_receipt,
            reflexion,
            adaptation,
            decision,
            snapshot: self.run_loop.snapshot(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{
        AdaptationCandidate, AdaptationTarget, EvaluationRequest, ExecutionId, LoopTermination,
        PlanId, RequestDigest, RunLoopConfig, RunLoopId, SessionId,
    };

    fn feedback() -> CodingFeedbackLoop {
        let mut run_loop = RunLoop::new(
            RunLoopId::new("coding-feedback-test").unwrap(),
            PlanId::new("coding-feedback-test").unwrap(),
            RunLoopConfig::new(3, 1_000, 4, 30, 1_000, 1, LoopTermination::GoalReached).unwrap(),
        )
        .unwrap();
        run_loop.start().unwrap();
        CodingFeedbackLoop::new(run_loop, AdaptationPolicy::new(1, 4, 500, 100).unwrap(), 2)
            .unwrap()
    }

    fn input(output: &str, expected: &str, retryable: bool) -> CodingFeedbackInput {
        let execution_id = ExecutionId::new("execution-feedback-test").unwrap();
        let evaluation =
            EvaluationRequest::new(execution_id.clone(), Vec::new(), output, Vec::new()).unwrap();
        let candidates = if retryable {
            vec![
                AdaptationCandidate::new(
                    "safe-retry",
                    AdaptationTarget::recovery("safe-retry").unwrap(),
                    100,
                    true,
                    false,
                    0,
                    0,
                )
                .unwrap(),
            ]
        } else {
            Vec::new()
        };
        let adaptation = AdaptationRequest::new(
            execution_id,
            SessionId::new("session-feedback-test").unwrap(),
            RequestDigest::new("request-feedback-test").unwrap(),
            None,
            candidates,
        )
        .unwrap();
        CodingFeedbackInput::new(
            evaluation,
            expected,
            adaptation,
            Usage::new(0, 0, 0, 0),
            retryable,
        )
        .unwrap()
    }

    #[test]
    fn successful_iteration_completes_without_reflexion_or_adaptation() {
        let mut feedback = feedback();
        let result = feedback
            .record_iteration(input("ok", "ok", false), Timestamp::from_unix_seconds(10))
            .unwrap();
        assert_eq!(result.decision, LoopDecision::Completed);
        assert!(result.reflexion.is_none());
        assert!(result.adaptation.is_none());
    }

    #[test]
    fn retryable_failure_creates_reflexion_and_recovery_selection() {
        let mut feedback = feedback();
        let result = feedback
            .record_iteration(input("bad", "ok", true), Timestamp::from_unix_seconds(10))
            .unwrap();
        assert_eq!(result.decision, LoopDecision::Retry);
        assert!(result.reflexion.is_some());
        assert_eq!(
            result
                .adaptation
                .as_ref()
                .and_then(|value| value.decision().selected())
                .map(AdaptationTarget::label),
            Some("safe-retry")
        );
    }
}
