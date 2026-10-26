#![forbid(unsafe_code)]

use crate::ladder::OptimizationLadderLevel;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Hyperparameters governing whole-agent generation and reasoning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenomeHyperparameters {
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    pub reasoning_budget: Option<u32>,
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

impl Default for GenomeHyperparameters {
    fn default() -> Self {
        Self {
            temperature: 0.2,
            top_p: 0.95,
            max_tokens: 4096,
            reasoning_budget: Some(2048),
            extra: BTreeMap::new(),
        }
    }
}

/// A Whole-Agent Genome as synthesized from the Darwin paradigm:
/// prompt, self-written or bounded tool bindings, hyperparameters,
/// and the model target itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentGenome {
    /// Unique identifier for this genome version.
    pub genome_id: String,
    /// Parent genome ID if derived via mutation or crossover.
    pub parent_id: Option<String>,
    /// Optimization ladder level of this genome.
    pub ladder_level: OptimizationLadderLevel,
    /// Generation number in the evolutionary archive.
    pub generation: u32,
    /// Foundation model target (e.g., "claude-3-7-sonnet", "gemini-2.5-pro", "deepseek-r1").
    pub model_target: String,
    /// The foundational system prompt / persona.
    pub system_prompt: String,
    /// Active tool IDs and bound capability scripts.
    pub tool_bindings: Vec<String>,
    /// Model hyperparameters and reasoning budgets.
    pub hyperparameters: GenomeHyperparameters,
    /// Fitness evaluation score (0.0 .. 1.0).
    pub fitness_score: f64,
    /// Empirically measured average latency in milliseconds.
    pub latency_ms: u64,
    /// Measured cost in USD or token count per 1k operations.
    pub cost_per_k_ops: f64,
}

impl AgentGenome {
    pub fn new(
        genome_id: impl Into<String>,
        model_target: impl Into<String>,
        system_prompt: impl Into<String>,
        tool_bindings: Vec<String>,
    ) -> Self {
        Self {
            genome_id: genome_id.into(),
            parent_id: None,
            ladder_level: OptimizationLadderLevel::L0Prompt,
            generation: 0,
            model_target: model_target.into(),
            system_prompt: system_prompt.into(),
            tool_bindings,
            hyperparameters: GenomeHyperparameters::default(),
            fitness_score: 0.0,
            latency_ms: 0,
            cost_per_k_ops: 0.0,
        }
    }

    pub fn mutate(
        &self,
        new_id: impl Into<String>,
        mutated_prompt: Option<String>,
        mutated_tools: Option<Vec<String>>,
        mutated_model: Option<String>,
    ) -> Self {
        Self {
            genome_id: new_id.into(),
            parent_id: Some(self.genome_id.clone()),
            ladder_level: self.ladder_level,
            generation: self.generation + 1,
            model_target: mutated_model.unwrap_or_else(|| self.model_target.clone()),
            system_prompt: mutated_prompt.unwrap_or_else(|| self.system_prompt.clone()),
            tool_bindings: mutated_tools.unwrap_or_else(|| self.tool_bindings.clone()),
            hyperparameters: self.hyperparameters.clone(),
            fitness_score: 0.0,
            latency_ms: 0,
            cost_per_k_ops: 0.0,
        }
    }
}

/// An entry in a Task-Domain Routing Card, binding a domain pattern
/// to its empirical Pareto-optimal agent genome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingCardEntry {
    /// Domain category (e.g. "systems_programming", "math_symbolic", "web_search", "code_review").
    pub domain: String,
    /// Matching regex or glob task pattern.
    pub task_pattern: String,
    /// The Pareto-winning genome for this task domain.
    pub recommended_genome: AgentGenome,
    /// Empirical confidence score or historical sample count.
    pub sample_count: usize,
    /// Human-readable justification of the selection.
    pub rationale: String,
}

/// A comprehensive Task-Domain Routing Card synthesized via sandbox evolutionary search.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutingCard {
    pub card_id: String,
    pub created_at_ms: u64,
    pub entries: Vec<RoutingCardEntry>,
}

impl RoutingCard {
    pub fn new(card_id: impl Into<String>, created_at_ms: u64) -> Self {
        Self {
            card_id: card_id.into(),
            created_at_ms,
            entries: Vec::new(),
        }
    }

    pub fn add_entry(&mut self, entry: RoutingCardEntry) {
        self.entries.push(entry);
    }

    pub fn route_for_domain(&self, domain: &str) -> Option<&RoutingCardEntry> {
        self.entries
            .iter()
            .find(|e| e.domain.eq_ignore_ascii_case(domain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_genome_mutation_tracks_lineage() {
        let parent = AgentGenome::new(
            "genome-v0",
            "claude-3-7-sonnet",
            "Solve problems systematically.",
            vec!["bash".to_string(), "read_file".to_string()],
        );
        let child = parent.mutate(
            "genome-v1",
            Some("Solve problems systematically with chain of thought.".to_string()),
            None,
            None,
        );

        assert_eq!(child.parent_id, Some("genome-v0".to_string()));
        assert_eq!(child.generation, 1);
        assert_eq!(child.model_target, "claude-3-7-sonnet");
        assert_eq!(child.tool_bindings.len(), 2);
    }

    #[test]
    fn test_routing_card_lookup() {
        let mut card = RoutingCard::new("routing-test", 1700000000);
        let genome = AgentGenome::new("code-agent", "deepseek-r1", "Write code", vec![]);
        card.add_entry(RoutingCardEntry {
            domain: "coding".to_string(),
            task_pattern: "coding.*".to_string(),
            recommended_genome: genome,
            sample_count: 50,
            rationale: "Highest pass rate with lowest token cost on code benchmarks".to_string(),
        });

        let found = card.route_for_domain("coding");
        assert!(found.is_some());
        assert_eq!(
            found.unwrap().recommended_genome.model_target,
            "deepseek-r1"
        );
    }
}
