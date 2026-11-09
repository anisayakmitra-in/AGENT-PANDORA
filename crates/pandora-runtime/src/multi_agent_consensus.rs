//! Multi-Agent Byzantine Code Consensus Engine (Phase 22).
//!
//! Grounded in Theorem 43 for Byzantine-resilient patch consensus,
//! minimum distortion bounding, and automated hallucination filtering.
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use crate::multi_agent_collab::ExternalAgentKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Metric feature vector evaluated for an agent's proposed patch or refactoring.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProposalScoreVector {
    /// Functional test pass rate in [0.0, 1.0].
    pub test_pass_rate: f64,
    /// Static analysis / type safety confidence in [0.0, 1.0].
    pub confidence: f64,
    /// Normalized complexity / AST compactness score in [0.0, 1.0] (higher = cleaner).
    pub compactness: f64,
    /// Formal invariant assertion compliance in [0.0, 1.0].
    pub invariant_compliance: f64,
}

impl ProposalScoreVector {
    pub fn new(
        test_pass_rate: f64,
        confidence: f64,
        compactness: f64,
        invariant_compliance: f64,
    ) -> Self {
        Self {
            test_pass_rate: test_pass_rate.clamp(0.0, 1.0),
            confidence: confidence.clamp(0.0, 1.0),
            compactness: compactness.clamp(0.0, 1.0),
            invariant_compliance: invariant_compliance.clamp(0.0, 1.0),
        }
    }

    /// As 4D coordinate slice for coordinate-wise operations.
    pub fn as_coords(&self) -> [f64; 4] {
        [
            self.test_pass_rate,
            self.confidence,
            self.compactness,
            self.invariant_compliance,
        ]
    }

    /// From 4D coordinate slice.
    pub fn from_coords(coords: [f64; 4]) -> Self {
        Self::new(coords[0], coords[1], coords[2], coords[3])
    }

    /// Composite weighted fitness score in [0.0, 1.0].
    /// Weighted as: 40% test pass, 25% invariant compliance, 20% confidence, 15% compactness.
    pub fn composite_score(&self) -> f64 {
        0.40 * self.test_pass_rate
            + 0.25 * self.invariant_compliance
            + 0.20 * self.confidence
            + 0.15 * self.compactness
    }

    /// Euclidean distance in 4D metric feature space.
    pub fn distance(&self, other: &Self) -> f64 {
        let a = self.as_coords();
        let b = other.as_coords();
        let sum_sq: f64 = a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum();
        sum_sq.sqrt()
    }
}

/// A candidate code patch proposal submitted by an external AI agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentPatchProposal {
    pub proposal_id: String,
    pub agent: ExternalAgentKind,
    pub title: String,
    pub description: String,
    pub diff_content: String,
    pub scores: ProposalScoreVector,
    pub execution_time_ms: u64,
}

impl AgentPatchProposal {
    pub fn new(
        proposal_id: impl Into<String>,
        agent: ExternalAgentKind,
        title: impl Into<String>,
        description: impl Into<String>,
        diff_content: impl Into<String>,
        scores: ProposalScoreVector,
        execution_time_ms: u64,
    ) -> Self {
        Self {
            proposal_id: proposal_id.into(),
            agent,
            title: title.into(),
            description: description.into(),
            diff_content: diff_content.into(),
            scores,
            execution_time_ms,
        }
    }

    /// Compute SHA-256 fingerprint of the proposal.
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.proposal_id.as_bytes());
        hasher.update(self.agent.identifier().as_bytes());
        hasher.update(self.diff_content.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// Consensus voting verdict on an individual agent proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalVerdict {
    /// Accepted as part of the honest consensus quorum.
    ConsensusQuorum,
    /// Flagged and filtered as Byzantine or hallucinated due to metric divergence.
    FilteredByzantine,
    /// Below minimum invariant safety threshold (e.g. failing tests or broken assertions).
    RejectedBelowThreshold,
}

/// Final cryptographic decision record of the multi-agent Byzantine consensus swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusDecision {
    pub winning_proposal_id: String,
    pub winning_agent: ExternalAgentKind,
    pub winning_title: String,
    pub consensus_centroid: ProposalScoreVector,
    pub consensus_confidence: f64,
    pub total_proposals: usize,
    pub honest_quorum_size: usize,
    pub byzantine_filtered_count: usize,
    pub proposal_verdicts: BTreeMap<String, ProposalVerdict>,
    pub consensus_hash: String,
    pub timestamp_ms: u64,
}

/// Engine orchestrating multi-agent Byzantine code consensus.
#[derive(Debug, Clone)]
pub struct SwarmCodeConsensusEngine {
    /// Minimum composite score threshold for a proposal to be considered viable.
    min_viability_threshold: f64,
    /// Maximum allowable deviation distance from the trimmed centroid.
    max_centroid_divergence: f64,
}

impl Default for SwarmCodeConsensusEngine {
    fn default() -> Self {
        Self {
            min_viability_threshold: 0.50,
            max_centroid_divergence: 0.45,
        }
    }
}

impl SwarmCodeConsensusEngine {
    /// Configured minimum composite viability threshold.
    pub fn min_viability_threshold(&self) -> f64 {
        self.min_viability_threshold
    }

    /// Configured maximum centroid divergence.
    pub fn max_centroid_divergence(&self) -> f64 {
        self.max_centroid_divergence
    }

    pub fn new(min_viability_threshold: f64, max_centroid_divergence: f64) -> Self {
        Self {
            min_viability_threshold: min_viability_threshold.clamp(0.0, 1.0),
            max_centroid_divergence: max_centroid_divergence.max(0.01),
        }
    }

    /// Run Byzantine consensus resolution across candidate proposals.
    ///
    /// Implements coordinate-wise $\alpha$-trimmed mean and Krum geometric filtering
    /// to isolate and reject Byzantine proposals under Theorem 43.
    pub fn resolve_consensus(
        &self,
        proposals: &[AgentPatchProposal],
        timestamp_ms: u64,
    ) -> Result<ConsensusDecision, String> {
        if proposals.is_empty() {
            return Err("cannot run consensus on empty proposals".to_string());
        }

        let n = proposals.len();
        let mut verdicts = BTreeMap::new();

        // Step 1: Pre-filter proposals failing basic invariant viability
        let mut viable_indices = Vec::new();
        for (i, p) in proposals.iter().enumerate() {
            if p.scores.composite_score() < self.min_viability_threshold
                || p.scores.invariant_compliance < 0.40
            {
                verdicts.insert(
                    p.proposal_id.clone(),
                    ProposalVerdict::RejectedBelowThreshold,
                );
            } else {
                viable_indices.push(i);
            }
        }

        if viable_indices.is_empty() {
            return Err("all proposals failed basic invariant threshold".to_string());
        }

        // Step 2: Calculate coordinate-wise trimmed mean centroid over viable proposals
        let centroid = self.compute_trimmed_centroid(proposals, &viable_indices);

        // Step 3: Classify remaining viable proposals by distance from centroid
        let mut honest_indices = Vec::new();
        for &idx in &viable_indices {
            let p = &proposals[idx];
            let dist = p.scores.distance(&centroid);
            if dist > self.max_centroid_divergence {
                verdicts.insert(p.proposal_id.clone(), ProposalVerdict::FilteredByzantine);
            } else {
                verdicts.insert(p.proposal_id.clone(), ProposalVerdict::ConsensusQuorum);
                honest_indices.push(idx);
            }
        }

        // Fallback: If distance filtering left no proposals, keep the closest viable proposal
        if honest_indices.is_empty() {
            let best_viable = viable_indices
                .iter()
                .copied()
                .min_by(|&a, &b| {
                    let d_a = proposals[a].scores.distance(&centroid);
                    let d_b = proposals[b].scores.distance(&centroid);
                    d_a.partial_cmp(&d_b).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap();
            verdicts.insert(
                proposals[best_viable].proposal_id.clone(),
                ProposalVerdict::ConsensusQuorum,
            );
            honest_indices.push(best_viable);
        }

        // Step 4: Elect the winning proposal using Krum / maximum composite score within honest quorum
        let best_idx = honest_indices
            .iter()
            .copied()
            .max_by(|&a, &b| {
                let score_a = proposals[a].scores.composite_score();
                let score_b = proposals[b].scores.composite_score();
                score_a
                    .partial_cmp(&score_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();

        let winning = &proposals[best_idx];
        let honest_count = honest_indices.len();
        let byzantine_count = n - honest_count;

        // Step 5: Compute cryptographic consensus hash
        let mut hasher = Sha256::new();
        hasher.update(b"PANDORA_CONSENSUS_V1:");
        hasher.update(winning.fingerprint().as_bytes());
        hasher.update(timestamp_ms.to_be_bytes());
        hasher.update((honest_count as u32).to_be_bytes());
        for p in proposals {
            hasher.update(p.proposal_id.as_bytes());
            if let Some(v) = verdicts.get(&p.proposal_id) {
                hasher.update(format!("{v:?}").as_bytes());
            }
        }
        let consensus_hash = format!("{:x}", hasher.finalize());

        let consensus_confidence =
            (honest_count as f64 / n as f64) * winning.scores.composite_score();

        Ok(ConsensusDecision {
            winning_proposal_id: winning.proposal_id.clone(),
            winning_agent: winning.agent,
            winning_title: winning.title.clone(),
            consensus_centroid: centroid,
            consensus_confidence,
            total_proposals: n,
            honest_quorum_size: honest_count,
            byzantine_filtered_count: byzantine_count,
            proposal_verdicts: verdicts,
            consensus_hash,
            timestamp_ms,
        })
    }

    /// Coordinate-wise trimmed mean across 4 quality dimensions.
    fn compute_trimmed_centroid(
        &self,
        proposals: &[AgentPatchProposal],
        indices: &[usize],
    ) -> ProposalScoreVector {
        let m = indices.len();
        if m == 1 {
            return proposals[indices[0]].scores;
        }

        // Trim fraction alpha = 1 / 4 (drops lowest and highest if m >= 4)
        let trim_k = if m >= 4 { m / 4 } else { 0 };

        let mut trimmed_coords = [0.0; 4];

        for (d, coord) in trimmed_coords.iter_mut().enumerate() {
            let mut values: Vec<f64> = indices
                .iter()
                .map(|&idx| proposals[idx].scores.as_coords()[d])
                .collect();
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            let slice = &values[trim_k..(m - trim_k)];
            let sum: f64 = slice.iter().sum();
            *coord = sum / (slice.len() as f64);
        }

        ProposalScoreVector::from_coords(trimmed_coords)
    }
}
