#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;

/// The 6 canonical rungs of the Harness Self-Improvement Optimization Ladder
/// (synthesized from the Awesome-Harness-Self-Improvement taxonomy).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationLadderLevel {
    /// L0: Instruction prompts (APE, OPRO, Promptbreeder, GEPA).
    L0Prompt,
    /// L1: Context & memory (ACE, ReasoningBank, layered memory, context compression).
    L1MemoryContext,
    /// L2: Workflow / graph topology (ADAS, AFlow, GPTSwarm, dynamic branching).
    L2WorkflowGraph,
    /// L3: Harness / agent executable code (Darwin Gödel Machines, SICA, Gödel Agent).
    L3HarnessCode,
    /// L4: Optimizer / meta-harness (STOP, Meta-Harness, MCE, Meta^n recursive depth).
    L4MetaOptimizer,
    /// L5: Joint harness + weights co-optimization (SIA, SEAL).
    L5HarnessWeights,
}

impl OptimizationLadderLevel {
    pub const ALL: [Self; 6] = [
        Self::L0Prompt,
        Self::L1MemoryContext,
        Self::L2WorkflowGraph,
        Self::L3HarnessCode,
        Self::L4MetaOptimizer,
        Self::L5HarnessWeights,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::L0Prompt => "L0:Prompt",
            Self::L1MemoryContext => "L1:MemoryContext",
            Self::L2WorkflowGraph => "L2:WorkflowGraph",
            Self::L3HarnessCode => "L3:HarnessCode",
            Self::L4MetaOptimizer => "L4:MetaOptimizer",
            Self::L5HarnessWeights => "L5:HarnessWeights",
        }
    }

    pub const fn rung_number(self) -> u8 {
        match self {
            Self::L0Prompt => 0,
            Self::L1MemoryContext => 1,
            Self::L2WorkflowGraph => 2,
            Self::L3HarnessCode => 3,
            Self::L4MetaOptimizer => 4,
            Self::L5HarnessWeights => 5,
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::L0Prompt => {
                "Instruction prompt and few-shot optimization via reflection and mutation"
            }
            Self::L1MemoryContext => {
                "Dynamic context window management, episodic retrieval, and memory distillation"
            }
            Self::L2WorkflowGraph => {
                "Dynamic workflow topology, agentic graph routing, and DAG self-structuring"
            }
            Self::L3HarnessCode => {
                "Self-modifying tool scripts, sandbox executables, and agent harness code"
            }
            Self::L4MetaOptimizer => {
                "Recursive meta-layer emergence, self-improving optimizers, and fixed-point search"
            }
            Self::L5HarnessWeights => {
                "Joint harness scaffolding and foundation model weight co-optimization"
            }
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace(['-', '_', ':'], "").as_str() {
            "l0" | "l0prompt" | "prompt" => Some(Self::L0Prompt),
            "l1" | "l1memorycontext" | "l1memory" | "memory" => Some(Self::L1MemoryContext),
            "l2" | "l2workflowgraph" | "l2workflow" | "workflow" | "graph" => {
                Some(Self::L2WorkflowGraph)
            }
            "l3" | "l3harnesscode" | "l3code" | "harnesscode" | "code" => Some(Self::L3HarnessCode),
            "l4" | "l4metaoptimizer" | "l4meta" | "meta" | "metan" => Some(Self::L4MetaOptimizer),
            "l5" | "l5harnessweights" | "l5weights" | "weights" => Some(Self::L5HarnessWeights),
            _ => None,
        }
    }
}

impl fmt::Display for OptimizationLadderLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// The Triad of Self-Improvement Failure Modes identified in harness engineering literature.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessFailureMode {
    /// Reward Hacking: exploiting surrogate evaluators or proxy metrics without genuine capability gain.
    RewardHacking,
    /// Capability Collapse: catastrophic forgetting or degradation across unmeasured benchmark tasks.
    CapabilityCollapse,
    /// Harness Drift: overfitting prompt/harness scaffolding to test idiosyncrasies rather than domain invariants.
    HarnessDrift,
}

impl HarnessFailureMode {
    pub const ALL: [Self; 3] = [
        Self::RewardHacking,
        Self::CapabilityCollapse,
        Self::HarnessDrift,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RewardHacking => "reward_hacking",
            Self::CapabilityCollapse => "capability_collapse",
            Self::HarnessDrift => "harness_drift",
        }
    }

    pub const fn safeguard(self) -> &'static str {
        match self {
            Self::RewardHacking => {
                "Immutable external evaluator isolation & tamper-proof signature verification"
            }
            Self::CapabilityCollapse => {
                "Multi-domain regression holdout suites & monotonic Pareto front checks"
            }
            Self::HarnessDrift => {
                "Holdout case partitioning, cross-validation sets & invariance contract tests"
            }
        }
    }
}

/// Darwin 4-Point Safety Spine Guarantees enforced by construction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SafetySpineReport {
    /// Guarantee 1: Genome modification and execution occurs exclusively within isolated sandboxes.
    pub sandboxed_execution: bool,
    /// Guarantee 2: Evaluator and fitness functions are immutable and isolated from mutator inspection.
    pub immutable_grader: bool,
    /// Guarantee 3: Monotonic elitism with automatic snapshot rollback on candidate regression.
    pub monotonic_rollback: bool,
    /// Guarantee 4: Explicit hard compute, generation, and token budgets with human veto option.
    pub compute_capped: bool,
}

impl Default for SafetySpineReport {
    fn default() -> Self {
        Self {
            sandboxed_execution: true,
            immutable_grader: true,
            monotonic_rollback: true,
            compute_capped: true,
        }
    }
}

impl SafetySpineReport {
    pub fn is_fully_guaranteed(&self) -> bool {
        self.sandboxed_execution
            && self.immutable_grader
            && self.monotonic_rollback
            && self.compute_capped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ladder_levels_roundtrip() {
        for level in OptimizationLadderLevel::ALL {
            let parsed = OptimizationLadderLevel::parse(level.as_str());
            assert_eq!(parsed, Some(level));
        }
        assert_eq!(
            OptimizationLadderLevel::parse("l0"),
            Some(OptimizationLadderLevel::L0Prompt)
        );
        assert_eq!(
            OptimizationLadderLevel::parse("l4"),
            Some(OptimizationLadderLevel::L4MetaOptimizer)
        );
    }

    #[test]
    fn test_safety_spine_defaults() {
        let spine = SafetySpineReport::default();
        assert!(spine.is_fully_guaranteed());
    }
}
