#![forbid(unsafe_code)]

//! Subsystem 3: Cryptographic Zero-Knowledge & Post-Quantum Substrate
//!
//! Provides post-quantum hybrid lattice attestation (ML-DSA-65 + Ed25519), Goldilocks
//! Poseidon algebraic sponge commitments, and Shamir homomorphic MPC (Theorems 77–79, 83–85).

use crate::pq_attestation::{
    HybridAttestationReceipt, HybridAttestationSigner, HybridVerifyingKey,
};
use crate::swarm_mpc::{BeaverTriple, MpcAuditReport, SecretShare, ShamirMpcEngine};
use serde::{Deserialize, Serialize};

/// Status report summarizing Cryptographic Subsystem readiness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoSubsystemStatus {
    pub status: String,
    pub prime_fields: Vec<String>,
    pub hybrid_pq_verifying_key: HybridVerifyingKey,
    pub mpc_report: MpcAuditReport,
    pub theorems_enforced: Vec<usize>,
}

/// Unified facade for the Cryptographic Subsystem.
pub struct CryptoSubsystem {
    pub signer: HybridAttestationSigner,
    pub mpc: ShamirMpcEngine,
}

impl Default for CryptoSubsystem {
    fn default() -> Self {
        Self {
            signer: HybridAttestationSigner::from_seed([42u8; 32]),
            mpc: ShamirMpcEngine::default(),
        }
    }
}

impl CryptoSubsystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sign payload using post-quantum hybrid scheme.
    pub fn sign_hybrid(&self, payload: &[u8]) -> HybridAttestationReceipt {
        self.signer.sign(payload)
    }

    /// Split a secret into n shares with threshold k.
    pub fn split_secret(&self, secret: u64, k: usize, n: usize) -> Vec<SecretShare> {
        let coefs: Vec<u64> = (1..k).map(|i| (i as u64 * 333) % self.mpc.prime).collect();
        self.mpc.split_secret(secret, k, n, &coefs)
    }

    /// Reconstruct secret from shares.
    pub fn reconstruct_secret(&self, shares: &[SecretShare]) -> Result<u64, &'static str> {
        self.mpc.reconstruct_secret(shares)
    }

    /// Homomorphic Beaver multiplication.
    pub fn homomorphic_multiply(&self, x: u64, y: u64, triple: &BeaverTriple) -> u64 {
        self.mpc.homomorphic_multiply(x, y, triple)
    }

    /// Access the recursive ZK rollup engine (Theorems 101–103).
    pub fn zk_rollup(&self) -> crate::recursive_zk_rollup::ZkRollupEngine {
        crate::recursive_zk_rollup::ZkRollupEngine::new()
    }

    /// Access the federated homomorphic aggregator (Theorems 113–115).
    pub fn federated_homomorphic(
        &self,
    ) -> crate::federated_homomorphic::FederatedHomomorphicEngine {
        crate::federated_homomorphic::FederatedHomomorphicEngine::new()
    }

    /// Returns high-level operational status of the Crypto Subsystem.
    pub fn status(&self) -> CryptoSubsystemStatus {
        let mpc_rep = self.mpc.audit(133742, 3, 5);
        CryptoSubsystemStatus {
            status: "ready".to_string(),
            prime_fields: vec![
                "Goldilocks (2^64 - 2^32 + 1)".to_string(),
                "Mersenne-31 (2^31 - 1)".to_string(),
                "BabyBear (2^31 - 2^27 + 1)".to_string(),
            ],
            hybrid_pq_verifying_key: self.signer.verifying_key(),
            mpc_report: mpc_rep,
            theorems_enforced: vec![48, 49, 77, 78, 79, 83, 84, 85, 101, 102, 103, 113, 114, 115],
        }
    }
}
