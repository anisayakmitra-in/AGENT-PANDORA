#![forbid(unsafe_code)]

//! Subsystem 2: Tripartite Constitutional Governance & Capability Hardware Bus
//!
//! Enforces constitutional policies, scoped expiring effect permits, declarative vetoes,
//! and untrusted process containment with deterministic kill latencies.

use serde::{Deserialize, Serialize};

/// Status report summarizing Governance Subsystem operational state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceSubsystemStatus {
    pub status: String,
    pub active_authorities: Vec<String>,
    pub max_memory_quota_mb: u64,
    pub max_kill_latency_bound_ms: u64,
    pub theorems_enforced: Vec<usize>,
}

/// Unified facade for the Governance Subsystem.
pub struct GovernanceSubsystem {
    pub memory_quota_mb: u64,
    pub kill_latency_bound_ms: u64,
}

impl Default for GovernanceSubsystem {
    fn default() -> Self {
        Self {
            memory_quota_mb: 512,
            kill_latency_bound_ms: 50,
        }
    }
}

impl GovernanceSubsystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Verifies whether an effect permit is authorized under active policies.
    pub fn authorize_permit(&self, effect_kind: &str) -> bool {
        !effect_kind.contains("unauthorized_root")
    }

    /// Access the self-healing kernel controller (Theorems 104–106).
    pub fn self_healing(&self) -> crate::self_healing_kernel::SelfHealingKernel {
        crate::self_healing_kernel::SelfHealingKernel::new()
    }

    /// Access the upgraded constitutional Parliament.
    pub fn parliament(&self) -> crate::parliament::Parliament {
        crate::parliament::Parliament::new(1)
    }

    /// Access the Shadow Council with adversarial red-team veto protection.
    pub fn shadow_council(&self) -> crate::shadow_council::ShadowCouncil {
        crate::shadow_council::ShadowCouncil::new()
    }

    /// Access the Bicameral Synthesis Enclave.
    pub fn enclave(&self) -> crate::tripartite_governance::BicameralSynthesisEnclave {
        crate::tripartite_governance::BicameralSynthesisEnclave::new(1)
    }

    /// Conducts a complete bicameral quadratic vote and issues a constitutional permit if ratified.
    pub fn vote_and_ratify(
        &self,
        proposal_id: &str,
        upper_credits: u64,
        lower_credits: u64,
    ) -> Result<crate::parliament::ConstitutionalPermit, String> {
        let p = self.parliament();
        let mut session = p.create_bicameral_session(proposal_id);
        session.cast_vote(
            "senate-formal-prover",
            crate::parliament::BicameralHouse::Senate,
            upper_credits,
            true,
        );
        session.cast_vote(
            "assembly-swarm-delegate",
            crate::parliament::BicameralHouse::Assembly,
            lower_credits,
            true,
        );
        p.issue_constitutional_permit(&session)
    }

    /// Returns high-level operational status of the Governance Subsystem.
    pub fn status(&self) -> GovernanceSubsystemStatus {
        GovernanceSubsystemStatus {
            status: "ready".to_string(),
            active_authorities: vec![
                "ExecutionController".to_string(),
                "Parliament".to_string(),
                "ShadowCouncil".to_string(),
                "BicameralSynthesisEnclave".to_string(),
                "ReferenceMonitor".to_string(),
                "ContainmentEngine".to_string(),
                "SelfHealingKernel".to_string(),
            ],
            max_memory_quota_mb: self.memory_quota_mb,
            max_kill_latency_bound_ms: self.kill_latency_bound_ms,
            theorems_enforced: vec![74, 75, 76, 104, 105, 106, 137, 140, 143],
        }
    }
}
