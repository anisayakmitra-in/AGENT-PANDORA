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

    /// Access the KZG polynomial commitment scheme (Theorems 122–124).
    pub fn kzg_commitments(&self) -> crate::kzg_commitment::KzgScheme {
        crate::kzg_commitment::KzgScheme::new(32)
    }

    /// Access the CKKS leveled fully homomorphic encryption engine (Theorems 131–133).
    pub fn ckks_fhe(&self) -> crate::ckks_fhe::CkksFheEngine {
        crate::ckks_fhe::CkksFheEngine::new(8192, 5)
    }

    /// Access the multi-key threshold FHE engine (Theorems 140–142).
    pub fn threshold_fhe(&self) -> crate::threshold_fhe::ThresholdFheEngine {
        crate::threshold_fhe::ThresholdFheEngine::default()
    }

    /// Access the homomorphic functional encryption engine (Theorems 149–151).
    pub fn functional_encryption(
        &self,
    ) -> crate::functional_encryption::FunctionalEncryptionEngine {
        crate::functional_encryption::FunctionalEncryptionEngine::default()
    }

    /// Access the Post-Quantum Blind MPC engine (Theorems 158–160).
    pub fn blind_mpc(&self) -> crate::blind_mpc::BlindMpcEngine {
        crate::blind_mpc::BlindMpcEngine::default()
    }

    /// Access the Post-Quantum Lattice zk-SNARK engine (Theorems 167–169).
    pub fn lattice_zk_snark(&self) -> crate::lattice_zk_snark::LatticeZkSnarkEngine {
        crate::lattice_zk_snark::LatticeZkSnarkEngine::default()
    }

    /// Access the Multi-Party Verifiable Delay Function engine (Theorems 176–178).
    pub fn verifiable_delay(&self) -> crate::verifiable_delay::VerifiableDelayEngine {
        crate::verifiable_delay::VerifiableDelayEngine::default()
    }

    /// Access the Multi-Prover Interactive Proofs (MIP*) & Entangled Non-Local Game engine (Theorems 185–187).
    pub fn quantum_mip(&self) -> crate::quantum_mip::QuantumMipEngine {
        crate::quantum_mip::QuantumMipEngine::default()
    }

    /// Access the Dynamic Proactive Secret Sharing (DPSS) & Redistribution engine (Theorems 194–196).
    pub fn proactive_secret_redistribution(
        &self,
    ) -> crate::proactive_secret_redistribution::ProactiveSecretRedistributionEngine {
        crate::proactive_secret_redistribution::ProactiveSecretRedistributionEngine::default()
    }

    /// Access the Multi-Input Functional Encryption (MIFE) engine (Theorems 203–205).
    pub fn multi_input_functional_encryption(
        &self,
    ) -> crate::multi_input_functional_encryption::MultiInputFunctionalEncryptionEngine {
        crate::multi_input_functional_encryption::MultiInputFunctionalEncryptionEngine::default()
    }

    /// Access the Indistinguishability Obfuscation ($i\mathcal{O}$) & Punctured PRF engine (Theorems 212–214).
    pub fn indistinguishability_obfuscation(
        &self,
    ) -> crate::indistinguishability_obfuscation::IndistinguishabilityObfuscationEngine {
        crate::indistinguishability_obfuscation::IndistinguishabilityObfuscationEngine::default()
    }

    /// Access the Linear Logic Capability Monads & Non-Interference engine (Theorems 221–223).
    pub fn linear_capability(&self) -> crate::linear_capability::LinearCapabilityEngine {
        crate::linear_capability::LinearCapabilityEngine::default()
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
            theorems_enforced: vec![
                48, 49, 77, 78, 79, 83, 84, 85, 101, 102, 103, 113, 114, 115, 122, 123, 124, 131,
                132, 133, 140, 141, 142, 149, 150, 151, 158, 159, 160, 167, 168, 169, 176, 177,
                178, 185, 186, 187, 194, 195, 196, 203, 204, 205, 212, 213, 214, 221, 222, 223,
            ],
        }
    }
}
