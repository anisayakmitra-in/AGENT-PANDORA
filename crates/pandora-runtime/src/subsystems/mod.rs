#![forbid(unsafe_code)]

//! AGENT-PANDORA Subsystems (v4.0 Architecture - THANATOS)
//!
//! Provides the 5 foundational micro-kernel architectural pillars:
//! - Subsystem 1: Cognitive Micro-Kernel & Neuro-Symbolic Inference Engine
//! - Subsystem 2: Tripartite Constitutional Governance & Capability Bus
//! - Subsystem 3: Cryptographic Zero-Knowledge & Post-Quantum Substrate
//! - Subsystem 4: Decentralized Swarm Mesh & Autonomous Synchronization
//! - Subsystem 5: Continual Memory VFS & Certified Privacy Substrate

pub mod cognitive;
pub mod crypto;
pub mod governance;
pub mod memory;
pub mod swarm;

pub use cognitive::{CognitiveSubsystem, CognitiveSubsystemStatus};
pub use crypto::{CryptoSubsystem, CryptoSubsystemStatus};
pub use governance::{GovernanceSubsystem, GovernanceSubsystemStatus};
pub use memory::{MemorySubsystem, MemorySubsystemStatus};
pub use swarm::{SwarmSubsystem, SwarmSubsystemStatus};

use crate::formal_proof::{FormalProofEngine, FormalVerificationReport, ProofDialect};
use serde::{Deserialize, Serialize};

/// Comprehensive overview aggregating all 5 foundational subsystems and formal verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OlympusSystemOverview {
    pub architecture_version: String,
    pub codename: String,
    pub cognitive: CognitiveSubsystemStatus,
    pub governance: GovernanceSubsystemStatus,
    pub crypto: CryptoSubsystemStatus,
    pub swarm: SwarmSubsystemStatus,
    pub memory: MemorySubsystemStatus,
    pub formal_verification: FormalVerificationReport,
    pub total_theorems_enforced: usize,
}

impl OlympusSystemOverview {
    pub fn inspect() -> Self {
        let mut cog = CognitiveSubsystem::new();
        let gov = GovernanceSubsystem::new();
        let cry = CryptoSubsystem::new();
        let swm = SwarmSubsystem::new(4);
        let mem = MemorySubsystem::new();
        let formal = FormalProofEngine::new().verify_centennial_suite(ProofDialect::Lean4);

        Self {
            architecture_version: "4.1.1".to_string(),
            codename: "THANATOS".to_string(),
            cognitive: cog.status(),
            governance: gov.status(),
            crypto: cry.status(),
            swarm: swm.status(),
            memory: mem.status(),
            formal_verification: formal,
            total_theorems_enforced: 163,
        }
    }
}
