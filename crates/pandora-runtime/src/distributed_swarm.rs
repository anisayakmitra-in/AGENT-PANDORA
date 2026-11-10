#![forbid(unsafe_code)]

//! Distributed Cross-Node Swarm Mesh, Byzantine Patch Consensus Engine & Federated Gradient Dissemination
//!
//! Implements Phase 24 Milestone M2 runtime capabilities under Theorem 47:
//! 1. Wire protocol framing (`DistributedSwarmMessage`, `SwarmPayload`) with mutual Ed25519 signing and verification.
//! 2. Cross-machine Byzantine patch consensus engine (`DistributedSwarmConsensusEngine`) enforcing $N \ge 3f + 1$,
//!    supermajority quorum $Q = 2f + 1$, equivocation detection, and coordinate-wise trimmed mean centroid filtering.
//! 3. Distributed federated policy gradient dissemination (`DistributedFederatedEngine`) aggregating 6D policy
//!    gradient vectors across cluster nodes with reputation weighting and variance reduction.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use ed25519_dalek::{Signature, Signer, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::autopilot::CertifiedPatchRecord;
use crate::federated_policy::{
    DEFAULT_LEARNING_RATE, DEFAULT_MAX_GRADIENT_NORM, DEFAULT_MOMENTUM, FederatedPolicyEngine,
    FederatedPolicyError, FederatedUpdateReceipt, PolicyGradientVector,
};
use crate::mesh_sync::{MeshHandshake, PeerIdentity};
use crate::multi_agent_collab::ExternalAgentKind;
use crate::multi_agent_consensus::{ProposalScoreVector, ProposalVerdict};
use crate::swarm_mesh::VectorClock;

// ============================================================================
// Safe Hex Utilities (Pure Safe Rust)
// ============================================================================

pub fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub fn decode_hex_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut buf = [0u8; 32];
    for (i, byte) in buf.iter_mut().enumerate() {
        let hi = decode_hex_nibble(s.as_bytes()[i * 2])?;
        let lo = decode_hex_nibble(s.as_bytes()[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(buf)
}

pub fn decode_hex_64(s: &str) -> Option<[u8; 64]> {
    if s.len() != 128 {
        return None;
    }
    let mut buf = [0u8; 64];
    for (i, byte) in buf.iter_mut().enumerate() {
        let hi = decode_hex_nibble(s.as_bytes()[i * 2])?;
        let lo = decode_hex_nibble(s.as_bytes()[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(buf)
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DistributedSwarmError {
    InvalidBftParameters {
        n: usize,
        f: usize,
    },
    ProposalAlreadyExists(String),
    ProposalNotFound(String),
    EquivocationDetected {
        voter: String,
        proposal: String,
    },
    ConsensusPending {
        proposal: String,
        approvals: usize,
        needed: usize,
    },
    ConsensusRejected {
        proposal: String,
        reason: String,
    },
    InvalidSignature(String),
    InvalidPublicKey(String),
    SerializationError(String),
    EmptyGradients,
    FederatedPolicy(String),
    TransportError(String),
    InvalidGradient(String),
}

impl fmt::Display for DistributedSwarmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBftParameters { n, f: byz } => {
                write!(
                    f,
                    "invalid BFT parameters: total nodes N={n} must be >= 3f + 1 (with f={byz}, minimum is {})",
                    3 * byz + 1
                )
            }
            Self::ProposalAlreadyExists(id) => write!(f, "proposal '{id}' already registered"),
            Self::ProposalNotFound(id) => write!(f, "proposal '{id}' not found"),
            Self::EquivocationDetected { voter, proposal } => {
                write!(
                    f,
                    "equivocation detected: node '{voter}' cast conflicting votes for proposal '{proposal}'"
                )
            }
            Self::ConsensusPending {
                proposal,
                approvals,
                needed,
            } => {
                write!(
                    f,
                    "consensus pending for proposal '{proposal}': {approvals}/{needed} approvals"
                )
            }
            Self::ConsensusRejected { proposal, reason } => {
                write!(f, "consensus rejected for proposal '{proposal}': {reason}")
            }
            Self::InvalidSignature(err) => write!(f, "invalid Ed25519 signature: {err}"),
            Self::InvalidPublicKey(err) => write!(f, "invalid Ed25519 public key: {err}"),
            Self::SerializationError(err) => write!(f, "payload serialization error: {err}"),
            Self::EmptyGradients => write!(f, "cannot aggregate empty gradient set"),
            Self::FederatedPolicy(err) => write!(f, "federated policy engine error: {err}"),
            Self::TransportError(err) => write!(f, "swarm transport error: {err}"),
            Self::InvalidGradient(err) => write!(f, "invalid gradient vector: {err}"),
        }
    }
}

impl std::error::Error for DistributedSwarmError {}

impl From<FederatedPolicyError> for DistributedSwarmError {
    fn from(err: FederatedPolicyError) -> Self {
        Self::FederatedPolicy(err.to_string())
    }
}

// ============================================================================
// Distributed Swarm Message Framing
// ============================================================================

/// Wire payload types exchanged between distributed Pandora nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwarmPayload {
    /// Peer discovery and mutual authentication handshake.
    Handshake(MeshHandshake),
    /// Handshake acknowledgment with cluster epoch.
    HandshakeAck {
        peer_id: String,
        success: bool,
        cluster_epoch: u64,
    },
    /// Cross-machine candidate patch proposal for Byzantine verification.
    PatchProposal(DistributedPatchProposal),
    /// Byzantine consensus vote on a candidate patch.
    ConsensusVote(DistributedConsensusVote),
    /// Distributed federated policy gradient packet.
    PolicyGradient(DistributedGradientPacket),
    /// Federated policy update synchronization receipt.
    PolicyReceipt(FederatedUpdateReceipt),
}

/// Cryptographically signed envelope carrying swarm messages across network sockets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributedSwarmMessage {
    pub message_id: String,
    pub sender_peer_id: String,
    pub vector_clock: VectorClock,
    pub hop_count: u32,
    pub payload: SwarmPayload,
    pub signature: String,
}

impl DistributedSwarmMessage {
    /// Create a new unsigned message envelope.
    pub fn new(
        message_id: impl Into<String>,
        sender_peer_id: impl Into<String>,
        vector_clock: VectorClock,
        payload: SwarmPayload,
    ) -> Self {
        Self {
            message_id: message_id.into(),
            sender_peer_id: sender_peer_id.into(),
            vector_clock,
            hop_count: 0,
            payload,
            signature: String::new(),
        }
    }

    /// Compute canonical SHA-256 payload digest for signing and verification.
    pub fn payload_digest(&self) -> Result<[u8; 32], DistributedSwarmError> {
        let payload_json = serde_json::to_vec(&self.payload)
            .map_err(|e| DistributedSwarmError::SerializationError(e.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"pandora-swarm-msg:v1:");
        hasher.update(self.message_id.as_bytes());
        hasher.update(b":");
        hasher.update(self.sender_peer_id.as_bytes());
        hasher.update(b":");
        hasher.update(&payload_json);
        Ok(hasher.finalize().into())
    }

    /// Sign this message using the local node's Ed25519 identity.
    pub fn sign(&mut self, identity: &PeerIdentity) -> Result<(), DistributedSwarmError> {
        let digest = self.payload_digest()?;
        let sig = identity.signing_key.sign(&digest);
        self.signature = encode_hex(&sig.to_bytes());
        Ok(())
    }

    /// Verify this message's signature against the sender's public key hex string.
    pub fn verify(&self, public_key_hex: &str) -> Result<bool, DistributedSwarmError> {
        let pk_bytes = decode_hex_32(public_key_hex).ok_or_else(|| {
            DistributedSwarmError::InvalidPublicKey("invalid 32-byte hex public key".to_string())
        })?;
        let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
            .map_err(|e| DistributedSwarmError::InvalidPublicKey(e.to_string()))?;

        let sig_bytes = decode_hex_64(&self.signature).ok_or_else(|| {
            DistributedSwarmError::InvalidSignature("invalid 64-byte hex signature".to_string())
        })?;
        let signature = Signature::from_bytes(&sig_bytes);

        let digest = self.payload_digest()?;
        verifying_key
            .verify_strict(&digest, &signature)
            .map(|_| true)
            .map_err(|e| DistributedSwarmError::InvalidSignature(e.to_string()))
    }

    /// Check if this message causally precedes another message via vector clocks.
    pub fn causally_precedes(&self, other: &Self) -> bool {
        self.vector_clock.causally_precedes(&other.vector_clock)
    }

    /// Check if this message is concurrent with another message.
    pub fn is_concurrent_with(&self, other: &Self) -> bool {
        self.vector_clock.is_concurrent_with(&other.vector_clock)
    }
}

// ============================================================================
// Cross-Machine Byzantine Patch Proposal & Vote
// ============================================================================

/// Candidate code patch proposed across cluster workstations or CI runners.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistributedPatchProposal {
    pub proposal_id: String,
    pub proposer_node_id: String,
    pub author_agent: ExternalAgentKind,
    pub title: String,
    pub goal: String,
    pub diff_content: String,
    pub local_scores: ProposalScoreVector,
    pub patch_fingerprint: String,
    pub timestamp_ms: u64,
}

impl DistributedPatchProposal {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        proposal_id: impl Into<String>,
        proposer_node_id: impl Into<String>,
        author_agent: ExternalAgentKind,
        title: impl Into<String>,
        goal: impl Into<String>,
        diff_content: impl Into<String>,
        local_scores: ProposalScoreVector,
        timestamp_ms: u64,
    ) -> Self {
        let proposal_id = proposal_id.into();
        let proposer_node_id = proposer_node_id.into();
        let title = title.into();
        let goal = goal.into();
        let diff_content = diff_content.into();

        let mut hasher = Sha256::new();
        hasher.update(b"pandora-patch-fp:v1:");
        hasher.update(proposal_id.as_bytes());
        hasher.update(proposer_node_id.as_bytes());
        hasher.update(author_agent.identifier().as_bytes());
        hasher.update(diff_content.as_bytes());
        let patch_fingerprint = format!("{:x}", hasher.finalize());

        Self {
            proposal_id,
            proposer_node_id,
            author_agent,
            title,
            goal,
            diff_content,
            local_scores,
            patch_fingerprint,
            timestamp_ms,
        }
    }

    /// Canonical digest for signing proposal submission.
    pub fn proposal_digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"pandora-proposal:v1:");
        hasher.update(self.proposal_id.as_bytes());
        hasher.update(self.proposer_node_id.as_bytes());
        hasher.update(self.patch_fingerprint.as_bytes());
        hasher.update(self.timestamp_ms.to_le_bytes());
        hasher.finalize().into()
    }
}

/// Cryptographically signed vote cast by a cluster node on a candidate patch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistributedConsensusVote {
    pub proposal_id: String,
    pub voter_node_id: String,
    pub approve: bool,
    pub verified_scores: ProposalScoreVector,
    pub signature: String,
    pub timestamp_ms: u64,
}

impl DistributedConsensusVote {
    pub fn new(
        proposal_id: impl Into<String>,
        voter_node_id: impl Into<String>,
        approve: bool,
        verified_scores: ProposalScoreVector,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            proposal_id: proposal_id.into(),
            voter_node_id: voter_node_id.into(),
            approve,
            verified_scores,
            signature: String::new(),
            timestamp_ms,
        }
    }

    /// Compute canonical digest for signing the vote.
    pub fn vote_digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"pandora-vote:v1:");
        hasher.update(self.proposal_id.as_bytes());
        hasher.update(self.voter_node_id.as_bytes());
        hasher.update(if self.approve {
            b"approve".as_slice()
        } else {
            b"reject".as_slice()
        });
        for c in self.verified_scores.as_coords() {
            hasher.update(c.to_le_bytes());
        }
        hasher.update(self.timestamp_ms.to_le_bytes());
        hasher.finalize().into()
    }

    /// Sign this vote with the node's Ed25519 identity.
    pub fn sign_vote(&mut self, identity: &PeerIdentity) {
        let digest = self.vote_digest();
        let sig = identity.signing_key.sign(&digest);
        self.signature = encode_hex(&sig.to_bytes());
    }

    /// Verify vote signature against the voter's public key.
    pub fn verify_vote(&self, public_key_hex: &str) -> Result<bool, DistributedSwarmError> {
        let pk_bytes = decode_hex_32(public_key_hex).ok_or_else(|| {
            DistributedSwarmError::InvalidPublicKey("invalid 32-byte hex public key".to_string())
        })?;
        let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
            .map_err(|e| DistributedSwarmError::InvalidPublicKey(e.to_string()))?;

        let sig_bytes = decode_hex_64(&self.signature).ok_or_else(|| {
            DistributedSwarmError::InvalidSignature("invalid 64-byte hex signature".to_string())
        })?;
        let signature = Signature::from_bytes(&sig_bytes);

        let digest = self.vote_digest();
        verifying_key
            .verify_strict(&digest, &signature)
            .map(|_| true)
            .map_err(|e| DistributedSwarmError::InvalidSignature(e.to_string()))
    }
}

/// Final consensus decision status on a distributed patch proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DistributedConsensusDecision {
    Pending {
        current_approvals: usize,
        current_rejections: usize,
        quorum_needed: usize,
    },
    Approved(CertifiedPatchRecord),
    Rejected {
        reason: String,
        approvals: usize,
        rejections: usize,
    },
}

impl DistributedConsensusDecision {
    pub fn is_approved(&self) -> bool {
        matches!(self, Self::Approved(_))
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. })
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected { .. })
    }
}

// ============================================================================
// Cross-Machine Byzantine Patch Consensus Engine
// ============================================================================

/// Cross-machine Byzantine consensus engine orchestrating patch certification
/// across distributed developer workstations and CI runners under Theorem 47.
#[derive(Debug, Clone)]
pub struct DistributedSwarmConsensusEngine {
    pub local_peer_id: String,
    pub total_nodes: usize,
    pub byzantine_tolerance: usize,
    pub quorum_threshold: usize,
    pub proposals: HashMap<String, DistributedPatchProposal>,
    pub votes: HashMap<String, HashMap<String, DistributedConsensusVote>>,
    pub decisions: HashMap<String, DistributedConsensusDecision>,
    pub min_viability_threshold: f64,
    pub max_centroid_divergence: f64,
}

impl DistributedSwarmConsensusEngine {
    /// Initialize a new distributed consensus engine.
    ///
    /// # Safety and Invariants (Theorem 47 Lemma 47.1)
    /// Enforces $N \ge 3f + 1$ and calculates quorum threshold $Q = \lfloor (P + f)/2 \rfloor + 1$
    /// (equivalently $\lceil (P + f + 1)/2 \rceil$), guaranteeing that any two quorums
    /// intersect by at least $|Q_1 \cap Q_2| \ge f + 1$ nodes, ensuring at least one honest
    /// node in the intersection and strictly preventing split-brain forks for all valid cluster sizes.
    pub fn new(
        local_peer_id: impl Into<String>,
        total_nodes: usize,
        byzantine_tolerance: usize,
    ) -> Result<Self, DistributedSwarmError> {
        if total_nodes < 3 * byzantine_tolerance + 1 {
            return Err(DistributedSwarmError::InvalidBftParameters {
                n: total_nodes,
                f: byzantine_tolerance,
            });
        }
        let quorum_threshold = (total_nodes + byzantine_tolerance) / 2 + 1;
        Ok(Self {
            local_peer_id: local_peer_id.into(),
            total_nodes,
            byzantine_tolerance,
            quorum_threshold,
            proposals: HashMap::new(),
            votes: HashMap::new(),
            decisions: HashMap::new(),
            min_viability_threshold: 0.50,
            max_centroid_divergence: 0.45,
        })
    }

    /// Register a newly proposed candidate patch.
    pub fn submit_proposal(
        &mut self,
        proposal: DistributedPatchProposal,
    ) -> Result<String, DistributedSwarmError> {
        let id = proposal.proposal_id.clone();
        if self.proposals.contains_key(&id) {
            return Err(DistributedSwarmError::ProposalAlreadyExists(id));
        }
        self.proposals.insert(id.clone(), proposal);
        self.votes.entry(id.clone()).or_default();
        self.decisions.insert(
            id.clone(),
            DistributedConsensusDecision::Pending {
                current_approvals: 0,
                current_rejections: 0,
                quorum_needed: self.quorum_threshold,
            },
        );
        Ok(id)
    }

    /// Record a verified vote with strict equivocation detection.
    ///
    /// Detects Byzantine double-voting if a node casts conflicting approvals or
    /// divergent score vectors ($\|\mathbf{s}_1 - \mathbf{s}_2\|_2 > 10^{-4}$).
    pub fn record_vote(
        &mut self,
        vote: DistributedConsensusVote,
    ) -> Result<DistributedConsensusDecision, DistributedSwarmError> {
        let proposal_votes = self.votes.entry(vote.proposal_id.clone()).or_default();

        if let Some(existing) = proposal_votes.get(&vote.voter_node_id) {
            let score_divergence = existing.verified_scores.distance(&vote.verified_scores);
            if existing.approve != vote.approve || score_divergence > 1e-4 {
                return Err(DistributedSwarmError::EquivocationDetected {
                    voter: vote.voter_node_id,
                    proposal: vote.proposal_id,
                });
            }
            // Idempotent re-submission of identical vote
            return Ok(self.tally(&vote.proposal_id));
        }

        proposal_votes.insert(vote.voter_node_id.clone(), vote.clone());
        Ok(self.tally(&vote.proposal_id))
    }

    /// Tally registered votes, compute coordinate-wise trimmed mean centroid,
    /// filter Byzantine score outliers ($\|\mathbf{s}_i - \mathbf{c}_\alpha\|_2 > 0.45$),
    /// and generate certified patch record if quorum $Q = 2f + 1$ is satisfied.
    pub fn tally(&mut self, proposal_id: &str) -> DistributedConsensusDecision {
        let proposal = match self.proposals.get(proposal_id) {
            Some(p) => p.clone(),
            None => {
                return DistributedConsensusDecision::Rejected {
                    reason: "Proposal not found".to_string(),
                    approvals: 0,
                    rejections: 0,
                };
            }
        };

        let proposal_votes = match self.votes.get(proposal_id) {
            Some(v) => v,
            None => {
                return DistributedConsensusDecision::Pending {
                    current_approvals: 0,
                    current_rejections: 0,
                    quorum_needed: self.quorum_threshold,
                };
            }
        };

        let mut approvals = 0;
        let mut rejections = 0;
        let mut approval_votes: Vec<&DistributedConsensusVote> = Vec::new();

        for vote in proposal_votes.values() {
            if vote.approve {
                approvals += 1;
                approval_votes.push(vote);
            } else {
                rejections += 1;
            }
        }

        // BFT decision rule: check if approvals reach supermajority quorum Q = 2f + 1
        if approvals >= self.quorum_threshold {
            // Step 1: Pre-filter votes failing basic invariant viability
            let viable_votes: Vec<&&DistributedConsensusVote> = approval_votes
                .iter()
                .filter(|v| {
                    v.verified_scores.composite_score() >= self.min_viability_threshold
                        && v.verified_scores.invariant_compliance >= 0.40
                })
                .collect();

            if viable_votes.len() < self.quorum_threshold - self.byzantine_tolerance {
                let decision = DistributedConsensusDecision::Rejected {
                    reason: "insufficient viable non-divergent votes meeting invariant thresholds"
                        .to_string(),
                    approvals,
                    rejections,
                };
                self.decisions
                    .insert(proposal_id.to_string(), decision.clone());
                return decision;
            }

            // Step 2: Compute coordinate-wise trimmed mean centroid across viable approval scores
            let centroid = self.compute_trimmed_centroid(&viable_votes);

            // Step 3: Filter Byzantine outlier score vectors deviating beyond delta_max = 0.45
            let mut honest_votes = Vec::new();
            let mut byzantine_count = 0;
            let mut verdicts: BTreeMap<String, ProposalVerdict> = BTreeMap::new();

            for &vote in &approval_votes {
                let dist = vote.verified_scores.distance(&centroid);
                if dist > self.max_centroid_divergence {
                    verdicts.insert(
                        vote.voter_node_id.clone(),
                        ProposalVerdict::FilteredByzantine,
                    );
                    byzantine_count += 1;
                } else {
                    verdicts.insert(vote.voter_node_id.clone(), ProposalVerdict::ConsensusQuorum);
                    honest_votes.push(vote);
                }
            }

            // At least f + 1 honest nodes must affirm consensus within delta_max
            if honest_votes.len() < self.quorum_threshold - self.byzantine_tolerance {
                let decision = DistributedConsensusDecision::Rejected {
                    reason: format!(
                        "excessive Byzantine score divergence: {byzantine_count} votes filtered"
                    ),
                    approvals,
                    rejections,
                };
                self.decisions
                    .insert(proposal_id.to_string(), decision.clone());
                return decision;
            }

            // Step 4: Elect winning score (centroid of honest quorum)
            let honest_count = honest_votes.len();

            // Step 5: Generate cryptographic consensus hash and certification receipt
            let mut hasher = Sha256::new();
            hasher.update(b"PANDORA_CONSENSUS_V1:");
            hasher.update(proposal.patch_fingerprint.as_bytes());
            hasher.update(proposal.timestamp_ms.to_be_bytes());
            hasher.update((honest_count as u32).to_be_bytes());
            for v in &honest_votes {
                hasher.update(v.voter_node_id.as_bytes());
                hasher.update(if v.approve { b":1:" } else { b":0:" });
            }
            let consensus_hash = format!("{:x}", hasher.finalize());

            // Certified patch digest: PANDORA_PATCH_CERT_V1:<sha256>
            let mut cert_hasher = Sha256::new();
            cert_hasher.update(b"PANDORA_PATCH_CERT_V1:");
            cert_hasher.update(proposal.goal.as_bytes());
            cert_hasher.update(proposal.proposal_id.as_bytes());
            cert_hasher.update(proposal.author_agent.identifier().as_bytes());
            cert_hasher.update(proposal.patch_fingerprint.as_bytes());
            cert_hasher.update(consensus_hash.as_bytes());
            cert_hasher.update((honest_count as f64 / self.total_nodes as f64).to_le_bytes());
            let certificate_digest = format!("PANDORA_PATCH_CERT_V1:{:x}", cert_hasher.finalize());

            let cert_id = format!("cert-{}", &certificate_digest[21..37]);

            let certified_record = CertifiedPatchRecord {
                certificate_id: cert_id,
                task_goal: proposal.goal.clone(),
                winning_proposal_id: proposal.proposal_id.clone(),
                winning_agent: proposal.author_agent,
                diff_content: proposal.diff_content.clone(),
                scores: centroid,
                consensus_hash,
                certificate_digest,
                timestamp_ms: proposal.timestamp_ms,
            };

            let decision = DistributedConsensusDecision::Approved(certified_record);
            self.decisions
                .insert(proposal_id.to_string(), decision.clone());
            decision
        } else if rejections > self.total_nodes - self.quorum_threshold {
            // Impossible for remaining uncast votes to ever reach Q approvals
            let decision = DistributedConsensusDecision::Rejected {
                reason: "rejections exceeded maximum allowable threshold to form quorum"
                    .to_string(),
                approvals,
                rejections,
            };
            self.decisions
                .insert(proposal_id.to_string(), decision.clone());
            decision
        } else {
            let decision = DistributedConsensusDecision::Pending {
                current_approvals: approvals,
                current_rejections: rejections,
                quorum_needed: self.quorum_threshold,
            };
            self.decisions
                .insert(proposal_id.to_string(), decision.clone());
            decision
        }
    }

    /// Retrieve an existing certified patch record if consensus has committed.
    pub fn certify_patch(
        &self,
        proposal_id: &str,
    ) -> Result<CertifiedPatchRecord, DistributedSwarmError> {
        match self.decisions.get(proposal_id) {
            Some(DistributedConsensusDecision::Approved(cert)) => Ok(cert.clone()),
            Some(DistributedConsensusDecision::Pending {
                current_approvals,
                quorum_needed,
                ..
            }) => Err(DistributedSwarmError::ConsensusPending {
                proposal: proposal_id.to_string(),
                approvals: *current_approvals,
                needed: *quorum_needed,
            }),
            Some(DistributedConsensusDecision::Rejected { reason, .. }) => {
                Err(DistributedSwarmError::ConsensusRejected {
                    proposal: proposal_id.to_string(),
                    reason: reason.clone(),
                })
            }
            None => Err(DistributedSwarmError::ProposalNotFound(
                proposal_id.to_string(),
            )),
        }
    }

    /// Coordinate-wise trimmed mean centroid across reported 4D score vectors.
    fn compute_trimmed_centroid(
        &self,
        votes: &[&&DistributedConsensusVote],
    ) -> ProposalScoreVector {
        let m = votes.len();
        if m == 0 {
            return ProposalScoreVector::new(0.5, 0.5, 0.5, 0.5);
        }
        if m == 1 {
            return votes[0].verified_scores;
        }

        let trim_k = self.byzantine_tolerance.min(m.saturating_sub(1) / 2);
        let mut trimmed_coords = [0.0; 4];

        for (d, coord) in trimmed_coords.iter_mut().enumerate() {
            let mut values: Vec<f64> = votes
                .iter()
                .map(|v| v.verified_scores.as_coords()[d])
                .collect();
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            let slice = &values[trim_k..(m - trim_k)];
            let sum: f64 = slice.iter().sum();
            *coord = sum / (slice.len() as f64);
        }

        ProposalScoreVector::from_coords(trimmed_coords)
    }
}

// ============================================================================
// Distributed Federated Policy Gradient Dissemination
// ============================================================================

/// Network packet carrying local empirical policy gradients across swarm nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistributedGradientPacket {
    pub packet_id: String,
    pub source_node_id: String,
    pub epoch: u64,
    pub gradient: PolicyGradientVector,
    pub signature: String,
    pub timestamp_ms: u64,
}

impl DistributedGradientPacket {
    pub fn new(
        packet_id: impl Into<String>,
        source_node_id: impl Into<String>,
        epoch: u64,
        gradient: PolicyGradientVector,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            packet_id: packet_id.into(),
            source_node_id: source_node_id.into(),
            epoch,
            gradient,
            signature: String::new(),
            timestamp_ms,
        }
    }

    /// Canonical digest for signing the gradient packet.
    pub fn packet_digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"pandora-gradient:v1:");
        hasher.update(self.packet_id.as_bytes());
        hasher.update(self.source_node_id.as_bytes());
        hasher.update(self.epoch.to_le_bytes());
        hasher.update(self.gradient.agent.identifier().as_bytes());
        for g in &self.gradient.gradients {
            hasher.update(g.to_le_bytes());
        }
        hasher.update(self.gradient.loss.to_le_bytes());
        hasher.update((self.gradient.sample_count as u64).to_le_bytes());
        hasher.update(self.timestamp_ms.to_le_bytes());
        hasher.finalize().into()
    }

    /// Sign this packet with the node's Ed25519 identity.
    pub fn sign_packet(&mut self, identity: &PeerIdentity) {
        let digest = self.packet_digest();
        let sig = identity.signing_key.sign(&digest);
        self.signature = encode_hex(&sig.to_bytes());
    }

    /// Verify signature against the sender's public key hex string.
    pub fn verify_packet(&self, public_key_hex: &str) -> Result<bool, DistributedSwarmError> {
        let pk_bytes = decode_hex_32(public_key_hex).ok_or_else(|| {
            DistributedSwarmError::InvalidPublicKey("invalid 32-byte hex public key".to_string())
        })?;
        let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
            .map_err(|e| DistributedSwarmError::InvalidPublicKey(e.to_string()))?;

        let sig_bytes = decode_hex_64(&self.signature).ok_or_else(|| {
            DistributedSwarmError::InvalidSignature("invalid 64-byte hex signature".to_string())
        })?;
        let signature = Signature::from_bytes(&sig_bytes);

        let digest = self.packet_digest();
        verifying_key
            .verify_strict(&digest, &signature)
            .map(|_| true)
            .map_err(|e| DistributedSwarmError::InvalidSignature(e.to_string()))
    }
}

/// Distributed engine orchestrating federated policy gradient dissemination
/// and Byzantine-resilient aggregation across a cluster of Pandora nodes.
#[derive(Debug, Clone)]
pub struct DistributedFederatedEngine {
    pub local_node_id: String,
    pub current_policy: [f64; 6],
    pub federated_engine: FederatedPolicyEngine,
    pub packets: HashMap<u64, Vec<DistributedGradientPacket>>,
    pub receipts: HashMap<u64, FederatedUpdateReceipt>,
}

impl DistributedFederatedEngine {
    pub fn new(local_node_id: impl Into<String>, initial_policy: [f64; 6]) -> Self {
        let mut federated_engine = FederatedPolicyEngine::new(
            DEFAULT_LEARNING_RATE,
            DEFAULT_MOMENTUM,
            DEFAULT_MAX_GRADIENT_NORM,
        );
        federated_engine.current_policy = initial_policy;
        Self {
            local_node_id: local_node_id.into(),
            current_policy: initial_policy,
            federated_engine,
            packets: HashMap::new(),
            receipts: HashMap::new(),
        }
    }

    /// Create and sign a gradient packet for broadcast to the swarm cluster.
    pub fn broadcast_gradient(
        &self,
        identity: &PeerIdentity,
        epoch: u64,
        gradient: PolicyGradientVector,
        timestamp_ms: u64,
    ) -> Result<DistributedGradientPacket, DistributedSwarmError> {
        let packet_id = format!("grad-{}-{}-{}", self.local_node_id, epoch, timestamp_ms);
        let mut packet = DistributedGradientPacket::new(
            packet_id,
            &self.local_node_id,
            epoch,
            gradient,
            timestamp_ms,
        );
        packet.sign_packet(identity);
        Ok(packet)
    }

    /// Ingest an incoming gradient packet with optional signature verification.
    pub fn receive_gradient_packet(
        &mut self,
        packet: DistributedGradientPacket,
        public_key_hex: Option<&str>,
    ) -> Result<(), DistributedSwarmError> {
        if let Some(pk) = public_key_hex {
            packet.verify_packet(pk)?;
        }
        for (i, &g) in packet.gradient.gradients.iter().enumerate() {
            if g.is_nan() || g.is_infinite() {
                return Err(DistributedSwarmError::InvalidGradient(format!(
                    "gradient component {i} is non-finite: {g}"
                )));
            }
        }
        let epoch = packet.epoch;
        self.packets.entry(epoch).or_default().push(packet);
        Ok(())
    }

    /// Aggregate cluster gradients for an epoch using coordinate-wise trimmed mean
    /// and apply momentum-accelerated gradient descent step under Theorem 47.
    pub fn aggregate_cluster_gradients(
        &mut self,
        epoch: u64,
        timestamp_ms: u64,
    ) -> Result<FederatedUpdateReceipt, DistributedSwarmError> {
        let epoch_packets = self
            .packets
            .get(&epoch)
            .ok_or(DistributedSwarmError::EmptyGradients)?;

        if epoch_packets.is_empty() {
            return Err(DistributedSwarmError::EmptyGradients);
        }

        let worker_gradients: Vec<PolicyGradientVector> =
            epoch_packets.iter().map(|p| p.gradient.clone()).collect();

        let receipt = self
            .federated_engine
            .step(&worker_gradients, timestamp_ms)
            .map_err(|e| DistributedSwarmError::FederatedPolicy(e.to_string()))?;

        self.current_policy = self.federated_engine.current_policy;
        self.receipts.insert(epoch, receipt.clone());
        Ok(receipt)
    }

    /// Synchronize local policy state from an external validated update receipt.
    pub fn sync_receipt(
        &mut self,
        receipt: FederatedUpdateReceipt,
    ) -> Result<(), DistributedSwarmError> {
        self.receipts.insert(receipt.step_index, receipt);
        Ok(())
    }
}

// ============================================================================
// Distributed Swarm Node (Unified Mesh State)
// ============================================================================

/// Metadata of a discovered or connected peer in the swarm mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmPeer {
    pub peer_id: String,
    pub listen_addr: String,
    pub public_key: String,
    pub last_seen: u64,
    pub connected: bool,
}

/// Stateful node managing the local Pandora instance's cross-node swarm mesh.
pub struct DistributedSwarmNode {
    pub identity: PeerIdentity,
    pub listen_addr: String,
    pub peers: HashMap<String, SwarmPeer>,
    pub consensus: DistributedSwarmConsensusEngine,
    pub federated: DistributedFederatedEngine,
    pub vector_clock: VectorClock,
    pub epoch: u64,
}

impl fmt::Debug for DistributedSwarmNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DistributedSwarmNode")
            .field("peer_id", &self.identity.peer_id)
            .field("listen_addr", &self.listen_addr)
            .field("peers", &self.peers)
            .field("consensus", &self.consensus)
            .field("federated", &self.federated)
            .field("vector_clock", &self.vector_clock)
            .field("epoch", &self.epoch)
            .finish()
    }
}

impl DistributedSwarmNode {
    /// Initialize a new swarm node with Ed25519 identity and BFT parameters.
    pub fn new(
        identity: PeerIdentity,
        listen_addr: impl Into<String>,
        total_nodes: usize,
        byzantine_tolerance: usize,
    ) -> Result<Self, DistributedSwarmError> {
        let listen_addr = listen_addr.into();
        let peer_id = identity.peer_id.clone();
        let consensus =
            DistributedSwarmConsensusEngine::new(&peer_id, total_nodes, byzantine_tolerance)?;
        let federated = DistributedFederatedEngine::new(&peer_id, [0.5; 6]);
        let mut vector_clock = VectorClock::new();
        vector_clock.increment(&peer_id);

        Ok(Self {
            identity,
            listen_addr,
            peers: HashMap::new(),
            consensus,
            federated,
            vector_clock,
            epoch: 1,
        })
    }

    /// Produce a cryptographically signed handshake for connecting to a remote peer.
    pub fn create_handshake(&self, timestamp_ms: u64) -> MeshHandshake {
        let mut hs = MeshHandshake {
            peer_id: self.identity.peer_id.clone(),
            public_key: self.identity.public_key_hex(),
            listen_addr: self.listen_addr.clone(),
            version: "v3.0.11-phase24".to_string(),
            capabilities: vec![
                "byzantine_consensus".to_string(),
                "federated_gradients".to_string(),
                "git_canary".to_string(),
            ],
            timestamp: timestamp_ms / 1000,
            signature: String::new(),
        };
        hs.signature = self
            .identity
            .sign_digest(&String::from_utf8_lossy(&hs.payload_bytes()));
        hs
    }

    /// Process an incoming peer handshake, verify its signature, and record the peer.
    pub fn handle_handshake(
        &mut self,
        handshake: MeshHandshake,
        timestamp_ms: u64,
    ) -> Result<DistributedSwarmMessage, DistributedSwarmError> {
        let valid = handshake
            .verify()
            .map_err(|e| DistributedSwarmError::InvalidSignature(e.to_string()))?;
        if !valid {
            return Err(DistributedSwarmError::InvalidSignature(
                "remote peer handshake signature check failed".to_string(),
            ));
        }

        let peer = SwarmPeer {
            peer_id: handshake.peer_id.clone(),
            listen_addr: handshake.listen_addr.clone(),
            public_key: handshake.public_key.clone(),
            last_seen: timestamp_ms,
            connected: true,
        };
        self.peers.insert(handshake.peer_id.clone(), peer);
        self.vector_clock.increment(&self.identity.peer_id);

        let ack = SwarmPayload::HandshakeAck {
            peer_id: self.identity.peer_id.clone(),
            success: true,
            cluster_epoch: self.epoch,
        };

        let msg_id = format!("ack-{}-{}", self.identity.peer_id, timestamp_ms);
        let mut msg = DistributedSwarmMessage::new(
            msg_id,
            &self.identity.peer_id,
            self.vector_clock.clone(),
            ack,
        );
        msg.sign(&self.identity)?;
        Ok(msg)
    }

    /// Propose a candidate code patch to the distributed cluster.
    pub fn propose_patch(
        &mut self,
        goal: &str,
        diff_content: &str,
        author_agent: ExternalAgentKind,
        local_scores: ProposalScoreVector,
        timestamp_ms: u64,
    ) -> Result<DistributedSwarmMessage, DistributedSwarmError> {
        let proposal_id = format!("prop-{}-{}", self.identity.peer_id, timestamp_ms);
        let proposal = DistributedPatchProposal::new(
            &proposal_id,
            &self.identity.peer_id,
            author_agent,
            format!("Patch for {goal}"),
            goal,
            diff_content,
            local_scores,
            timestamp_ms,
        );

        self.consensus.submit_proposal(proposal.clone())?;
        self.vector_clock.increment(&self.identity.peer_id);

        let payload = SwarmPayload::PatchProposal(proposal);
        let msg_id = format!("msg-prop-{timestamp_ms}");
        let mut msg = DistributedSwarmMessage::new(
            msg_id,
            &self.identity.peer_id,
            self.vector_clock.clone(),
            payload,
        );
        msg.sign(&self.identity)?;
        Ok(msg)
    }

    /// Cast a signed consensus vote on a candidate patch.
    pub fn vote_patch(
        &mut self,
        proposal_id: &str,
        approve: bool,
        scores: ProposalScoreVector,
        timestamp_ms: u64,
    ) -> Result<DistributedSwarmMessage, DistributedSwarmError> {
        let mut vote = DistributedConsensusVote::new(
            proposal_id,
            &self.identity.peer_id,
            approve,
            scores,
            timestamp_ms,
        );
        vote.sign_vote(&self.identity);

        self.consensus.record_vote(vote.clone())?;
        self.vector_clock.increment(&self.identity.peer_id);

        let payload = SwarmPayload::ConsensusVote(vote);
        let msg_id = format!("msg-vote-{proposal_id}-{timestamp_ms}");
        let mut msg = DistributedSwarmMessage::new(
            msg_id,
            &self.identity.peer_id,
            self.vector_clock.clone(),
            payload,
        );
        msg.sign(&self.identity)?;
        Ok(msg)
    }

    /// Disseminate local policy gradient to cluster peers.
    pub fn submit_policy_gradient(
        &mut self,
        gradient: PolicyGradientVector,
        timestamp_ms: u64,
    ) -> Result<DistributedSwarmMessage, DistributedSwarmError> {
        let packet = self.federated.broadcast_gradient(
            &self.identity,
            self.epoch,
            gradient,
            timestamp_ms,
        )?;
        self.federated
            .receive_gradient_packet(packet.clone(), None)?;
        self.vector_clock.increment(&self.identity.peer_id);

        let payload = SwarmPayload::PolicyGradient(packet);
        let msg_id = format!("msg-grad-{}-{timestamp_ms}", self.epoch);
        let mut msg = DistributedSwarmMessage::new(
            msg_id,
            &self.identity.peer_id,
            self.vector_clock.clone(),
            payload,
        );
        msg.sign(&self.identity)?;
        Ok(msg)
    }

    /// Aggregate cluster gradients and step the global policy parameter vector.
    pub fn sync_gradients(
        &mut self,
        timestamp_ms: u64,
    ) -> Result<FederatedUpdateReceipt, DistributedSwarmError> {
        let receipt = self
            .federated
            .aggregate_cluster_gradients(self.epoch, timestamp_ms)?;
        self.epoch += 1;
        self.vector_clock.increment(&self.identity.peer_id);
        Ok(receipt)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swarm_message_signing_and_verification() {
        let identity = PeerIdentity::new_random();
        let mut clock = VectorClock::new();
        clock.increment(&identity.peer_id);

        let ack = SwarmPayload::HandshakeAck {
            peer_id: identity.peer_id.clone(),
            success: true,
            cluster_epoch: 1,
        };

        let mut msg = DistributedSwarmMessage::new("msg-1", &identity.peer_id, clock, ack);
        msg.sign(&identity).unwrap();

        assert!(!msg.signature.is_empty());
        assert!(msg.verify(&identity.public_key_hex()).unwrap());

        // Tamper test
        let mut tampered = msg.clone();
        tampered.payload = SwarmPayload::HandshakeAck {
            peer_id: identity.peer_id.clone(),
            success: false,
            cluster_epoch: 1,
        };
        assert!(tampered.verify(&identity.public_key_hex()).is_err());
    }

    #[test]
    fn test_bft_quorum_consensus_lifecycle() {
        // N = 4, f = 1 => Q = 3
        let mut engine = DistributedSwarmConsensusEngine::new("node-0", 4, 1).unwrap();
        assert_eq!(engine.quorum_threshold, 3);

        let scores = ProposalScoreVector::new(0.95, 0.90, 0.85, 0.95);
        let proposal = DistributedPatchProposal::new(
            "prop-1",
            "node-0",
            ExternalAgentKind::Agy,
            "Fix buffer overflow",
            "Correct memory safety",
            "diff --git a/src b/src",
            scores,
            1000,
        );

        engine.submit_proposal(proposal).unwrap();

        // Node 0 votes approve
        let v0 = DistributedConsensusVote::new("prop-1", "node-0", true, scores, 1005);
        let dec0 = engine.record_vote(v0).unwrap();
        assert!(dec0.is_pending());

        // Node 1 votes approve
        let v1 = DistributedConsensusVote::new("prop-1", "node-1", true, scores, 1010);
        let dec1 = engine.record_vote(v1).unwrap();
        assert!(dec1.is_pending());

        // Node 2 votes approve -> reaching Q = 3
        let v2 = DistributedConsensusVote::new("prop-1", "node-2", true, scores, 1015);
        let dec2 = engine.record_vote(v2).unwrap();

        match dec2 {
            DistributedConsensusDecision::Approved(cert) => {
                assert_eq!(cert.winning_proposal_id, "prop-1");
                assert!(
                    cert.certificate_digest
                        .starts_with("PANDORA_PATCH_CERT_V1:")
                );
            }
            _ => panic!("Expected Approved decision once quorum is reached"),
        }
    }

    #[test]
    fn test_equivocation_detection() {
        let mut engine = DistributedSwarmConsensusEngine::new("node-0", 4, 1).unwrap();
        let scores = ProposalScoreVector::new(0.90, 0.90, 0.90, 0.90);
        let proposal = DistributedPatchProposal::new(
            "prop-eq",
            "node-0",
            ExternalAgentKind::ClaudeCode,
            "Refactor AST",
            "Simplify tree",
            "diff",
            scores,
            2000,
        );
        engine.submit_proposal(proposal).unwrap();

        let v1 = DistributedConsensusVote::new("prop-eq", "node-byz", true, scores, 2010);
        engine.record_vote(v1).unwrap();

        // Same voter sends conflicting vote (false instead of true)
        let v1_conflicting =
            DistributedConsensusVote::new("prop-eq", "node-byz", false, scores, 2020);
        let err = engine.record_vote(v1_conflicting).unwrap_err();
        assert!(matches!(
            err,
            DistributedSwarmError::EquivocationDetected { .. }
        ));
    }

    #[test]
    fn test_byzantine_score_outlier_filtering() {
        // N = 4, f = 1 => Q = 3
        let mut engine = DistributedSwarmConsensusEngine::new("node-0", 4, 1).unwrap();
        let honest_scores = ProposalScoreVector::new(0.95, 0.92, 0.88, 0.95);
        let proposal = DistributedPatchProposal::new(
            "prop-filter",
            "node-0",
            ExternalAgentKind::Agy,
            "Title",
            "Goal",
            "diff",
            honest_scores,
            3000,
        );
        engine.submit_proposal(proposal).unwrap();

        // 3 honest nodes vote with close scores
        engine
            .record_vote(DistributedConsensusVote::new(
                "prop-filter",
                "node-0",
                true,
                honest_scores,
                3010,
            ))
            .unwrap();
        engine
            .record_vote(DistributedConsensusVote::new(
                "prop-filter",
                "node-1",
                true,
                honest_scores,
                3011,
            ))
            .unwrap();

        // Byzantine node votes with divergent score (> 0.45 away)
        let byz_scores = ProposalScoreVector::new(0.10, 0.20, 0.10, 0.20);
        engine
            .record_vote(DistributedConsensusVote::new(
                "prop-filter",
                "node-byz",
                true,
                byz_scores,
                3012,
            ))
            .unwrap();

        // 3rd honest node votes approve, bringing total approvals to 4
        let dec = engine
            .record_vote(DistributedConsensusVote::new(
                "prop-filter",
                "node-2",
                true,
                honest_scores,
                3013,
            ))
            .unwrap();

        match dec {
            DistributedConsensusDecision::Approved(cert) => {
                // Centroid must be close to honest scores, unaffected by byz_scores
                assert!(cert.scores.distance(&honest_scores) < 0.15);
            }
            _ => panic!("Expected Approved decision with filtered Byzantine score"),
        }
    }

    #[test]
    fn test_general_cluster_quorum_thresholds_p_greater_than_3f_plus_1() {
        // Case 1: P = 5, f = 1 => Q = (5 + 1)/2 + 1 = 4
        let mut engine_5_1 = DistributedSwarmConsensusEngine::new("node-0", 5, 1)
            .expect("valid BFT parameters (5 >= 3*1 + 1)");
        assert_eq!(engine_5_1.quorum_threshold, 4);

        let scores = ProposalScoreVector::new(0.92, 0.90, 0.88, 0.94);
        let proposal_5 = DistributedPatchProposal::new(
            "patch-5-1",
            "node-0",
            ExternalAgentKind::Agy,
            "Fix buffer boundary check",
            "Bound checking in slice decoding",
            "--- a/slice.rs\n+++ b/slice.rs",
            scores,
            8000,
        );
        engine_5_1.submit_proposal(proposal_5).unwrap();

        // 3 approvals (previously would have committed under Q=2f+1=3)
        // Under corrected Q=4, 3 approvals must remain PENDING
        for i in 0..3 {
            let dec = engine_5_1
                .record_vote(DistributedConsensusVote::new(
                    "patch-5-1",
                    format!("peer-{i}"),
                    true,
                    scores,
                    8010 + i as u64,
                ))
                .unwrap();
            assert!(dec.is_pending());
        }

        // 4th approval reaches Q = 4 => Approved!
        let dec4 = engine_5_1
            .record_vote(DistributedConsensusVote::new(
                "patch-5-1",
                "peer-3",
                true,
                scores,
                8020,
            ))
            .unwrap();
        assert!(dec4.is_approved());

        // Case 2: P = 6, f = 1 => Q = (6 + 1)/2 + 1 = 4
        let engine_6_1 = DistributedSwarmConsensusEngine::new("node-0", 6, 1).unwrap();
        assert_eq!(engine_6_1.quorum_threshold, 4);

        // Case 3: P = 8, f = 2 => Q = (8 + 2)/2 + 1 = 6
        let engine_8_2 = DistributedSwarmConsensusEngine::new("node-0", 8, 2).unwrap();
        assert_eq!(engine_8_2.quorum_threshold, 6);
    }

    #[test]
    fn test_minimal_quorum_byzantine_trimming_m3_f1() {
        let mut engine_4 = DistributedSwarmConsensusEngine::new("coord", 4, 1).unwrap();
        let honest_s1 = ProposalScoreVector::new(0.90, 0.90, 0.90, 0.90);
        let honest_s2 = ProposalScoreVector::new(0.92, 0.92, 0.92, 0.92);
        let byz_outlier = ProposalScoreVector::new(0.40, 0.40, 0.40, 0.90);

        let prop = DistributedPatchProposal::new(
            "prop-trim-min",
            "coord",
            ExternalAgentKind::Agy,
            "Goal",
            "Desc",
            "diff",
            honest_s1,
            1000,
        );
        engine_4.submit_proposal(prop).unwrap();
        engine_4
            .record_vote(DistributedConsensusVote::new(
                "prop-trim-min",
                "h1",
                true,
                honest_s1,
                1010,
            ))
            .unwrap();
        engine_4
            .record_vote(DistributedConsensusVote::new(
                "prop-trim-min",
                "h2",
                true,
                honest_s2,
                1020,
            ))
            .unwrap();
        let dec = engine_4
            .record_vote(DistributedConsensusVote::new(
                "prop-trim-min",
                "byz",
                true,
                byz_outlier,
                1030,
            ))
            .unwrap();

        assert!(dec.is_approved());
        if let DistributedConsensusDecision::Approved(cert) = dec {
            let drift = cert
                .scores
                .distance(&ProposalScoreVector::new(0.91, 0.91, 0.91, 0.91));
            assert!(
                drift <= 0.025,
                "Minimal quorum m=3 must trim outlier! Drift was {drift}"
            );
        }
    }
}
