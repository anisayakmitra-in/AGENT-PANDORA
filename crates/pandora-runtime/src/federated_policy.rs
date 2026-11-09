//! Phase 23: Federated Policy & Dynamic Agent Reputation Engine.
//!
//! Grounded in Theorems 45 and 46 for multi-agent policy gradient convergence,
//! coordinate-wise trimmed mean Byzantine gradient filtering, and dynamic Thompson-sampling
//! Beta-conjugate reputation decay and quarantine.
//!
//! # Mathematical Foundations
//! - **Theorem 45 (Federated Multi-Agent Policy Gradient Convergence & Byzantine Resilience Bound)**:
//!   Under coordinate-wise trimmed mean gradient aggregation with trim fraction $\alpha = f/M < 1/2$,
//!   the aggregated gradient deviation is bounded by:
//!   $$\|\hat{\mathbf{g}}_t - \nabla J(\boldsymbol{\theta}_t)\|_2 \le \mathcal{E}_{\mathrm{Byz}}(t)
//!   \triangleq \frac{4 \sum_{b \in \mathcal{B}} w_b(t)}{\sum_{h \in \mathcal{H}} w_h(t)} G \sqrt{d}
//!   + \frac{\sigma \sqrt{d}}{\sqrt{\sum_{h \in \mathcal{H}} w_h(t)}}.$$
//!
//!   The ergodic policy gradient sequence converges at rate $O(1/T) + \mathcal{E}_{\mathrm{Byz}}^2$.
//!
//! - **Theorem 46 (Dynamic Reputation Weighting & Thompson Sampling Bandit Elimination Bound)**:
//!   Under asymmetric Beta-conjugate updates with Byzantine penalty $\kappa_{\mathrm{pen}} \ge 2.0$:
//!   Adversarial agents decay exponentially below quarantine threshold $\tau_{\mathrm{quar}} = 0.25$,
//!   with expected quarantine hitting time $\mathbb{E}[T_{\mathrm{quar}}] = O(1)$ and cumulative
//!   contamination $\sum_{t=1}^T w_b(t) = O(\ln T)$.
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use crate::bandit_router::SafePrng;
use crate::multi_agent_collab::ExternalAgentKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

/// Dimension of the policy parameter space $\boldsymbol{\theta} \in [0, 1]^6$.
pub const POLICY_DIMENSIONS: usize = 6;

/// Default federated policy learning rate $\eta = 0.05$.
pub const DEFAULT_LEARNING_RATE: f64 = 0.05;

/// Default momentum decay coefficient $\beta = 0.90$.
pub const DEFAULT_MOMENTUM: f64 = 0.90;

/// Default maximum gradient clipping L2-norm $G = 1.0$.
pub const DEFAULT_MAX_GRADIENT_NORM: f64 = 1.0;

/// Critical reliability threshold below which an agent is quarantined ($\tau_{\mathrm{quar}} = 0.25$).
pub const CRITICAL_REPUTATION_THRESHOLD: f64 = 0.25;

/// Minimum evaluations required before an agent can be placed in quarantine ($N_{\min} = 3$).
pub const MIN_EVALS_FOR_QUARANTINE: u64 = 3;

/// Clamped gradient aggregation weight penalty for quarantined agents ($w_{\mathrm{quar}} = 0.05$).
pub const QUARANTINE_WEIGHT_PENALTY: f64 = 0.05;

/// Asymmetric penalty added to Beta posterior parameter $\beta$ upon Byzantine divergence ($\kappa_{\mathrm{pen}} = 2.0$).
pub const BYZANTINE_BETA_PENALTY: f64 = 2.0;

/// Semantic labels for the 6 policy dimensions.
pub const POLICY_DIMENSION_LABELS: [&str; 6] = [
    "Code Correctness",
    "Context Efficiency",
    "Invariant Preservation",
    "Latency Minimization",
    "AST Compactness",
    "Quorum Agreement",
];

/// Strongly-typed error enum for federated policy engine and reputation operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FederatedPolicyError {
    /// Attempted gradient aggregation over an empty gradient set.
    EmptyGradients,
    /// Vector dimension did not match the expected policy dimension.
    DimensionMismatch { expected: usize, actual: usize },
    /// Learning rate was outside valid bounds $(0.0, 1.0]$.
    InvalidLearningRate(f64),
    /// Momentum was outside valid bounds $[0.0, 1.0)$.
    InvalidMomentum(f64),
    /// Max gradient norm was non-positive or non-finite.
    InvalidGradientNorm(f64),
    /// An agent is currently quarantined and prohibited from critical operations.
    QuarantinedAgent(ExternalAgentKind),
    /// Not enough participating agents to form a valid consensus quorum.
    InsufficientQuorum { required: usize, actual: usize },
    /// Byzantine fraction exceeds the theoretical trimmed mean breakdown limit ($f \ge M/2$).
    ByzantineQuorumExceeded { participating: usize, byzantine_count: usize },
    /// Gradient components or loss values contained non-finite numbers (NaN/Inf).
    InvalidGradient(String),
}

impl fmt::Display for FederatedPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyGradients => write!(f, "cannot aggregate empty gradient set"),
            Self::DimensionMismatch { expected, actual } => {
                write!(f, "dimension mismatch: expected {expected}, actual {actual}")
            }
            Self::InvalidLearningRate(lr) => {
                write!(f, "invalid learning rate: {lr} (must be in (0.0, 1.0])")
            }
            Self::InvalidMomentum(m) => {
                write!(f, "invalid momentum: {m} (must be in [0.0, 1.0))")
            }
            Self::InvalidGradientNorm(gn) => {
                write!(f, "invalid gradient norm: {gn} (must be > 0.0)")
            }
            Self::QuarantinedAgent(agent) => {
                write!(f, "agent {} is quarantined and rejected from consensus", agent.identifier())
            }
            Self::InsufficientQuorum { required, actual } => {
                write!(f, "insufficient quorum: required {required}, actual {actual}")
            }
            Self::ByzantineQuorumExceeded { participating, byzantine_count } => {
                write!(
                    f,
                    "byzantine quorum exceeded: {byzantine_count} byzantine out of {participating} total (requires < 50% for trimmed mean)"
                )
            }
            Self::InvalidGradient(msg) => write!(f, "invalid gradient: {msg}"),
        }
    }
}

impl std::error::Error for FederatedPolicyError {}

/// High-dimensional policy gradient vector evaluated by an individual coding agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyGradientVector {
    pub agent: ExternalAgentKind,
    /// Gradient components for policy dimensions:
    /// [0] Code correctness, [1] Context efficiency, [2] Invariant preservation,
    /// [3] Latency minimization, [4] AST compactness, [5] Quorum agreement.
    pub gradients: [f64; 6],
    pub sample_count: usize,
    pub loss: f64,
}

impl PolicyGradientVector {
    /// Create a new policy gradient vector with strict finite-number validation.
    pub fn new(
        agent: ExternalAgentKind,
        gradients: [f64; 6],
        sample_count: usize,
        loss: f64,
    ) -> Result<Self, FederatedPolicyError> {
        for (idx, &g) in gradients.iter().enumerate() {
            if g.is_nan() || g.is_infinite() {
                return Err(FederatedPolicyError::InvalidGradient(format!(
                    "gradient component at dimension {idx} is non-finite ({g})"
                )));
            }
        }
        if loss.is_nan() || loss.is_infinite() || loss < 0.0 {
            return Err(FederatedPolicyError::InvalidGradient(format!(
                "loss value is invalid or negative ({loss})"
            )));
        }

        Ok(Self {
            agent,
            gradients,
            sample_count: sample_count.max(1),
            loss,
        })
    }

    /// L2 norm of the gradient vector.
    pub fn l2_norm(&self) -> f64 {
        let sum_sq: f64 = self.gradients.iter().map(|&x| x * x).sum();
        sum_sq.sqrt()
    }

    /// Euclidean distance to another gradient vector.
    pub fn distance(&self, other: &Self) -> f64 {
        let sum_sq: f64 = self
            .gradients
            .iter()
            .zip(other.gradients.iter())
            .map(|(&a, &b)| (a - b).powi(2))
            .sum();
        sum_sq.sqrt()
    }

    /// Clip gradient to max norm (gradient clipping bound under Theorem 45).
    pub fn clipped(&self, max_norm: f64) -> Self {
        let norm = self.l2_norm();
        if norm > max_norm && norm > 1e-9 {
            let scale = max_norm / norm;
            let mut clipped_grad = [0.0; 6];
            for (i, val) in self.gradients.iter().enumerate() {
                clipped_grad[i] = val * scale;
            }
            Self {
                agent: self.agent,
                gradients: clipped_grad,
                sample_count: self.sample_count,
                loss: self.loss,
            }
        } else {
            self.clone()
        }
    }
}

/// Agent reputation state tracked under Theorem 46 using Beta-conjugate distributions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReputationRecord {
    pub agent: ExternalAgentKind,
    /// Alpha parameter: accumulated honest / successful consensus contributions.
    pub alpha: f64,
    /// Beta parameter: accumulated Byzantine / divergent / invariant-violating events.
    pub beta: f64,
    /// Total evaluations observed.
    pub total_evaluations: u64,
    /// Current normalized expected reliability weight in [0.0, 1.0].
    pub consensus_weight: f64,
    /// Active quarantine status: true if reputation drops below critical threshold.
    pub is_quarantined: bool,
}

impl AgentReputationRecord {
    /// Initialize with a mildly optimistic prior $\alpha_0 = 2.0, \beta_0 = 1.0$.
    pub fn new(agent: ExternalAgentKind) -> Self {
        let alpha = 2.0;
        let beta = 1.0;
        Self {
            agent,
            alpha,
            beta,
            total_evaluations: 0,
            consensus_weight: alpha / (alpha + beta),
            is_quarantined: false,
        }
    }

    /// Expected reliability score $\mathbb{E}[w] = \frac{\alpha}{\alpha + \beta}$.
    pub fn expected_reliability(&self) -> f64 {
        let sum = self.alpha + self.beta;
        if sum <= 0.0 {
            0.5
        } else {
            self.alpha / sum
        }
    }

    /// Posterior variance $\mathrm{Var}[w] = \frac{\alpha \beta}{(\alpha+\beta)^2(\alpha+\beta+1)}$.
    pub fn variance(&self) -> f64 {
        let sum = self.alpha + self.beta;
        if sum <= 0.0 {
            0.0
        } else {
            (self.alpha * self.beta) / (sum * sum * (sum + 1.0))
        }
    }

    /// Sample reliability from posterior using Thompson Sampling (Theorem 46).
    pub fn sample_thompson(&self, rng_seed: u64) -> f64 {
        let mut prng = SafePrng::new(rng_seed);
        prng.sample_beta(self.alpha, self.beta)
    }

    /// Record an evaluation outcome (honest vs Byzantine/divergent).
    pub fn record_outcome(&mut self, is_honest: bool, quality_score: f64) {
        self.total_evaluations += 1;
        if is_honest {
            self.alpha += quality_score.clamp(0.1, 1.0);
        } else {
            // Byzantine penalty scales heavier under Theorem 46 to accelerate outlier isolation
            self.beta += BYZANTINE_BETA_PENALTY;
        }

        self.consensus_weight = self.expected_reliability();
        // Quarantine if reliability falls below critical threshold after at least MIN_EVALS_FOR_QUARANTINE
        self.is_quarantined = self.total_evaluations >= MIN_EVALS_FOR_QUARANTINE
            && self.consensus_weight < CRITICAL_REPUTATION_THRESHOLD;
    }
}

/// Cryptographic receipt of a federated policy optimization step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedUpdateReceipt {
    pub step_index: u64,
    pub aggregated_gradient: [f64; 6],
    pub aggregate_norm: f64,
    pub learning_rate: f64,
    pub participating_agents: usize,
    pub trimmed_byzantine_count: usize,
    pub agent_weights: BTreeMap<String, f64>,
    pub update_hash: String,
    pub timestamp_ms: u64,
}

/// Federated Policy Engine coordinating decentralized policy gradients and dynamic reputations.
#[derive(Debug, Clone)]
pub struct FederatedPolicyEngine {
    /// Global model policy parameters in 6D $\boldsymbol{\theta} \in [0, 1]^6$.
    pub current_policy: [f64; 6],
    pub learning_rate: f64,
    pub momentum: f64,
    pub max_gradient_norm: f64,
    pub velocity: [f64; 6],
    pub step_count: u64,
    pub reputation_tracker: BTreeMap<ExternalAgentKind, AgentReputationRecord>,
}

impl Default for FederatedPolicyEngine {
    fn default() -> Self {
        Self::new(DEFAULT_LEARNING_RATE, DEFAULT_MOMENTUM, DEFAULT_MAX_GRADIENT_NORM)
    }
}

impl FederatedPolicyEngine {
    /// Create a new FederatedPolicyEngine with hyperparameter clamping.
    pub fn new(learning_rate: f64, momentum: f64, max_gradient_norm: f64) -> Self {
        let mut tracker = BTreeMap::new();
        for &agent in ExternalAgentKind::all() {
            tracker.insert(agent, AgentReputationRecord::new(agent));
        }

        Self {
            current_policy: [0.5; 6],
            learning_rate: learning_rate.clamp(0.001, 1.0),
            momentum: momentum.clamp(0.0, 0.99),
            max_gradient_norm: max_gradient_norm.max(0.1),
            velocity: [0.0; 6],
            step_count: 0,
            reputation_tracker: tracker,
        }
    }

    /// Retrieve the reputation record of an agent.
    pub fn get_reputation(&self, agent: ExternalAgentKind) -> Option<&AgentReputationRecord> {
        self.reputation_tracker.get(&agent)
    }

    /// Update reputation of an agent based on consensus audit results.
    pub fn update_agent_reputation(
        &mut self,
        agent: ExternalAgentKind,
        is_honest: bool,
        quality_score: f64,
    ) {
        let entry = self
            .reputation_tracker
            .entry(agent)
            .or_insert_with(|| AgentReputationRecord::new(agent));
        entry.record_outcome(is_honest, quality_score);
    }

    /// Return dimension semantic labels and current policy weights.
    pub fn policy_summary(&self) -> Vec<(&'static str, f64)> {
        POLICY_DIMENSION_LABELS
            .iter()
            .zip(self.current_policy.iter())
            .map(|(&label, &val)| (label, val))
            .collect()
    }

    /// Aggregate worker gradients using reputation-weighted coordinate-wise trimmed mean (Theorem 45).
    pub fn aggregate_gradients(
        &self,
        gradients: &[PolicyGradientVector],
    ) -> Result<([f64; 6], usize), FederatedPolicyError> {
        if gradients.is_empty() {
            return Err(FederatedPolicyError::EmptyGradients);
        }

        // Clip all incoming gradients to max norm to guarantee bounded contamination
        let clipped: Vec<PolicyGradientVector> = gradients
            .iter()
            .map(|g| g.clipped(self.max_gradient_norm))
            .collect();

        let n = clipped.len();
        let trim_k = if n >= 4 { n / 4 } else { 0 };

        let mut aggregated = [0.0; 6];

        for (d, agg_d) in aggregated.iter_mut().enumerate() {
            // Collect (gradient[d], reputation_weight)
            let mut entries: Vec<(f64, f64)> = clipped
                .iter()
                .map(|g| {
                    let weight = self
                        .reputation_tracker
                        .get(&g.agent)
                        .map(|r| {
                            if r.is_quarantined {
                                QUARANTINE_WEIGHT_PENALTY
                            } else {
                                r.consensus_weight
                            }
                        })
                        .unwrap_or(0.5);
                    (g.gradients[d], weight)
                })
                .collect();

            // Sort coordinate values
            entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            // Retain trimmed interior slice
            let valid_slice = &entries[trim_k..(n - trim_k)];
            let total_weight: f64 = valid_slice.iter().map(|e| e.1).sum();

            if total_weight > 1e-9 {
                let weighted_sum: f64 = valid_slice.iter().map(|e| e.0 * e.1).sum();
                *agg_d = weighted_sum / total_weight;
            } else {
                let sum: f64 = valid_slice.iter().map(|e| e.0).sum();
                *agg_d = sum / (valid_slice.len() as f64);
            }
        }

        let trimmed_count = trim_k * 2;
        Ok((aggregated, trimmed_count))
    }

    /// Perform one federated optimization step and return certified receipt.
    pub fn step(
        &mut self,
        worker_gradients: &[PolicyGradientVector],
        timestamp_ms: u64,
    ) -> Result<FederatedUpdateReceipt, FederatedPolicyError> {
        let (agg_grad, trimmed_count) = self.aggregate_gradients(worker_gradients)?;

        // Update velocity and apply momentum: v_{t+1} = \beta v_t + (1 - \beta) g_t
        for (i, &g_i) in agg_grad.iter().enumerate() {
            self.velocity[i] =
                self.momentum * self.velocity[i] + (1.0 - self.momentum) * g_i;
            // Gradient descent step: \theta_{t+1} = \theta_t - \eta v_{t+1}
            self.current_policy[i] =
                (self.current_policy[i] - self.learning_rate * self.velocity[i]).clamp(0.0, 1.0);
        }

        self.step_count += 1;

        // Compute norm of aggregated gradient
        let agg_norm: f64 = agg_grad.iter().map(|&x| x * x).sum::<f64>().sqrt();

        // Snapshot weights
        let mut weights_map = BTreeMap::new();
        for (agent, rec) in &self.reputation_tracker {
            weights_map.insert(agent.identifier().to_string(), rec.consensus_weight);
        }

        // Cryptographic receipt hash
        let mut hasher = Sha256::new();
        hasher.update(self.step_count.to_le_bytes());
        for val in &self.current_policy {
            hasher.update(val.to_le_bytes());
        }
        for val in &agg_grad {
            hasher.update(val.to_le_bytes());
        }
        hasher.update(timestamp_ms.to_le_bytes());
        let update_hash = format!("{:x}", hasher.finalize());

        Ok(FederatedUpdateReceipt {
            step_index: self.step_count,
            aggregated_gradient: agg_grad,
            aggregate_norm: agg_norm,
            learning_rate: self.learning_rate,
            participating_agents: worker_gradients.len(),
            trimmed_byzantine_count: trimmed_count,
            agent_weights: weights_map,
            update_hash,
            timestamp_ms,
        })
    }
}
