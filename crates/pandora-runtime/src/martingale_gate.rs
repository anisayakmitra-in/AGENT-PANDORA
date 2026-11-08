//! Phase 14: Martingale Anytime-Valid Canary Gate with e-Processes & Confidence Sequences.
//!
//! Provides sequential, time-uniform hypothesis testing and confidence sequences for
//! autonomous self-evolution canary rollouts without fixed-horizon $p$-value peeking inflation:
//! - **Non-Negative Test Supermartingale ($e$-Process, Theorem 27):**
//!   For sequential bounded observations $X_t \in [-1, 1]$ with null hypothesis $H_0: \mathbb{E}[X_t \mid \mathcal{F}_{t-1}] \le 0$
//!   (where $X_t = L_{\text{canary}, t} - L_{\text{baseline}, t} - \delta_{\text{tol}}$ measures excess regression),
//!   the wealth process
//!   $$E_t = \prod_{i=1}^t (1 + \lambda_i X_i), \quad E_0 = 1$$
//!   with predictable $\mathcal{F}_{t-1}$-measurable betting fraction $\lambda_t \in [0, 1)$ is a non-negative
//!   supermartingale under $H_0$. By **Ville's Maximal Inequality (Theorem 27)**:
//!   $$\Pr_{H_0}\left(\exists t \ge 1 : E_t \ge \frac{1}{\alpha}\right) \le \alpha$$
//!   guaranteeing anytime-valid Type-I false-alarm control across arbitrary data-dependent stopping times $\tau$.
//! - **Howard et al. Sub-Gaussian / Empirical-Bernstein Confidence Sequences (Theorem 28):**
//!   Constructs a time-uniform boundary $r_t(\alpha)$ such that:
//!   $$\Pr\left(\forall t \ge 1 : |\hat{\mu}_t - \mu| \le r_t(\alpha)\right) \ge 1 - \alpha$$
//!   where $r_t(\alpha) = \sigma \sqrt{\frac{2(t \rho^2 + 1)}{t^2 \rho^2} \ln\left(\frac{\sqrt{t \rho^2 + 1}}{\alpha}\right)}$
//!   shrinks at the optimal iterated-logarithm / Laplace-mixture rate $O(\sqrt{\ln(t) / t})$.
//!
//! Strictly `#![forbid(unsafe_code)]` compliant.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fmt;

/// Error taxonomy for Martingale Anytime-Valid Canary Gating.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MartingaleError {
    /// Significance level $\alpha \notin (0, 1)$.
    InvalidAlpha(f64),
    /// Sub-Gaussian scale parameter $\sigma \le 0$ or non-finite.
    InvalidSigma(f64),
    /// Observation outside bounded range $[-1, 1]$ or non-finite.
    InvalidObservation { value: f64, reason: &'static str },
    /// Betting fraction $\lambda \notin [0, 1)$.
    InvalidBettingFraction(f64),
}

impl fmt::Display for MartingaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAlpha(a) => {
                write!(
                    f,
                    "invalid significance level alpha={a}; must satisfy 0 < alpha < 1"
                )
            }
            Self::InvalidSigma(s) => {
                write!(
                    f,
                    "invalid sub-Gaussian scale sigma={s}; must satisfy sigma > 0"
                )
            }
            Self::InvalidObservation { value, reason } => {
                write!(f, "invalid sequential observation {value}: {reason}")
            }
            Self::InvalidBettingFraction(lam) => {
                write!(
                    f,
                    "invalid betting fraction lambda={lam}; must satisfy 0 <= lambda < 1"
                )
            }
        }
    }
}

impl std::error::Error for MartingaleError {}

/// Decision state of the anytime-valid canary gate at step $t$.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryGateVerdict {
    /// Rollback triggered: $E_t \ge 1/\alpha$ rejects $H_0$ (statistically significant canary regression).
    RollbackRegression,
    /// Promote triggered: upper confidence bound $\hat{\mu}_t + r_t(\alpha) < 0$ proves canary superiority.
    PromoteCanary,
    /// Continue monitoring: evidence has not yet crossed either anytime-valid boundary.
    ContinueMonitoring,
}

impl CanaryGateVerdict {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::RollbackRegression => "rollback_regression",
            Self::PromoteCanary => "promote_canary",
            Self::ContinueMonitoring => "continue_monitoring",
        }
    }
}

/// Snapshot of the martingale canary gate state at step $t$.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MartingaleGateSnapshot {
    /// Step index $t \ge 0$.
    pub step_t: usize,
    /// Current test martingale wealth $E_t = \prod_{i=1}^t (1 + \lambda_i X_i)$.
    pub e_value: f64,
    /// Running maximum $e$-value $\max_{1 \le s \le t} E_s$.
    pub peak_e_value: f64,
    /// Anytime-valid p-value $p_t = \min\left(1, \frac{1}{\max_{1 \le s \le t} E_s}\right)$ via Ville's inequality.
    pub anytime_p_value: f64,
    /// Rejection threshold $1 / \alpha$.
    pub ville_rejection_threshold: f64,
    /// Empirical mean difference $\hat{\mu}_t = \frac{1}{t} \sum_{i=1}^t X_i$.
    pub empirical_mean: f64,
    /// Time-uniform confidence radius $r_t(\alpha)$.
    pub confidence_radius: f64,
    /// Lower confidence sequence bound $\hat{\mu}_t - r_t(\alpha)$.
    pub lower_confidence_bound: f64,
    /// Upper confidence sequence bound $\hat{\mu}_t + r_t(\alpha)$.
    pub upper_confidence_bound: f64,
    ///Predictable betting fraction $\lambda_t$ used at step $t$.
    pub betting_fraction_lambda: f64,
    /// Current gate verdict.
    pub verdict: CanaryGateVerdict,
}

/// Stateful anytime-valid martingale canary gate tracking sequential observations $X_1, X_2, \dots$.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MartingaleCanaryGate {
    /// Target false-positive significance level $\alpha \in (0, 1)$.
    pub alpha: f64,
    /// Sub-Gaussian variance proxy scale $\sigma > 0$ (default $0.5$ for $[-1, 1]$ bounded variables).
    pub sigma: f64,
    /// Laplace mixture precision parameter $\rho > 0$.
    pub mixture_rho: f64,
    /// Minimum sample count before allowing promotion.
    pub min_promote_samples: usize,
    /// Current step count $t$.
    pub step_t: usize,
    /// Log-wealth $\ln E_t = \sum_{i=1}^t \ln(1 + \lambda_i X_i)$ for numerical stability.
    pub log_e_value: f64,
    /// Peak $e$-value $\max_{0 \le s \le t} E_s$.
    pub peak_e_value: f64,
    /// Cumulative sum $\sum_{i=1}^t X_i$.
    pub sum_x: f64,
    /// Cumulative squared deviation sum $\sum_{i=1}^t (X_i - \hat{\mu}_{i-1})^2$.
    pub sum_sq_dev: f64,
    /// Latest verdict.
    pub verdict: CanaryGateVerdict,
}

impl MartingaleCanaryGate {
    /// Initializes a new anytime-valid martingale canary gate with false-alarm rate $\alpha \in (0, 1)$.
    pub fn new(
        alpha: f64,
        sigma: f64,
        min_promote_samples: usize,
    ) -> Result<Self, MartingaleError> {
        if !alpha.is_finite() || alpha <= 0.0 || alpha >= 1.0 {
            return Err(MartingaleError::InvalidAlpha(alpha));
        }
        if !sigma.is_finite() || sigma <= 0.0 {
            return Err(MartingaleError::InvalidSigma(sigma));
        }
        Ok(Self {
            alpha,
            sigma,
            mixture_rho: 0.25,
            min_promote_samples: min_promote_samples.max(1),
            step_t: 0,
            log_e_value: 0.0,
            peak_e_value: 1.0,
            sum_x: 0.0,
            sum_sq_dev: 0.0,
            verdict: CanaryGateVerdict::ContinueMonitoring,
        })
    }

    /// Computes the Ville rejection threshold $1 / \alpha$.
    #[inline]
    pub fn ville_threshold(&self) -> f64 {
        1.0 / self.alpha
    }

    /// Computes the predictable plug-in Kelly-optimal betting fraction $\lambda_t \in [0, 0.75]$
    /// based strictly on $\mathcal{F}_{t-1}$ history $(S_{t-1}, V_{t-1})$ (Waudby-Smith & Ramdas).
    pub fn predictable_betting_fraction(&self) -> f64 {
        if self.step_t == 0 {
            return 0.0;
        }
        let t_prev = self.step_t as f64;
        let mu_hat = (self.sum_x / t_prev).max(0.0);
        let var_hat = (self.sum_sq_dev + self.sigma * self.sigma) / (t_prev + 1.0);
        let raw_lambda = mu_hat / (var_hat + mu_hat * mu_hat + 1e-9);
        raw_lambda.clamp(0.0, 0.75)
    }

    /// Theorem 28: Normal-Mixture Sub-Gaussian Time-Uniform Confidence Radius $r_t(\alpha)$:
    /// $$r_t(\alpha) = \sigma \sqrt{\frac{2(t \rho^2 + 1)}{t^2 \rho^2} \ln\left(\frac{\sqrt{t \rho^2 + 1}}{\alpha}\right)}$$
    pub fn confidence_radius_at(
        t: usize,
        sigma: f64,
        rho: f64,
        alpha: f64,
    ) -> Result<f64, MartingaleError> {
        if !alpha.is_finite() || alpha <= 0.0 || alpha >= 1.0 {
            return Err(MartingaleError::InvalidAlpha(alpha));
        }
        if !sigma.is_finite() || sigma <= 0.0 {
            return Err(MartingaleError::InvalidSigma(sigma));
        }
        if t == 0 {
            return Ok(f64::INFINITY);
        }
        let t_f = t as f64;
        let rho_sq = rho * rho;
        let v_t = t_f * rho_sq + 1.0;
        let log_term = (v_t.sqrt() / alpha).ln().max(0.0);
        let scale = (2.0 * v_t) / (t_f * t_f * rho_sq);
        Ok(sigma * (scale * log_term).sqrt())
    }

    /// Feeds a single bounded sequential observation $X_t \in [-1, 1]$ into the martingale gate.
    ///
    /// Positive $X_t > 0$ indicates canary regression relative to baseline; negative $X_t < 0$
    /// indicates canary improvement relative to baseline.
    pub fn observe(&mut self, x_t: f64) -> Result<MartingaleGateSnapshot, MartingaleError> {
        if !x_t.is_finite() {
            return Err(MartingaleError::InvalidObservation {
                value: x_t,
                reason: "observation must be finite",
            });
        }
        if !(-1.0..=1.0).contains(&x_t) {
            return Err(MartingaleError::InvalidObservation {
                value: x_t,
                reason: "observation must lie in [-1.0, 1.0]",
            });
        }

        // Compute predictable betting fraction lambda_t BEFORE seeing x_t (F_{t-1}-measurable)
        let lambda_t = self.predictable_betting_fraction();
        let prev_mean = if self.step_t > 0 {
            self.sum_x / (self.step_t as f64)
        } else {
            0.0
        };

        self.step_t += 1;
        self.sum_x += x_t;
        let new_mean = self.sum_x / (self.step_t as f64);
        self.sum_sq_dev += (x_t - prev_mean) * (x_t - new_mean);

        let factor = (1.0 + lambda_t * x_t).max(1e-12);
        self.log_e_value += factor.ln();
        let e_value = self.log_e_value.min(700.0).exp();
        if e_value > self.peak_e_value {
            self.peak_e_value = e_value;
        }

        let anytime_p_value = (1.0 / self.peak_e_value).min(1.0);
        let radius =
            Self::confidence_radius_at(self.step_t, self.sigma, self.mixture_rho, self.alpha)?;
        let lower_cb = new_mean - radius;
        let upper_cb = new_mean + radius;
        let ville_threshold = self.ville_threshold();

        if self.verdict == CanaryGateVerdict::ContinueMonitoring {
            if e_value >= ville_threshold || lower_cb > 0.0 {
                self.verdict = CanaryGateVerdict::RollbackRegression;
            } else if self.step_t >= self.min_promote_samples && upper_cb < 0.0 {
                self.verdict = CanaryGateVerdict::PromoteCanary;
            }
        }

        Ok(MartingaleGateSnapshot {
            step_t: self.step_t,
            e_value,
            peak_e_value: self.peak_e_value,
            anytime_p_value,
            ville_rejection_threshold: ville_threshold,
            empirical_mean: new_mean,
            confidence_radius: radius,
            lower_confidence_bound: lower_cb,
            upper_confidence_bound: upper_cb,
            betting_fraction_lambda: lambda_t,
            verdict: self.verdict,
        })
    }

    /// Returns the current gate snapshot without adding a new observation.
    pub fn snapshot(&self) -> Result<MartingaleGateSnapshot, MartingaleError> {
        let e_value = self.log_e_value.min(700.0).exp();
        let anytime_p_value = (1.0 / self.peak_e_value).min(1.0);
        let empirical_mean = if self.step_t > 0 {
            self.sum_x / (self.step_t as f64)
        } else {
            0.0
        };
        let radius =
            Self::confidence_radius_at(self.step_t, self.sigma, self.mixture_rho, self.alpha)?;
        Ok(MartingaleGateSnapshot {
            step_t: self.step_t,
            e_value,
            peak_e_value: self.peak_e_value,
            anytime_p_value,
            ville_rejection_threshold: self.ville_threshold(),
            empirical_mean,
            confidence_radius: radius,
            lower_confidence_bound: empirical_mean - radius,
            upper_confidence_bound: empirical_mean + radius,
            betting_fraction_lambda: self.predictable_betting_fraction(),
            verdict: self.verdict,
        })
    }
}
