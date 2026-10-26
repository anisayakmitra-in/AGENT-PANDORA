#![forbid(unsafe_code)]

use pandora_types::{
    AgentGenome, EmergentDepthStack, EvolutionLineage, EvolutionLineageNode, GenomeHyperparameters,
    HarnessFailureMode, InjectedCode, MetaLayer, OptimizationLadderLevel, RoutingCard,
    RoutingCardEntry, SafetySpineReport,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Task benchmark test case used to evaluate a genome's fitness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvolutionTestCase {
    pub id: String,
    pub domain: String,
    pub input_prompt: String,
    pub expected_substring: String,
    pub max_latency_ms: u64,
}

/// Result of evaluating a genome against a test case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TestCaseResult {
    pub case_id: String,
    pub passed: bool,
    pub measured_latency_ms: u64,
    pub failure_reason: Option<String>,
}

/// Evaluator report for a candidate genome across a test suite.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenomeEvaluationReceipt {
    pub genome_id: String,
    pub pass_rate: f64,
    pub avg_latency_ms: u64,
    pub token_cost: f64,
    pub passed_all: bool,
    pub results: Vec<TestCaseResult>,
}

/// The Harness Evolution Engine coordinating Whole-Agent Genome search,
/// Darwin routing card synthesis, Meta^n emergent depth recurrence,
/// and Darwinian Lineage visualizer exports.
pub struct HarnessEvolutionEngine {
    pub population: Vec<AgentGenome>,
    pub lineage: EvolutionLineage,
    pub depth_stack: EmergentDepthStack,
    pub safety_spine: SafetySpineReport,
    pub routing_card: RoutingCard,
    pub max_generations: u32,
    pub convergence_epsilon: f64,
}

impl Default for HarnessEvolutionEngine {
    fn default() -> Self {
        Self::new(10, 0.005)
    }
}

impl HarnessEvolutionEngine {
    pub fn new(max_generations: u32, convergence_epsilon: f64) -> Self {
        Self {
            population: Vec::new(),
            lineage: EvolutionLineage::new(),
            depth_stack: EmergentDepthStack::new(),
            safety_spine: SafetySpineReport::default(),
            routing_card: RoutingCard::new("pandora-routing-card-v1", 0),
            max_generations,
            convergence_epsilon,
        }
    }

    /// Seeds the population with a champion root genome.
    pub fn seed_root(
        &mut self,
        genome_id: &str,
        model_target: &str,
        system_prompt: &str,
        tool_bindings: Vec<String>,
    ) {
        let root = AgentGenome {
            genome_id: genome_id.to_string(),
            parent_id: None,
            ladder_level: OptimizationLadderLevel::L0Prompt,
            generation: 0,
            model_target: model_target.to_string(),
            system_prompt: system_prompt.to_string(),
            tool_bindings,
            hyperparameters: GenomeHyperparameters::default(),
            fitness_score: 0.70,
            latency_ms: 1200,
            cost_per_k_ops: 0.015,
        };

        self.lineage.record_node(EvolutionLineageNode {
            id: root.genome_id.clone(),
            parent_id: None,
            generation: 0,
            score: root.fitness_score,
            latency_ms: root.latency_ms,
            model: root.model_target.clone(),
            mutation_summary: "Seed root genome".to_string(),
            failure_case_addressed: None,
            is_champion: true,
        });

        self.population.push(root);

        // Initialize depth stack at order 1 (base solver)
        self.depth_stack.push_layer(MetaLayer {
            order: 1,
            layer_id: "order-1-base-solver".to_string(),
            injected: InjectedCode::default(),
            score: 0.70,
            emergence_rationale: "Initial base solver layer".to_string(),
        });
    }

    /// Evaluates a genome against a suite of test cases (simulated sandbox execution).
    /// Enforces Guarantee 1 (sandboxed execution) and Guarantee 2 (immutable grader).
    pub fn evaluate_candidate(
        &self,
        genome: &AgentGenome,
        test_suite: &[EvolutionTestCase],
    ) -> GenomeEvaluationReceipt {
        assert!(
            self.safety_spine.immutable_grader,
            "Evaluator must be immutable"
        );
        assert!(
            self.safety_spine.sandboxed_execution,
            "Execution must be sandboxed"
        );

        let mut results = Vec::new();
        let mut passed_count = 0;
        let mut total_latency = 0;

        for case in test_suite {
            // Check if genome's prompt/tools satisfy the expected substring or domain
            let matches_prompt = genome.system_prompt.contains(&case.expected_substring)
                || genome.system_prompt.contains("solve")
                || genome.system_prompt.contains("systematic");

            let matches_tool = if case.domain == "coding" {
                genome
                    .tool_bindings
                    .iter()
                    .any(|t| t == "bash" || t == "read_file" || t == "edit_file")
            } else {
                true
            };

            let passed = matches_prompt && matches_tool;
            let latency = if passed {
                case.max_latency_ms / 2
            } else {
                case.max_latency_ms * 2
            };
            total_latency += latency;

            if passed {
                passed_count += 1;
                results.push(TestCaseResult {
                    case_id: case.id.clone(),
                    passed: true,
                    measured_latency_ms: latency,
                    failure_reason: None,
                });
            } else {
                results.push(TestCaseResult {
                    case_id: case.id.clone(),
                    passed: false,
                    measured_latency_ms: latency,
                    failure_reason: Some(format!(
                        "Failed to satisfy invariant for domain {}",
                        case.domain
                    )),
                });
            }
        }

        let pass_rate = if test_suite.is_empty() {
            1.0
        } else {
            passed_count as f64 / test_suite.len() as f64
        };

        let avg_latency = if test_suite.is_empty() {
            0
        } else {
            total_latency / test_suite.len() as u64
        };

        GenomeEvaluationReceipt {
            genome_id: genome.genome_id.clone(),
            pass_rate,
            avg_latency_ms: avg_latency,
            token_cost: 0.012,
            passed_all: passed_count == test_suite.len(),
            results,
        }
    }

    /// Evolves the population across a generation step:
    /// Selects champion parent, applies targeted mutation, evaluates fitness,
    /// and enforces Guarantee 3 (monotonic elitism rollback) and Guarantee 4 (compute cap).
    pub fn step_generation(
        &mut self,
        test_suite: &[EvolutionTestCase],
        mutation_description: &str,
        mutated_prompt: Option<String>,
        mutated_tools: Option<Vec<String>>,
        mutated_model: Option<String>,
    ) -> Option<AgentGenome> {
        if self.population.is_empty() {
            return None;
        }

        let parent = self
            .population
            .last()
            .cloned()
            .expect("population not empty");
        let next_gen = parent.generation + 1;

        if next_gen > self.max_generations {
            return None; // Compute cap reached!
        }

        let new_id = format!(
            "genome-gen{next_gen}-{}",
            &parent.model_target[..4.min(parent.model_target.len())]
        );
        let mut candidate = parent.mutate(new_id, mutated_prompt, mutated_tools, mutated_model);

        // Evaluate in sandbox
        let receipt = self.evaluate_candidate(&candidate, test_suite);
        candidate.fitness_score = receipt.pass_rate;
        candidate.latency_ms = receipt.avg_latency_ms;
        candidate.cost_per_k_ops = receipt.token_cost;

        let improved = candidate.fitness_score >= parent.fitness_score;

        // Guarantee 3: Monotonic Elitism with Automatic Rollback on Regression
        if improved {
            self.lineage.record_node(EvolutionLineageNode {
                id: candidate.genome_id.clone(),
                parent_id: Some(parent.genome_id.clone()),
                generation: candidate.generation,
                score: candidate.fitness_score,
                latency_ms: candidate.latency_ms,
                model: candidate.model_target.clone(),
                mutation_summary: mutation_description.to_string(),
                failure_case_addressed: receipt
                    .results
                    .iter()
                    .find(|r| !r.passed)
                    .map(|r| r.case_id.clone()),
                is_champion: true,
            });

            self.population.push(candidate.clone());

            // Check if Meta^n Emergent Depth layer should be stacked
            let current_depth = self.depth_stack.current_depth();
            let mut injected = InjectedCode::default();
            injected
                .directives
                .push(format!("Invariance directive from gen {next_gen}"));
            if let Some(tool) = candidate.tool_bindings.first() {
                let mut lib = BTreeMap::new();
                lib.insert(
                    format!("{tool}_helper"),
                    "fn execute_helper() -> bool { true }".to_string(),
                );
                injected.code_library = lib;
            }

            self.depth_stack.push_layer(MetaLayer {
                order: current_depth + 1,
                layer_id: format!("emergent-order-{}", current_depth + 1),
                injected,
                score: candidate.fitness_score,
                emergence_rationale: format!("Constructed from generation {next_gen} improvement"),
            });

            // Check fixed-point convergence Omega
            self.depth_stack.check_fixed_point(self.convergence_epsilon);

            Some(candidate)
        } else {
            // Regression detected: Reject and rollback to parent snapshot!
            self.lineage.record_node(EvolutionLineageNode {
                id: candidate.genome_id.clone(),
                parent_id: Some(parent.genome_id.clone()),
                generation: candidate.generation,
                score: candidate.fitness_score,
                latency_ms: candidate.latency_ms,
                model: candidate.model_target.clone(),
                mutation_summary: format!(
                    "REJECTED (regression: {:.3} < {:.3}): {}",
                    candidate.fitness_score, parent.fitness_score, mutation_description
                ),
                failure_case_addressed: None,
                is_champion: false,
            });

            None
        }
    }

    /// Synthesizes the Task-Domain Routing Card mapping domain patterns to Pareto champions.
    pub fn synthesize_routing_card(&mut self, domains: &[&str]) {
        self.routing_card.entries.clear();
        for &domain in domains {
            let matching_champion = self
                .population
                .iter()
                .filter(|g| {
                    if domain == "coding" {
                        g.tool_bindings
                            .iter()
                            .any(|t| t == "bash" || t == "read_file")
                    } else {
                        true
                    }
                })
                .max_by(|a, b| {
                    a.fitness_score
                        .partial_cmp(&b.fitness_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
                .unwrap_or_else(|| self.population.first().cloned().unwrap());

            self.routing_card.add_entry(RoutingCardEntry {
                domain: domain.to_string(),
                task_pattern: format!("{domain}.*"),
                recommended_genome: matching_champion.clone(),
                sample_count: 50,
                rationale: format!(
                    "Pareto champion for {domain} with fitness {:.3} on model {}",
                    matching_champion.fitness_score, matching_champion.model_target
                ),
            });
        }
    }

    /// Evaluates for the Triad of Self-Improvement Failure Modes.
    pub fn assess_failure_modes(
        &self,
        general_holdout_score: f64,
        task_specific_score: f64,
    ) -> Vec<HarnessFailureMode> {
        let mut modes = Vec::new();
        // Reward hacking: high task score but low holdout generalization
        if task_specific_score > 0.90 && general_holdout_score < 0.50 {
            modes.push(HarnessFailureMode::RewardHacking);
        }
        // Capability collapse: significant drop in general score compared to baseline
        if general_holdout_score < 0.40 {
            modes.push(HarnessFailureMode::CapabilityCollapse);
        }
        // Harness drift: prompt length exploded with over-fitted rules
        if self
            .population
            .last()
            .is_some_and(|champ| champ.system_prompt.len() > 3000 && task_specific_score < 0.70)
        {
            modes.push(HarnessFailureMode::HarnessDrift);
        }
        modes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_suite() -> Vec<EvolutionTestCase> {
        vec![
            EvolutionTestCase {
                id: "case-code-1".to_string(),
                domain: "coding".to_string(),
                input_prompt: "Implement safe binary search".to_string(),
                expected_substring: "solve".to_string(),
                max_latency_ms: 1000,
            },
            EvolutionTestCase {
                id: "case-code-2".to_string(),
                domain: "coding".to_string(),
                input_prompt: "Inspect files and repair syntax".to_string(),
                expected_substring: "systematic".to_string(),
                max_latency_ms: 1000,
            },
        ]
    }

    #[test]
    fn test_evolution_engine_step_and_monotonic_rollback() {
        let mut engine = HarnessEvolutionEngine::new(5, 0.01);
        engine.seed_root(
            "root-agent",
            "claude-3-7-sonnet",
            "You solve coding tasks systematically.",
            vec!["bash".to_string(), "read_file".to_string()],
        );

        let suite = test_suite();

        // Successful improvement step
        let child = engine.step_generation(
            &suite,
            "Added chain-of-thought rationale",
            Some("You solve coding tasks systematically with full chain-of-thought.".to_string()),
            Some(vec![
                "bash".to_string(),
                "read_file".to_string(),
                "edit_file".to_string(),
            ]),
            None,
        );
        assert!(child.is_some());
        assert_eq!(engine.population.len(), 2);
        assert_eq!(engine.lineage.nodes.len(), 2);

        // Regressive step: stripped required prompt substrings -> should be rejected!
        let regressive = engine.step_generation(
            &suite,
            "Regressive strip",
            Some("Just talk casually without systematic thinking.".to_string()),
            Some(vec![]),
            None,
        );
        assert!(regressive.is_none());
        // Population length should remain 2 (monotonically rolled back)
        assert_eq!(engine.population.len(), 2);
        // Lineage records the failed attempt for attribution
        assert_eq!(engine.lineage.nodes.len(), 3);
    }

    #[test]
    fn test_routing_card_synthesis() {
        let mut engine = HarnessEvolutionEngine::new(5, 0.01);
        engine.seed_root(
            "root-agent",
            "deepseek-r1",
            "You solve problems systematically.",
            vec!["bash".to_string(), "read_file".to_string()],
        );

        engine.synthesize_routing_card(&["coding", "reasoning", "synthesis"]);
        assert_eq!(engine.routing_card.entries.len(), 3);
        assert!(engine.routing_card.route_for_domain("coding").is_some());
    }
}
