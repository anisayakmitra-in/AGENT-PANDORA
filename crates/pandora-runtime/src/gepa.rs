#![forbid(unsafe_code)]

use pandora_types::{DsrDemonstration, Signature};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Errors arising during Genetic-Pareto (GEPA) prompt and program optimization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GepaError {
    EmptyFrontier,
    InvalidObjective(String),
    EvaluationFailed(String),
}

impl fmt::Display for GepaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFrontier => write!(formatter, "GEPA Pareto frontier contains no candidates"),
            Self::InvalidObjective(msg) => {
                write!(formatter, "invalid GEPA Pareto objective: {msg}")
            }
            Self::EvaluationFailed(msg) => {
                write!(formatter, "GEPA candidate evaluation failed: {msg}")
            }
        }
    }
}

impl std::error::Error for GepaError {}

/// Multi-objective evaluation criteria for GEPA.
/// Tracks three orthogonal dimensions: Pass Rate, Token Cost, and Step Latency.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GepaObjective {
    /// Pass rate proportion (0.0 to 1.0, higher is better).
    pub pass_rate: f64,
    /// Total tokens consumed (lower is better).
    pub token_cost: u64,
    /// Step latency in milliseconds (lower is better).
    pub latency_ms: u64,
}

impl GepaObjective {
    pub fn new(pass_rate: f64, token_cost: u64, latency_ms: u64) -> Result<Self, GepaError> {
        if !(0.0..=1.0).contains(&pass_rate) || pass_rate.is_nan() {
            return Err(GepaError::InvalidObjective(format!(
                "pass_rate must be between 0.0 and 1.0; received {pass_rate}"
            )));
        }
        Ok(Self {
            pass_rate,
            token_cost,
            latency_ms,
        })
    }

    /// Evaluates Pareto dominance.
    /// Returns true if `self` dominates `other` (A ≻ B):
    /// No worse in all dimensions, and strictly better in at least one.
    pub fn dominates(&self, other: &Self) -> bool {
        let no_worse = self.pass_rate >= other.pass_rate
            && self.token_cost <= other.token_cost
            && self.latency_ms <= other.latency_ms;

        let strictly_better = self.pass_rate > other.pass_rate
            || self.token_cost < other.token_cost
            || self.latency_ms < other.latency_ms;

        no_worse && strictly_better
    }
}

/// An evaluated candidate in the GEPA evolutionary population.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GepaCandidate<T> {
    pub id: String,
    pub candidate: T,
    pub objective: GepaObjective,
    pub reflection: Option<String>,
    pub parent_ids: Vec<String>,
    pub generation: u32,
}

/// The non-dominated Pareto Frontier maintaining the optimal trade-off surface.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GepaParetoFrontier<T> {
    pub candidates: Vec<GepaCandidate<T>>,
}

impl<T: Clone> GepaParetoFrontier<T> {
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
        }
    }

    /// Attempts to insert a new candidate into the Pareto frontier.
    /// - If dominated by any existing frontier candidate, it is rejected (returns false).
    /// - If non-dominated, it is inserted, and any existing candidates dominated by the newcomer are pruned (returns true).
    pub fn try_insert(&mut self, candidate: GepaCandidate<T>) -> bool {
        // If any current candidate dominates the newcomer, reject
        if self
            .candidates
            .iter()
            .any(|c| c.objective.dominates(&candidate.objective))
        {
            return false;
        }

        // Prune all candidates dominated by the newcomer
        self.candidates
            .retain(|c| !candidate.objective.dominates(&c.objective));

        // Insert non-dominated candidate
        self.candidates.push(candidate);
        true
    }

    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Retrieves the candidate with the highest pass rate on the frontier.
    pub fn best_by_pass_rate(&self) -> Option<&GepaCandidate<T>> {
        self.candidates
            .iter()
            .max_by(|a, b| a.objective.pass_rate.total_cmp(&b.objective.pass_rate))
    }

    /// Retrieves the candidate with the lowest token cost that satisfies a minimum pass rate.
    pub fn cheapest_at_target_pass_rate(&self, min_pass_rate: f64) -> Option<&GepaCandidate<T>> {
        self.candidates
            .iter()
            .filter(|c| c.objective.pass_rate >= min_pass_rate)
            .min_by_key(|c| c.objective.token_cost)
    }

    /// Retrieves the candidate with the lowest latency that satisfies a minimum pass rate.
    pub fn lowest_latency_at_target(&self, min_pass_rate: f64) -> Option<&GepaCandidate<T>> {
        self.candidates
            .iter()
            .filter(|c| c.objective.pass_rate >= min_pass_rate)
            .min_by_key(|c| c.objective.latency_ms)
    }
}

/// Execution trace with rich text diagnostic feedback for reflective mutation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GepaFeedbackTrace {
    pub task_id: String,
    pub input_summary: String,
    pub failed_output: String,
    pub diagnostic: String,
    pub root_cause: String,
}

/// Reflective mutation engine for Declarative Signatures (DSRs) using natural language traces.
pub struct GepaReflectiveMutator;

impl GepaReflectiveMutator {
    /// Mutates a signature's system instructions by synthesizing natural language
    /// avoidance constraints extracted from failed execution traces.
    pub fn mutate_signature_instructions(
        base: &Signature,
        traces: &[GepaFeedbackTrace],
    ) -> Signature {
        let mut mutated = base.clone();
        if traces.is_empty() {
            return mutated;
        }

        let mut constraint_append = String::from(
            "\n\n### Critical Operational Invariants (Derived from Execution Reflection):\n",
        );
        for (i, trace) in traces.iter().enumerate() {
            constraint_append.push_str(&format!(
                "{}. On task '{}' (input: {}): avoid '{}'. Root cause: {}.\n",
                i + 1,
                trace.task_id,
                trace.input_summary,
                trace.diagnostic,
                trace.root_cause
            ));
        }

        mutated.instructions.push_str(&constraint_append);
        mutated
    }

    /// Augments a signature with high-value few-shot demonstrations.
    pub fn augment_demonstrations(
        demonstrations: &mut Vec<DsrDemonstration>,
        new_demo: DsrDemonstration,
        max_demos: usize,
    ) {
        if !demonstrations.contains(&new_demo) {
            demonstrations.push(new_demo);
            if demonstrations.len() > max_demos {
                demonstrations.remove(0);
            }
        }
    }
}

/// Genetic-Pareto (GEPA) Optimizer for Declarative Structured Reasoning.
pub struct GepaOptimizer {
    pub frontier: GepaParetoFrontier<Signature>,
    pub generation: u32,
}

impl GepaOptimizer {
    pub fn new(baseline: Signature, baseline_objective: GepaObjective) -> Self {
        let mut frontier = GepaParetoFrontier::new();
        frontier.try_insert(GepaCandidate {
            id: "cand-gen0-base".to_owned(),
            candidate: baseline,
            objective: baseline_objective,
            reflection: None,
            parent_ids: Vec::new(),
            generation: 0,
        });
        Self {
            frontier,
            generation: 0,
        }
    }

    /// Executes one round of reflective mutation and Pareto evaluation.
    pub fn step_reflective<F>(
        &mut self,
        traces: &[GepaFeedbackTrace],
        eval_fn: F,
    ) -> Result<Option<GepaCandidate<Signature>>, GepaError>
    where
        F: FnOnce(&Signature) -> Result<GepaObjective, GepaError>,
    {
        let best_parent = self
            .frontier
            .best_by_pass_rate()
            .ok_or(GepaError::EmptyFrontier)?
            .clone();

        self.generation += 1;
        let mutated_sig =
            GepaReflectiveMutator::mutate_signature_instructions(&best_parent.candidate, traces);

        let objective = eval_fn(&mutated_sig)?;
        let candidate_id = format!("cand-gen{}-{}", self.generation, self.frontier.len() + 1);

        let candidate = GepaCandidate {
            id: candidate_id,
            candidate: mutated_sig,
            objective,
            reflection: Some(format!(
                "Reflected across {} execution failures from generation {}",
                traces.len(),
                best_parent.generation
            )),
            parent_ids: vec![best_parent.id],
            generation: self.generation,
        };

        if self.frontier.try_insert(candidate.clone()) {
            Ok(Some(candidate))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gepa_objective_dominance_contract() {
        let superior = GepaObjective::new(1.0, 100, 50).unwrap();
        let inferior = GepaObjective::new(0.9, 120, 60).unwrap();
        let trade_off = GepaObjective::new(1.0, 150, 40).unwrap();

        assert!(superior.dominates(&inferior));
        assert!(!inferior.dominates(&superior));
        assert!(!superior.dominates(&trade_off));
        assert!(!trade_off.dominates(&superior));
    }

    #[test]
    fn gepa_frontier_insertion_and_pruning() {
        let mut frontier = GepaParetoFrontier::new();

        let cand1 = GepaCandidate {
            id: "c1".to_owned(),
            candidate: "prompt-base".to_owned(),
            objective: GepaObjective::new(0.8, 200, 100).unwrap(),
            reflection: None,
            parent_ids: Vec::new(),
            generation: 0,
        };
        assert!(frontier.try_insert(cand1));
        assert_eq!(frontier.len(), 1);

        // cand2 dominates cand1 (0.9 > 0.8, 150 < 200, 90 < 100)
        let cand2 = GepaCandidate {
            id: "c2".to_owned(),
            candidate: "prompt-improved".to_owned(),
            objective: GepaObjective::new(0.9, 150, 90).unwrap(),
            reflection: None,
            parent_ids: vec!["c1".to_owned()],
            generation: 1,
        };
        assert!(frontier.try_insert(cand2));
        assert_eq!(frontier.len(), 1); // c1 was pruned!
        assert_eq!(frontier.candidates[0].id, "c2");

        // cand3 is a trade-off: pass 1.0, but tokens 250, latency 80
        let cand3 = GepaCandidate {
            id: "c3".to_owned(),
            candidate: "prompt-accurate-heavy".to_owned(),
            objective: GepaObjective::new(1.0, 250, 80).unwrap(),
            reflection: None,
            parent_ids: vec!["c2".to_owned()],
            generation: 2,
        };
        assert!(frontier.try_insert(cand3));
        assert_eq!(frontier.len(), 2); // Both c2 and c3 are non-dominated

        // Test queries
        assert_eq!(frontier.best_by_pass_rate().unwrap().id, "c3");
        assert_eq!(
            frontier.cheapest_at_target_pass_rate(0.85).unwrap().id,
            "c2"
        );
    }

    #[test]
    fn gepa_reflective_signature_mutation() {
        let base_sig = Signature::new("SqlGen", "Generate BigQuery SQL.")
            .unwrap()
            .with_input("query", "Natural language query")
            .unwrap()
            .with_output("sql", "Executable SQL")
            .unwrap();

        let traces = vec![GepaFeedbackTrace {
            task_id: "task-01".to_owned(),
            input_summary: "Select top customers".to_owned(),
            failed_output: "SELECT * FROM customers LIMIT 10".to_owned(),
            diagnostic: "Table requires project qualification".to_owned(),
            root_cause: "Missing dataset prefix".to_owned(),
        }];

        let mutated = GepaReflectiveMutator::mutate_signature_instructions(&base_sig, &traces);
        assert!(
            mutated
                .instructions
                .contains("Critical Operational Invariants")
        );
        assert!(mutated.instructions.contains("Missing dataset prefix"));
    }
}
