#![forbid(unsafe_code)]

use pandora_types::{
    EfficiencyContractError, EfficiencyObjective, EfficiencySample, EfficiencySummary,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::Mutex;

pub const DEFAULT_MAX_SAMPLES_PER_TARGET: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EfficiencyError {
    InvalidCapacity,
    Contract(EfficiencyContractError),
    StoreUnavailable,
}

impl fmt::Display for EfficiencyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCapacity => {
                formatter.write_str("sample capacity must be greater than zero")
            }
            Self::Contract(error) => error.fmt(formatter),
            Self::StoreUnavailable => formatter.write_str("efficiency store is unavailable"),
        }
    }
}

impl std::error::Error for EfficiencyError {}

impl From<EfficiencyContractError> for EfficiencyError {
    fn from(error: EfficiencyContractError) -> Self {
        Self::Contract(error)
    }
}

pub struct EfficiencyEngine {
    max_samples_per_target: usize,
    samples: Mutex<BTreeMap<(String, String), VecDeque<EfficiencySample>>>,
}

impl Default for EfficiencyEngine {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_SAMPLES_PER_TARGET).expect("default capacity is valid")
    }
}

impl EfficiencyEngine {
    pub fn new(max_samples_per_target: usize) -> Result<Self, EfficiencyError> {
        if max_samples_per_target == 0 {
            return Err(EfficiencyError::InvalidCapacity);
        }
        Ok(Self {
            max_samples_per_target,
            samples: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn from_samples(
        max_samples_per_target: usize,
        samples: impl IntoIterator<Item = EfficiencySample>,
    ) -> Result<Self, EfficiencyError> {
        let engine = Self::new(max_samples_per_target)?;
        for sample in samples {
            engine.record(sample)?;
        }
        Ok(engine)
    }

    pub fn record(&self, sample: EfficiencySample) -> Result<(), EfficiencyError> {
        let key = (sample.task_class().to_owned(), sample.target().to_owned());
        let mut samples = self
            .samples
            .lock()
            .map_err(|_| EfficiencyError::StoreUnavailable)?;
        let history = samples.entry(key).or_default();
        if history.len() == self.max_samples_per_target {
            history.pop_front();
        }
        history.push_back(sample);
        Ok(())
    }

    pub fn rank(
        &self,
        task_class: &str,
        objective: EfficiencyObjective,
    ) -> Result<Vec<EfficiencySummary>, EfficiencyError> {
        let task_class = validate_task_class(task_class)?;
        let samples = self
            .samples
            .lock()
            .map_err(|_| EfficiencyError::StoreUnavailable)?;
        let mut summaries = Vec::new();

        for ((sample_task_class, target), history) in samples.iter() {
            if sample_task_class != task_class {
                continue;
            }
            let summary = summarize(sample_task_class, target, history)?;
            summaries.push(summary);
        }

        summaries.sort_by(|left, right| match objective {
            EfficiencyObjective::LowestCost => compare_known_cost(left, right)
                .then_with(|| right.completion_bps().cmp(&left.completion_bps()))
                .then_with(|| left.average_latency_ms().cmp(&right.average_latency_ms()))
                .then_with(|| left.target().cmp(right.target())),
            EfficiencyObjective::LowestLatency => left
                .average_latency_ms()
                .cmp(&right.average_latency_ms())
                .then_with(|| right.completion_bps().cmp(&left.completion_bps()))
                .then_with(|| compare_known_cost(left, right))
                .then_with(|| left.target().cmp(right.target())),
            EfficiencyObjective::LowestTokenUsage => left
                .average_tokens()
                .cmp(&right.average_tokens())
                .then_with(|| right.completion_bps().cmp(&left.completion_bps()))
                .then_with(|| compare_known_cost(left, right))
                .then_with(|| left.target().cmp(right.target())),
            EfficiencyObjective::HighestCertainty => right
                .completion_bps()
                .cmp(&left.completion_bps())
                .then_with(|| compare_known_cost(left, right))
                .then_with(|| left.average_latency_ms().cmp(&right.average_latency_ms()))
                .then_with(|| left.target().cmp(right.target())),
        });

        Ok(summaries)
    }
}

fn summarize(
    task_class: &str,
    target: &str,
    history: &VecDeque<EfficiencySample>,
) -> Result<EfficiencySummary, EfficiencyError> {
    let mut total_tokens: u64 = 0;
    let mut total_cost_micros: u64 = 0;
    let mut total_latency_ms: u64 = 0;
    let mut completed_samples = 0;
    for sample in history {
        total_tokens = total_tokens.saturating_add(sample.total_tokens());
        if sample.cost_known() {
            total_cost_micros = total_cost_micros.saturating_add(sample.cost_micros());
        }
        total_latency_ms = total_latency_ms.saturating_add(sample.latency_ms());
        if sample.completed() {
            completed_samples += 1;
        }
    }
    let cost_sample_count = history.iter().filter(|sample| sample.cost_known()).count() as u32;
    Ok(EfficiencySummary::from_totals_with_cost_samples(
        task_class,
        target,
        history.len() as u32,
        total_tokens,
        total_cost_micros,
        cost_sample_count,
        total_latency_ms,
        completed_samples,
    )?)
}

fn compare_known_cost(left: &EfficiencySummary, right: &EfficiencySummary) -> Ordering {
    match (
        left.average_known_cost_micros(),
        right.average_known_cost_micros(),
    ) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn validate_task_class(task_class: &str) -> Result<&str, EfficiencyError> {
    if task_class.trim().is_empty() {
        return Err(EfficiencyError::Contract(
            EfficiencyContractError::EmptyField("task class"),
        ));
    }
    if task_class.len() > 128 {
        return Err(EfficiencyError::Contract(
            EfficiencyContractError::FieldTooLong("task class"),
        ));
    }
    if task_class.chars().any(char::is_control) {
        return Err(EfficiencyError::Contract(
            EfficiencyContractError::ControlCharacter("task class"),
        ));
    }
    Ok(task_class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandora_types::{ExecutionId, Timestamp};

    fn sample(
        execution_id: &str,
        target: &str,
        cost_micros: u64,
        latency_ms: u64,
        completed: bool,
    ) -> EfficiencySample {
        EfficiencySample::new(
            ExecutionId::new(execution_id).unwrap(),
            "coding",
            target,
            100,
            50,
            cost_micros,
            latency_ms,
            completed,
            Timestamp::from_unix_seconds(10),
        )
        .unwrap()
    }

    #[test]
    fn ranks_cost_latency_and_certainty_as_separate_objectives() {
        let engine = EfficiencyEngine::new(8).unwrap();
        engine.record(sample("one", "fast", 40, 10, false)).unwrap();
        engine.record(sample("two", "fast", 40, 10, true)).unwrap();
        engine
            .record(sample("three", "cheap", 10, 100, true))
            .unwrap();
        engine
            .record(sample("four", "reliable", 80, 80, true))
            .unwrap();

        let cost = engine
            .rank("coding", EfficiencyObjective::LowestCost)
            .unwrap();
        let latency = engine
            .rank("coding", EfficiencyObjective::LowestLatency)
            .unwrap();
        let certainty = engine
            .rank("coding", EfficiencyObjective::HighestCertainty)
            .unwrap();

        assert_eq!(cost[0].target(), "cheap");
        assert_eq!(latency[0].target(), "fast");
        assert_eq!(certainty[0].target(), "cheap");
    }

    #[test]
    fn ranks_lowest_token_usage_without_changing_other_objectives() {
        let engine = EfficiencyEngine::new(8).unwrap();
        engine
            .record(
                EfficiencySample::new(
                    ExecutionId::new("compact-run").unwrap(),
                    "coding",
                    "compact",
                    10,
                    5,
                    80,
                    80,
                    true,
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap(),
            )
            .unwrap();
        engine
            .record(
                EfficiencySample::new(
                    ExecutionId::new("large-run").unwrap(),
                    "coding",
                    "large",
                    100,
                    50,
                    10,
                    10,
                    true,
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap(),
            )
            .unwrap();

        let ranking = engine
            .rank("coding", EfficiencyObjective::LowestTokenUsage)
            .unwrap();

        assert_eq!(ranking[0].target(), "compact");
        assert_eq!(ranking[0].average_tokens(), 15);
    }

    #[test]
    fn unknown_cost_is_not_ranked_as_zero_cost() {
        let engine = EfficiencyEngine::new(8).unwrap();
        engine
            .record(
                EfficiencySample::new_without_cost(
                    ExecutionId::new("unknown-cost").unwrap(),
                    "coding",
                    "unknown",
                    20,
                    10,
                    10,
                    true,
                    Timestamp::from_unix_seconds(10),
                )
                .unwrap(),
            )
            .unwrap();
        engine
            .record(sample("known-cost", "known", 20, 10, true))
            .unwrap();

        let ranking = engine
            .rank("coding", EfficiencyObjective::LowestCost)
            .unwrap();

        assert_eq!(ranking[0].target(), "known");
        assert!(!ranking[1].has_cost_evidence());
    }

    #[test]
    fn rolling_window_discards_oldest_evidence() {
        let engine = EfficiencyEngine::new(2).unwrap();
        engine
            .record(sample("one", "target", 10, 10, false))
            .unwrap();
        engine
            .record(sample("two", "target", 20, 20, false))
            .unwrap();
        engine
            .record(sample("three", "target", 30, 30, true))
            .unwrap();

        let summary = &engine
            .rank("coding", EfficiencyObjective::HighestCertainty)
            .unwrap()[0];
        assert_eq!(summary.sample_count(), 2);
        assert_eq!(summary.total_cost_micros(), 50);
        assert_eq!(summary.completion_bps(), 5_000);
    }

    #[test]
    fn zero_capacity_is_rejected() {
        assert!(matches!(
            EfficiencyEngine::new(0),
            Err(EfficiencyError::InvalidCapacity)
        ));
    }
}
