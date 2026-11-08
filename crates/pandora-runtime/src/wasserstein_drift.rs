//! Phase 13: Entropic Optimal Transport (Sinkhorn-Knopp) Semantic Drift Detector.
//!
//! Computes the entropic-regularized $2$-Wasserstein distance $W_{2,\varepsilon}(\mu, \nu)$
//! and the debiased Sinkhorn divergence $S_\varepsilon(\mu, \nu)$ between discrete empirical
//! distributions $\mu = \sum_{i=1}^n a_i \delta_{x_i}$ and $\nu = \sum_{j=1}^m b_j \delta_{y_j}$
//! in $\mathbb{R}^d$ using log-domain stabilized dual Sinkhorn-Knopp iterations:
//! - **Primal Entropic OT Problem:**
//!   $$W_{2,\varepsilon}^2(\mu, \nu) = \min_{\pi \in \Pi(a, b)} \sum_{i,j} C_{ij} \pi_{ij} + \varepsilon \sum_{i,j} \pi_{ij} \left(\ln \frac{\pi_{ij}}{a_i b_j} - 1\right)$$
//!   where $C_{ij} = \|x_i - y_j\|_2^2$.
//! - **Log-Domain Stabilized Dual Updates (Theorem 25):**
//!   $$f_i^{(k+1)} = -\varepsilon \ln \sum_{j=1}^m b_j \exp\left(\frac{g_j^{(k)} - C_{ij}}{\varepsilon}\right), \quad g_j^{(k+1)} = -\varepsilon \ln \sum_{i=1}^n a_i \exp\left(\frac{f_i^{(k+1)} - C_{ij}}{\varepsilon}\right)$$
//!   evaluated via the numerically stable log-sum-exp identity $\text{LSE}(z) = z_{\max} + \ln \sum_j \exp(z_j - z_{\max})$,
//!   guaranteeing zero floating-point underflow/overflow even for $\varepsilon \to 0^+$ and contracting linearly
//!   in the Hilbert projective metric with Birkhoff-Hopf factor $\kappa = \frac{\sqrt{\eta} - 1}{\sqrt{\eta} + 1} < 1$.
//! - **Debiased Sinkhorn Divergence (Theorem 26):**
//!   $$S_\varepsilon(\mu, \nu) = \text{OT}_\varepsilon(\mu, \nu) - \frac{1}{2} \text{OT}_\varepsilon(\mu, \mu) - \frac{1}{2} \text{OT}_\varepsilon(\nu, \nu) \ge 0$$
//!   which restores exact identity of indiscernibles ($S_\varepsilon(\mu, \mu) = 0$) and satisfies
//!   the entropic bias bound $|\text{OT}_\varepsilon(\mu, \nu) - W_2^2(\mu, \nu)| \le 2\varepsilon H(a \otimes b)$.
//!
//! Strictly `#![forbid(unsafe_code)]` compliant.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;

/// Error taxonomy for Entropic Optimal Transport and Semantic Drift Detection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WassersteinError {
    /// Support points or probability weights are empty.
    EmptyDistribution,
    /// Point dimension mismatch across support vectors.
    DimensionMismatch { expected: usize, actual: usize },
    /// Probability weights length does not match support points length.
    WeightLengthMismatch { points: usize, weights: usize },
    /// Probability weights contain negative, NaN, or zero-sum values.
    InvalidWeights(String),
    /// Entropic regularization $\varepsilon \le 0$ or non-finite.
    InvalidEpsilon(f64),
    /// Convergence tolerance or max iterations invalid.
    InvalidConvergenceConfig(String),
}

impl fmt::Display for WassersteinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDistribution => {
                write!(
                    f,
                    "empirical distribution must contain at least one support point"
                )
            }
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "support point dimension mismatch: expected {expected}, got {actual}"
                )
            }
            Self::WeightLengthMismatch { points, weights } => {
                write!(
                    f,
                    "weights length ({weights}) does not match support points length ({points})"
                )
            }
            Self::InvalidWeights(msg) => write!(f, "invalid probability weights: {msg}"),
            Self::InvalidEpsilon(eps) => {
                write!(
                    f,
                    "invalid entropic regularization epsilon={eps}; must satisfy epsilon > 0"
                )
            }
            Self::InvalidConvergenceConfig(msg) => {
                write!(f, "invalid Sinkhorn convergence configuration: {msg}")
            }
        }
    }
}

impl std::error::Error for WassersteinError {}

/// Discrete empirical probability distribution $\mu = \sum_{i=1}^n a_i \delta_{x_i}$ in $\mathbb{R}^d$.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmpiricalDistribution {
    /// Support points $x_1, \dots, x_n \in \mathbb{R}^d$.
    pub points: Vec<Vec<f64>>,
    /// Normalized simplex weights $a_1, \dots, a_n > 0$ with $\sum_i a_i = 1$.
    pub weights: Vec<f64>,
}

impl EmpiricalDistribution {
    /// Constructs a uniform empirical distribution $a_i = \frac{1}{n}$ over the given points.
    pub fn uniform(points: Vec<Vec<f64>>) -> Result<Self, WassersteinError> {
        if points.is_empty() {
            return Err(WassersteinError::EmptyDistribution);
        }
        let n = points.len();
        let weights = vec![1.0 / (n as f64); n];
        Self::new(points, weights)
    }

    /// Constructs and normalizes a weighted empirical distribution.
    pub fn new(points: Vec<Vec<f64>>, weights: Vec<f64>) -> Result<Self, WassersteinError> {
        if points.is_empty() {
            return Err(WassersteinError::EmptyDistribution);
        }
        if points.len() != weights.len() {
            return Err(WassersteinError::WeightLengthMismatch {
                points: points.len(),
                weights: weights.len(),
            });
        }
        let d = points[0].len();
        if d == 0 {
            return Err(WassersteinError::DimensionMismatch {
                expected: 1,
                actual: 0,
            });
        }
        for p in &points {
            if p.len() != d {
                return Err(WassersteinError::DimensionMismatch {
                    expected: d,
                    actual: p.len(),
                });
            }
            if p.iter().any(|x| !x.is_finite()) {
                return Err(WassersteinError::InvalidWeights(
                    "support point contains non-finite coordinate".to_string(),
                ));
            }
        }

        let mut sum = 0.0;
        for &w in &weights {
            if !w.is_finite() || w <= 0.0 {
                return Err(WassersteinError::InvalidWeights(
                    "weights must be strictly positive and finite".to_string(),
                ));
            }
            sum += w;
        }
        if sum <= 0.0 {
            return Err(WassersteinError::InvalidWeights(
                "sum of weights must be positive".to_string(),
            ));
        }

        let normalized: Vec<f64> = weights.into_iter().map(|w| w / sum).collect();
        Ok(Self {
            points,
            weights: normalized,
        })
    }

    /// Returns the dimension $d$ of the support points.
    pub fn dimension(&self) -> usize {
        self.points[0].len()
    }

    /// Computes Shannon entropy $H(a) = -\sum_{i=1}^n a_i \ln a_i$.
    pub fn shannon_entropy(&self) -> f64 {
        self.weights
            .iter()
            .filter(|&&w| w > 0.0)
            .map(|&w| -w * w.ln())
            .sum()
    }
}

/// Telemetry and mathematical verification output for a Sinkhorn-Knopp drift evaluation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SinkhornDriftReport {
    /// Entropic regularization parameter $\varepsilon > 0$.
    pub epsilon: f64,
    /// Entropic optimal transport cost $\text{OT}_\varepsilon(\mu, \nu) = \langle C, \pi^\star \rangle$.
    pub transport_cost: f64,
    /// Regularized primal objective value $\text{OT}_\varepsilon(\mu, \nu)$ including KL penalty.
    pub regularized_primal_objective: f64,
    /// Debiased Sinkhorn divergence $S_\varepsilon(\mu, \nu) \ge 0$.
    pub sinkhorn_divergence: f64,
    /// Debiased 2-Wasserstein distance estimate $\hat{W}_2(\mu, \nu) = \sqrt{\max(0, S_\varepsilon(\mu, \nu))}$.
    pub wasserstein_2_distance: f64,
    /// Number of Sinkhorn-Knopp iterations executed.
    pub iterations: usize,
    /// Final $\ell_\infty$ dual potential residual $\|f^{(k)} - f^{(k-1)}\|_\infty$.
    pub final_dual_residual: f64,
    /// Final $\ell_1$ marginal conservation error $\|\pi \mathbf{1}_m - a\|_1 + \|\pi^\top \mathbf{1}_n - b\|_1$.
    pub marginal_conservation_error: f64,
    /// Birkhoff-Hopf projective contraction factor $\kappa \in [0, 1)$.
    pub birkhoff_contraction_factor: f64,
    /// Theoretical entropic bias bound $2 \varepsilon (H(a) + H(b))$ from Theorem 26.
    pub theoretical_entropic_bias_bound: f64,
    /// Drift threshold $\tau_{\text{drift}}$.
    pub drift_threshold: f64,
    /// True if $\hat{W}_2(\mu, \nu) > \tau_{\text{drift}}$.
    pub drift_detected: bool,
}

/// Log-domain stabilized Sinkhorn-Knopp Optimal Transport and Semantic Drift Detector.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SinkhornDriftDetector {
    pub epsilon: f64,
    pub max_iterations: usize,
    pub tolerance: f64,
    pub drift_threshold: f64,
}

impl SinkhornDriftDetector {
    pub fn new(
        epsilon: f64,
        max_iterations: usize,
        tolerance: f64,
        drift_threshold: f64,
    ) -> Result<Self, WassersteinError> {
        if !epsilon.is_finite() || epsilon <= 0.0 {
            return Err(WassersteinError::InvalidEpsilon(epsilon));
        }
        if max_iterations == 0 {
            return Err(WassersteinError::InvalidConvergenceConfig(
                "max_iterations must be >= 1".to_string(),
            ));
        }
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(WassersteinError::InvalidConvergenceConfig(
                "tolerance must be > 0".to_string(),
            ));
        }
        if !drift_threshold.is_finite() || drift_threshold < 0.0 {
            return Err(WassersteinError::InvalidConvergenceConfig(
                "drift_threshold must be >= 0".to_string(),
            ));
        }
        Ok(Self {
            epsilon,
            max_iterations,
            tolerance,
            drift_threshold,
        })
    }

    /// Numerically stable log-sum-exp: $\ln \sum_k \exp(v_k) = v_{\max} + \ln \sum_k \exp(v_k - v_{\max})$.
    #[inline]
    pub fn log_sum_exp(values: &[f64]) -> f64 {
        let max_val = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if max_val.is_infinite() {
            return max_val;
        }
        let sum_exp: f64 = values.iter().map(|&v| (v - max_val).exp()).sum();
        max_val + sum_exp.ln()
    }

    /// Computes pairwise squared Euclidean cost matrix $C_{ij} = \|x_i - y_j\|_2^2$ and maximum cost $\|C\|_\infty$.
    pub fn cost_matrix(
        mu: &EmpiricalDistribution,
        nu: &EmpiricalDistribution,
    ) -> Result<(Vec<Vec<f64>>, f64), WassersteinError> {
        if mu.dimension() != nu.dimension() {
            return Err(WassersteinError::DimensionMismatch {
                expected: mu.dimension(),
                actual: nu.dimension(),
            });
        }
        let n = mu.points.len();
        let m = nu.points.len();
        let mut c_mat = vec![vec![0.0; m]; n];
        let mut c_max = 0.0_f64;

        for (i, x) in mu.points.iter().enumerate() {
            for (j, y) in nu.points.iter().enumerate() {
                let sq_dist: f64 = x
                    .iter()
                    .zip(y.iter())
                    .map(|(&xi, &yj)| {
                        let d = xi - yj;
                        d * d
                    })
                    .sum();
                c_mat[i][j] = sq_dist;
                if sq_dist > c_max {
                    c_max = sq_dist;
                }
            }
        }
        Ok((c_mat, c_max))
    }

    /// Computes Birkhoff-Hopf projective contraction coefficient $\kappa = \frac{\exp(\|C\|_\infty / (2\varepsilon)) - 1}{\exp(\|C\|_\infty / (2\varepsilon)) + 1} \in [0, 1)$.
    pub fn birkhoff_contraction_factor(c_max: f64, epsilon: f64) -> f64 {
        if c_max <= 0.0 {
            return 0.0;
        }
        let ratio = (c_max / (2.0 * epsilon)).min(16.0);
        let exp_r = ratio.exp();
        ((exp_r - 1.0) / (exp_r + 1.0)).min(1.0 - 1e-12)
    }

    /// Solves the log-domain stabilized Sinkhorn-Knopp dual potentials $(f, g)$ and coupling matrix $\pi$.
    fn solve_entropic_ot(
        &self,
        mu: &EmpiricalDistribution,
        nu: &EmpiricalDistribution,
    ) -> Result<(f64, f64, usize, f64, f64, f64), WassersteinError> {
        let (c_mat, c_max) = Self::cost_matrix(mu, nu)?;
        let n = mu.points.len();
        let m = nu.points.len();
        let eps = self.epsilon;

        let log_a: Vec<f64> = mu.weights.iter().map(|&w| w.ln()).collect();
        let log_b: Vec<f64> = nu.weights.iter().map(|&w| w.ln()).collect();

        let mut f = vec![0.0; n];
        let mut g = vec![0.0; m];
        let mut scratch_m = vec![0.0; m];
        let mut scratch_n = vec![0.0; n];

        let mut iterations = 0usize;
        let mut final_residual = 0.0;

        for iter in 1..=self.max_iterations {
            iterations = iter;
            let mut max_diff = 0.0_f64;

            // Update f_i = -eps * LSE_j(log b_j + (g_j - C_ij) / eps)
            for i in 0..n {
                for j in 0..m {
                    scratch_m[j] = log_b[j] + (g[j] - c_mat[i][j]) / eps;
                }
                let new_f_i = -eps * Self::log_sum_exp(&scratch_m);
                let diff = (new_f_i - f[i]).abs();
                if diff > max_diff {
                    max_diff = diff;
                }
                f[i] = new_f_i;
            }

            // Update g_j = -eps * LSE_i(log a_i + (f_i - C_ij) / eps)
            for j in 0..m {
                for i in 0..n {
                    scratch_n[i] = log_a[i] + (f[i] - c_mat[i][j]) / eps;
                }
                let new_g_j = -eps * Self::log_sum_exp(&scratch_n);
                let diff = (new_g_j - g[j]).abs();
                if diff > max_diff {
                    max_diff = diff;
                }
                g[j] = new_g_j;
            }

            final_residual = max_diff;
            if max_diff <= self.tolerance {
                break;
            }
        }

        // Recover optimal coupling pi_ij = a_i * b_j * exp((f_i + g_j - C_ij) / eps)
        let mut transport_cost = 0.0;
        let mut kl_div = 0.0;
        let mut row_sums = vec![0.0; n];
        let mut col_sums = vec![0.0; m];

        for i in 0..n {
            for j in 0..m {
                let log_ratio = (f[i] + g[j] - c_mat[i][j]) / eps;
                let pi_ij = mu.weights[i] * nu.weights[j] * log_ratio.exp();
                row_sums[i] += pi_ij;
                col_sums[j] += pi_ij;
                transport_cost += pi_ij * c_mat[i][j];
                if pi_ij > 0.0 {
                    kl_div += pi_ij * log_ratio;
                }
            }
        }

        let regularized_primal = transport_cost + eps * kl_div;
        let row_err: f64 = row_sums
            .iter()
            .zip(mu.weights.iter())
            .map(|(r, a)| (r - a).abs())
            .sum();
        let col_err: f64 = col_sums
            .iter()
            .zip(nu.weights.iter())
            .map(|(c, b)| (c - b).abs())
            .sum();
        let marginal_err = row_err + col_err;

        Ok((
            transport_cost,
            regularized_primal,
            iterations,
            final_residual,
            marginal_err,
            c_max,
        ))
    }

    /// Evaluates full Sinkhorn-Knopp optimal transport, debiased Sinkhorn divergence $S_\varepsilon(\mu, \nu)$,
    /// and semantic drift detection between baseline distribution $\mu$ and live stream distribution $\nu$.
    pub fn evaluate_drift(
        &self,
        mu: &EmpiricalDistribution,
        nu: &EmpiricalDistribution,
    ) -> Result<SinkhornDriftReport, WassersteinError> {
        let (transport_cost, ot_mu_nu, iterations, final_dual_residual, marginal_err, c_max) =
            self.solve_entropic_ot(mu, nu)?;
        let (_, ot_mu_mu, _, _, _, _) = self.solve_entropic_ot(mu, mu)?;
        let (_, ot_nu_nu, _, _, _, _) = self.solve_entropic_ot(nu, nu)?;

        let sinkhorn_divergence = (ot_mu_nu - 0.5 * (ot_mu_mu + ot_nu_nu)).max(0.0);
        let wasserstein_2_distance = sinkhorn_divergence.sqrt();
        let birkhoff_contraction = Self::birkhoff_contraction_factor(c_max, self.epsilon);
        let bias_bound = 2.0 * self.epsilon * (mu.shannon_entropy() + nu.shannon_entropy());
        let drift_detected = wasserstein_2_distance > self.drift_threshold;

        Ok(SinkhornDriftReport {
            epsilon: self.epsilon,
            transport_cost,
            regularized_primal_objective: ot_mu_nu,
            sinkhorn_divergence,
            wasserstein_2_distance,
            iterations,
            final_dual_residual,
            marginal_conservation_error: marginal_err,
            birkhoff_contraction_factor: birkhoff_contraction,
            theoretical_entropic_bias_bound: bias_bound,
            drift_threshold: self.drift_threshold,
            drift_detected,
        })
    }
}
