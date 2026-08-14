use super::StrategyError;
use pandora_types::{EvolutionContractError, ExecutionId, ReflexionArtifact, Timestamp};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReflexionStrategy {
    max_failure_signals: usize,
}

impl ReflexionStrategy {
    pub const fn new(max_failure_signals: usize) -> Result<Self, StrategyError> {
        if max_failure_signals == 0 {
            return Err(StrategyError::InvalidBudget);
        }
        Ok(Self {
            max_failure_signals,
        })
    }

    pub const fn max_failure_signals(self) -> usize {
        self.max_failure_signals
    }

    pub fn distill(
        &self,
        execution_id: ExecutionId,
        summary: impl Into<String>,
        failure_signals: Vec<String>,
        lesson: impl Into<String>,
        created_at: Timestamp,
    ) -> Result<ReflexionArtifact, StrategyError> {
        if failure_signals.len() > self.max_failure_signals {
            return Err(StrategyError::Contract(
                EvolutionContractError::TooManyFailureSignals,
            ));
        }
        ReflexionArtifact::new(execution_id, summary, failure_signals, lesson, created_at)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflexion_accepts_bounded_redacted_evidence() {
        let artifact = ReflexionStrategy::new(2)
            .unwrap()
            .distill(
                ExecutionId::new("execution-1").unwrap(),
                "verification failed",
                vec!["exit code 1".to_owned()],
                "check the allowlist before retrying",
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();

        assert_eq!(artifact.execution_id().as_str(), "execution-1");
        assert_eq!(artifact.failure_signals().len(), 1);
    }
}
