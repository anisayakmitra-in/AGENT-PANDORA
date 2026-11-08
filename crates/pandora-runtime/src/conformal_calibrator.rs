#![forbid(unsafe_code)]

//! Phase 15: Conformal Prediction & Distribution-Free Uncertainty Calibration
//!
//! Implements Split Conformal Prediction (Vovk, Gammerman & Shafer 2005; Lei et al. 2018)
//! and Adaptive Conformal Inference (ACI, Gibbs & Candes 2021) for distribution-free
//! uncertainty quantification and governance escalation of autonomous agent decisions.
//!
//! ## Mathematical Guarantees
//!
//! 1. **Theorem 29 (Finite-Sample Exchangeable Coverage Guarantee):**
//!    For exchangeable calibration non-conformity scores $S_1, \dots, S_n$ and test point
//!    $S_{n+1}$, setting the empirical conformal quantile at level
//!    $$k = \lceil (n + 1)(1 - \alpha) \rceil$$
//!    (with $\widehat{q}_{1-\alpha} = S_{(k)}$ when $k \le n$ and $+\infty$ otherwise) yields
//!    $$\Pr\bigl(Y_{n+1} \in \widehat{C}_\alpha(X_{n+1})\bigr) = \Pr\bigl(S_{n+1} \le \widehat{q}_{1-\alpha}\bigr) \ge 1 - \alpha$$
//!    furthermore, if $S_1, \dots, S_{n+1}$ have no ties almost surely,
//!    $$\Pr\bigl(Y_{n+1} \in \widehat{C}_\alpha(X_{n+1})\bigr) \le 1 - \alpha + \frac{1}{n + 1}.$$
//!
//! 2. **Theorem 30 (Gibbs-Candes Adaptive Conformal Inference Long-Run Coverage):**
//!    Under arbitrary (possibly adversarial or non-exchangeable) distribution shift, the online update
//!    $$\alpha_{t+1} = \alpha_t + \gamma \bigl(\alpha - \mathrm{err}_t\bigr), \qquad \mathrm{err}_t = \mathbf{1}\bigl\{Y_t \notin \widehat{C}_{\alpha_t}(X_t)\bigr\}$$
//!    with bounded iterate $\alpha_t \in [-\gamma, 1 + \gamma]$ satisfies the deterministic finite-horizon bound:
//!    $$\left|\frac{1}{T}\sum_{t=1}^T \mathrm{err}_t - \alpha\right| \le \frac{\max(\alpha_1, 1 - \alpha_1) + \gamma}{T \gamma} \xrightarrow{T \to \infty} 0.$$

use serde::{Deserialize, Serialize};
use std::fmt;

/// Errors emitted by the conformal calibration engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConformalError {
    /// Miscoverage level $\alpha$ must lie in $(0, 1)$.
    InvalidAlpha { alpha: f64 },
    /// Calibration score slice is empty or contains non-finite values.
    InvalidCalibrationScores(String),
    /// Candidate action list is empty or contains invalid scores.
    InvalidCandidates(String),
    /// Step size $\gamma$ for Adaptive Conformal Inference must be strictly positive.
    InvalidStepSize { gamma: f64 },
}

impl fmt::Display for ConformalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAlpha { alpha } => {
                write!(f, "miscoverage rate alpha ({alpha}) must lie in (0, 1)")
            }
            Self::InvalidCalibrationScores(msg) => {
                write!(f, "invalid calibration scores: {msg}")
            }
            Self::InvalidCandidates(msg) => write!(f, "invalid candidate actions: {msg}"),
            Self::InvalidStepSize { gamma } => {
                write!(f, "ACI learning rate gamma ({gamma}) must be in (0, 1]")
            }
        }
    }
}

impl std::error::Error for ConformalError {}

/// Governance decision derived from the cardinality of the conformal prediction set $\widehat{C}_\alpha(X)$.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformalGovernanceVerdict {
    /// $|\widehat{C}_\alpha(X)| = 1$: Unique statistically certified action; autonomous execution permitted.
    CertifySingleton,
    /// $|\widehat{C}_\alpha(X)| > 1$: Multiple statistically plausible actions; escalate to Parliament / human review.
    EscalateAmbiguous,
    /// $|\widehat{C}_\alpha(X)| = 0$: All candidate actions exceed non-conformity threshold; abstain and halt.
    AbstainEmpty,
}

/// Candidate action or tool call evaluated by the conformal calibrator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateAction {
    pub action_id: String,
    /// Estimated probability or confidence $\hat{p}(y \mid X) \in [0, 1]$, or direct non-conformity score.
    pub nonconformity_score: f64,
}

/// Result of calibrating a split conformal quantile on $n$ exchangeable residuals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConformalCalibrationSummary {
    pub sample_size: usize,
    pub target_alpha: f64,
    pub order_statistic_rank: usize,
    pub quantile_threshold: f64,
    pub marginal_coverage_lower_bound: f64,
    pub marginal_coverage_upper_bound: f64,
}

/// Result of constructing a conformal prediction set $\widehat{C}_\alpha(X)$.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConformalPredictionReport {
    pub target_alpha: f64,
    pub quantile_threshold: f64,
    pub prediction_set: Vec<String>,
    pub set_size: usize,
    pub verdict: ConformalGovernanceVerdict,
    pub marginal_coverage_lower_bound: f64,
}

/// Split Conformal Calibrator implementing Theorem 29.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitConformalCalibrator {
    sorted_scores: Vec<f64>,
    target_alpha: f64,
    order_statistic_rank: usize,
    quantile_threshold: f64,
}

impl SplitConformalCalibrator {
    /// Calibrate on exchangeable non-conformity scores $S_1, \dots, S_n$ at miscoverage level $\alpha \in (0, 1)$.
    pub fn calibrate(scores: &[f64], target_alpha: f64) -> Result<Self, ConformalError> {
        if !(0.0..1.0).contains(&target_alpha) || !target_alpha.is_finite() {
            return Err(ConformalError::InvalidAlpha {
                alpha: target_alpha,
            });
        }
        if scores.is_empty() {
            return Err(ConformalError::InvalidCalibrationScores(
                "calibration set must contain at least 1 score".to_owned(),
            ));
        }
        if scores.iter().any(|s| !s.is_finite() || *s < 0.0) {
            return Err(ConformalError::InvalidCalibrationScores(
                "all non-conformity scores must be finite and non-negative".to_owned(),
            ));
        }

        let mut sorted_scores = scores.to_vec();
        sorted_scores.sort_by(|a, b| a.total_cmp(b));

        let n = sorted_scores.len();
        // Theorem 29: rank k = ceil((n + 1) * (1 - alpha))
        let raw_rank = ((n as f64 + 1.0) * (1.0 - target_alpha)).ceil();
        let rank = (raw_rank as usize).max(1);
        let quantile_threshold = if rank > n {
            f64::INFINITY
        } else {
            sorted_scores[rank - 1]
        };

        Ok(Self {
            sorted_scores,
            target_alpha,
            order_statistic_rank: rank,
            quantile_threshold,
        })
    }

    /// Return the calibrated threshold $\widehat{q}_{1-\alpha}$ at a custom effective $\alpha_t$ (used by ACI).
    pub fn quantile_for_alpha(&self, effective_alpha: f64) -> f64 {
        let n = self.sorted_scores.len();
        if effective_alpha <= 0.0 {
            return f64::INFINITY;
        }
        if effective_alpha >= 1.0 {
            return -f64::INFINITY;
        }
        let raw_rank = ((n as f64 + 1.0) * (1.0 - effective_alpha)).ceil();
        if raw_rank <= 0.0 {
            return -f64::INFINITY;
        }
        let rank = raw_rank as usize;
        if rank == 0 {
            -f64::INFINITY
        } else if rank > n {
            f64::INFINITY
        } else {
            self.sorted_scores[rank - 1]
        }
    }

    /// Return the calibration summary including Theorem 29 finite-sample bounds.
    pub fn summary(&self) -> ConformalCalibrationSummary {
        let n = self.sorted_scores.len();
        let lower = 1.0 - self.target_alpha;
        let upper = (1.0 - self.target_alpha + 1.0 / (n as f64 + 1.0)).min(1.0);
        ConformalCalibrationSummary {
            sample_size: n,
            target_alpha: self.target_alpha,
            order_statistic_rank: self.order_statistic_rank,
            quantile_threshold: self.quantile_threshold,
            marginal_coverage_lower_bound: lower,
            marginal_coverage_upper_bound: upper,
        }
    }

    /// Construct the conformal prediction set $\widehat{C}_\alpha(X) = \{y : s(X, y) \le \widehat{q}_{1-\alpha}\}$.
    pub fn predict_set(
        &self,
        candidates: &[CandidateAction],
    ) -> Result<ConformalPredictionReport, ConformalError> {
        self.predict_set_with_threshold(candidates, self.quantile_threshold, self.target_alpha)
    }

    fn predict_set_with_threshold(
        &self,
        candidates: &[CandidateAction],
        threshold: f64,
        alpha: f64,
    ) -> Result<ConformalPredictionReport, ConformalError> {
        if candidates.is_empty() {
            return Err(ConformalError::InvalidCandidates(
                "candidate action list cannot be empty".to_owned(),
            ));
        }
        if candidates
            .iter()
            .any(|c| !c.nonconformity_score.is_finite() || c.nonconformity_score < 0.0)
        {
            return Err(ConformalError::InvalidCandidates(
                "all candidate non-conformity scores must be finite and non-negative".to_owned(),
            ));
        }

        let prediction_set: Vec<String> = candidates
            .iter()
            .filter(|c| c.nonconformity_score <= threshold)
            .map(|c| c.action_id.clone())
            .collect();
        let set_size = prediction_set.len();
        let verdict = match set_size {
            0 => ConformalGovernanceVerdict::AbstainEmpty,
            1 => ConformalGovernanceVerdict::CertifySingleton,
            _ => ConformalGovernanceVerdict::EscalateAmbiguous,
        };

        Ok(ConformalPredictionReport {
            target_alpha: alpha,
            quantile_threshold: threshold,
            prediction_set,
            set_size,
            verdict,
            marginal_coverage_lower_bound: (1.0 - alpha).clamp(0.0, 1.0),
        })
    }
}

/// Telemetry and theoretical error bound for Adaptive Conformal Inference (Theorem 30).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveConformalStatus {
    pub target_alpha: f64,
    pub current_alpha_t: f64,
    pub gamma: f64,
    pub steps_observed: usize,
    pub miscoverage_events: usize,
    pub empirical_miscoverage_rate: f64,
    pub empirical_coverage_rate: f64,
    pub theoretical_error_bound: f64,
    pub within_theoretical_bound: bool,
}

/// Gibbs & Candes Adaptive Conformal Inference (ACI) tracker implementing Theorem 30.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveConformalTracker {
    target_alpha: f64,
    initial_alpha: f64,
    current_alpha: f64,
    gamma: f64,
    steps_observed: usize,
    miscoverage_events: usize,
}

impl AdaptiveConformalTracker {
    /// Initialize an ACI tracker with target miscoverage $\alpha \in (0, 1)$ and step size $\gamma \in (0, 1]$.
    pub fn new(target_alpha: f64, gamma: f64) -> Result<Self, ConformalError> {
        if !(0.0..1.0).contains(&target_alpha) || !target_alpha.is_finite() {
            return Err(ConformalError::InvalidAlpha {
                alpha: target_alpha,
            });
        }
        if !gamma.is_finite() || gamma <= 0.0 || gamma > 1.0 {
            return Err(ConformalError::InvalidStepSize { gamma });
        }
        Ok(Self {
            target_alpha,
            initial_alpha: target_alpha,
            current_alpha: target_alpha,
            gamma,
            steps_observed: 0,
            miscoverage_events: 0,
        })
    }

    /// Observe whether the true outcome was miscovered ($\mathrm{err}_t = \mathbf{1}\{Y_t \notin \widehat{C}_{\alpha_t}(X_t)\}$)
    /// and update $\alpha_{t+1} = \alpha_t + \gamma(\alpha - \mathrm{err}_t)$ clamped to $[-\gamma, 1 + \gamma]$.
    pub fn observe_step(&mut self, miscovered: bool) -> f64 {
        self.steps_observed += 1;
        if miscovered {
            self.miscoverage_events += 1;
        }
        let err_t = if miscovered { 1.0 } else { 0.0 };
        let next_alpha = self.current_alpha + self.gamma * (self.target_alpha - err_t);
        self.current_alpha = next_alpha.clamp(-self.gamma, 1.0 + self.gamma);
        self.current_alpha
    }

    /// Compute the current ACI telemetry and verify Theorem 30's finite-horizon bound.
    pub fn status(&self) -> AdaptiveConformalStatus {
        if self.steps_observed == 0 {
            return AdaptiveConformalStatus {
                target_alpha: self.target_alpha,
                current_alpha_t: self.current_alpha,
                gamma: self.gamma,
                steps_observed: 0,
                miscoverage_events: 0,
                empirical_miscoverage_rate: 0.0,
                empirical_coverage_rate: 1.0,
                theoretical_error_bound: f64::INFINITY,
                within_theoretical_bound: true,
            };
        }

        let t = self.steps_observed as f64;
        let emp_miscoverage = self.miscoverage_events as f64 / t;
        let emp_coverage = 1.0 - emp_miscoverage;
        // Theorem 30: |(1/T) sum err_t - alpha| <= (max(alpha_1, 1 - alpha_1) + gamma) / (T * gamma)
        let numerator = self.initial_alpha.max(1.0 - self.initial_alpha) + self.gamma;
        let theoretical_bound = numerator / (t * self.gamma);
        let actual_deviation = (emp_miscoverage - self.target_alpha).abs();

        AdaptiveConformalStatus {
            target_alpha: self.target_alpha,
            current_alpha_t: self.current_alpha,
            gamma: self.gamma,
            steps_observed: self.steps_observed,
            miscoverage_events: self.miscoverage_events,
            empirical_miscoverage_rate: emp_miscoverage,
            empirical_coverage_rate: emp_coverage,
            theoretical_error_bound: theoretical_bound,
            within_theoretical_bound: actual_deviation <= theoretical_bound + 1e-12,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_conformal_quantile_satisfies_theorem_29_bounds() {
        let scores: Vec<f64> = (1..=19).map(|i| i as f64 * 0.05).collect();
        let cal = SplitConformalCalibrator::calibrate(&scores, 0.10).unwrap();
        let summary = cal.summary();
        // n = 19, ceil((19 + 1) * 0.90) = 18 -> 18th score = 0.90
        assert_eq!(summary.order_statistic_rank, 18);
        assert!((summary.quantile_threshold - 0.90).abs() < 1e-12);
        assert!((summary.marginal_coverage_lower_bound - 0.90).abs() < 1e-12);
        assert!((summary.marginal_coverage_upper_bound - 0.95).abs() < 1e-12);
    }

    #[test]
    fn prediction_set_governance_verdicts() {
        let scores = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
        let cal = SplitConformalCalibrator::calibrate(&scores, 0.20).unwrap();
        // n = 9, ceil(10 * 0.8) = 8 -> threshold = 0.8

        let singleton = vec![
            CandidateAction {
                action_id: "tool_read".to_owned(),
                nonconformity_score: 0.25,
            },
            CandidateAction {
                action_id: "tool_delete".to_owned(),
                nonconformity_score: 0.92,
            },
        ];
        let rep1 = cal.predict_set(&singleton).unwrap();
        assert_eq!(rep1.verdict, ConformalGovernanceVerdict::CertifySingleton);
        assert_eq!(rep1.prediction_set, vec!["tool_read".to_owned()]);

        let ambiguous = vec![
            CandidateAction {
                action_id: "tool_a".to_owned(),
                nonconformity_score: 0.30,
            },
            CandidateAction {
                action_id: "tool_b".to_owned(),
                nonconformity_score: 0.55,
            },
        ];
        let rep2 = cal.predict_set(&ambiguous).unwrap();
        assert_eq!(rep2.verdict, ConformalGovernanceVerdict::EscalateAmbiguous);
        assert_eq!(rep2.set_size, 2);

        let empty = vec![CandidateAction {
            action_id: "tool_risky".to_owned(),
            nonconformity_score: 0.95,
        }];
        let rep3 = cal.predict_set(&empty).unwrap();
        assert_eq!(rep3.verdict, ConformalGovernanceVerdict::AbstainEmpty);
    }

    #[test]
    fn aci_satisfies_theorem_30_under_adversarial_shift() {
        let scores: Vec<f64> = (1..=100).map(|i| i as f64 / 100.0).collect();
        let cal = SplitConformalCalibrator::calibrate(&scores, 0.10).unwrap();
        let mut aci = AdaptiveConformalTracker::new(0.10, 0.05).unwrap();

        // Simulate an abrupt adversarial shift where scores spike above 0.90
        for t in 0..500 {
            let threshold = cal.quantile_for_alpha(aci.status().current_alpha_t);
            let test_score = if t % 3 == 0 { 0.98 } else { 0.50 };
            let miscovered = test_score > threshold;
            aci.observe_step(miscovered);
        }

        let status = aci.status();
        assert!(status.within_theoretical_bound);
        assert!((status.empirical_miscoverage_rate - 0.10).abs() <= status.theoretical_error_bound);
    }
}
