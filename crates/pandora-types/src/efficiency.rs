use crate::effect::Timestamp;
use crate::ids::ExecutionId;
use serde::{Deserialize, Serialize};
use std::fmt;

const MAX_LABEL_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EfficiencyObjective {
    LowestCost,
    LowestLatency,
    LowestTokenUsage,
    HighestCertainty,
}

impl EfficiencyObjective {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LowestCost => "lowest_cost",
            Self::LowestLatency => "lowest_latency",
            Self::LowestTokenUsage => "lowest_token_usage",
            Self::HighestCertainty => "highest_certainty",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EfficiencyContractError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    ControlCharacter(&'static str),
    InvalidSampleCount,
    CompletedExceedsSamples,
    CostSamplesExceedSamples,
}

impl fmt::Display for EfficiencyContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
            Self::InvalidSampleCount => {
                formatter.write_str("sample count must be greater than zero")
            }
            Self::CompletedExceedsSamples => {
                formatter.write_str("completed samples cannot exceed sample count")
            }
            Self::CostSamplesExceedSamples => {
                formatter.write_str("cost samples cannot exceed sample count")
            }
        }
    }
}

impl std::error::Error for EfficiencyContractError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EfficiencySample {
    execution_id: ExecutionId,
    task_class: String,
    target: String,
    input_tokens: u64,
    output_tokens: u64,
    cost_micros: u64,
    cost_known: bool,
    latency_ms: u64,
    completed: bool,
    recorded_at: Timestamp,
}

impl EfficiencySample {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        execution_id: ExecutionId,
        task_class: impl Into<String>,
        target: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        cost_micros: u64,
        latency_ms: u64,
        completed: bool,
        recorded_at: Timestamp,
    ) -> Result<Self, EfficiencyContractError> {
        Self::build(
            execution_id,
            task_class.into(),
            target.into(),
            input_tokens,
            output_tokens,
            cost_micros,
            true,
            latency_ms,
            completed,
            recorded_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_without_cost(
        execution_id: ExecutionId,
        task_class: impl Into<String>,
        target: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        latency_ms: u64,
        completed: bool,
        recorded_at: Timestamp,
    ) -> Result<Self, EfficiencyContractError> {
        Self::build(
            execution_id,
            task_class.into(),
            target.into(),
            input_tokens,
            output_tokens,
            0,
            false,
            latency_ms,
            completed,
            recorded_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        execution_id: ExecutionId,
        task_class: String,
        target: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_micros: u64,
        cost_known: bool,
        latency_ms: u64,
        completed: bool,
        recorded_at: Timestamp,
    ) -> Result<Self, EfficiencyContractError> {
        Ok(Self {
            execution_id,
            task_class: validate_label("task class", task_class)?,
            target: validate_label("target", target)?,
            input_tokens,
            output_tokens,
            cost_micros,
            cost_known,
            latency_ms,
            completed,
            recorded_at,
        })
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn task_class(&self) -> &str {
        &self.task_class
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub const fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    pub const fn output_tokens(&self) -> u64 {
        self.output_tokens
    }

    pub const fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    pub const fn cost_micros(&self) -> u64 {
        self.cost_micros
    }

    pub const fn cost_known(&self) -> bool {
        self.cost_known
    }

    pub const fn latency_ms(&self) -> u64 {
        self.latency_ms
    }

    pub const fn completed(&self) -> bool {
        self.completed
    }

    pub const fn recorded_at(&self) -> Timestamp {
        self.recorded_at
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EfficiencySummary {
    task_class: String,
    target: String,
    sample_count: u32,
    total_tokens: u64,
    total_cost_micros: u64,
    cost_sample_count: u32,
    total_latency_ms: u64,
    completed_samples: u32,
}

impl EfficiencySummary {
    pub fn from_totals(
        task_class: impl Into<String>,
        target: impl Into<String>,
        sample_count: u32,
        total_tokens: u64,
        total_cost_micros: u64,
        total_latency_ms: u64,
        completed_samples: u32,
    ) -> Result<Self, EfficiencyContractError> {
        Self::from_totals_with_cost_samples(
            task_class,
            target,
            sample_count,
            total_tokens,
            total_cost_micros,
            sample_count,
            total_latency_ms,
            completed_samples,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_totals_with_cost_samples(
        task_class: impl Into<String>,
        target: impl Into<String>,
        sample_count: u32,
        total_tokens: u64,
        total_cost_micros: u64,
        cost_sample_count: u32,
        total_latency_ms: u64,
        completed_samples: u32,
    ) -> Result<Self, EfficiencyContractError> {
        if sample_count == 0 {
            return Err(EfficiencyContractError::InvalidSampleCount);
        }
        if completed_samples > sample_count {
            return Err(EfficiencyContractError::CompletedExceedsSamples);
        }
        if cost_sample_count > sample_count {
            return Err(EfficiencyContractError::CostSamplesExceedSamples);
        }
        Ok(Self {
            task_class: validate_label("task class", task_class.into())?,
            target: validate_label("target", target.into())?,
            sample_count,
            total_tokens,
            total_cost_micros,
            cost_sample_count,
            total_latency_ms,
            completed_samples,
        })
    }

    pub fn task_class(&self) -> &str {
        &self.task_class
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub const fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub const fn total_tokens(&self) -> u64 {
        self.total_tokens
    }

    pub const fn total_cost_micros(&self) -> u64 {
        self.total_cost_micros
    }

    pub const fn cost_sample_count(&self) -> u32 {
        self.cost_sample_count
    }

    pub const fn has_cost_evidence(&self) -> bool {
        self.cost_sample_count > 0
    }

    pub const fn average_known_cost_micros(&self) -> Option<u64> {
        if self.cost_sample_count == 0 {
            None
        } else {
            Some(self.total_cost_micros / self.cost_sample_count as u64)
        }
    }

    pub const fn total_latency_ms(&self) -> u64 {
        self.total_latency_ms
    }

    pub const fn completed_samples(&self) -> u32 {
        self.completed_samples
    }

    pub const fn average_tokens(&self) -> u64 {
        self.total_tokens / self.sample_count as u64
    }

    pub const fn average_cost_micros(&self) -> u64 {
        self.total_cost_micros / self.sample_count as u64
    }

    pub const fn average_latency_ms(&self) -> u64 {
        self.total_latency_ms / self.sample_count as u64
    }

    pub const fn completion_bps(&self) -> u16 {
        ((self.completed_samples as u64 * 10_000) / self.sample_count as u64) as u16
    }
}

fn validate_label(field: &'static str, value: String) -> Result<String, EfficiencyContractError> {
    if value.trim().is_empty() {
        return Err(EfficiencyContractError::EmptyField(field));
    }
    if value.len() > MAX_LABEL_BYTES {
        return Err(EfficiencyContractError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(EfficiencyContractError::ControlCharacter(field));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_binds_task_class_and_measured_usage() {
        let sample = EfficiencySample::new(
            ExecutionId::new("execution-1").unwrap(),
            "coding",
            "coding-domain/workspace.read",
            120,
            40,
            25,
            180,
            true,
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

        assert_eq!(sample.task_class(), "coding");
        assert_eq!(sample.target(), "coding-domain/workspace.read");
        assert_eq!(sample.total_tokens(), 160);
        assert!(sample.completed());
    }

    #[test]
    fn summary_reports_verified_completion_rate() {
        let summary = EfficiencySummary::from_totals(
            "coding",
            "coding-domain/workspace.read",
            4,
            640,
            100,
            800,
            3,
        )
        .unwrap();

        assert_eq!(summary.sample_count(), 4);
        assert_eq!(summary.average_tokens(), 160);
        assert_eq!(summary.average_cost_micros(), 25);
        assert_eq!(summary.average_latency_ms(), 200);
        assert_eq!(summary.completion_bps(), 7_500);
    }

    #[test]
    fn missing_cost_is_explicit() {
        let sample = EfficiencySample::new_without_cost(
            ExecutionId::new("execution-2").unwrap(),
            "coding",
            "provider/model",
            20,
            10,
            180,
            true,
            Timestamp::from_unix_seconds(10),
        )
        .unwrap();

        assert!(!sample.cost_known());
        assert_eq!(sample.cost_micros(), 0);
    }
}
