//! Phase 12: Byzantine-Resilient Multi-Agent Consensus & Gradient Aggregator.
//!
//! Provides provably robust vector aggregation across $n$ autonomous agents in $\mathbb{R}^d$
//! when up to $f$ agents act as arbitrary (omniscient, colluding) Byzantine adversaries:
//! - **Coordinate-Wise $\beta$-Trimmed Mean (Theorem 23):** Removes the smallest $k = \lfloor \beta n \rfloor$
//!   and largest $k$ values along each coordinate $j \in \{1, \dots, d\}$ before averaging the remaining
//!   $n - 2k$ values. Achieves breakdown point $\alpha^* = \frac{f}{n} < \beta < \frac{1}{2}$ and
//!   order-optimal statistical error $\mathbb{E}\|\hat{\mu}_{\text{TM}} - \mu\|_2^2 \le O\left(\frac{d\sigma^2}{n}\left(1 + \frac{f^2}{n}\right)\right)$.
//! - **Coordinate-Wise Geometric/Marginal Median:** Special case $\beta \to \frac{1}{2}$ achieving the
//!   maximum theoretical 50% breakdown point $2f < n$.
//! - **$(n, f)$-Krum & Multi-Krum Geometric Consensus (Theorem 24):** Scores each vector $v_i$ by the sum of
//!   squared Euclidean distances to its $m = n - f - 2$ closest neighbors:
//!   $S(i) = \sum_{i \to j \in \mathcal{N}_i(n - f - 2)} \|v_i - v_j\|_2^2$.
//!   Under $2f + 2 < n$, guarantees bounded cone deviation $\|\hat{\mu}_{\text{Krum}} - \mu\|_2 \le O\left(\sigma \sqrt{d}\right)$
//!   independent of the magnitude of Byzantine attack vectors.
//!
//! Strictly `#![forbid(unsafe_code)]` compliant.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;

/// Error taxonomy for Byzantine-resilient aggregation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ByzantineError {
    /// No agent updates were provided.
    EmptyUpdates,
    /// Vector dimension is zero or inconsistent across agents.
    DimensionMismatch { expected: usize, actual: usize },
    /// Non-finite (NaN or Inf) coordinate encountered in an honest-format check.
    NonFiniteCoordinate { agent_id: String, index: usize },
    /// Byzantine quorum condition violated (e.g., $2f \ge n$ for trimmed mean/median or $2f + 2 \ge n$ for Krum).
    ByzantineQuorumViolated {
        n: usize,
        f: usize,
        rule: &'static str,
        required_condition: &'static str,
    },
    /// Invalid trim fraction $\beta \notin [0, 0.5)$.
    InvalidTrimFraction(f64),
    /// Invalid Multi-Krum selection count $q \notin [1, n - f]$.
    InvalidMultiKrumCount { q: usize, max_allowed: usize },
}

impl fmt::Display for ByzantineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUpdates => write!(f, "Byzantine aggregator requires at least one update"),
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "vector dimension mismatch: expected {expected}, got {actual}"
                )
            }
            Self::NonFiniteCoordinate { agent_id, index } => {
                write!(
                    f,
                    "agent '{agent_id}' supplied non-finite coordinate at index {index}"
                )
            }
            Self::ByzantineQuorumViolated {
                n,
                f: byz,
                rule,
                required_condition,
            } => {
                write!(
                    f,
                    "Byzantine quorum violated for {rule} (n={n}, f={byz}): requires {required_condition}"
                )
            }
            Self::InvalidTrimFraction(beta) => {
                write!(
                    f,
                    "invalid trim fraction beta={beta}; must satisfy 0.0 <= beta < 0.5"
                )
            }
            Self::InvalidMultiKrumCount { q, max_allowed } => {
                write!(
                    f,
                    "invalid Multi-Krum selection count q={q}; must be in 1..={max_allowed}"
                )
            }
        }
    }
}

impl std::error::Error for ByzantineError {}

/// An individual agent's vector update in $\mathbb{R}^d$.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentVectorUpdate {
    pub agent_id: String,
    pub values: Vec<f64>,
}

impl AgentVectorUpdate {
    pub fn new(agent_id: impl Into<String>, values: Vec<f64>) -> Self {
        Self {
            agent_id: agent_id.into(),
            values,
        }
    }
}

/// Supported Byzantine-resilient aggregation rules.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum AggregationRule {
    /// Coordinate-wise $\beta$-trimmed mean discarding $\max(f, \lfloor \beta n \rfloor)$ extremes per coordinate.
    TrimmedMean { beta: f64 },
    /// Coordinate-wise median achieving 50% breakdown point ($2f < n$).
    CoordinateMedian,
    ///Blanchard et al. $(n, f)$-Multi-Krum selecting the $q$ lowest-score geometric neighbors ($2f + 2 < n$).
    MultiKrum { selection_q: usize },
}

/// Result and telemetry of a Byzantine-resilient consensus aggregation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ByzantineConsensusResult {
    /// Aggregated robust consensus vector $\hat{\mu} \in \mathbb{R}^d$.
    pub consensus_vector: Vec<f64>,
    /// Naive unguarded arithmetic mean $\bar{\mu} = \frac{1}{n} \sum_{i=1}^n v_i$ for comparison.
    pub naive_mean_vector: Vec<f64>,
    /// Number of participating agents $n$.
    pub total_agents_n: usize,
    /// Maximum tolerated Byzantine adversaries $f$.
    pub max_byzantine_f: usize,
    /// Vector dimension $d$.
    pub dimension_d: usize,
    /// Selected or retained agent IDs (for Krum/Multi-Krum).
    pub selected_agents: Vec<String>,
    /// Suspected Byzantine outlier agent IDs (highest Krum scores).
    pub suspected_outliers: Vec<String>,
    /// Per-agent geometric Krum distance scores $S(i)$.
    pub krum_scores: Vec<(String, f64)>,
    /// Theoretical upper bound factor $\alpha(n, f, d)$ from Theorem 23 / Theorem 24.
    pub theoretical_deviation_factor: f64,
    /// Euclidean norm difference between naive mean and robust consensus $\|\bar{\mu} - \hat{\mu}\|_2$.
    pub mitigation_shift_l2: f64,
}

/// Stateless and stateful coordinator for Byzantine-resilient multi-agent aggregation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ByzantineAggregator {
    pub max_byzantine_f: usize,
    pub rule: AggregationRule,
}

impl ByzantineAggregator {
    pub fn new(max_byzantine_f: usize, rule: AggregationRule) -> Result<Self, ByzantineError> {
        if let AggregationRule::TrimmedMean { beta } = rule
            && (!(0.0..0.5).contains(&beta) || beta.is_nan())
        {
            return Err(ByzantineError::InvalidTrimFraction(beta));
        }
        Ok(Self {
            max_byzantine_f,
            rule,
        })
    }

    /// Computes squared Euclidean distance $\|a - b\|_2^2$ between two slices.
    #[inline]
    pub fn squared_l2_distance(a: &[f64], b: &[f64]) -> f64 {
        a.iter()
            .zip(b.iter())
            .map(|(&x, &y)| {
                let diff = x - y;
                diff * diff
            })
            .sum()
    }

    /// Computes Euclidean distance $\|a - b\|_2$.
    #[inline]
    pub fn l2_distance(a: &[f64], b: &[f64]) -> f64 {
        Self::squared_l2_distance(a, b).sqrt()
    }

    /// Theorem 24 theoretical cone factor for $(n, f)$-Krum:
    /// $\alpha_{\text{Krum}}(n, f) = \sqrt{2\left(n - f + \frac{f \cdot (n - f - 2) + f^2 \cdot (n - f - 1)}{n - 2f - 2}\right)}$.
    pub fn theoretical_krum_factor(n: usize, f: usize) -> Result<f64, ByzantineError> {
        if 2 * f + 2 >= n {
            return Err(ByzantineError::ByzantineQuorumViolated {
                n,
                f,
                rule: "Krum",
                required_condition: "2f + 2 < n",
            });
        }
        let n_f = n as f64;
        let f_f = f as f64;
        let m = (n - f - 2) as f64;
        let denom = (n - 2 * f - 2) as f64;
        let inner = 2.0 * (n_f - f_f + (f_f * m + f_f * f_f) / denom);
        Ok(inner.sqrt())
    }

    /// Theorem 23 theoretical variance inflation factor for Coordinate-Wise $\beta$-Trimmed Mean:
    /// $\Upsilon(n, f, \beta) = \frac{1}{(1 - 2\beta)^2} \left(1 + \frac{f}{n}\right)$.
    pub fn theoretical_trimmed_mean_factor(
        n: usize,
        f: usize,
        beta: f64,
    ) -> Result<f64, ByzantineError> {
        if 2 * f >= n {
            return Err(ByzantineError::ByzantineQuorumViolated {
                n,
                f,
                rule: "TrimmedMean",
                required_condition: "2f < n",
            });
        }
        if !(0.0..0.5).contains(&beta) || beta.is_nan() {
            return Err(ByzantineError::InvalidTrimFraction(beta));
        }
        let eff_beta = beta.max((f as f64) / (n as f64));
        if eff_beta >= 0.5 {
            return Err(ByzantineError::ByzantineQuorumViolated {
                n,
                f,
                rule: "TrimmedMean",
                required_condition: "f / n < 0.5",
            });
        }
        let denom = (1.0 - 2.0 * eff_beta).powi(2);
        Ok((1.0 + (f as f64) / (n as f64)) / denom)
    }

    fn validate_updates(updates: &[AgentVectorUpdate]) -> Result<(usize, usize), ByzantineError> {
        if updates.is_empty() {
            return Err(ByzantineError::EmptyUpdates);
        }
        let d = updates[0].values.len();
        if d == 0 {
            return Err(ByzantineError::DimensionMismatch {
                expected: 1,
                actual: 0,
            });
        }
        for u in updates {
            if u.values.len() != d {
                return Err(ByzantineError::DimensionMismatch {
                    expected: d,
                    actual: u.values.len(),
                });
            }
            for (idx, &val) in u.values.iter().enumerate() {
                if !val.is_finite() {
                    return Err(ByzantineError::NonFiniteCoordinate {
                        agent_id: u.agent_id.clone(),
                        index: idx,
                    });
                }
            }
        }
        Ok((updates.len(), d))
    }

    fn compute_naive_mean(updates: &[AgentVectorUpdate], d: usize) -> Vec<f64> {
        let n = updates.len() as f64;
        let mut mean = vec![0.0; d];
        for u in updates {
            for (j, &v) in u.values.iter().enumerate() {
                mean[j] += v;
            }
        }
        for val in &mut mean {
            *val /= n;
        }
        mean
    }

    /// Computes Krum geometric neighbor scores $S(i) = \sum_{j \in \mathcal{N}_i(m)} \|v_i - v_j\|_2^2$
    /// where $m = \max(1, n - f - 2)$.
    pub fn compute_krum_scores(
        updates: &[AgentVectorUpdate],
        f: usize,
    ) -> Vec<(usize, String, f64)> {
        let n = updates.len();
        let m = n.saturating_sub(f + 2).max(1).min(n.saturating_sub(1));
        let mut scores = Vec::with_capacity(n);

        for i in 0..n {
            let mut dists: Vec<f64> = (0..n)
                .filter(|&j| j != i)
                .map(|j| Self::squared_l2_distance(&updates[i].values, &updates[j].values))
                .collect();
            dists.sort_by(|a, b| a.total_cmp(b));
            let score: f64 = dists.into_iter().take(m).sum();
            scores.push((i, updates[i].agent_id.clone(), score));
        }

        scores.sort_by(|a, b| a.2.total_cmp(&b.2).then_with(|| a.0.cmp(&b.0)));
        scores
    }

    /// Executes Byzantine-resilient consensus aggregation across $n$ agent updates.
    pub fn aggregate(
        &self,
        updates: &[AgentVectorUpdate],
    ) -> Result<ByzantineConsensusResult, ByzantineError> {
        let (n, d) = Self::validate_updates(updates)?;
        let f = self.max_byzantine_f;
        let naive_mean = Self::compute_naive_mean(updates, d);
        let scored = Self::compute_krum_scores(updates, f);

        let suspected_outliers: Vec<String> = if f > 0 && n > f {
            scored
                .iter()
                .rev()
                .take(f)
                .map(|(_, id, _)| id.clone())
                .collect()
        } else {
            Vec::new()
        };

        let krum_scores: Vec<(String, f64)> =
            scored.iter().map(|(_, id, s)| (id.clone(), *s)).collect();

        let (consensus_vector, selected_agents, theoretical_deviation_factor) = match self.rule {
            AggregationRule::TrimmedMean { beta } => {
                if 2 * f >= n {
                    return Err(ByzantineError::ByzantineQuorumViolated {
                        n,
                        f,
                        rule: "TrimmedMean",
                        required_condition: "2f < n",
                    });
                }
                let k_beta = ((n as f64) * beta).floor() as usize;
                let k = k_beta.max(f);
                if 2 * k >= n {
                    return Err(ByzantineError::ByzantineQuorumViolated {
                        n,
                        f: k,
                        rule: "TrimmedMean",
                        required_condition: "2 * trim_k < n",
                    });
                }
                let retained_count = (n - 2 * k) as f64;
                let mut out = vec![0.0; d];
                let mut col = Vec::with_capacity(n);
                for (j, out_j) in out.iter_mut().enumerate().take(d) {
                    col.clear();
                    for u in updates {
                        col.push(u.values[j]);
                    }
                    col.sort_by(|a, b| a.total_cmp(b));
                    let sum: f64 = col[k..(n - k)].iter().sum();
                    *out_j = sum / retained_count;
                }
                let selected: Vec<String> = scored
                    .iter()
                    .take(n - f)
                    .map(|(_, id, _)| id.clone())
                    .collect();
                let factor = Self::theoretical_trimmed_mean_factor(n, f, beta)?;
                (out, selected, factor)
            }
            AggregationRule::CoordinateMedian => {
                if 2 * f >= n {
                    return Err(ByzantineError::ByzantineQuorumViolated {
                        n,
                        f,
                        rule: "CoordinateMedian",
                        required_condition: "2f < n",
                    });
                }
                let mut out = vec![0.0; d];
                let mut col = Vec::with_capacity(n);
                for (j, out_j) in out.iter_mut().enumerate().take(d) {
                    col.clear();
                    for u in updates {
                        col.push(u.values[j]);
                    }
                    col.sort_by(|a, b| a.total_cmp(b));
                    *out_j = if n % 2 == 1 {
                        col[n / 2]
                    } else {
                        0.5 * (col[n / 2 - 1] + col[n / 2])
                    };
                }
                let selected: Vec<String> = scored
                    .iter()
                    .take(n - f)
                    .map(|(_, id, _)| id.clone())
                    .collect();
                let factor = 1.0 / (1.0 - (2.0 * (f as f64) / (n as f64))).powi(2);
                (out, selected, factor)
            }
            AggregationRule::MultiKrum { selection_q } => {
                let factor = Self::theoretical_krum_factor(n, f)?;
                let max_q = n - f;
                if selection_q == 0 || selection_q > max_q {
                    return Err(ByzantineError::InvalidMultiKrumCount {
                        q: selection_q,
                        max_allowed: max_q,
                    });
                }
                let mut out = vec![0.0; d];
                let mut selected = Vec::with_capacity(selection_q);
                for &(orig_idx, ref agent_id, _) in scored.iter().take(selection_q) {
                    selected.push(agent_id.clone());
                    for (j, &v) in updates[orig_idx].values.iter().enumerate() {
                        out[j] += v;
                    }
                }
                for val in &mut out {
                    *val /= selection_q as f64;
                }
                (out, selected, factor)
            }
        };

        let mitigation_shift_l2 = Self::l2_distance(&naive_mean, &consensus_vector);

        Ok(ByzantineConsensusResult {
            consensus_vector,
            naive_mean_vector: naive_mean,
            total_agents_n: n,
            max_byzantine_f: f,
            dimension_d: d,
            selected_agents,
            suspected_outliers,
            krum_scores,
            theoretical_deviation_factor,
            mitigation_shift_l2,
        })
    }
}
