use crate::decision_engine::{DecisionError, SinglePassLogitEngine};
use crate::lats_engine::LatsEngine;
use crate::strategies::StrategyBudget;
use pandora_types::{OperationRequest, ParliamentDecision, PolicyContext, ToolSafetyChoice};
use std::sync::Arc;

/// Verdict produced by fast-path reflex tool safety evaluation (System 1) with potential
/// escalation to System 2 MCTS tree search.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSafetyReflexVerdict {
    pub decision: ParliamentDecision,
    pub escalated: bool,
    pub margin: f64,
    pub normalized_entropy: f64,
    pub reflex_choice: ToolSafetyChoice,
    pub mcts_action: Option<usize>,
}

impl ToolSafetyReflexVerdict {
    pub fn is_allow(&self) -> bool {
        matches!(self.decision, ParliamentDecision::Allow { .. })
    }

    pub fn is_escalated(&self) -> bool {
        self.escalated
    }

    pub fn decision(&self) -> &ParliamentDecision {
        &self.decision
    }
}

/// Categorical tool safety gate classification reporting allow, require-confirm, reject,
/// or explicit escalation to System 2 MCTS.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolSafetyGateVerdict {
    Allow {
        choice: ToolSafetyChoice,
        confidence: f64,
        margin: f64,
        normalized_entropy: f64,
    },
    Reject {
        reason: String,
        confidence: f64,
    },
    RequireConfirm {
        reason: String,
        confidence: f64,
    },
    EscalateToSystem2 {
        reason: String,
        margin: f64,
        normalized_entropy: f64,
        threshold: f64,
    },
}

impl ToolSafetyGateVerdict {
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    pub fn is_escalated(&self) -> bool {
        matches!(self, Self::EscalateToSystem2 { .. })
    }
}

#[derive(Clone, Debug)]
pub struct Parliament {
    policy_version: u32,
    decision_engine: Option<Arc<SinglePassLogitEngine>>,
    lats_engine: Option<Arc<LatsEngine>>,
    uncertainty_threshold: Option<f64>,
}

impl Parliament {
    pub const fn new(policy_version: u32) -> Self {
        Self {
            policy_version,
            decision_engine: None,
            lats_engine: None,
            uncertainty_threshold: None,
        }
    }

    pub fn with_decision_engine(mut self, decision_engine: Arc<SinglePassLogitEngine>) -> Self {
        self.uncertainty_threshold = Some(decision_engine.uncertainty_threshold());
        self.decision_engine = Some(decision_engine);
        self
    }

    pub fn with_lats_engine(mut self, lats_engine: Arc<LatsEngine>) -> Self {
        self.lats_engine = Some(lats_engine);
        self
    }

    pub fn with_uncertainty_threshold(mut self, threshold: f64) -> Self {
        if threshold.is_finite() && threshold >= 0.0 {
            self.uncertainty_threshold = Some(threshold);
        }
        self
    }

    pub fn set_decision_engine(&mut self, decision_engine: Arc<SinglePassLogitEngine>) {
        self.uncertainty_threshold = Some(decision_engine.uncertainty_threshold());
        self.decision_engine = Some(decision_engine);
    }

    pub fn set_lats_engine(&mut self, lats_engine: Arc<LatsEngine>) {
        self.lats_engine = Some(lats_engine);
    }

    pub fn set_uncertainty_threshold(&mut self, threshold: f64) {
        if threshold.is_finite() && threshold >= 0.0 {
            self.uncertainty_threshold = Some(threshold);
        }
    }

    pub const fn policy_version(&self) -> u32 {
        self.policy_version
    }

    pub fn decision_engine(&self) -> Option<&Arc<SinglePassLogitEngine>> {
        self.decision_engine.as_ref()
    }

    pub fn lats_engine(&self) -> Option<&Arc<LatsEngine>> {
        self.lats_engine.as_ref()
    }

    pub fn uncertainty_threshold(&self) -> f64 {
        self.uncertainty_threshold
            .or_else(|| {
                self.decision_engine
                    .as_ref()
                    .map(|e| e.uncertainty_threshold())
            })
            .unwrap_or(0.15)
    }

    pub fn decide(
        &self,
        request: &OperationRequest,
        context: &PolicyContext,
    ) -> ParliamentDecision {
        if context.policy_version() != self.policy_version {
            return ParliamentDecision::deny(
                request,
                context,
                "policy version does not match the active Parliament",
            );
        }
        if !context.allows(request.capability()) {
            return ParliamentDecision::deny(
                request,
                context,
                "capability is not enabled by policy",
            );
        }
        if context.requires_approval(request.operation()) {
            return ParliamentDecision::require_approval(
                request,
                context,
                "operation requires explicit approval",
            );
        }
        ParliamentDecision::allow(request, context, "capability and operation are allowed")
    }

    pub fn should_escalate(&self, verdict: &pandora_types::DynamicVerdict) -> bool {
        let engine = self
            .decision_engine
            .clone()
            .unwrap_or_else(|| Arc::new(SinglePassLogitEngine::new()));
        engine.should_escalate(verdict)
    }

    pub fn decide_with_safety_gate(
        &self,
        request: &OperationRequest,
        context: &PolicyContext,
        verdict: &pandora_types::DynamicVerdict,
    ) -> (ParliamentDecision, bool) {
        let decision = self.decide(request, context);
        let escalate = self.should_escalate(verdict);
        (decision, escalate)
    }

    /// Fast-path reflex tool safety evaluation (System 1) with automatic escalation
    /// to System 2 MCTS tree search when confidence margin < uncertainty_threshold
    /// OR normalized entropy > 0.666.
    pub fn evaluate_tool_safety_reflex(
        &self,
        request: &OperationRequest,
        context: &PolicyContext,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Result<ToolSafetyReflexVerdict, DecisionError> {
        // 1. Policy version invariant
        if context.policy_version() != self.policy_version {
            return Ok(ToolSafetyReflexVerdict {
                decision: ParliamentDecision::deny(
                    request,
                    context,
                    "policy version does not match the active Parliament",
                ),
                escalated: false,
                margin: 1.0,
                normalized_entropy: 0.0,
                reflex_choice: ToolSafetyChoice::Reject,
                mcts_action: None,
            });
        }

        // 2. Capability policy invariant
        if !context.allows(request.capability()) {
            return Ok(ToolSafetyReflexVerdict {
                decision: ParliamentDecision::deny(
                    request,
                    context,
                    "capability is not enabled by policy",
                ),
                escalated: false,
                margin: 1.0,
                normalized_entropy: 0.0,
                reflex_choice: ToolSafetyChoice::Reject,
                mcts_action: None,
            });
        }

        // 3. System 1 Reflex tool safety scoring
        let engine = self
            .decision_engine
            .clone()
            .unwrap_or_else(|| Arc::new(SinglePassLogitEngine::new()));
        let verdict = engine.evaluate_tool_safety(tool_name, tool_args)?;
        let reflex_choice = verdict.choice();

        let probs: Vec<f64> = verdict
            .distribution
            .candidates
            .iter()
            .map(|c| c.probability)
            .collect();
        let mut sorted = probs.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let margin = sorted.first().copied().unwrap_or(0.0) - sorted.get(1).copied().unwrap_or(0.0);
        let normalized_entropy =
            crate::decision_engine::TropicalDecisionDriver::normalized_entropy(&probs);
        let threshold = self
            .uncertainty_threshold
            .unwrap_or_else(|| engine.uncertainty_threshold());

        // Phase 15 Conformal Prediction Set Uncertainty Certification (Theorem 29)
        let candidate_actions: Vec<crate::conformal_calibrator::CandidateAction> = verdict
            .distribution
            .candidates
            .iter()
            .map(|c| crate::conformal_calibrator::CandidateAction {
                action_id: c.choice.as_str().to_string(),
                nonconformity_score: (1.0 - c.probability).clamp(0.0, 1.0),
            })
            .collect();
        let conformal_escalate = if !candidate_actions.is_empty() {
            let cal_scores: Vec<f64> = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.85];
            crate::conformal_calibrator::SplitConformalCalibrator::calibrate(
                &cal_scores,
                threshold.clamp(0.05, 0.50),
            )
            .and_then(|cal| cal.predict_set(&candidate_actions))
            .map(|rep| {
                matches!(
                    rep.verdict,
                    crate::conformal_calibrator::ConformalGovernanceVerdict::EscalateAmbiguous
                        | crate::conformal_calibrator::ConformalGovernanceVerdict::AbstainEmpty
                )
            })
            .unwrap_or(false)
        } else {
            false
        };

        let needs_escalation =
            crate::decision_engine::should_escalate(margin, threshold, normalized_entropy)
                || conformal_escalate;

        if needs_escalation {
            // Escalate to System 2 MCTS
            let lats = self.lats_engine.clone().unwrap_or_else(|| {
                let budget =
                    StrategyBudget::new(4, 8, 1000, 8, 10, 1000).expect("valid default budget");
                Arc::new(LatsEngine::new(budget, (*engine).clone()))
            });

            let prompt = format!("tool_safety: {} {}", tool_name, tool_args);
            let actions = [0usize, 1, 2];
            let labels = ["allow", "require_confirm", "reject"];
            let search_res = lats.search_with_tropical(&prompt, &actions, &labels);
            let best_action = search_res.ok().and_then(|r| r.best_action);

            let decision = match best_action {
                Some(0) => {
                    if context.requires_approval(request.operation()) {
                        ParliamentDecision::require_approval(
                            request,
                            context,
                            "operation requires explicit approval (escalated MCTS)",
                        )
                    } else {
                        ParliamentDecision::allow(
                            request,
                            context,
                            "escalated MCTS tree search verified tool safety",
                        )
                    }
                }
                Some(1) => ParliamentDecision::require_approval(
                    request,
                    context,
                    "escalated MCTS tree search requires explicit confirmation",
                ),
                _ => ParliamentDecision::deny(
                    request,
                    context,
                    "escalated MCTS tree search rejected unsafe tool invocation",
                ),
            };

            Ok(ToolSafetyReflexVerdict {
                decision,
                escalated: true,
                margin,
                normalized_entropy,
                reflex_choice,
                mcts_action: best_action,
            })
        } else {
            // Fast-path reflex confident decision
            let decision = match reflex_choice {
                ToolSafetyChoice::Allow => {
                    if context.requires_approval(request.operation()) {
                        ParliamentDecision::require_approval(
                            request,
                            context,
                            "operation requires explicit approval",
                        )
                    } else {
                        ParliamentDecision::allow(
                            request,
                            context,
                            "fast-path reflex determined tool is safe",
                        )
                    }
                }
                ToolSafetyChoice::RequireConfirm => ParliamentDecision::require_approval(
                    request,
                    context,
                    "fast-path reflex requires explicit confirmation",
                ),
                ToolSafetyChoice::Reject => ParliamentDecision::deny(
                    request,
                    context,
                    "fast-path reflex rejected dangerous tool invocation",
                ),
            };

            Ok(ToolSafetyReflexVerdict {
                decision,
                escalated: false,
                margin,
                normalized_entropy,
                reflex_choice,
                mcts_action: None,
            })
        }
    }

    /// Evaluates tool safety reflex and returns the final [`ParliamentDecision`].
    pub fn decide_tool_safety(
        &self,
        request: &OperationRequest,
        context: &PolicyContext,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> ParliamentDecision {
        self.evaluate_tool_safety_reflex(request, context, tool_name, tool_args)
            .map(|v| v.decision)
            .unwrap_or_else(|_| self.decide(request, context))
    }

    /// Evaluates tool safety into [`ToolSafetyGateVerdict`], reporting reflex classification
    /// or explicit System 2 escalation trigger.
    pub fn evaluate_tool_safety(
        &self,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Result<ToolSafetyGateVerdict, DecisionError> {
        let engine = self
            .decision_engine
            .clone()
            .unwrap_or_else(|| Arc::new(SinglePassLogitEngine::new()));
        let verdict = engine.evaluate_tool_safety(tool_name, tool_args)?;

        let probs: Vec<f64> = verdict
            .distribution
            .candidates
            .iter()
            .map(|c| c.probability)
            .collect();
        let mut sorted = probs.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let margin = sorted.first().copied().unwrap_or(0.0) - sorted.get(1).copied().unwrap_or(0.0);
        let normalized_entropy =
            crate::decision_engine::TropicalDecisionDriver::normalized_entropy(&probs);
        let threshold = self.uncertainty_threshold();

        if crate::decision_engine::should_escalate(margin, threshold, normalized_entropy) {
            return Ok(ToolSafetyGateVerdict::EscalateToSystem2 {
                reason: format!(
                    "fast-path reflex uncertainty: margin {margin:.4} < {threshold:.4} or entropy {normalized_entropy:.4} > 0.666"
                ),
                margin,
                normalized_entropy,
                threshold,
            });
        }

        match verdict.selected {
            ToolSafetyChoice::Allow => Ok(ToolSafetyGateVerdict::Allow {
                choice: ToolSafetyChoice::Allow,
                confidence: verdict.confidence,
                margin,
                normalized_entropy,
            }),
            ToolSafetyChoice::Reject => Ok(ToolSafetyGateVerdict::Reject {
                reason: "tool operation classified as destructive or rejected".into(),
                confidence: verdict.confidence,
            }),
            ToolSafetyChoice::RequireConfirm => Ok(ToolSafetyGateVerdict::RequireConfirm {
                reason: "tool operation mutates state and requires confirmation".into(),
                confidence: verdict.confidence,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use pandora_types::{
        Capability, EffectTarget, ExecutionId, GeneId, Operation, OperationRequest,
        ParliamentDecision, PolicyContext, PrincipalId, ResourceScope, SessionId,
    };

    use super::Parliament;

    fn request(
        capability: Capability,
        operation: Operation,
        target: EffectTarget,
    ) -> OperationRequest {
        OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("filesystem"),
            GeneId::new("test.gene").unwrap(),
            None,
            capability,
            operation,
            target,
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap()
    }

    #[test]
    fn read_only_workspace_inspection_is_allowed() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let request = request(
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("src/lib.rs"),
        );

        assert!(matches!(
            parliament.decide(&request, &context),
            ParliamentDecision::Allow { .. }
        ));
    }

    #[test]
    fn capability_not_in_policy_is_denied() {
        let parliament = Parliament::new(1);
        let request = request(
            Capability::ProviderInvoke,
            Operation::Invoke,
            EffectTarget::provider(
                "provider",
                pandora_types::SecretReference::new("credential").unwrap(),
            ),
        );

        assert!(matches!(
            parliament.decide(&request, &PolicyContext::default()),
            ParliamentDecision::Deny { .. }
        ));
    }

    #[test]
    fn writes_require_approval() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemWrite], [Operation::Write]);
        let request = request(
            Capability::FilesystemWrite,
            Operation::Write,
            EffectTarget::path("src/lib.rs"),
        );

        assert!(matches!(
            parliament.decide(&request, &context),
            ParliamentDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn process_execution_requires_approval() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::ProcessExecute], [Operation::Execute]);
        let request = request(
            Capability::ProcessExecute,
            Operation::Execute,
            EffectTarget::process("cargo"),
        );

        assert!(matches!(
            parliament.decide(&request, &context),
            ParliamentDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn test_tool_safety_fast_path_reflex_evaluation() {
        use pandora_types::ToolSafetyChoice;

        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let req = request(
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("src/lib.rs"),
        );

        // Safe inspection tool: evaluate reflex
        let verdict = parliament
            .evaluate_tool_safety_reflex(
                &req,
                &context,
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
            )
            .expect("reflex evaluation succeeded");

        assert!(verdict.is_allow());
        assert!(!verdict.is_escalated());
        assert!(verdict.margin >= 0.15);
        assert!(verdict.normalized_entropy <= 0.666);
        assert_eq!(verdict.reflex_choice, ToolSafetyChoice::Allow);
        assert!(verdict.mcts_action.is_none());

        // Also test gate classification
        let gate = parliament
            .evaluate_tool_safety("read_file", &serde_json::json!({"path": "src/lib.rs"}))
            .expect("gate evaluation succeeded");
        assert!(gate.is_allow());
        assert!(!gate.is_escalated());
    }

    #[test]
    fn test_confident_tool_execution_allow_without_escalation() {
        let parliament = Parliament::new(1);
        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let req = request(
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("Cargo.toml"),
        );

        let decision = parliament.decide_tool_safety(
            &req,
            &context,
            "inspect_file",
            &serde_json::json!({"path": "Cargo.toml"}),
        );
        assert!(matches!(decision, ParliamentDecision::Allow { .. }));

        let verdict = parliament
            .evaluate_tool_safety_reflex(
                &req,
                &context,
                "inspect_file",
                &serde_json::json!({"path": "Cargo.toml"}),
            )
            .expect("verdict succeeds");
        assert!(!verdict.escalated);
        assert!(verdict.margin > 0.5);
        assert!(verdict.normalized_entropy < 0.1);
    }

    #[test]
    fn test_escalation_triggering_when_margin_below_threshold() {
        // Direct free function contract test
        assert!(crate::decision_engine::should_escalate(0.04, 0.15, 0.20));
        assert!(crate::decision_engine::should_escalate(0.149, 0.15, 0.20));
        assert!(!crate::decision_engine::should_escalate(0.151, 0.15, 0.20));

        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let req = request(
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("src/lib.rs"),
        );

        // Ambiguous command triggers narrow margin < 0.15
        let parliament = Parliament::new(1);
        let verdict = parliament
            .evaluate_tool_safety_reflex(
                &req,
                &context,
                "eval_tool",
                &serde_json::json!({"code": "override force bypass"}),
            )
            .expect("evaluation succeeds");
        assert!(verdict.escalated);
        assert!(verdict.margin < 0.15);

        // Elevated uncertainty threshold triggers escalation even for read tool
        let parliament_strict = Parliament::new(1).with_uncertainty_threshold(0.9999);
        let verdict_strict = parliament_strict
            .evaluate_tool_safety_reflex(
                &req,
                &context,
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
            )
            .expect("evaluation succeeds");
        assert!(verdict_strict.escalated);
    }

    #[test]
    fn test_escalation_triggering_when_normalized_entropy_exceeds_0_666() {
        use crate::decision_engine::SinglePassLogitEngine;

        // Direct math check: margin 0.30 >= threshold 0.15, but entropy 0.9078 > 0.666 triggers escalation!
        assert!(crate::decision_engine::should_escalate(0.30, 0.15, 0.9078));
        assert!(!crate::decision_engine::should_escalate(0.30, 0.15, 0.665));
        assert!(crate::decision_engine::should_escalate(0.30, 0.15, 0.667));

        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let req = request(
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("src/lib.rs"),
        );

        // High temperature flattens distribution yielding high entropy > 0.666
        let engine = SinglePassLogitEngine::with_temperature(30.0).expect("valid high temperature");
        let parliament = Parliament::new(1).with_decision_engine(std::sync::Arc::new(engine));

        let verdict = parliament
            .evaluate_tool_safety_reflex(
                &req,
                &context,
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
            )
            .expect("evaluation succeeds");

        assert!(verdict.escalated);
        assert!(verdict.normalized_entropy > 0.666);
    }

    #[test]
    fn test_bounded_system2_mcts_tree_search_upon_escalation() {
        use crate::lats_engine::LatsEngine;

        let context = PolicyContext::new(1, [Capability::FilesystemRead], []);
        let req = request(
            Capability::FilesystemRead,
            Operation::Read,
            EffectTarget::path("src/lib.rs"),
        );

        let driver =
            std::sync::Arc::new(crate::decision_engine::TropicalDecisionDriver::calibrated());
        let lats = std::sync::Arc::new(LatsEngine::with_tropical_driver(driver));
        let parliament = Parliament::new(1)
            .with_lats_engine(lats)
            .with_uncertainty_threshold(0.9999);

        let verdict = parliament
            .evaluate_tool_safety_reflex(
                &req,
                &context,
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
            )
            .expect("evaluation succeeds");

        assert!(verdict.escalated);
        assert!(verdict.mcts_action.is_some());
        assert!(verdict.is_allow());
    }
}
