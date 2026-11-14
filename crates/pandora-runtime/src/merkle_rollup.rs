//! Phase 25: Merkle Rollup Batch Aggregation & Log-Time Inclusion Proofs (Theorem 48).
//!
//! Provides cryptographic batch aggregation rolling up $M$ distributed consensus decisions
//! (`DistributedConsensusDecision::Approved(CertifiedPatchRecord)`) into a single 32-byte
//! epoch root ($R_{\mathrm{epoch}}$) with log-time $O(\log M)$ inclusion proofs and
//! sequential epoch header chaining.

#![forbid(unsafe_code)]

use crate::autopilot::CertifiedPatchRecord;
use crate::distributed_swarm::DistributedConsensusDecision;
use crate::merkle_dag::{InclusionProof, MerkleDagAuditLog, MerkleProofStep, NodeDirection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Domain separation tag for rollup leaf node hashes (prevents second-preimage attacks).
pub const TAG_ROLLUP_LEAF: u8 = 0x03;

/// Domain separation tag for cryptographic epoch rollup headers.
pub const TAG_EPOCH_HEADER: u8 = 0x04;

/// Error conditions encountered during Merkle rollup operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollupError {
    /// Attempted to build an epoch with zero decisions.
    EmptyDecisions,
    /// A consensus decision passed for rollup was not approved (e.g., Pending or Rejected).
    UnapprovedDecision(String),
    /// Leaf index was out of bounds for the epoch.
    IndexOutOfBounds { index: usize, total: usize },
    /// Sibling or proof path verification failed against the epoch root.
    InclusionVerificationFailed(String),
    /// Hash chain broken between previous epoch root and current epoch header.
    EpochChainBroken {
        expected_prev_root: String,
        actual_prev_root: String,
    },
    /// Serialization or deserialization error.
    SerializationError(String),
}

impl fmt::Display for RollupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDecisions => write!(f, "cannot create rollup epoch with zero decisions"),
            Self::UnapprovedDecision(reason) => {
                write!(
                    f,
                    "decision cannot be rolled up because it is not approved: {reason}"
                )
            }
            Self::IndexOutOfBounds { index, total } => {
                write!(
                    f,
                    "leaf index {index} out of bounds for epoch containing {total} decisions"
                )
            }
            Self::InclusionVerificationFailed(msg) => {
                write!(f, "merkle inclusion proof verification failed: {msg}")
            }
            Self::EpochChainBroken {
                expected_prev_root,
                actual_prev_root,
            } => {
                write!(
                    f,
                    "epoch chain broken: expected previous root {expected_prev_root}, got {actual_prev_root}"
                )
            }
            Self::SerializationError(msg) => write!(f, "rollup serialization error: {msg}"),
        }
    }
}

impl std::error::Error for RollupError {}

/// A single rolled-up distributed consensus decision entry within an epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollupDecisionEntry {
    /// Zero-based sequential index of the decision within the epoch batch.
    pub decision_index: u64,
    /// Winning patch proposal ID or unique patch identifier.
    pub patch_id: String,
    /// 32-byte canonical consensus hash from distributed Byzantine quorum.
    pub consensus_hash: [u8; 32],
    /// Cryptographic patch certificate digest (`PANDORA_PATCH_CERT_V1:<sha256>`).
    pub certificate_digest: String,
    /// Consensus commit timestamp in milliseconds since UNIX epoch.
    pub timestamp_ms: u64,
}

impl RollupDecisionEntry {
    /// Creates a new `RollupDecisionEntry`.
    pub fn new(
        decision_index: u64,
        patch_id: impl Into<String>,
        consensus_hash: [u8; 32],
        certificate_digest: impl Into<String>,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            decision_index,
            patch_id: patch_id.into(),
            consensus_hash,
            certificate_digest: certificate_digest.into(),
            timestamp_ms,
        }
    }

    /// Computes the cryptographic leaf hash using domain separation tag `TAG_ROLLUP_LEAF = 0x03`:
    /// $$h = \text{SHA256}(0x03 \mathbin{\Vert} \text{decision\_index} \mathbin{\Vert} \text{len}(\text{patch\_id}) \mathbin{\Vert} \text{patch\_id} \mathbin{\Vert} \text{consensus\_hash} \mathbin{\Vert} \text{len}(\text{cert}) \mathbin{\Vert} \text{cert} \mathbin{\Vert} \text{timestamp\_ms})$$
    pub fn compute_leaf_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update([TAG_ROLLUP_LEAF]);
        hasher.update(self.decision_index.to_be_bytes());
        hasher.update((self.patch_id.len() as u32).to_be_bytes());
        hasher.update(self.patch_id.as_bytes());
        hasher.update(self.consensus_hash);
        hasher.update((self.certificate_digest.len() as u32).to_be_bytes());
        hasher.update(self.certificate_digest.as_bytes());
        hasher.update(self.timestamp_ms.to_be_bytes());
        hasher.finalize().into()
    }

    /// Converts a `CertifiedPatchRecord` from Autopilot / Swarm Consensus into a `RollupDecisionEntry`.
    pub fn from_certified_patch(record: &CertifiedPatchRecord, decision_index: u64) -> Self {
        let ch_bytes = if let Some(decoded) = decode_hex_32(&record.consensus_hash) {
            decoded
        } else {
            Sha256::digest(record.consensus_hash.as_bytes()).into()
        };

        Self {
            decision_index,
            patch_id: record.winning_proposal_id.clone(),
            consensus_hash: ch_bytes,
            certificate_digest: record.certificate_digest.clone(),
            timestamp_ms: record.timestamp_ms,
        }
    }

    /// Converts a `DistributedConsensusDecision` into a `RollupDecisionEntry` if approved.
    pub fn from_consensus_decision(
        decision: &DistributedConsensusDecision,
        decision_index: u64,
    ) -> Result<Self, RollupError> {
        match decision {
            DistributedConsensusDecision::Approved(record) => {
                Ok(Self::from_certified_patch(record, decision_index))
            }
            DistributedConsensusDecision::Pending {
                current_approvals,
                quorum_needed,
                ..
            } => Err(RollupError::UnapprovedDecision(format!(
                "decision is pending ({current_approvals}/{quorum_needed} approvals)"
            ))),
            DistributedConsensusDecision::Rejected { reason, .. } => Err(
                RollupError::UnapprovedDecision(format!("decision rejected: {reason}")),
            ),
        }
    }
}

/// Cryptographic header binding an epoch rollup root to its predecessor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRollupHeader {
    /// Monotonically increasing epoch identifier ($e \ge 1$).
    pub epoch_id: u64,
    /// Total count of decisions batched into this epoch ($M \ge 1$).
    pub decision_count: u64,
    /// 32-byte cryptographic Merkle root of the batch ($R_{\mathrm{epoch}}$).
    pub epoch_root: [u8; 32],
    /// 32-byte cryptographic root of the immediately preceding epoch ($R_{\mathrm{epoch}}^{(e-1)}$).
    /// For epoch 1 (genesis), this is 32 zero bytes.
    pub previous_epoch_root: [u8; 32],
    /// Batch commitment timestamp in milliseconds since UNIX epoch.
    pub timestamp_ms: u64,
    /// Canonical 32-byte hash binding all header fields.
    pub header_hash: [u8; 32],
}

impl EpochRollupHeader {
    /// Computes the cryptographic header hash using domain separation tag `TAG_EPOCH_HEADER = 0x04`:
    /// $$H = \text{SHA256}(0x04 \mathbin{\Vert} \text{epoch\_id} \mathbin{\Vert} \text{decision\_count} \mathbin{\Vert} R_{\mathrm{epoch}} \mathbin{\Vert} R_{\mathrm{prev}} \mathbin{\Vert} \text{timestamp\_ms})$$
    pub fn compute_header_hash(
        epoch_id: u64,
        decision_count: u64,
        epoch_root: &[u8; 32],
        previous_epoch_root: &[u8; 32],
        timestamp_ms: u64,
    ) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update([TAG_EPOCH_HEADER]);
        hasher.update(epoch_id.to_be_bytes());
        hasher.update(decision_count.to_be_bytes());
        hasher.update(epoch_root);
        hasher.update(previous_epoch_root);
        hasher.update(timestamp_ms.to_be_bytes());
        hasher.finalize().into()
    }

    /// Verifies the cryptographic integrity of the header hash.
    pub fn verify_header_hash(&self) -> bool {
        let expected = Self::compute_header_hash(
            self.epoch_id,
            self.decision_count,
            &self.epoch_root,
            &self.previous_epoch_root,
            self.timestamp_ms,
        );
        self.header_hash == expected
    }

    /// Verifies sequential chain continuity against the expected predecessor epoch root.
    pub fn verify_chain(&self, expected_prev_root: &[u8; 32]) -> Result<(), RollupError> {
        if !self.verify_header_hash() {
            return Err(RollupError::InclusionVerificationFailed(
                "header hash mismatch".to_string(),
            ));
        }
        if self.previous_epoch_root != *expected_prev_root {
            return Err(RollupError::EpochChainBroken {
                expected_prev_root: encode_hex(expected_prev_root),
                actual_prev_root: encode_hex(&self.previous_epoch_root),
            });
        }
        Ok(())
    }
}

/// Complete Merkle Rollup Epoch containing header, raw entries, and leaf hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollupEpoch {
    /// Canonical epoch header binding roots and predecessor.
    pub header: EpochRollupHeader,
    /// List of all decision entries included in this batch.
    pub entries: Vec<RollupDecisionEntry>,
    /// Precomputed 32-byte leaf hashes in canonical index order.
    pub leaf_hashes: Vec<[u8; 32]>,
}

impl RollupEpoch {
    /// Computes the balanced binary Merkle root from leaf hashes with trailing duplication for odd counts.
    pub fn compute_root_from_leaves(leaves: &[[u8; 32]]) -> [u8; 32] {
        if leaves.is_empty() {
            return [0u8; 32];
        }
        if leaves.len() == 1 {
            return leaves[0];
        }
        let mut current_level = leaves.to_vec();
        while current_level.len() > 1 {
            let mut next_level = Vec::with_capacity(current_level.len().div_ceil(2));
            let mut i = 0;
            while i < current_level.len() {
                let left = current_level[i];
                let right = if i + 1 < current_level.len() {
                    current_level[i + 1]
                } else {
                    current_level[i]
                };
                next_level.push(MerkleDagAuditLog::compute_parent_hash(&left, &right));
                i += 2;
            }
            current_level = next_level;
        }
        current_level[0]
    }

    /// Creates a new `RollupEpoch` by batching $M$ decision entries and computing $R_{\mathrm{epoch}}$.
    pub fn create_epoch(
        epoch_id: u64,
        entries: Vec<RollupDecisionEntry>,
        previous_epoch_root: [u8; 32],
        timestamp_ms: u64,
    ) -> Result<Self, RollupError> {
        if entries.is_empty() {
            return Err(RollupError::EmptyDecisions);
        }

        let mut leaf_hashes = Vec::with_capacity(entries.len());
        for (idx, entry) in entries.iter().enumerate() {
            let mut normalized_entry = entry.clone();
            normalized_entry.decision_index = idx as u64;
            let leaf = normalized_entry.compute_leaf_hash();
            leaf_hashes.push(leaf);
        }

        let epoch_root = Self::compute_root_from_leaves(&leaf_hashes);
        let decision_count = entries.len() as u64;

        let header_hash = EpochRollupHeader::compute_header_hash(
            epoch_id,
            decision_count,
            &epoch_root,
            &previous_epoch_root,
            timestamp_ms,
        );

        let header = EpochRollupHeader {
            epoch_id,
            decision_count,
            epoch_root,
            previous_epoch_root,
            timestamp_ms,
            header_hash,
        };

        Ok(Self {
            header,
            entries,
            leaf_hashes,
        })
    }

    /// Convenience builder creating an epoch from a slice of `DistributedConsensusDecision`.
    pub fn create_epoch_from_decisions(
        epoch_id: u64,
        decisions: &[DistributedConsensusDecision],
        previous_epoch_root: [u8; 32],
        timestamp_ms: u64,
    ) -> Result<Self, RollupError> {
        let mut entries = Vec::with_capacity(decisions.len());
        for (idx, d) in decisions.iter().enumerate() {
            entries.push(RollupDecisionEntry::from_consensus_decision(d, idx as u64)?);
        }
        Self::create_epoch(epoch_id, entries, previous_epoch_root, timestamp_ms)
    }

    /// Returns the 32-byte epoch root hash ($R_{\mathrm{epoch}}$).
    pub fn epoch_root(&self) -> [u8; 32] {
        self.header.epoch_root
    }

    /// Returns the tree height $h = \lceil \log_2 M \rceil$.
    pub fn tree_height(&self) -> usize {
        let count = self.leaf_hashes.len();
        if count <= 1 {
            1
        } else {
            (count as f64).log2().ceil() as usize
        }
    }

    /// Generates a logarithmic $O(\log M)$ inclusion proof for the decision at `decision_index`.
    pub fn generate_inclusion_proof(
        &self,
        decision_index: usize,
    ) -> Result<InclusionProof, RollupError> {
        if decision_index >= self.leaf_hashes.len() {
            return Err(RollupError::IndexOutOfBounds {
                index: decision_index,
                total: self.leaf_hashes.len(),
            });
        }
        let leaf_hash = self.leaf_hashes[decision_index];
        let root = self.epoch_root();

        if self.leaf_hashes.len() == 1 {
            return Ok(InclusionProof {
                leaf_index: decision_index,
                leaf_hash,
                steps: Vec::new(),
                root_hash: root,
            });
        }

        let mut steps = Vec::new();
        let mut current_level = self.leaf_hashes.clone();
        let mut target_idx = decision_index;

        while current_level.len() > 1 {
            let is_right_child = target_idx % 2 == 1;
            let sibling_idx = if is_right_child {
                target_idx - 1
            } else if target_idx + 1 < current_level.len() {
                target_idx + 1
            } else {
                target_idx // Sibling is duplicate of self
            };

            let sibling_hash = current_level[sibling_idx];
            let direction = if is_right_child {
                NodeDirection::Right
            } else {
                NodeDirection::Left
            };

            steps.push(MerkleProofStep {
                sibling_hash,
                direction,
            });

            let mut next_level = Vec::with_capacity(current_level.len().div_ceil(2));
            let mut i = 0;
            while i < current_level.len() {
                let left = current_level[i];
                let right = if i + 1 < current_level.len() {
                    current_level[i + 1]
                } else {
                    current_level[i]
                };
                next_level.push(MerkleDagAuditLog::compute_parent_hash(&left, &right));
                i += 2;
            }
            target_idx /= 2;
            current_level = next_level;
        }

        Ok(InclusionProof {
            leaf_index: decision_index,
            leaf_hash,
            steps,
            root_hash: root,
        })
    }

    /// Verifies an inclusion proof against an epoch root and a candidate `RollupDecisionEntry`.
    pub fn verify_inclusion(
        epoch_root: &[u8; 32],
        proof: &InclusionProof,
        entry: &RollupDecisionEntry,
    ) -> bool {
        // Step 1: Root equality check
        if proof.root_hash != *epoch_root {
            return false;
        }

        // Step 2: Leaf digest equality check
        let expected_leaf = entry.compute_leaf_hash();
        if proof.leaf_hash != expected_leaf {
            return false;
        }

        // Step 3: Recompute hash path to root
        proof.verify()
    }

    /// Computes the empirical compression ratio relative to storing raw patch records:
    /// $$\mathcal{C}_R(M) = \frac{M \cdot S_{\mathrm{raw}}}{S_{\mathrm{header}} + 32}$$
    pub fn compression_ratio(&self, raw_bytes_per_decision: usize) -> f64 {
        let raw_total = (self.header.decision_count as f64) * (raw_bytes_per_decision as f64);
        let rollup_bytes = 88.0 + 32.0; // Header metadata + Epoch Root
        raw_total / rollup_bytes
    }

    /// Computes the storage savings percentage: $(1 - 1/\mathcal{C}_R) \times 100\%$.
    pub fn storage_savings_pct(&self, raw_bytes_per_decision: usize) -> f64 {
        let ratio = self.compression_ratio(raw_bytes_per_decision);
        if ratio <= 1.0 {
            0.0
        } else {
            (1.0 - (1.0 / ratio)) * 100.0
        }
    }
}

/// Encodes byte slice to lowercase hex string.
pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes 64-character hex string into a 32-byte array.
pub fn decode_hex_32(s: &str) -> Option<[u8; 32]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = s.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let high = decode_nibble(bytes[i * 2])?;
        let low = decode_nibble(bytes[i * 2 + 1])?;
        *slot = (high << 4) | low;
    }
    Some(out)
}

fn decode_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
