#![forbid(unsafe_code)]

//! Subsystem 1: Cognitive Micro-Kernel & Neuro-Symbolic Inference Engine
//!
//! Unifies fast-path System 1 reflex decision making, System 2 LATS tree search,
//! BNF pushdown automata token masking, speculative tree decoding, and introspective
//! metacognitive entropy supervision into a coherent neuro-symbolic pipeline.

use crate::grammar_mask::{GrammarKind, GrammarMaskEngine, GrammarSynthesisReport};
use crate::metacognition::{MetacognitiveAuditReport, MetacognitiveSupervisor};
use crate::speculative_tree::{SpeculativeAuditReport, SpeculativeTreeEngine, SpeculativeTreeNode};
use serde::{Deserialize, Serialize};

/// Status report summarizing Cognitive Subsystem operational readiness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveSubsystemStatus {
    pub status: String,
    pub grammars_supported: Vec<String>,
    pub speculative_default_speedup: f64,
    pub metacognitive_pac_bayes_risk: f64,
    pub theorems_enforced: Vec<usize>,
}

/// Unified facade for the Cognitive Subsystem.
pub struct CognitiveSubsystem {
    pub grammar_engine: GrammarMaskEngine,
    pub speculative_engine: SpeculativeTreeEngine,
    pub supervisor: MetacognitiveSupervisor,
}

impl Default for CognitiveSubsystem {
    fn default() -> Self {
        Self {
            grammar_engine: GrammarMaskEngine::new(GrammarKind::Json),
            speculative_engine: SpeculativeTreeEngine::default(),
            supervisor: MetacognitiveSupervisor::new(),
        }
    }
}

impl CognitiveSubsystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates speculative tree drafting with rejection sampling.
    pub fn evaluate_speculative(
        &self,
        draft: &[f64],
        target: &[f64],
    ) -> (Vec<SpeculativeTreeNode>, SpeculativeAuditReport) {
        self.speculative_engine.evaluate_tree(draft, target)
    }

    /// Synthesizes guided tokens constrained by BNF grammar rules.
    pub fn synthesize_guided(&mut self, tokens: &[&str]) -> Result<GrammarSynthesisReport, String> {
        self.grammar_engine.synthesize_guided(tokens)
    }

    /// Audits an execution step with Shannon entropy and semantic drift.
    pub fn audit_step(&mut self, logits: &[f64], drift: f64) -> MetacognitiveAuditReport {
        let _ = self.supervisor.audit_step(logits, drift);
        self.supervisor.generate_report()
    }

    /// Access the Pearlian causal do-calculus inference engine (Theorems 116–118).
    pub fn causal_engine(&self) -> crate::causal_engine::PearlianCausalEngine {
        crate::causal_engine::PearlianCausalEngine::new()
    }

    /// Access the game-theoretic mechanism design engine (Theorems 125–127).
    pub fn game_mechanism(&self) -> crate::game_mechanism::GameMechanismEngine {
        crate::game_mechanism::GameMechanismEngine::new(4)
    }

    /// Access the mean-field stochastic differential game engine (Theorems 134–136).
    pub fn mean_field_control(&self) -> crate::mean_field_control::MeanFieldControlEngine {
        crate::mean_field_control::MeanFieldControlEngine::new(100)
    }

    /// Access the neural jump-diffusion stochastic optimal control engine (Theorems 143–145).
    pub fn jump_diffusion(&self) -> crate::jump_diffusion::JumpDiffusionEngine {
        crate::jump_diffusion::JumpDiffusionEngine::default()
    }

    /// Returns high-level operational status of the Cognitive Subsystem.
    pub fn status(&mut self) -> CognitiveSubsystemStatus {
        let draft = vec![0.8, 0.7, 0.6, 0.9];
        let target = vec![0.85, 0.75, 0.65, 0.95];
        let (_, spec_rep) = self.evaluate_speculative(&draft, &target);
        let meta_rep = self.audit_step(&[8.0, 2.0, 1.0], 0.05);

        CognitiveSubsystemStatus {
            status: "ready".to_string(),
            grammars_supported: vec![
                "json".to_string(),
                "sql".to_string(),
                "rust".to_string(),
                "python".to_string(),
            ],
            speculative_default_speedup: spec_rep.speedup_factor,
            metacognitive_pac_bayes_risk: meta_rep.theorem_82_pac_bayes_risk,
            theorems_enforced: vec![
                1, 2, 3, 9, 10, 68, 69, 70, 80, 81, 82, 89, 90, 91, 116, 117, 118, 125, 126, 127,
                134, 135, 136, 143, 144, 145,
            ],
        }
    }
}
