use pandora_types::{
    IterationOutcome, LoopDecision, LoopTermination, OrchestrationContractError, PlanId,
    RunLoopConfig, RunLoopId, RunLoopSnapshot, RunLoopState,
};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunLoopError {
    Contract(OrchestrationContractError),
    InvalidTransition {
        state: RunLoopState,
        action: &'static str,
    },
    InvalidSnapshot,
}

impl fmt::Display for RunLoopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::InvalidTransition { state, action } => {
                write!(formatter, "cannot {action} a run loop in {state:?} state")
            }
            Self::InvalidSnapshot => formatter.write_str("run-loop snapshot is invalid"),
        }
    }
}

impl std::error::Error for RunLoopError {}

impl From<OrchestrationContractError> for RunLoopError {
    fn from(error: OrchestrationContractError) -> Self {
        Self::Contract(error)
    }
}

pub struct RunLoop {
    id: RunLoopId,
    plan_id: PlanId,
    config: RunLoopConfig,
    state: RunLoopState,
    iterations: u32,
    retries: u32,
    used_tokens: u64,
    used_tools: u32,
    used_duration_seconds: u64,
    used_cost_micros: u64,
}

impl RunLoop {
    pub fn new(
        id: RunLoopId,
        plan_id: PlanId,
        config: RunLoopConfig,
    ) -> Result<Self, RunLoopError> {
        if id.as_str().trim().is_empty() || plan_id.as_str().trim().is_empty() {
            return Err(RunLoopError::InvalidSnapshot);
        }
        Ok(Self {
            id,
            plan_id,
            config,
            state: RunLoopState::Ready,
            iterations: 0,
            retries: 0,
            used_tokens: 0,
            used_tools: 0,
            used_duration_seconds: 0,
            used_cost_micros: 0,
        })
    }

    pub fn start(&mut self) -> Result<(), RunLoopError> {
        self.transition(RunLoopState::Ready, RunLoopState::Running, "start")
    }

    pub fn pause(&mut self) -> Result<(), RunLoopError> {
        self.transition(RunLoopState::Running, RunLoopState::Paused, "pause")
    }

    pub fn resume(&mut self) -> Result<(), RunLoopError> {
        self.transition(RunLoopState::Paused, RunLoopState::Running, "resume")
    }

    pub fn cancel(&mut self) -> Result<(), RunLoopError> {
        match self.state {
            RunLoopState::Ready | RunLoopState::Running | RunLoopState::Paused => {
                self.state = RunLoopState::Cancelled;
                Ok(())
            }
            state => Err(RunLoopError::InvalidTransition {
                state,
                action: "cancel",
            }),
        }
    }

    pub fn record_iteration(
        &mut self,
        outcome: IterationOutcome,
    ) -> Result<LoopDecision, RunLoopError> {
        if self.state != RunLoopState::Running {
            return Err(RunLoopError::InvalidTransition {
                state: self.state,
                action: "record iteration",
            });
        }

        self.iterations = self.iterations.saturating_add(1);
        let usage = outcome.usage();
        self.used_tokens = self.used_tokens.saturating_add(usage.tokens());
        self.used_tools = self.used_tools.saturating_add(usage.tools());
        self.used_duration_seconds = self
            .used_duration_seconds
            .saturating_add(usage.duration_seconds());
        self.used_cost_micros = self.used_cost_micros.saturating_add(usage.cost_micros());

        if self.budget_exhausted() {
            self.state = RunLoopState::Exhausted;
            return Ok(LoopDecision::Exhausted);
        }
        if outcome.goal_reached() {
            self.state = RunLoopState::Completed;
            return Ok(LoopDecision::Completed);
        }
        if outcome.failed() {
            if outcome.retryable()
                && self.retries < self.config.max_retries()
                && self.iterations < self.config.max_iterations()
            {
                self.retries += 1;
                return Ok(LoopDecision::Retry);
            }
            self.state = RunLoopState::Exhausted;
            return Ok(LoopDecision::Exhausted);
        }
        if !outcome.progress() && self.config.termination() == LoopTermination::NoProgress {
            self.state = RunLoopState::Completed;
            return Ok(LoopDecision::Completed);
        }
        if self.iterations >= self.config.max_iterations() {
            self.state = RunLoopState::Exhausted;
            return Ok(LoopDecision::Exhausted);
        }
        Ok(LoopDecision::Continue)
    }

    pub fn state(&self) -> RunLoopState {
        self.state
    }

    pub fn snapshot(&self) -> RunLoopSnapshot {
        RunLoopSnapshot::new(
            self.id.clone(),
            self.plan_id.clone(),
            self.config.clone(),
            self.state,
            self.iterations,
            self.retries,
            self.used_tokens,
            self.used_tools,
            self.used_duration_seconds,
            self.used_cost_micros,
        )
    }

    pub fn from_snapshot(snapshot: RunLoopSnapshot) -> Result<Self, RunLoopError> {
        if snapshot.id().as_str().trim().is_empty() || snapshot.plan_id().as_str().trim().is_empty()
        {
            return Err(RunLoopError::InvalidSnapshot);
        }
        if snapshot.iterations() > snapshot.config().max_iterations()
            || snapshot.used_tokens() > snapshot.config().max_tokens()
            || snapshot.used_tools() > snapshot.config().max_tools()
            || snapshot.used_duration_seconds() > snapshot.config().max_duration_seconds()
            || snapshot.used_cost_micros() > snapshot.config().max_cost_micros()
            || snapshot.retries() > snapshot.config().max_retries()
        {
            return Err(RunLoopError::InvalidSnapshot);
        }
        Ok(Self {
            id: snapshot.id().clone(),
            plan_id: snapshot.plan_id().clone(),
            config: snapshot.config().clone(),
            state: snapshot.state(),
            iterations: snapshot.iterations(),
            retries: snapshot.retries(),
            used_tokens: snapshot.used_tokens(),
            used_tools: snapshot.used_tools(),
            used_duration_seconds: snapshot.used_duration_seconds(),
            used_cost_micros: snapshot.used_cost_micros(),
        })
    }

    fn transition(
        &mut self,
        expected: RunLoopState,
        next: RunLoopState,
        action: &'static str,
    ) -> Result<(), RunLoopError> {
        if self.state != expected {
            return Err(RunLoopError::InvalidTransition {
                state: self.state,
                action,
            });
        }
        self.state = next;
        Ok(())
    }

    fn budget_exhausted(&self) -> bool {
        self.used_tokens > self.config.max_tokens()
            || self.used_tools > self.config.max_tools()
            || self.used_duration_seconds > self.config.max_duration_seconds()
            || self.used_cost_micros > self.config.max_cost_micros()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{IterationOutcome, LoopDecision, LoopTermination, Usage};

    fn config() -> RunLoopConfig {
        RunLoopConfig::new(3, 100, 4, 30, 1_000, 1, LoopTermination::GoalReached).unwrap()
    }

    fn outcome(tokens: u64, progress: bool, goal_reached: bool) -> IterationOutcome {
        IterationOutcome::new(
            Usage::new(tokens, 1, 1, 10),
            progress,
            goal_reached,
            false,
            false,
        )
    }

    #[test]
    fn pause_resume_and_goal_termination_are_explicit() {
        let mut run = RunLoop::new(
            RunLoopId::new("loop-1").unwrap(),
            PlanId::new("coding").unwrap(),
            config(),
        )
        .unwrap();
        run.start().unwrap();
        run.pause().unwrap();
        assert_eq!(run.state(), RunLoopState::Paused);
        run.resume().unwrap();
        assert_eq!(
            run.record_iteration(outcome(10, true, true)).unwrap(),
            LoopDecision::Completed
        );
        assert_eq!(run.state(), RunLoopState::Completed);
    }

    #[test]
    fn budget_exhaustion_stops_before_an_unbounded_loop() {
        let mut run = RunLoop::new(
            RunLoopId::new("loop-1").unwrap(),
            PlanId::new("coding").unwrap(),
            config(),
        )
        .unwrap();
        run.start().unwrap();

        assert_eq!(
            run.record_iteration(outcome(90, true, false)).unwrap(),
            LoopDecision::Continue
        );
        assert_eq!(
            run.record_iteration(outcome(20, true, false)).unwrap(),
            LoopDecision::Exhausted
        );
        assert_eq!(run.state(), RunLoopState::Exhausted);
    }

    #[test]
    fn retry_pause_and_cancel_are_bounded() {
        let mut run = RunLoop::new(
            RunLoopId::new("loop-1").unwrap(),
            PlanId::new("coding").unwrap(),
            config(),
        )
        .unwrap();
        run.start().unwrap();
        let failed = IterationOutcome::new(Usage::new(1, 1, 1, 1), false, false, true, true);

        assert_eq!(run.record_iteration(failed).unwrap(), LoopDecision::Retry);
        run.cancel().unwrap();
        assert_eq!(run.state(), RunLoopState::Cancelled);
    }

    #[test]
    fn snapshot_restore_preserves_usage_and_state() {
        let mut run = RunLoop::new(
            RunLoopId::new("loop-1").unwrap(),
            PlanId::new("coding").unwrap(),
            config(),
        )
        .unwrap();
        run.start().unwrap();
        run.record_iteration(outcome(10, true, false)).unwrap();
        let snapshot = run.snapshot();
        let restored = RunLoop::from_snapshot(snapshot.clone()).unwrap();

        assert_eq!(restored.snapshot(), snapshot);
        assert_eq!(restored.state(), RunLoopState::Running);

        let serialized = serde_json::to_string(&snapshot).unwrap();
        let decoded: RunLoopSnapshot = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            RunLoop::from_snapshot(decoded).unwrap().snapshot(),
            snapshot
        );
    }
}
