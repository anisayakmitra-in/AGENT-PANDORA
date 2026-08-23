use crate::adaptive_engine::{AdaptationResult, AdaptiveEngine, AdaptiveError};
use crate::evaluation_engine::EvaluationEngine;
use crate::run_loop::{RunLoop, RunLoopError};
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
            adaptive: AdaptiveEngine::new(adaptation_policy),
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
            Some(self.adaptive.select(&input.adaptation, now)?)
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
