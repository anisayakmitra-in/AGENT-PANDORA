#![forbid(unsafe_code)]

//! Phase 19: PAC-Bayesian Generalization & Prompt/Policy Risk Certificates
//!
//! Implements McAllester / Maurer empirical-Bernstein relative entropy PAC-Bayes
//! generalization bounds with convex bisection / Newton-Raphson root inversion,
//! Catoni optimal Gibbs posterior temperature selection, and finite-sample policy
//! promotion gating.
//!
//! ## Mathematical Guarantees
//!
//! 1. **Theorem 37 (Maurer Empirical-Bernstein PAC-Bayes KL Bound):**
//!    For any evaluation sample size $n \ge 1$, prior $P$ on policy space $\mathcal{H}$,
//!    bounded empirical loss $\hat{L}_n(Q) \in [0, 1]$, and confidence parameter $\delta \in (0, 1)$,
//!    with probability at least $1 - \delta$ over $S \sim \mathcal{D}^n$, simultaneously for all
//!    data-dependent posterior distributions $Q$ on $\mathcal{H}$:
//!    $$\mathrm{kl}\bigl(\hat{L}_n(Q) \,\|\, L(Q)\bigr) \le \frac{D_{\mathrm{KL}}(Q \,\|\, P) + \ln\left(\frac{2\sqrt{n}}{\delta}\right)}{n},$$
//!    where the binary relative entropy is defined as:
//!    $$\mathrm{kl}(q \,\|\, p) \triangleq q \ln\frac{q}{p} + (1 - q)\ln\frac{1 - q}{1 - p}.$$
//!    The unique upper root $L_{\max}(Q) \in [\hat{L}_n(Q), 1)$ of
//!    $\mathrm{kl}(\hat{L}_n(Q) \,\|\, p) = \Delta$ provides an exact certificate satisfying
//!    $\mathbb{P}(L(Q) \le L_{\max}(Q)) \ge 1 - \delta$, which achieves fast $O(1/n)$ convergence
//!    for near-zero empirical loss, strictly tighter than Pinsker's sub-Gaussian bound
//!    $L_{\mathrm{Pinsker}} = \hat{L} + \sqrt{\Delta/2}$.
//!
//! 2. **Theorem 38 (Catoni Optimal Gibbs Posterior Temperature & Promotion Gate):**
//!    For the Gibbs posterior family $Q_\beta(h) \propto P(h)\exp(-\beta \hat{L}_n(h))$ with
//!    analytical divergence $D_{\mathrm{KL}}(Q_\beta \,\|\, P) = -\beta \hat{L}_n(Q_\beta) - \ln Z_n(\beta)$,
//!    the certified Catoni upper generalization risk is minimized at optimal temperature:
//!    $$\beta^\star = \arg\min_{\beta \in (0, 2n)} \frac{-\ln Z_n(\beta) + \ln(1/\delta)}{\beta(1 - \beta/(2n))}.$$
//!    A candidate prompt/policy $Q_{\mathrm{cand}}$ is promoted to production if and only if
//!    $$L_{\max}(Q_{\mathrm{cand}}) \le L_{\mathrm{target}},$$
//!    guaranteeing a false promotion probability of at most $\delta$.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Binary relative entropy (Kullback-Leibler divergence between two Bernoulli distributions):
/// $\mathrm{kl}(q \,\|\, p) = q \ln(q/p) + (1-q)\ln((1-q)/(1-p))$ with continuous limits at 0 and 1.
#[inline]
pub fn binary_kl(q: f64, p: f64) -> f64 {
    if (q - p).abs() < 1e-15 {
        return 0.0;
    }
    let mut term1 = 0.0;
    if q > 1e-15 {
        if p <= 1e-15 {
            return f64::INFINITY;
        }
        term1 = q * (q / p).ln();
    }
    let mut term2 = 0.0;
    if q < 1.0 - 1e-15 {
        if p >= 1.0 - 1e-15 {
            return f64::INFINITY;
        }
        term2 = (1.0 - q) * ((1.0 - q) / (1.0 - p)).ln();
    }
    term1 + term2
}

/// First derivative of $\mathrm{kl}(q \,\|\, p)$ with respect to $p$:
/// $\frac{\partial}{\partial p}\mathrm{kl}(q \,\|\, p) = \frac{p - q}{p(1 - p)}$.
#[inline]
fn binary_kl_deriv(q: f64, p: f64) -> f64 {
    let p_clamped = p.clamp(1e-14, 1.0 - 1e-14);
    (p_clamped - q) / (p_clamped * (1.0 - p_clamped))
}

/// Errors returned by the PAC-Bayesian monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PacBayesError {
    /// Distribution is empty or dimension mismatch.
    EmptyDistribution,
    /// Dimensions of prior and posterior distributions do not match.
    DimensionMismatch { expected: usize, actual: usize },
    /// Sample size must be at least 1.
    InsufficientSamples(usize),
    /// Confidence parameter $\delta$ must lie in $(0, 1)$.
    InvalidDelta(f64),
    /// Empirical risk must lie in $[0, 1]$.
    InvalidRisk { value: f64 },
    /// Root finding failed to invert $\mathrm{kl}$ divergence.
    RootFindingFailed(String),
    /// Temperature $\beta$ out of valid domain $(0, 2n)$.
    InvalidTemperature { beta: f64, max_allowed: f64 },
}

impl fmt::Display for PacBayesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDistribution => write!(f, "probability distribution cannot be empty"),
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "distribution dimension mismatch: expected {expected}, got {actual}"
                )
            }
            Self::InsufficientSamples(n) => {
                write!(f, "sample size must be >= 1, got {n}")
            }
            Self::InvalidDelta(d) => {
                write!(f, "confidence delta ({d}) must be in (0, 1)")
            }
            Self::InvalidRisk { value } => {
                write!(f, "empirical risk ({value}) must lie in [0, 1]")
            }
            Self::RootFindingFailed(msg) => {
                write!(f, "PAC-Bayes root inversion failed: {msg}")
            }
            Self::InvalidTemperature { beta, max_allowed } => {
                write!(
                    f,
                    "Gibbs temperature beta ({beta}) must be in (0, {max_allowed})"
                )
            }
        }
    }
}

impl std::error::Error for PacBayesError {}

/// Configuration parameters for PAC-Bayesian certification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacBayesConfig {
    /// Failure probability tolerance $\delta \in (0, 1)$.
    pub delta: f64,
    /// Target upper generalization risk threshold.
    pub target_risk: f64,
    /// Safety margin $\epsilon_{\mathrm{margin}} \ge 0$.
    pub risk_margin: f64,
}

impl Default for PacBayesConfig {
    fn default() -> Self {
        Self {
            delta: 0.05,
            target_risk: 0.15,
            risk_margin: 0.02,
        }
    }
}

/// Certified PAC-Bayesian generalization risk certificate (Theorem 37).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacBayesCertificate {
    /// Empirical risk $\hat{L}_n(Q) \in [0, 1]$.
    pub empirical_risk: f64,
    /// Relative entropy / KL-divergence $D_{\mathrm{KL}}(Q \,\|\, P) \ge 0$.
    pub kl_divergence: f64,
    /// Number of evaluation samples $n \ge 1$.
    pub sample_size: usize,
    /// Confidence parameter $\delta \in (0, 1)$.
    pub delta: f64,
    /// Maurer empirical-Bernstein exact inverted risk bound $L_{\max}(Q) \in [0, 1]$.
    pub maurer_risk_bound: f64,
    /// Loose Pinsker upper bound $\hat{L} + \sqrt{\Delta/2}$.
    pub pinsker_risk_bound: f64,
    /// Complexity budget $\Delta = \frac{D_{\mathrm{KL}} + \ln(2\sqrt{n}/\delta)}{n}$.
    pub complexity_delta: f64,
    /// Invariant verification check: $L_{\max}(Q) \le L_{\mathrm{Pinsker}} + 10^{-10}$.
    pub bound_verified: bool,
}

/// Type alias for compatibility with `PROJECT.md`.
pub type RiskCertificate = PacBayesCertificate;

/// Policy promotion verdict under finite-sample PAC-Bayes guarantees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyPromotionVerdict {
    /// Certified risk strictly meets target: $L_{\max}(Q) \le L_{\mathrm{target}}$.
    PromoteToProduction,
    /// Certified risk exceeds target: generalization risk too high to deploy safely.
    RejectHighGeneralizationRisk,
}

/// Type alias for compatibility with `PROJECT.md`.
pub type PromotionGateVerdict = PolicyPromotionVerdict;

/// Complete policy promotion gate evaluation report (Theorem 38).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyPromotionGate {
    /// Promotion verdict.
    pub verdict: PolicyPromotionVerdict,
    /// Certified upper risk bound $L_{\max}(Q)$.
    pub certified_risk_bound: f64,
    /// Target risk threshold.
    pub target_risk: f64,
    /// Risk margin $L_{\mathrm{target}} - L_{\max}(Q)$ (positive means compliant).
    pub risk_margin: f64,
    /// Catoni optimal temperature $\beta^\star$.
    pub optimal_temperature_beta: f64,
    /// Gibbs posterior distribution $Q_{\beta^\star}$.
    pub gibbs_posterior: Vec<f64>,
    /// Catoni theoretical upper risk bound.
    pub catoni_bound: f64,
}

/// Type alias for compatibility with `PROJECT.md`.
pub type GibbsPosteriorResult = PolicyPromotionGate;

/// PAC-Bayesian Generalization & Policy Risk Monitor.
#[derive(Debug, Clone)]
pub struct PacBayesMonitor {
    config: PacBayesConfig,
}

/// Type alias for compatibility with `PROJECT.md`.
pub type PacBayesEngine = PacBayesMonitor;

impl PacBayesMonitor {
    /// Create a new monitor with default configuration.
    pub fn new() -> Self {
        Self {
            config: PacBayesConfig::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: PacBayesConfig) -> Result<Self, PacBayesError> {
        if config.delta <= 0.0 || config.delta >= 1.0 || config.delta.is_nan() {
            return Err(PacBayesError::InvalidDelta(config.delta));
        }
        if config.target_risk < 0.0 || config.target_risk > 1.0 || config.target_risk.is_nan() {
            return Err(PacBayesError::InvalidRisk {
                value: config.target_risk,
            });
        }
        Ok(Self { config })
    }

    /// Access current configuration.
    pub fn config(&self) -> &PacBayesConfig {
        &self.config
    }

    /// Compute Kullback-Leibler divergence $D_{\mathrm{KL}}(Q \,\|\, P) = \sum_i Q_i \ln(Q_i / P_i)$.
    pub fn compute_kl_divergence(q: &[f64], p: &[f64]) -> Result<f64, PacBayesError> {
        if q.is_empty() {
            return Err(PacBayesError::EmptyDistribution);
        }
        if q.len() != p.len() {
            return Err(PacBayesError::DimensionMismatch {
                expected: q.len(),
                actual: p.len(),
            });
        }
        let mut kl = 0.0;
        for (&qi, &pi) in q.iter().zip(p.iter()) {
            if qi < 0.0 || pi < 0.0 {
                return Err(PacBayesError::RootFindingFailed(
                    "probabilities cannot be negative".to_string(),
                ));
            }
            if qi > 1e-15 {
                if pi <= 1e-15 {
                    return Ok(f64::INFINITY);
                }
                kl += qi * (qi / pi).ln();
            }
        }
        Ok(kl.max(0.0))
    }

    /// Invert the binary relative entropy equation $\mathrm{kl}(\hat{L} \,\|\, p) = \Delta$
    /// for $p \in [\hat{L}, 1)$ using bracketed bisection & Newton-Raphson iterations (Theorem 37).
    pub fn invert_binary_kl(
        empirical_risk: f64,
        delta_complexity: f64,
    ) -> Result<f64, PacBayesError> {
        if !(0.0..=1.0).contains(&empirical_risk) || empirical_risk.is_nan() {
            return Err(PacBayesError::InvalidRisk {
                value: empirical_risk,
            });
        }
        if delta_complexity <= 0.0 {
            return Ok(empirical_risk);
        }

        // Exact analytical limit for zero empirical risk:
        // kl(0 || p) = -ln(1 - p) = Delta => 1 - p = exp(-Delta) => p = 1 - exp(-Delta)
        if empirical_risk <= 1e-12 {
            let p_zero = (-delta_complexity).exp_m1().abs(); // 1 - exp(-Delta)
            return Ok(p_zero.clamp(0.0, 1.0));
        }

        // Upper boundary saturation
        if empirical_risk >= 1.0 - 1e-12 {
            return Ok(1.0);
        }

        // Bracket initialization: [empirical_risk, 1.0 - 1e-14]
        let mut low = empirical_risk;
        let mut high = 1.0 - 1e-14;

        // Verify that kl(empirical_risk || high) >= delta_complexity
        if binary_kl(empirical_risk, high) < delta_complexity {
            return Ok(1.0);
        }

        // Bisection to narrow bracket to within 1e-4
        for _ in 0..40 {
            let mid = 0.5 * (low + high);
            let val = binary_kl(empirical_risk, mid);
            if val < delta_complexity {
                low = mid;
            } else {
                high = mid;
            }
            if high - low < 1e-6 {
                break;
            }
        }

        // Newton-Raphson refinement on the convex function f(p) = kl(q || p) - Delta
        let mut p = high;
        for _ in 0..20 {
            let f = binary_kl(empirical_risk, p) - delta_complexity;
            if f.abs() < 1e-12 {
                break;
            }
            let f_prime = binary_kl_deriv(empirical_risk, p);
            if f_prime.abs() < 1e-14 {
                break;
            }
            let p_next = p - f / f_prime;
            // Guard against stepping out of bracket
            p = p_next.clamp(low, high);
        }

        Ok(p.clamp(empirical_risk, 1.0))
    }

    /// Certify generalization risk bound for given empirical risk and complexity (Theorem 37).
    pub fn certify_risk(
        &self,
        empirical_risk: f64,
        n_samples: usize,
        kl_prior_posterior: f64,
        delta: f64,
    ) -> Result<PacBayesCertificate, PacBayesError> {
        if n_samples == 0 {
            return Err(PacBayesError::InsufficientSamples(0));
        }
        if delta <= 0.0 || delta >= 1.0 || delta.is_nan() {
            return Err(PacBayesError::InvalidDelta(delta));
        }
        if !(0.0..=1.0).contains(&empirical_risk) || empirical_risk.is_nan() {
            return Err(PacBayesError::InvalidRisk {
                value: empirical_risk,
            });
        }
        let n_f = n_samples as f64;
        let numerator = kl_prior_posterior + (2.0 * n_f.sqrt() / delta).ln();
        let complexity_delta = numerator / n_f;

        let maurer_risk_bound = Self::invert_binary_kl(empirical_risk, complexity_delta)?;
        let pinsker_risk_bound = (empirical_risk + (0.5 * complexity_delta).sqrt()).min(1.0);

        let bound_verified = maurer_risk_bound <= pinsker_risk_bound + 1e-10;

        Ok(PacBayesCertificate {
            empirical_risk,
            kl_divergence: kl_prior_posterior,
            sample_size: n_samples,
            delta,
            maurer_risk_bound,
            pinsker_risk_bound,
            complexity_delta,
            bound_verified,
        })
    }

    /// Compute Catoni optimal Gibbs posterior temperature $\beta^\star$ and posterior weights (Theorem 38).
    pub fn compute_optimal_gibbs_posterior(
        prior: &[f64],
        sample_losses: &[f64],
        n_samples: usize,
        delta: f64,
    ) -> Result<(f64, Vec<f64>, f64), PacBayesError> {
        let m = prior.len();
        if m == 0 {
            return Err(PacBayesError::EmptyDistribution);
        }
        if sample_losses.len() != m {
            return Err(PacBayesError::DimensionMismatch {
                expected: m,
                actual: sample_losses.len(),
            });
        }
        if n_samples == 0 {
            return Err(PacBayesError::InsufficientSamples(0));
        }
        if delta <= 0.0 || delta >= 1.0 || delta.is_nan() {
            return Err(PacBayesError::InvalidDelta(delta));
        }

        let n_f = n_samples as f64;
        let beta_upper = (2.0 * n_f - 1e-4).max(1.0);

        // Evaluate Catoni bound: B(beta) = (-ln Z(beta) + ln(1/delta)) / (beta * (1 - beta / (2n)))
        let catoni_eval = |beta: f64| -> (f64, Vec<f64>) {
            let mut scaled: Vec<f64> = sample_losses.iter().map(|&l| -beta * l).collect();
            let max_s = scaled.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let mut sum_z = 0.0;
            for (w, &p) in scaled.iter_mut().zip(prior.iter()) {
                *w = p * (*w - max_s).exp();
                sum_z += *w;
            }
            let log_z = max_s + sum_z.ln();
            let posterior: Vec<f64> = if sum_z > 0.0 {
                scaled.into_iter().map(|w| w / sum_z).collect()
            } else {
                vec![1.0 / (m as f64); m]
            };

            let denom = beta * (1.0 - beta / (2.0 * n_f));
            let bound = (-log_z + (1.0 / delta).ln()) / denom.max(1e-12);
            (bound.clamp(0.0, 1.0), posterior)
        };

        // Golden section search on beta in [0.01, min(beta_upper, 4 * sqrt(n))]
        let mut a = 0.01_f64;
        let mut b = beta_upper.min(4.0 * n_f.sqrt());
        let phi = 0.5 * (5.0_f64.sqrt() - 1.0); // ~0.6180339887

        let mut c = b - phi * (b - a);
        let mut d = a + phi * (b - a);

        let (mut fc, _) = catoni_eval(c);
        let (mut fd, _) = catoni_eval(d);

        for _ in 0..50 {
            if (b - a).abs() < 1e-5 {
                break;
            }
            if fc < fd {
                b = d;
                d = c;
                fd = fc;
                c = b - phi * (b - a);
                fc = catoni_eval(c).0;
            } else {
                a = c;
                c = d;
                fc = fd;
                d = a + phi * (b - a);
                fd = catoni_eval(d).0;
            }
        }

        let best_beta = 0.5 * (a + b);
        let (best_bound, posterior) = catoni_eval(best_beta);

        Ok((best_beta, posterior, best_bound))
    }

    /// Evaluate finite-sample policy promotion gating against target risk (Theorem 38).
    pub fn evaluate_promotion_gate(
        &self,
        certificate: &PacBayesCertificate,
        risk_target: f64,
    ) -> PolicyPromotionVerdict {
        if certificate.maurer_risk_bound <= risk_target {
            PolicyPromotionVerdict::PromoteToProduction
        } else {
            PolicyPromotionVerdict::RejectHighGeneralizationRisk
        }
    }

    /// Full policy promotion workflow evaluating candidate policies against target.
    pub fn run_promotion_gate(
        &self,
        candidate_losses: &[f64],
        sample_size: usize,
    ) -> Result<PolicyPromotionGate, PacBayesError> {
        let m = candidate_losses.len();
        if m == 0 {
            return Err(PacBayesError::EmptyDistribution);
        }
        let prior = vec![1.0 / (m as f64); m];
        let (beta_star, posterior, catoni_bound) = Self::compute_optimal_gibbs_posterior(
            &prior,
            candidate_losses,
            sample_size,
            self.config.delta,
        )?;

        let kl = Self::compute_kl_divergence(&posterior, &prior)?;
        let exp_risk: f64 = posterior
            .iter()
            .zip(candidate_losses.iter())
            .map(|(&q, &l)| q * l)
            .sum();

        let cert = self.certify_risk(exp_risk, sample_size, kl, self.config.delta)?;
        let verdict = self.evaluate_promotion_gate(&cert, self.config.target_risk);
        let risk_margin = self.config.target_risk - cert.maurer_risk_bound;

        Ok(PolicyPromotionGate {
            verdict,
            certified_risk_bound: cert.maurer_risk_bound,
            target_risk: self.config.target_risk,
            risk_margin,
            optimal_temperature_beta: beta_star,
            gibbs_posterior: posterior,
            catoni_bound,
        })
    }
}

impl Default for PacBayesEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_kl_identities() {
        assert_eq!(binary_kl(0.5, 0.5), 0.0);
        assert_eq!(binary_kl(0.0, 0.0), 0.0);
        assert_eq!(binary_kl(1.0, 1.0), 0.0);

        // kl(0 || p) = -ln(1 - p)
        let p: f64 = 0.2;
        let expected = -(1.0 - p).ln();
        assert!((binary_kl(0.0, p) - expected).abs() < 1e-12);

        // Pinsker's inequality: kl(q || p) >= 2(q - p)^2
        let q: f64 = 0.1;
        let p: f64 = 0.4;
        let kl_val = binary_kl(q, p);
        let pinsker_lower = 2.0 * (q - p).powi(2);
        assert!(kl_val >= pinsker_lower);
    }

    #[test]
    fn test_theorem_37_maurer_root_inversion_vs_pinsker() {
        // Sample test case from survey: n = 1000, delta = 0.01, L_hat = 0.01, D_KL = ln(10)
        let n = 1000;
        let delta = 0.01;
        let empirical_risk = 0.01;
        let kl = 10.0_f64.ln(); // ~2.302585

        let monitor = PacBayesMonitor::new();
        let cert = monitor
            .certify_risk(empirical_risk, n, kl, delta)
            .expect("certification");

        assert!(cert.bound_verified);
        // Maurer bound must be strictly tighter than Pinsker bound
        assert!(cert.maurer_risk_bound < cert.pinsker_risk_bound);
        assert!(cert.maurer_risk_bound >= empirical_risk);
        assert!(cert.maurer_risk_bound <= 0.05); // Exact is ~0.0336
        assert!(cert.pinsker_risk_bound > 0.07); // Exact is ~0.0843
    }

    #[test]
    fn test_zero_empirical_risk_fast_rate() {
        let n = 500;
        let delta = 0.05;
        let empirical_risk = 0.0;
        let kl = 0.0;

        let monitor = PacBayesMonitor::new();
        let cert = monitor
            .certify_risk(empirical_risk, n, kl, delta)
            .expect("zero risk cert");

        assert!(cert.bound_verified);
        assert!(cert.maurer_risk_bound > 0.0);
        // Fast rate O(1/n) behavior
        assert!(cert.maurer_risk_bound < 0.03);
    }

    #[test]
    fn test_theorem_38_catoni_gibbs_temperature_and_promotion_gate() {
        let candidate_losses = vec![0.02, 0.05, 0.12, 0.25];
        let monitor = PacBayesMonitor::with_config(PacBayesConfig {
            delta: 0.05,
            target_risk: 0.10,
            risk_margin: 0.01,
        })
        .unwrap();

        let gate = monitor
            .run_promotion_gate(&candidate_losses, 800)
            .expect("promotion gate run");

        // Optimal beta should concentrate mass on best policy (index 0)
        assert!(gate.gibbs_posterior[0] > gate.gibbs_posterior[1]);
        assert!(gate.gibbs_posterior[1] > gate.gibbs_posterior[2]);
        assert!(gate.gibbs_posterior[2] > gate.gibbs_posterior[3]);

        // Certified bound should be well within target 0.10 => Promotion
        assert_eq!(gate.verdict, PolicyPromotionVerdict::PromoteToProduction);
        assert!(gate.risk_margin > 0.0);
    }

    #[test]
    fn test_high_risk_rejection_gate() {
        let high_losses = vec![0.35, 0.40, 0.55];
        let monitor = PacBayesMonitor::with_config(PacBayesConfig {
            delta: 0.05,
            target_risk: 0.10,
            risk_margin: 0.01,
        })
        .unwrap();

        let gate = monitor
            .run_promotion_gate(&high_losses, 500)
            .expect("promotion gate run");

        assert_eq!(
            gate.verdict,
            PolicyPromotionVerdict::RejectHighGeneralizationRisk
        );
        assert!(gate.risk_margin < 0.0);
    }
}
