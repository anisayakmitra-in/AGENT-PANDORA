#![forbid(unsafe_code)]

//! Phase 18: Mean-Field Game Equilibrium & Multi-Agent Congestion Control
//!
//! Implements discrete McKean-Vlasov fixed-point equilibrium computation,
//! Picard/Mann contraction dynamics under Lipschitz swarm interaction costs,
//! and Price-of-Anarchy ($PoA \le 4/3$) bounding for multi-agent tool/channel contention.
//!
//! ## Mathematical Guarantees
//!
//! 1. **Theorem 35 (Discrete McKean-Vlasov / Nash-MFG Fixed-Point Existence & Contraction):**
//!    Let $\mathbf{m} \in \Delta_K$ be the population distribution over $K \ge 2$ shared
//!    computational resources with affine congestion costs $c_k(m_k) = c_{0, k} + \beta_k m_k$
//!    ($\beta_{\max} = \max_{k} \beta_k$). Under Boltzmann-Gibbs best response
//!    $$\Phi_k(\mathbf{m}) = \frac{\exp(-c_k(m_k)/\tau)}{\sum_{j=1}^K \exp(-c_j(m_j)/\tau)}, \quad \tau > 0:$$
//!    - By Brouwer's Fixed-Point Theorem on the compact convex simplex $\Delta_K$,
//!      there exists at least one Mean-Field Equilibrium $\mathbf{m}^\star = \Phi(\mathbf{m}^\star)$.
//!    - In the $\ell_1$ norm $\|\mathbf{x}\|_1 = \sum_k |x_k|$, the best-response operator
//!      $\Phi$ has Lipschitz contraction constant:
//!      $$\|\Phi(\mathbf{m}) - \Phi(\mathbf{m}')\|_1 \le \kappa \|\mathbf{m} - \mathbf{m}'\|_1, \quad \kappa = \frac{\beta_{\max}}{2\tau}.$$
//!    - Whenever $\tau > \frac{\beta_{\max}}{2}$, $\kappa < 1$, the equilibrium $\mathbf{m}^\star$
//!      is strictly unique, and Picard iterates converge geometrically:
//!      $$\|\mathbf{m}^{(t)} - \mathbf{m}^\star\|_1 \le 2\kappa^t.$$
//!
//! 2. **Theorem 36 (Price of Anarchy Bound for Affine Congestion Games):**
//!    For affine latency costs $c_k(x) = a_k x + b_k$ ($a_k, b_k \ge 0$), the routing game is
//!    $(1, 1/4)$-smooth:
//!    $$\sum_{k=1}^K m_k' c_k(m_k) \le C(\mathbf{m}') + \frac{1}{4} C(\mathbf{m}).$$
//!    Consequently, the Price of Anarchy between the Nash equilibrium $\mathbf{m}^{\mathrm{NE}}$
//!    and the social optimum $\mathbf{m}^{\mathrm{OPT}}$ is strictly bounded by:
//!    $$PoA \triangleq \frac{C(\mathbf{m}^{\mathrm{NE}})}{C(\mathbf{m}^{\mathrm{OPT}})} \le \frac{4}{3} \approx 1.333333.$$
//!    This upper bound is tight and attained by Pigou's network.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Theoretical upper bound on the Price of Anarchy for affine congestion games (Theorem 36).
pub const THEORETICAL_POA_UPPER_BOUND: f64 = 4.0 / 3.0;

/// Errors returned by the Mean-Field Game congestion engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MfgError {
    /// System requires at least 2 resources for routing game.
    InsufficientResources { count: usize },
    /// Temperature $\tau$ must be strictly positive.
    InvalidTemperature { tau: f64 },
    /// Congestion slope or base cost must be non-negative.
    InvalidParameter {
        param: &'static str,
        value: f64,
        reason: &'static str,
    },
    /// Distribution dimensions mismatch or values do not sum to 1.
    InvalidDistribution(String),
    /// Iteration did not converge within maximum allowed steps.
    ConvergenceFailed { iterations: usize, residual: f64 },
}

impl fmt::Display for MfgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientResources { count } => {
                write!(
                    f,
                    "MFG congestion game requires at least 2 resources, got {count}"
                )
            }
            Self::InvalidTemperature { tau } => {
                write!(f, "temperature tau ({tau}) must be strictly positive (> 0)")
            }
            Self::InvalidParameter {
                param,
                value,
                reason,
            } => {
                write!(f, "invalid parameter {param} = {value}: {reason}")
            }
            Self::InvalidDistribution(msg) => {
                write!(f, "invalid population distribution: {msg}")
            }
            Self::ConvergenceFailed {
                iterations,
                residual,
            } => {
                write!(
                    f,
                    "equilibrium convergence failed after {iterations} iterations (residual: {residual:.2e})"
                )
            }
        }
    }
}

impl std::error::Error for MfgError {}

/// Configuration for a single shared resource (tool/model/channel) in the MFG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MfgResourceConfig {
    /// Unique resource identifier.
    pub id: String,
    /// Base uncongested latency / cost $c_{0, k} \ge 0$.
    pub base_cost: f64,
    /// Marginal congestion slope $\beta_k \ge 0$.
    pub congestion_slope: f64,
}

impl MfgResourceConfig {
    /// Create a new resource configuration.
    pub fn new(
        id: impl Into<String>,
        base_cost: f64,
        congestion_slope: f64,
    ) -> Result<Self, MfgError> {
        if base_cost < 0.0 || base_cost.is_nan() {
            return Err(MfgError::InvalidParameter {
                param: "base_cost",
                value: base_cost,
                reason: "base cost must be non-negative and finite",
            });
        }
        if congestion_slope < 0.0 || congestion_slope.is_nan() {
            return Err(MfgError::InvalidParameter {
                param: "congestion_slope",
                value: congestion_slope,
                reason: "congestion slope must be non-negative and finite",
            });
        }
        Ok(Self {
            id: id.into(),
            base_cost,
            congestion_slope,
        })
    }

    /// Compute affine cost $c_k(m_k) = c_{0, k} + \beta_k m_k$.
    #[inline]
    pub fn evaluate_cost(&self, occupancy: f64) -> f64 {
        self.base_cost + self.congestion_slope * occupancy.max(0.0)
    }
}

/// Agent cost state summary across all resources under a given population distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MfgAgentCostState {
    /// Current population distribution $\mathbf{m} \in \Delta_K$.
    pub distribution: Vec<f64>,
    /// Latency / cost per resource $\mathbf{c}(\mathbf{m})$.
    pub resource_costs: Vec<f64>,
    /// Expected cost per agent $\sum_k m_k c_k(m_k)$.
    pub expected_cost: f64,
}

/// Result of McKean-Vlasov fixed-point equilibrium solver (Theorem 35).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MfgEquilibriumResult {
    /// Equilibrium distribution $\mathbf{m}^\star \in \Delta_K$.
    pub equilibrium_distribution: Vec<f64>,
    /// Resource costs at equilibrium $\mathbf{c}(\mathbf{m}^\star)$.
    pub resource_costs: Vec<f64>,
    /// Number of iterations executed until convergence.
    pub iterations: usize,
    /// Final $\ell_1$ residual $\|\mathbf{m}^{(t+1)} - \mathbf{m}^{(t)}\|_1$.
    pub residual_norm: f64,
    /// Contraction modulus $\kappa = \frac{\beta_{\max}}{2\tau}$.
    pub contraction_modulus: f64,
    /// Whether the game is a strict contraction ($\kappa < 1.0$).
    pub is_strict_contraction: bool,
    /// Theoretical geometric bound on initial distance $2\kappa^t$.
    pub theoretical_error_bound: f64,
}

/// Type alias for compatibility with alternative naming conventions.
pub type MfgFixedPointResult = MfgEquilibriumResult;

/// Price-of-Anarchy analysis report (Theorem 36).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoABoundResult {
    /// Social cost at Nash / Wardrop equilibrium: $C(\mathbf{m}^{\mathrm{NE}})$.
    pub nash_social_cost: f64,
    /// Social cost at social optimum: $C(\mathbf{m}^{\mathrm{OPT}})$.
    pub optimal_social_cost: f64,
    /// Price of Anarchy: $PoA = C(\mathbf{m}^{\mathrm{NE}}) / C(\mathbf{m}^{\mathrm{OPT}})$.
    pub price_of_anarchy: f64,
    /// Theoretical upper bound $\frac{4}{3} \approx 1.333333$.
    pub theoretical_upper_bound: f64,
    /// Whether the computed $PoA \le \frac{4}{3} + 10^{-6}$.
    pub bound_satisfied: bool,
    /// Socially optimal distribution $\mathbf{m}^{\mathrm{OPT}}$.
    pub optimal_distribution: Vec<f64>,
}

/// Type alias for compatibility with alternative naming conventions.
pub type PriceOfAnarchyReport = PoABoundResult;

/// Comprehensive status summary of the Mean-Field Game engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeanFieldCongestionStatus {
    /// Total number of competing resources $K$.
    pub num_resources: usize,
    /// Entropic exploration temperature $\tau > 0$.
    pub temperature_tau: f64,
    /// Damping parameter $\alpha \in (0, 1]$.
    pub damping_alpha: f64,
    /// Contraction modulus $\kappa = \frac{\beta_{\max}}{2\tau}$.
    pub contraction_modulus: f64,
    /// Fixed-point equilibrium solution.
    pub fixed_point: MfgEquilibriumResult,
    /// Price-of-Anarchy analysis.
    pub poa: PoABoundResult,
}

/// Mean-Field Game congestion control engine.
#[derive(Debug, Clone)]
pub struct MeanFieldGame {
    resources: Vec<MfgResourceConfig>,
    temperature: f64,
    damping_alpha: f64,
}

/// Type alias for compatibility with `PROJECT.md`.
pub type MfgEngine = MeanFieldGame;
/// Type alias for compatibility with survey specs.
pub type MfgCongestionModel = MeanFieldGame;

impl MeanFieldGame {
    /// Create a new Mean-Field Game engine with given resources and temperature.
    pub fn new(resources: Vec<MfgResourceConfig>, temperature: f64) -> Result<Self, MfgError> {
        if resources.len() < 2 {
            return Err(MfgError::InsufficientResources {
                count: resources.len(),
            });
        }
        if temperature <= 0.0 || temperature.is_nan() {
            return Err(MfgError::InvalidTemperature { tau: temperature });
        }
        let beta_max = resources
            .iter()
            .map(|r| r.congestion_slope)
            .fold(0.0_f64, f64::max);
        let damping_alpha = if beta_max <= 0.0 {
            1.0
        } else {
            (2.0 * temperature / (2.0 * temperature + beta_max)).clamp(0.05, 1.0)
        };

        Ok(Self {
            resources,
            temperature,
            damping_alpha,
        })
    }

    /// Create with explicit damping parameter $\alpha \in (0, 1]$.
    pub fn with_damping(
        resources: Vec<MfgResourceConfig>,
        temperature: f64,
        damping_alpha: f64,
    ) -> Result<Self, MfgError> {
        let mut engine = Self::new(resources, temperature)?;
        if damping_alpha <= 0.0 || damping_alpha > 1.0 || damping_alpha.is_nan() {
            return Err(MfgError::InvalidParameter {
                param: "damping_alpha",
                value: damping_alpha,
                reason: "damping parameter alpha must be in (0, 1]",
            });
        }
        engine.damping_alpha = damping_alpha;
        Ok(engine)
    }

    /// Resources participating in the game.
    pub fn resources(&self) -> &[MfgResourceConfig] {
        &self.resources
    }

    /// Number of resources $K$.
    pub fn num_resources(&self) -> usize {
        self.resources.len()
    }

    /// Exploration temperature $\tau$.
    pub fn temperature(&self) -> f64 {
        self.temperature
    }

    /// Damping parameter $\alpha$.
    pub fn damping_alpha(&self) -> f64 {
        self.damping_alpha
    }

    /// Maximal congestion slope $\beta_{\max} = \max_k \beta_k$.
    pub fn beta_max(&self) -> f64 {
        self.resources
            .iter()
            .map(|r| r.congestion_slope)
            .fold(0.0_f64, f64::max)
    }

    /// Contraction modulus $\kappa = \frac{\beta_{\max}}{2\tau}$ (Theorem 35).
    pub fn contraction_modulus(&self) -> f64 {
        self.beta_max() / (2.0 * self.temperature)
    }

    /// Whether the operator is guaranteed to be a strict contraction ($\kappa < 1$).
    pub fn is_strict_contraction(&self) -> bool {
        self.contraction_modulus() < 1.0
    }

    /// Evaluate costs $\mathbf{c}(\mathbf{m})$ for a given population distribution $\mathbf{m} \in \Delta_K$.
    pub fn evaluate_costs(&self, distribution: &[f64]) -> Result<Vec<f64>, MfgError> {
        self.validate_distribution(distribution)?;
        Ok(self
            .resources
            .iter()
            .zip(distribution.iter())
            .map(|(res, &m_k)| res.evaluate_cost(m_k))
            .collect())
    }

    /// Evaluate Boltzmann-Gibbs best response $\Phi(\mathbf{m})$ (Equation 28).
    /// Uses max-subtraction log-domain stabilization to prevent numerical overflow/underflow.
    pub fn best_response(&self, distribution: &[f64]) -> Result<Vec<f64>, MfgError> {
        let costs = self.evaluate_costs(distribution)?;
        let mut scaled_neg_costs: Vec<f64> = costs.iter().map(|&c| -c / self.temperature).collect();

        // Max subtraction for numeric stability
        let max_val = scaled_neg_costs
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);

        let mut sum_exp = 0.0;
        for val in &mut scaled_neg_costs {
            *val = (*val - max_val).exp();
            sum_exp += *val;
        }

        if sum_exp <= 0.0 || sum_exp.is_nan() {
            // Degenerate case fallback: uniform distribution
            let k = self.num_resources() as f64;
            return Ok(vec![1.0 / k; self.num_resources()]);
        }

        let resp: Vec<f64> = scaled_neg_costs.into_iter().map(|v| v / sum_exp).collect();
        Ok(resp)
    }

    /// Compute Discrete McKean-Vlasov fixed-point equilibrium $\mathbf{m}^\star = \Phi(\mathbf{m}^\star)$ (Theorem 35).
    pub fn compute_equilibrium(
        &self,
        max_iterations: usize,
        tolerance: f64,
    ) -> Result<MfgEquilibriumResult, MfgError> {
        let k = self.num_resources();
        let mut m = vec![1.0 / (k as f64); k];
        let kappa = self.contraction_modulus();
        let is_strict = kappa < 1.0;

        let alpha = self.damping_alpha;
        let mut iterations = 0;
        let mut final_residual = 0.0;

        for step in 0..max_iterations {
            iterations = step + 1;
            let phi = self.best_response(&m)?;

            // Krasnoselskii-Mann damped step: m_{t+1} = (1 - alpha) * m_t + alpha * phi(m_t)
            let mut m_next = vec![0.0; k];
            let mut l1_diff = 0.0;
            let mut sum_next = 0.0;

            for i in 0..k {
                let next_val = (1.0 - alpha) * m[i] + alpha * phi[i];
                let clamped = next_val.max(0.0);
                m_next[i] = clamped;
                sum_next += clamped;
                l1_diff += (m_next[i] - m[i]).abs();
            }

            // Simplex normalization
            if sum_next > 0.0 {
                for val in &mut m_next {
                    *val /= sum_next;
                }
            }

            final_residual = l1_diff;
            m = m_next;

            if l1_diff < tolerance {
                break;
            }
        }

        if final_residual >= tolerance && iterations >= max_iterations {
            return Err(MfgError::ConvergenceFailed {
                iterations,
                residual: final_residual,
            });
        }

        let resource_costs = self.evaluate_costs(&m)?;
        let theoretical_error_bound = if is_strict {
            2.0 * kappa.powi(iterations as i32)
        } else {
            f64::INFINITY
        };

        Ok(MfgEquilibriumResult {
            equilibrium_distribution: m,
            resource_costs,
            iterations,
            residual_norm: final_residual,
            contraction_modulus: kappa,
            is_strict_contraction: is_strict,
            theoretical_error_bound,
        })
    }

    /// Compute Social Cost $C(\mathbf{m}) = \sum_k m_k c_k(m_k) = \sum_k (\beta_k m_k^2 + c_{0, k} m_k)$.
    pub fn social_cost(&self, distribution: &[f64]) -> Result<f64, MfgError> {
        self.validate_distribution(distribution)?;
        let mut total = 0.0;
        for (res, &m_k) in self.resources.iter().zip(distribution.iter()) {
            total += m_k * res.evaluate_cost(m_k);
        }
        Ok(total)
    }

    /// Compute Social Optimum $\mathbf{m}^{\mathrm{OPT}} = \arg\min_{\mathbf{m} \in \Delta_K} C(\mathbf{m})$.
    /// Uses projected gradient descent on the simplex $\Delta_K$.
    pub fn compute_social_optimum(&self) -> Result<(Vec<f64>, f64), MfgError> {
        let k = self.num_resources();
        let mut m = vec![1.0 / (k as f64); k];

        // Gradient of C(m) w.r.t m_k: 2 * beta_k * m_k + c_{0, k}
        // Lipschitz constant of gradient: L = 2 * beta_max
        let beta_max = self.beta_max();
        let step_size = if beta_max > 0.0 {
            1.0 / (2.0 * beta_max + 1e-4)
        } else {
            0.1
        };

        for _ in 0..1000 {
            let mut grad = vec![0.0; k];
            for (i, res) in self.resources.iter().enumerate() {
                grad[i] = 2.0 * res.congestion_slope * m[i] + res.base_cost;
            }

            // Gradient descent step: v = m - step_size * grad
            let mut v = vec![0.0; k];
            for i in 0..k {
                v[i] = m[i] - step_size * grad[i];
            }

            // Project v onto simplex Delta_K
            let m_next = project_onto_simplex(&v);

            let mut diff = 0.0;
            for i in 0..k {
                diff += (m_next[i] - m[i]).abs();
            }
            m = m_next;
            if diff < 1e-10 {
                break;
            }
        }

        let opt_cost = self.social_cost(&m)?;
        Ok((m, opt_cost))
    }

    /// Compute Price of Anarchy $PoA = \frac{C(\mathbf{m}^{\mathrm{NE}})}{C(\mathbf{m}^{\mathrm{OPT}})}$
    /// and certify Roughgarden $(1, 1/4)$-smoothness bound $PoA \le 4/3$ (Theorem 36).
    pub fn compute_price_of_anarchy(&self) -> Result<PoABoundResult, MfgError> {
        let eq = self.compute_equilibrium(1000, 1e-8)?;
        let nash_cost = self.social_cost(&eq.equilibrium_distribution)?;
        let (opt_m, opt_cost) = self.compute_social_optimum()?;

        let poa = if opt_cost <= 1e-12 {
            1.0
        } else {
            nash_cost / opt_cost
        };

        // Numerical tolerance: 4/3 + 1e-6
        let bound_satisfied = poa <= THEORETICAL_POA_UPPER_BOUND + 1e-6;

        Ok(PoABoundResult {
            nash_social_cost: nash_cost,
            optimal_social_cost: opt_cost,
            price_of_anarchy: poa,
            theoretical_upper_bound: THEORETICAL_POA_UPPER_BOUND,
            bound_satisfied,
            optimal_distribution: opt_m,
        })
    }

    /// Get complete status snapshot for assurance telemetry and CLI surfaces.
    pub fn status(&self) -> Result<MeanFieldCongestionStatus, MfgError> {
        let eq = self.compute_equilibrium(500, 1e-8)?;
        let poa = self.compute_price_of_anarchy()?;
        Ok(MeanFieldCongestionStatus {
            num_resources: self.num_resources(),
            temperature_tau: self.temperature,
            damping_alpha: self.damping_alpha,
            contraction_modulus: self.contraction_modulus(),
            fixed_point: eq,
            poa,
        })
    }

    fn validate_distribution(&self, dist: &[f64]) -> Result<(), MfgError> {
        if dist.len() != self.num_resources() {
            return Err(MfgError::InvalidDistribution(format!(
                "expected {} elements, got {}",
                self.num_resources(),
                dist.len()
            )));
        }
        let mut sum = 0.0;
        for &x in dist {
            if x < -1e-8 || x.is_nan() {
                return Err(MfgError::InvalidDistribution(format!(
                    "probability elements must be non-negative, got {x}"
                )));
            }
            sum += x;
        }
        if (sum - 1.0).abs() > 1e-4 {
            return Err(MfgError::InvalidDistribution(format!(
                "distribution must sum to 1.0, got sum {sum:.6}"
            )));
        }
        Ok(())
    }
}

/// Project an arbitrary vector $v \in \mathbb{R}^K$ onto the probability simplex $\Delta_K$:
/// $\min_{x \in \Delta_K} \frac{1}{2} \|x - v\|_2^2$ in $O(K \log K)$ time (Duchi et al. 2008).
pub fn project_onto_simplex(v: &[f64]) -> Vec<f64> {
    let k = v.len();
    if k == 0 {
        return Vec::new();
    }
    let mut u = v.to_vec();
    u.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let mut cssv = 0.0;
    let mut rho = 0;
    for (j, &val) in u.iter().enumerate() {
        cssv += val;
        let cond = val - (cssv - 1.0) / ((j + 1) as f64);
        if cond > 0.0 {
            rho = j + 1;
        }
    }

    let sum_rho: f64 = u[..rho].iter().sum();
    let theta = (sum_rho - 1.0) / (rho as f64);

    v.iter().map(|&val| (val - theta).max(0.0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simplex_projection() {
        let v = vec![0.5, 0.5];
        let p = project_onto_simplex(&v);
        assert!((p[0] - 0.5).abs() < 1e-6);
        assert!((p[1] - 0.5).abs() < 1e-6);

        let v2 = vec![1.2, -0.4, 0.2];
        let p2 = project_onto_simplex(&v2);
        let sum: f64 = p2.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
        for &x in &p2 {
            assert!(x >= 0.0);
        }
    }

    #[test]
    fn test_mfg_creation_and_parameter_validation() {
        let res = vec![
            MfgResourceConfig::new("r1", 1.0, 2.0).unwrap(),
            MfgResourceConfig::new("r2", 2.0, 1.0).unwrap(),
        ];
        let game = MeanFieldGame::new(res, 2.0).unwrap();
        assert_eq!(game.num_resources(), 2);
        assert_eq!(game.beta_max(), 2.0);
        // kappa = beta_max / (2 * tau) = 2.0 / (2 * 2.0) = 0.5
        assert!((game.contraction_modulus() - 0.5).abs() < 1e-6);
        assert!(game.is_strict_contraction());

        // Error cases
        assert!(MeanFieldGame::new(vec![], 1.0).is_err());
        assert!(
            MeanFieldGame::new(vec![MfgResourceConfig::new("r1", 1.0, 1.0).unwrap()], 1.0).is_err()
        );
        assert!(
            MeanFieldGame::new(
                vec![
                    MfgResourceConfig::new("r1", 1.0, 1.0).unwrap(),
                    MfgResourceConfig::new("r2", 1.0, 1.0).unwrap(),
                ],
                -1.0
            )
            .is_err()
        );
    }

    #[test]
    fn test_theorem_35_contraction_and_convergence() {
        let res = vec![
            MfgResourceConfig::new("fast_core", 1.0, 3.0).unwrap(),
            MfgResourceConfig::new("slow_core", 3.0, 1.0).unwrap(),
            MfgResourceConfig::new("gpu_pool", 2.0, 2.0).unwrap(),
        ];
        // tau = 5.0 => beta_max = 3.0 => kappa = 3.0 / 10.0 = 0.3 < 1.0
        let game = MeanFieldGame::new(res, 5.0).unwrap();
        assert!(game.is_strict_contraction());
        assert!((game.contraction_modulus() - 0.3).abs() < 1e-6);

        let eq = game.compute_equilibrium(200, 1e-8).expect("convergence");
        assert!(eq.residual_norm < 1e-8);
        assert!(eq.iterations < 50);

        let sum: f64 = eq.equilibrium_distribution.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);

        // Verification of fixed-point condition: Phi(m*) == m*
        let phi = game.best_response(&eq.equilibrium_distribution).unwrap();
        for (m_k, p_k) in eq.equilibrium_distribution.iter().zip(phi.iter()) {
            assert!((m_k - p_k).abs() < 1e-6);
        }
    }

    #[test]
    fn test_theorem_36_pigou_price_of_anarchy_bound() {
        // Pigou's classical tight example:
        // Channel 1: c1(x) = 1.0 * x + 0.0 (beta=1.0, c0=0.0)
        // Channel 2: c2(x) = 0.0 * x + 1.0 (beta=0.0, c0=1.0)
        let res = vec![
            MfgResourceConfig::new("pigou_congested", 0.0, 1.0).unwrap(),
            MfgResourceConfig::new("pigou_uncongested", 1.0, 0.0).unwrap(),
        ];

        // Low temperature to approximate Wardrop equilibrium
        let game = MeanFieldGame::with_damping(res, 0.05, 0.1).unwrap();
        let poa_report = game.compute_price_of_anarchy().expect("poa report");

        // Theoretical social optimum: m1 = 0.5, m2 = 0.5 => C(m_opt) = 0.5^2 + 0.5 = 0.75
        assert!((poa_report.optimal_distribution[0] - 0.5).abs() < 1e-2);
        assert!((poa_report.optimal_social_cost - 0.75).abs() < 1e-2);

        // Wardrop equilibrium concentrates on resource 1: m1 ~ 1.0, m2 ~ 0.0 => C(m_ne) ~ 1.0
        // PoA = 1.0 / 0.75 = 4/3 ~ 1.333333
        assert!(poa_report.bound_satisfied);
        assert!(poa_report.price_of_anarchy <= THEORETICAL_POA_UPPER_BOUND + 1e-3);
        assert!(poa_report.price_of_anarchy >= 1.0 - 1e-6);
    }

    #[test]
    fn test_symmetric_resources_equipartition() {
        // 3 identical resources must yield exact equipartition (1/3, 1/3, 1/3)
        let res = vec![
            MfgResourceConfig::new("r1", 2.0, 1.5).unwrap(),
            MfgResourceConfig::new("r2", 2.0, 1.5).unwrap(),
            MfgResourceConfig::new("r3", 2.0, 1.5).unwrap(),
        ];
        let game = MeanFieldGame::new(res, 1.0).unwrap();
        let eq = game.compute_equilibrium(100, 1e-8).unwrap();
        for &m_k in &eq.equilibrium_distribution {
            assert!((m_k - 1.0 / 3.0).abs() < 1e-6);
        }
        let poa = game.compute_price_of_anarchy().unwrap();
        // For identical symmetric resources, Nash = Social Optimum => PoA = 1.0
        assert!((poa.price_of_anarchy - 1.0).abs() < 1e-4);
    }
}
