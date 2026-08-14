use super::StrategyError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReActPhase {
    Reason,
    Act,
    Observe,
    Complete,
}

impl ReActPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reason => "reason",
            Self::Act => "act",
            Self::Observe => "observe",
            Self::Complete => "complete",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReActStep {
    phase: ReActPhase,
    observation: String,
    step: u32,
}

impl ReActStep {
    pub const fn phase(&self) -> ReActPhase {
        self.phase
    }

    pub fn observation(&self) -> &str {
        &self.observation
    }

    pub const fn step(&self) -> u32 {
        self.step
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReActStrategy {
    max_steps: u32,
}

impl ReActStrategy {
    pub const fn new(max_steps: u32) -> Result<Self, StrategyError> {
        if max_steps == 0 {
            return Err(StrategyError::InvalidBudget);
        }
        Ok(Self { max_steps })
    }

    pub const fn max_steps(self) -> u32 {
        self.max_steps
    }

    pub fn next(
        &self,
        step: u32,
        observation: impl Into<String>,
        goal_reached: bool,
    ) -> Result<ReActStep, StrategyError> {
        if step >= self.max_steps {
            return Err(StrategyError::BudgetExceeded);
        }
        let observation = observation.into().trim().to_owned();
        if observation.is_empty() {
            return Err(StrategyError::EmptyObservation);
        }
        let phase = if goal_reached {
            ReActPhase::Complete
        } else {
            match step % 3 {
                0 => ReActPhase::Reason,
                1 => ReActPhase::Act,
                _ => ReActPhase::Observe,
            }
        };
        Ok(ReActStep {
            phase,
            observation,
            step,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn react_cycles_reason_act_observe_without_executing() {
        let strategy = ReActStrategy::new(4).unwrap();

        assert_eq!(
            strategy.next(0, "task", false).unwrap().phase(),
            ReActPhase::Reason
        );
        assert_eq!(
            strategy.next(1, "task", false).unwrap().phase(),
            ReActPhase::Act
        );
        assert_eq!(
            strategy.next(2, "result", false).unwrap().phase(),
            ReActPhase::Observe
        );
        assert_eq!(
            strategy.next(3, "done", true).unwrap().phase(),
            ReActPhase::Complete
        );
    }
}
