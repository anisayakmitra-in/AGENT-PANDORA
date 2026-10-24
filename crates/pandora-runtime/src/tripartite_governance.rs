//! Tripartite cognitive governance (Phase 15.1).
//!
//! Enforces strict separation of concerns so self-modification cannot drift:
//! - **Generator** (Maker): formulates hypotheses, edits source, refines prompts.
//! - **Critic** (Reviewer): runs holdout suites, measures cost and latency.
//! - **Governor** (Parliament + ReferenceMonitor): verifies signatures, checks
//!   constitutional permissions, enforces permit boundaries.
//!
//! The Generator can *never* approve its own modifications.

use crate::evolution::{
    BayesianCanaryGate, BayesianCanaryVerdict, ParetoCandidate, ParetoFrontier,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TripartiteRole {
    Generator,
    Critic,
    Governor,
}

impl TripartiteRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Generator => "generator",
            Self::Critic => "critic",
            Self::Governor => "governor",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TripartiteError {
    SelfApprovalForbidden,
    RoleMismatch {
        expected: TripartiteRole,
        actual: TripartiteRole,
    },
    MissingCriticEvaluation,
    MissingGovernorApproval,
    EmptyProposal,
    ParetoDominated(String),
    CanaryCheckFailed {
        lower_bound: f64,
        baseline: f64,
    },
}

impl fmt::Display for TripartiteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelfApprovalForbidden => {
                formatter.write_str("generator cannot approve its own modification")
            }
            Self::RoleMismatch { expected, actual } => write!(
                formatter,
                "expected role {} but got {}",
                expected.as_str(),
                actual.as_str()
            ),
            Self::MissingCriticEvaluation => {
                formatter.write_str("critic evaluation is required before approval")
            }
            Self::MissingGovernorApproval => {
                formatter.write_str("governor approval is required before promotion")
            }
            Self::EmptyProposal => formatter.write_str("proposal must not be empty"),
            Self::ParetoDominated(id) => {
                write!(
                    formatter,
                    "proposal {id} is dominated on the Pareto frontier"
                )
            }
            Self::CanaryCheckFailed {
                lower_bound,
                baseline,
            } => {
                write!(
                    formatter,
                    "canary LCB95 ({lower_bound:.4}) did not exceed baseline ({baseline:.4})"
                )
            }
        }
    }
}

impl std::error::Error for TripartiteError {}

/// A modification proposal submitted by the Generator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TripartiteProposal {
    pub id: String,
    pub summary: String,
    pub proposed_by: TripartiteRole,
}

impl TripartiteProposal {
    pub fn new(id: impl Into<String>, summary: impl Into<String>) -> Result<Self, TripartiteError> {
        let summary = summary.into();
        if summary.trim().is_empty() {
            return Err(TripartiteError::EmptyProposal);
        }
        Ok(Self {
            id: id.into(),
            summary,
            proposed_by: TripartiteRole::Generator,
        })
    }
}

/// Critic's evaluation of a proposal.
#[derive(Clone, Debug, PartialEq)]
pub struct TripartiteEvaluation {
    pub proposal_id: String,
    pub pass_rate: f64,
    pub token_cost: f64,
    pub step_latency_ms: u64,
    pub evaluated_by: TripartiteRole,
    pub pareto_candidate: Option<ParetoCandidate>,
    pub canary_verdict: Option<BayesianCanaryVerdict>,
}

impl TripartiteEvaluation {
    pub fn new(
        proposal_id: impl Into<String>,
        pass_rate: f64,
        token_cost: f64,
        step_latency_ms: u64,
    ) -> Self {
        Self {
            proposal_id: proposal_id.into(),
            pass_rate,
            token_cost,
            step_latency_ms,
            evaluated_by: TripartiteRole::Critic,
            pareto_candidate: None,
            canary_verdict: None,
        }
    }

    pub fn with_pareto(mut self, candidate: ParetoCandidate) -> Self {
        self.pareto_candidate = Some(candidate);
        self
    }

    pub fn with_canary(mut self, verdict: BayesianCanaryVerdict) -> Self {
        self.canary_verdict = Some(verdict);
        self
    }

    pub fn is_pareto_accepted(&self) -> bool {
        self.pareto_candidate.is_some()
    }

    pub fn is_canary_promoted(&self) -> bool {
        self.canary_verdict.as_ref().is_none_or(|v| v.promoted)
    }
}

/// Governor's approval decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TripartiteApproval {
    pub proposal_id: String,
    pub approved: bool,
    pub reason: String,
    pub approved_by: TripartiteRole,
}

impl TripartiteApproval {
    pub fn new(proposal_id: impl Into<String>, approved: bool, reason: impl Into<String>) -> Self {
        Self {
            proposal_id: proposal_id.into(),
            approved,
            reason: reason.into(),
            approved_by: TripartiteRole::Governor,
        }
    }
}

/// The tripartite governance orchestrator.
pub struct TripartiteGovernance {
    policy_version: u32,
}

impl TripartiteGovernance {
    pub fn new(policy_version: u32) -> Self {
        Self { policy_version }
    }

    pub fn policy_version(&self) -> u32 {
        self.policy_version
    }

    /// Records that a proposal was submitted. Returns the proposal unchanged.
    pub fn submit(&self, proposal: TripartiteProposal) -> TripartiteProposal {
        proposal
    }

    /// Critic evaluates a proposal. The evaluator must be the Critic role.
    pub fn evaluate(
        &self,
        proposal: &TripartiteProposal,
        evaluator: TripartiteRole,
        pass_rate: f64,
        token_cost: f64,
        step_latency_ms: u64,
    ) -> Result<TripartiteEvaluation, TripartiteError> {
        if evaluator != TripartiteRole::Critic {
            return Err(TripartiteError::RoleMismatch {
                expected: TripartiteRole::Critic,
                actual: evaluator,
            });
        }
        Ok(TripartiteEvaluation::new(
            proposal.id.clone(),
            pass_rate,
            token_cost,
            step_latency_ms,
        ))
    }

    /// Critic evaluates a proposal using Pareto frontier tracking and optional Bayesian canary gating.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_with_verifiers(
        &self,
        proposal: &TripartiteProposal,
        evaluator: TripartiteRole,
        pass_rate: f64,
        token_cost: f64,
        step_latency_ms: u64,
        frontier: &mut ParetoFrontier,
        canary_eval: Option<(&BayesianCanaryGate, u32, u32, f64)>,
    ) -> Result<TripartiteEvaluation, TripartiteError> {
        if evaluator != TripartiteRole::Critic {
            return Err(TripartiteError::RoleMismatch {
                expected: TripartiteRole::Critic,
                actual: evaluator,
            });
        }
        let candidate =
            ParetoCandidate::new(&proposal.id, pass_rate, token_cost as u32, step_latency_ms);
        if !frontier.try_insert(candidate.clone()) {
            return Err(TripartiteError::ParetoDominated(proposal.id.clone()));
        }
        let mut eval =
            TripartiteEvaluation::new(proposal.id.clone(), pass_rate, token_cost, step_latency_ms)
                .with_pareto(candidate);

        if let Some((gate, passes, failures, baseline)) = canary_eval {
            let verdict = gate.evaluate(passes, failures, baseline);
            if !verdict.promoted {
                return Err(TripartiteError::CanaryCheckFailed {
                    lower_bound: verdict.lower_bound_95,
                    baseline: verdict.baseline_rate,
                });
            }
            eval = eval.with_canary(verdict);
        }

        Ok(eval)
    }

    /// Governor approves or rejects. The approver must be the Governor role,
    /// and the proposal's submitter (Generator) can never be the approver.
    pub fn approve(
        &self,
        proposal: &TripartiteProposal,
        approver: TripartiteRole,
        evaluation: Option<&TripartiteEvaluation>,
        approved: bool,
        reason: impl Into<String>,
    ) -> Result<TripartiteApproval, TripartiteError> {
        if approver != TripartiteRole::Governor {
            return Err(TripartiteError::RoleMismatch {
                expected: TripartiteRole::Governor,
                actual: approver,
            });
        }
        if approver == proposal.proposed_by {
            return Err(TripartiteError::SelfApprovalForbidden);
        }
        if evaluation.is_none() {
            return Err(TripartiteError::MissingCriticEvaluation);
        }
        let eval = evaluation.unwrap();
        if eval.proposal_id != proposal.id {
            return Err(TripartiteError::MissingCriticEvaluation);
        }
        if let Some(ref verdict) = eval.canary_verdict
            && !verdict.promoted
        {
            return Err(TripartiteError::CanaryCheckFailed {
                lower_bound: verdict.lower_bound_95,
                baseline: verdict.baseline_rate,
            });
        }
        Ok(TripartiteApproval::new(
            proposal.id.clone(),
            approved,
            reason,
        ))
    }

    /// Promotes a proposal only if Critic evaluated and Governor approved.
    pub fn promote(
        &self,
        proposal: &TripartiteProposal,
        evaluation: Option<&TripartiteEvaluation>,
        approval: Option<&TripartiteApproval>,
    ) -> Result<bool, TripartiteError> {
        let eval = evaluation.ok_or(TripartiteError::MissingCriticEvaluation)?;
        if eval.proposal_id != proposal.id {
            return Err(TripartiteError::MissingCriticEvaluation);
        }
        let appr = approval.ok_or(TripartiteError::MissingGovernorApproval)?;
        if appr.proposal_id != proposal.id {
            return Err(TripartiteError::MissingGovernorApproval);
        }
        if !appr.approved {
            return Ok(false);
        }
        Ok(true)
    }
}

/// Helper to derive an isolated Git worktree branch name for testing competing proposals.
pub fn branch_name_for_proposal(proposal: &TripartiteProposal) -> String {
    format!(
        "experiment/{}-{}",
        proposal.proposed_by.as_str(),
        proposal.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gov() -> TripartiteGovernance {
        TripartiteGovernance::new(1)
    }

    #[test]
    fn generator_cannot_approve_own() {
        let g = gov();
        let p = TripartiteProposal::new("p1", "change x").unwrap();
        let eval = g
            .evaluate(&p, TripartiteRole::Critic, 1.0, 10.0, 5)
            .unwrap();
        let res = g.approve(&p, TripartiteRole::Generator, Some(&eval), true, "ok");
        assert_eq!(
            res,
            Err(TripartiteError::RoleMismatch {
                expected: TripartiteRole::Governor,
                actual: TripartiteRole::Generator,
            })
        );
    }

    #[test]
    fn full_flow_promotes() {
        let g = gov();
        let p = TripartiteProposal::new("p1", "change x").unwrap();
        let eval = g
            .evaluate(&p, TripartiteRole::Critic, 1.0, 10.0, 5)
            .unwrap();
        let appr = g
            .approve(&p, TripartiteRole::Governor, Some(&eval), true, "ok")
            .unwrap();
        assert!(g.promote(&p, Some(&eval), Some(&appr)).unwrap());
    }

    #[test]
    fn missing_critic_blocks_approval() {
        let g = gov();
        let p = TripartiteProposal::new("p1", "change x").unwrap();
        let res = g.approve(&p, TripartiteRole::Governor, None, true, "ok");
        assert_eq!(res, Err(TripartiteError::MissingCriticEvaluation));
    }

    #[test]
    fn missing_governor_blocks_promotion() {
        let g = gov();
        let p = TripartiteProposal::new("p1", "change x").unwrap();
        let eval = g
            .evaluate(&p, TripartiteRole::Critic, 1.0, 10.0, 5)
            .unwrap();
        let res = g.promote(&p, Some(&eval), None);
        assert_eq!(res, Err(TripartiteError::MissingGovernorApproval));
    }

    #[test]
    fn empty_proposal_rejected() {
        assert_eq!(
            TripartiteProposal::new("p1", "   "),
            Err(TripartiteError::EmptyProposal)
        );
    }

    #[test]
    fn governor_rejection_does_not_promote() {
        let g = gov();
        let p = TripartiteProposal::new("p1", "change x").unwrap();
        let eval = g
            .evaluate(&p, TripartiteRole::Critic, 1.0, 10.0, 5)
            .unwrap();
        let appr = g
            .approve(&p, TripartiteRole::Governor, Some(&eval), false, "no")
            .unwrap();
        assert!(!g.promote(&p, Some(&eval), Some(&appr)).unwrap());
    }

    #[test]
    fn critic_evaluates_with_pareto_and_canary() {
        let g = gov();
        let p = TripartiteProposal::new("p_pareto", "better candidate").unwrap();
        let mut frontier = ParetoFrontier::new();
        let canary_gate = BayesianCanaryGate::new(0.95);

        // 100 passes, 0 failures, baseline 0.90 -> passes canary gate
        let eval = g
            .evaluate_with_verifiers(
                &p,
                TripartiteRole::Critic,
                1.0,
                50.0,
                10,
                &mut frontier,
                Some((&canary_gate, 100, 0, 0.90)),
            )
            .unwrap();

        assert!(eval.is_pareto_accepted());
        assert!(eval.is_canary_promoted());

        let appr = g
            .approve(&p, TripartiteRole::Governor, Some(&eval), true, "verified")
            .unwrap();
        assert!(g.promote(&p, Some(&eval), Some(&appr)).unwrap());
    }

    #[test]
    fn critic_rejects_pareto_dominated_proposal() {
        let g = gov();
        let mut frontier = ParetoFrontier::new();
        // Add strong incumbent: pass_rate: 1.0, token_cost: 10, latency: 5ms
        let incumbent = ParetoCandidate::new("incumbent", 1.0, 10, 5);
        frontier.try_insert(incumbent);

        // Worse candidate: pass_rate: 0.8, token_cost: 20, latency: 15ms -> dominated
        let p_worse = TripartiteProposal::new("p_worse", "worse candidate").unwrap();
        let err = g.evaluate_with_verifiers(
            &p_worse,
            TripartiteRole::Critic,
            0.8,
            20.0,
            15,
            &mut frontier,
            None,
        );
        assert_eq!(
            err,
            Err(TripartiteError::ParetoDominated("p_worse".to_string()))
        );
    }

    #[test]
    fn critic_rejects_failing_canary_gate() {
        let g = gov();
        let mut frontier = ParetoFrontier::new();
        let canary_gate = BayesianCanaryGate::new(0.95);

        // High failure rate: 10 passes, 90 failures, baseline 0.80 -> fails canary
        let p_fail = TripartiteProposal::new("p_fail", "flaky candidate").unwrap();
        let err = g.evaluate_with_verifiers(
            &p_fail,
            TripartiteRole::Critic,
            0.5,
            10.0,
            5,
            &mut frontier,
            Some((&canary_gate, 10, 90, 0.80)),
        );
        assert!(matches!(
            err,
            Err(TripartiteError::CanaryCheckFailed { .. })
        ));
    }

    #[test]
    fn branch_name_formatting() {
        let p = TripartiteProposal::new("sop-crystallize-42", "crystallized sop").unwrap();
        assert_eq!(
            branch_name_for_proposal(&p),
            "experiment/generator-sop-crystallize-42"
        );
    }
}
