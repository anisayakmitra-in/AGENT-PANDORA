pub mod lats;
pub mod population;
pub mod react;
pub mod reflexion;

use pandora_types::EvolutionContractError;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StrategyProfile {
    Production,
    Research,
}

impl StrategyProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Research => "research",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StrategyBudget {
    max_depth: u32,
    max_rollouts: u32,
    max_tokens: u64,
    max_tools: u32,
    max_duration_seconds: u64,
    max_cost_micros: u64,
}

impl StrategyBudget {
    pub const fn new(
        max_depth: u32,
        max_rollouts: u32,
        max_tokens: u64,
        max_tools: u32,
        max_duration_seconds: u64,
        max_cost_micros: u64,
    ) -> Result<Self, StrategyError> {
        if max_depth == 0
            || max_rollouts == 0
            || max_tokens == 0
            || max_tools == 0
            || max_duration_seconds == 0
            || max_cost_micros == 0
        {
            return Err(StrategyError::InvalidBudget);
        }
        Ok(Self {
            max_depth,
            max_rollouts,
            max_tokens,
            max_tools,
            max_duration_seconds,
            max_cost_micros,
        })
    }

    pub const fn max_depth(self) -> u32 {
        self.max_depth
    }

    pub const fn max_rollouts(self) -> u32 {
        self.max_rollouts
    }

    pub const fn max_tokens(self) -> u64 {
        self.max_tokens
    }

    pub const fn max_tools(self) -> u32 {
        self.max_tools
    }

    pub const fn max_duration_seconds(self) -> u64 {
        self.max_duration_seconds
    }

    pub const fn max_cost_micros(self) -> u64 {
        self.max_cost_micros
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StrategyError {
    DisabledInProduction,
    EmptyObservation,
    InvalidBudget,
    BudgetExceeded,
    Contract(EvolutionContractError),
}

impl fmt::Display for StrategyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisabledInProduction => {
                formatter.write_str("research strategy is disabled in production")
            }
            Self::EmptyObservation => formatter.write_str("strategy observation cannot be empty"),
            Self::InvalidBudget => formatter.write_str("strategy budget must be positive"),
            Self::BudgetExceeded => formatter.write_str("strategy budget is exhausted"),
            Self::Contract(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StrategyError {}

impl From<EvolutionContractError> for StrategyError {
    fn from(error: EvolutionContractError) -> Self {
        Self::Contract(error)
    }
}
