use super::{StrategyBudget, StrategyError, StrategyProfile};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatsStrategy {
    profile: StrategyProfile,
    budget: StrategyBudget,
    seed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatsResult {
    selected_index: usize,
    rollouts: u32,
    depth: u32,
    seed: u64,
}

impl LatsResult {
    pub const fn selected_index(self) -> usize {
        self.selected_index
    }

    pub const fn rollouts(self) -> u32 {
        self.rollouts
    }

    pub const fn depth(self) -> u32 {
        self.depth
    }

    pub const fn seed(self) -> u64 {
        self.seed
    }
}

impl LatsStrategy {
    pub const fn new(profile: StrategyProfile, budget: StrategyBudget, seed: u64) -> Self {
        Self {
            profile,
            budget,
            seed,
        }
    }

    pub const fn profile(self) -> StrategyProfile {
        self.profile
    }

    pub const fn budget(self) -> StrategyBudget {
        self.budget
    }

    pub fn search(&self, candidate_scores: &[i64]) -> Result<LatsResult, StrategyError> {
        if self.profile == StrategyProfile::Production {
            return Err(StrategyError::DisabledInProduction);
        }
        if candidate_scores.is_empty() {
            return Err(StrategyError::EmptyObservation);
        }
        let rollouts = self
            .budget
            .max_rollouts()
            .min(candidate_scores.len() as u32);
        if u64::from(rollouts) > self.budget.max_tokens()
            || rollouts > self.budget.max_tools()
            || u64::from(rollouts) > self.budget.max_duration_seconds()
            || u64::from(rollouts) > self.budget.max_cost_micros()
        {
            return Err(StrategyError::BudgetExceeded);
        }
        let mut selected_index = 0;
        for index in 1..rollouts as usize {
            if candidate_scores[index] > candidate_scores[selected_index] {
                selected_index = index;
            }
        }
        Ok(LatsResult {
            selected_index,
            rollouts,
            depth: self.budget.max_depth().min(rollouts),
            seed: self.seed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> StrategyBudget {
        StrategyBudget::new(4, 3, 3, 3, 3, 3).unwrap()
    }

    #[test]
    fn lats_is_disabled_in_production() {
        let strategy = LatsStrategy::new(StrategyProfile::Production, budget(), 7);

        assert_eq!(
            strategy.search(&[1, 2, 3]),
            Err(StrategyError::DisabledInProduction)
        );
    }

    #[test]
    fn lats_replays_deterministically_with_hard_budgets() {
        let strategy = LatsStrategy::new(StrategyProfile::Research, budget(), 7);
        let first = strategy.search(&[10, 30, 20]).unwrap();
        let second = strategy.search(&[10, 30, 20]).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.selected_index(), 1);
        assert_eq!(first.rollouts(), 3);
        assert_eq!(first.depth(), 3);
    }
}
