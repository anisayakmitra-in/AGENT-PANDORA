use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DistillationContractError {
    NonFiniteMetric(&'static str),
    InvalidMetricValue(&'static str),
    Io(String),
    Serialization(String),
}

impl fmt::Display for DistillationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteMetric(field) => {
                write!(
                    formatter,
                    "distillation telemetry metric '{field}' must be finite"
                )
            }
            Self::InvalidMetricValue(reason) => {
                write!(formatter, "invalid distillation telemetry metric: {reason}")
            }
            Self::Io(message) => write!(formatter, "distillation telemetry I/O error: {message}"),
            Self::Serialization(message) => {
                write!(
                    formatter,
                    "distillation telemetry serialization error: {message}"
                )
            }
        }
    }
}

impl std::error::Error for DistillationContractError {}

/// Phase 13 Sinkhorn-Knopp / 2-Wasserstein drift telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WassersteinTelemetrySummary {
    pub sinkhorn_divergence: f64,
    pub wasserstein_2_distance: f64,
    pub birkhoff_contraction_factor: f64,
    pub theoretical_entropic_bias_bound: f64,
    pub drift_threshold: f64,
    pub drift_detected: bool,
    pub iterations: usize,
    pub final_dual_residual: f64,
    pub marginal_conservation_error: f64,
}

impl Default for WassersteinTelemetrySummary {
    fn default() -> Self {
        Self {
            sinkhorn_divergence: 0.0,
            wasserstein_2_distance: 0.0,
            birkhoff_contraction_factor: 0.0,
            theoretical_entropic_bias_bound: 0.0,
            drift_threshold: 0.5,
            drift_detected: false,
            iterations: 0,
            final_dual_residual: 0.0,
            marginal_conservation_error: 0.0,
        }
    }
}

impl WassersteinTelemetrySummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sinkhorn_divergence: f64,
        wasserstein_2_distance: f64,
        birkhoff_contraction_factor: f64,
        theoretical_entropic_bias_bound: f64,
        drift_threshold: f64,
        drift_detected: bool,
        iterations: usize,
        final_dual_residual: f64,
        marginal_conservation_error: f64,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            sinkhorn_divergence,
            wasserstein_2_distance,
            birkhoff_contraction_factor,
            theoretical_entropic_bias_bound,
            drift_threshold,
            drift_detected,
            iterations,
            final_dual_residual,
            marginal_conservation_error,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        self.sinkhorn_divergence.is_finite()
            && self.wasserstein_2_distance.is_finite()
            && self.birkhoff_contraction_factor.is_finite()
            && self.theoretical_entropic_bias_bound.is_finite()
            && self.drift_threshold.is_finite()
            && self.final_dual_residual.is_finite()
            && self.marginal_conservation_error.is_finite()
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if !self.sinkhorn_divergence.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "sinkhorn_divergence",
            ));
        }
        if self.sinkhorn_divergence < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "sinkhorn_divergence must be >= 0",
            ));
        }
        if !self.wasserstein_2_distance.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "wasserstein_2_distance",
            ));
        }
        if self.wasserstein_2_distance < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "wasserstein_2_distance must be >= 0",
            ));
        }
        if !self.birkhoff_contraction_factor.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "birkhoff_contraction_factor",
            ));
        }
        if !(0.0..1.0).contains(&self.birkhoff_contraction_factor) {
            return Err(DistillationContractError::InvalidMetricValue(
                "birkhoff_contraction_factor must lie in [0, 1)",
            ));
        }
        if !self.theoretical_entropic_bias_bound.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "theoretical_entropic_bias_bound",
            ));
        }
        if self.theoretical_entropic_bias_bound < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "theoretical_entropic_bias_bound must be >= 0",
            ));
        }
        if !self.drift_threshold.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "drift_threshold",
            ));
        }
        if self.drift_threshold < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "drift_threshold must be >= 0",
            ));
        }
        if !self.final_dual_residual.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "final_dual_residual",
            ));
        }
        if self.final_dual_residual < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "final_dual_residual must be >= 0",
            ));
        }
        if !self.marginal_conservation_error.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "marginal_conservation_error",
            ));
        }
        if self.marginal_conservation_error < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "marginal_conservation_error must be >= 0",
            ));
        }
        Ok(())
    }
}

/// Phase 14 anytime-valid martingale canary gate telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MartingaleTelemetrySummary {
    pub step_t: usize,
    pub e_value: f64,
    pub peak_e_value: f64,
    pub anytime_p_value: f64,
    pub ville_rejection_threshold: f64,
    pub empirical_mean: f64,
    pub confidence_radius: f64,
    pub lower_confidence_bound: f64,
    pub upper_confidence_bound: f64,
    pub betting_fraction_lambda: f64,
    pub verdict: String,
}

impl Default for MartingaleTelemetrySummary {
    fn default() -> Self {
        Self {
            step_t: 0,
            e_value: 1.0,
            peak_e_value: 1.0,
            anytime_p_value: 1.0,
            ville_rejection_threshold: 20.0,
            empirical_mean: 0.0,
            confidence_radius: 1.0,
            lower_confidence_bound: -1.0,
            upper_confidence_bound: 1.0,
            betting_fraction_lambda: 0.0,
            verdict: "continue_monitoring".to_owned(),
        }
    }
}

impl MartingaleTelemetrySummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        step_t: usize,
        e_value: f64,
        peak_e_value: f64,
        anytime_p_value: f64,
        ville_rejection_threshold: f64,
        empirical_mean: f64,
        confidence_radius: f64,
        lower_confidence_bound: f64,
        upper_confidence_bound: f64,
        betting_fraction_lambda: f64,
        verdict: impl Into<String>,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            step_t,
            e_value,
            peak_e_value,
            anytime_p_value,
            ville_rejection_threshold,
            empirical_mean,
            confidence_radius,
            lower_confidence_bound,
            upper_confidence_bound,
            betting_fraction_lambda,
            verdict: verdict.into(),
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        self.e_value.is_finite()
            && self.peak_e_value.is_finite()
            && self.anytime_p_value.is_finite()
            && self.ville_rejection_threshold.is_finite()
            && self.empirical_mean.is_finite()
            && self.confidence_radius.is_finite()
            && self.lower_confidence_bound.is_finite()
            && self.upper_confidence_bound.is_finite()
            && self.betting_fraction_lambda.is_finite()
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if !self.e_value.is_finite() || self.e_value < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "e_value must be finite and >= 0",
            ));
        }
        if !self.peak_e_value.is_finite() || self.peak_e_value < 1.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "peak_e_value must be finite and >= 1",
            ));
        }
        if !self.anytime_p_value.is_finite() || !(0.0..=1.0).contains(&self.anytime_p_value) {
            return Err(DistillationContractError::InvalidMetricValue(
                "anytime_p_value must be finite and in [0, 1]",
            ));
        }
        if !self.ville_rejection_threshold.is_finite() || self.ville_rejection_threshold <= 1.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "ville_rejection_threshold must be finite and > 1",
            ));
        }
        if !self.empirical_mean.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric("empirical_mean"));
        }
        if !self.confidence_radius.is_finite() || self.confidence_radius < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "confidence_radius must be finite and >= 0",
            ));
        }
        if !self.lower_confidence_bound.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "lower_confidence_bound",
            ));
        }
        if !self.upper_confidence_bound.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "upper_confidence_bound",
            ));
        }
        if self.lower_confidence_bound > self.upper_confidence_bound + 1e-12 {
            return Err(DistillationContractError::InvalidMetricValue(
                "lower_confidence_bound must be <= upper_confidence_bound",
            ));
        }
        if !self.betting_fraction_lambda.is_finite()
            || !(0.0..1.0).contains(&self.betting_fraction_lambda)
        {
            return Err(DistillationContractError::InvalidMetricValue(
                "betting_fraction_lambda must be finite and in [0, 1)",
            ));
        }
        if self.verdict.trim().is_empty() {
            return Err(DistillationContractError::InvalidMetricValue(
                "verdict must not be empty",
            ));
        }
        Ok(())
    }
}

/// Phase 15 Split Conformal Prediction & Adaptive Conformal Inference telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConformalTelemetrySummary {
    pub calibration_size: usize,
    pub target_alpha: f64,
    pub order_statistic_rank: usize,
    pub quantile_threshold: f64,
    pub marginal_coverage_lower_bound: f64,
    pub marginal_coverage_upper_bound: f64,
    pub current_alpha_t: f64,
    pub empirical_coverage_rate: f64,
    pub empirical_miscoverage_rate: f64,
    pub within_theoretical_bound: bool,
    pub theoretical_error_bound: f64,
    pub last_set_size: usize,
    pub last_verdict: String,
}

impl Default for ConformalTelemetrySummary {
    fn default() -> Self {
        Self {
            calibration_size: 100,
            target_alpha: 0.10,
            order_statistic_rank: 91,
            quantile_threshold: 0.90,
            marginal_coverage_lower_bound: 0.90,
            marginal_coverage_upper_bound: 0.91,
            current_alpha_t: 0.10,
            empirical_coverage_rate: 0.90,
            empirical_miscoverage_rate: 0.10,
            within_theoretical_bound: true,
            theoretical_error_bound: 0.10,
            last_set_size: 1,
            last_verdict: "certify_singleton".to_owned(),
        }
    }
}

impl ConformalTelemetrySummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        calibration_size: usize,
        target_alpha: f64,
        order_statistic_rank: usize,
        quantile_threshold: f64,
        marginal_coverage_lower_bound: f64,
        marginal_coverage_upper_bound: f64,
        current_alpha_t: f64,
        empirical_coverage_rate: f64,
        empirical_miscoverage_rate: f64,
        within_theoretical_bound: bool,
        theoretical_error_bound: f64,
        last_set_size: usize,
        last_verdict: impl Into<String>,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            calibration_size,
            target_alpha,
            order_statistic_rank,
            quantile_threshold,
            marginal_coverage_lower_bound,
            marginal_coverage_upper_bound,
            current_alpha_t,
            empirical_coverage_rate,
            empirical_miscoverage_rate,
            within_theoretical_bound,
            theoretical_error_bound,
            last_set_size,
            last_verdict: last_verdict.into(),
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        self.target_alpha.is_finite()
            && self.quantile_threshold.is_finite()
            && self.marginal_coverage_lower_bound.is_finite()
            && self.marginal_coverage_upper_bound.is_finite()
            && self.current_alpha_t.is_finite()
            && self.empirical_coverage_rate.is_finite()
            && self.empirical_miscoverage_rate.is_finite()
            && self.theoretical_error_bound.is_finite()
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if !self.target_alpha.is_finite() || !(0.0..1.0).contains(&self.target_alpha) {
            return Err(DistillationContractError::InvalidMetricValue(
                "target_alpha must be finite and in (0, 1)",
            ));
        }
        if !self.quantile_threshold.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "quantile_threshold",
            ));
        }
        if !self.marginal_coverage_lower_bound.is_finite()
            || self.marginal_coverage_lower_bound < 0.0
        {
            return Err(DistillationContractError::InvalidMetricValue(
                "marginal_coverage_lower_bound must be finite and >= 0",
            ));
        }
        if !self.marginal_coverage_upper_bound.is_finite()
            || self.marginal_coverage_upper_bound > 1.0 + 1e-12
        {
            return Err(DistillationContractError::InvalidMetricValue(
                "marginal_coverage_upper_bound must be finite and <= 1",
            ));
        }
        if self.marginal_coverage_lower_bound > self.marginal_coverage_upper_bound + 1e-12 {
            return Err(DistillationContractError::InvalidMetricValue(
                "marginal_coverage_lower_bound must be <= marginal_coverage_upper_bound",
            ));
        }
        if !self.current_alpha_t.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "current_alpha_t",
            ));
        }
        if !self.empirical_coverage_rate.is_finite()
            || !(0.0..=1.0).contains(&self.empirical_coverage_rate)
        {
            return Err(DistillationContractError::InvalidMetricValue(
                "empirical_coverage_rate must be finite and in [0, 1]",
            ));
        }
        if !self.empirical_miscoverage_rate.is_finite()
            || !(0.0..=1.0).contains(&self.empirical_miscoverage_rate)
        {
            return Err(DistillationContractError::InvalidMetricValue(
                "empirical_miscoverage_rate must be finite and in [0, 1]",
            ));
        }
        if !self.theoretical_error_bound.is_finite() || self.theoretical_error_bound < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "theoretical_error_bound must be finite and >= 0",
            ));
        }
        if self.last_verdict.trim().is_empty() {
            return Err(DistillationContractError::InvalidMetricValue(
                "last_verdict must not be empty",
            ));
        }
        Ok(())
    }
}

/// Phase 16 Spectral Graph Partitioning & Algebraic Connectivity telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpectralTopologySummary {
    pub num_vertices: usize,
    pub num_edges: usize,
    pub is_connected: bool,
    pub fiedler_value: f64,
    pub normalized_fiedler_value: f64,
    pub cheeger_lower_bound: f64,
    pub cheeger_upper_bound: f64,
    pub sweep_cut_conductance: f64,
    pub sweep_cut_partition_size: usize,
    pub cheeger_inequality_verified: bool,
    pub verdict: String,
}

impl Default for SpectralTopologySummary {
    fn default() -> Self {
        Self {
            num_vertices: 6,
            num_edges: 7,
            is_connected: true,
            fiedler_value: 0.438,
            normalized_fiedler_value: 0.205,
            cheeger_lower_bound: 0.102,
            cheeger_upper_bound: 0.640,
            sweep_cut_conductance: 0.143,
            sweep_cut_partition_size: 3,
            cheeger_inequality_verified: true,
            verdict: "connected_resilient".to_owned(),
        }
    }
}

impl SpectralTopologySummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        num_vertices: usize,
        num_edges: usize,
        is_connected: bool,
        fiedler_value: f64,
        normalized_fiedler_value: f64,
        cheeger_lower_bound: f64,
        cheeger_upper_bound: f64,
        sweep_cut_conductance: f64,
        sweep_cut_partition_size: usize,
        cheeger_inequality_verified: bool,
        verdict: impl Into<String>,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            num_vertices,
            num_edges,
            is_connected,
            fiedler_value,
            normalized_fiedler_value,
            cheeger_lower_bound,
            cheeger_upper_bound,
            sweep_cut_conductance,
            sweep_cut_partition_size,
            cheeger_inequality_verified,
            verdict: verdict.into(),
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        self.fiedler_value.is_finite()
            && self.normalized_fiedler_value.is_finite()
            && self.cheeger_lower_bound.is_finite()
            && self.cheeger_upper_bound.is_finite()
            && self.sweep_cut_conductance.is_finite()
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if !self.fiedler_value.is_finite() || self.fiedler_value < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "fiedler_value must be finite and >= 0",
            ));
        }
        if !self.normalized_fiedler_value.is_finite() || self.normalized_fiedler_value < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "normalized_fiedler_value must be finite and >= 0",
            ));
        }
        if !self.cheeger_lower_bound.is_finite() || self.cheeger_lower_bound < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "cheeger_lower_bound must be finite and >= 0",
            ));
        }
        if !self.cheeger_upper_bound.is_finite() || self.cheeger_upper_bound < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "cheeger_upper_bound must be finite and >= 0",
            ));
        }
        if !self.sweep_cut_conductance.is_finite() || self.sweep_cut_conductance < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "sweep_cut_conductance must be finite and >= 0",
            ));
        }
        if self.verdict.trim().is_empty() {
            return Err(DistillationContractError::InvalidMetricValue(
                "verdict must not be empty",
            ));
        }
        Ok(())
    }
}

/// Phase 17 Exp3 Adversarial Multi-Armed Bandit router telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdversarialBanditSummary {
    pub num_arms: usize,
    pub total_rounds: usize,
    pub gamma: f64,
    pub eta: f64,
    pub total_reward: f64,
    pub theoretical_regret_bound: f64,
    pub arm_weights: Vec<f64>,
    pub arm_probabilities: Vec<f64>,
    pub arm_pull_counts: Vec<usize>,
    pub arm_rewards: Vec<f64>,
}

impl Default for AdversarialBanditSummary {
    fn default() -> Self {
        Self {
            num_arms: 3,
            total_rounds: 200,
            gamma: 0.098,
            eta: 0.033,
            total_reward: 135.0,
            theoretical_regret_bound: 67.31,
            arm_weights: vec![0.933, 0.033, 0.034],
            arm_probabilities: vec![0.875, 0.062, 0.063],
            arm_pull_counts: vec![130, 36, 34],
            arm_rewards: vec![110.5, 12.6, 11.9],
        }
    }
}

impl AdversarialBanditSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        num_arms: usize,
        total_rounds: usize,
        gamma: f64,
        eta: f64,
        total_reward: f64,
        theoretical_regret_bound: f64,
        arm_weights: Vec<f64>,
        arm_probabilities: Vec<f64>,
        arm_pull_counts: Vec<usize>,
        arm_rewards: Vec<f64>,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            num_arms,
            total_rounds,
            gamma,
            eta,
            total_reward,
            theoretical_regret_bound,
            arm_weights,
            arm_probabilities,
            arm_pull_counts,
            arm_rewards,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        self.gamma.is_finite()
            && self.eta.is_finite()
            && self.total_reward.is_finite()
            && self.theoretical_regret_bound.is_finite()
            && self.arm_weights.iter().all(|w| w.is_finite())
            && self.arm_probabilities.iter().all(|p| p.is_finite())
            && self.arm_rewards.iter().all(|r| r.is_finite())
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if !self.gamma.is_finite() || !(0.0..=1.0).contains(&self.gamma) {
            return Err(DistillationContractError::InvalidMetricValue(
                "gamma must be finite and in (0, 1]",
            ));
        }
        if !self.eta.is_finite() || self.eta <= 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "eta must be finite and > 0",
            ));
        }
        if !self.total_reward.is_finite() || self.total_reward < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "total_reward must be finite and >= 0",
            ));
        }
        if !self.theoretical_regret_bound.is_finite() || self.theoretical_regret_bound < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "theoretical_regret_bound must be finite and >= 0",
            ));
        }
        if self.arm_weights.iter().any(|w| !w.is_finite()) {
            return Err(DistillationContractError::NonFiniteMetric("arm_weights"));
        }
        if self
            .arm_probabilities
            .iter()
            .any(|p| !p.is_finite() || *p < 0.0)
        {
            return Err(DistillationContractError::InvalidMetricValue(
                "all arm_probabilities must be finite and >= 0",
            ));
        }
        if self.arm_rewards.iter().any(|r| !r.is_finite()) {
            return Err(DistillationContractError::NonFiniteMetric("arm_rewards"));
        }
        Ok(())
    }
}

/// Phase 18 Mean-Field Game (MFG) resource allocation & congestion control telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MfgTelemetrySummary {
    pub num_resources: usize,
    pub temperature_tau: f64,
    pub nash_social_cost: f64,
    pub optimal_social_cost: f64,
    pub price_of_anarchy: f64,
    pub theoretical_poa_bound: f64,
    pub iterations: usize,
    pub residual_norm: f64,
    pub is_strict_contraction: bool,
    pub poa_bound_satisfied: bool,
}

impl Default for MfgTelemetrySummary {
    fn default() -> Self {
        Self {
            num_resources: 3,
            temperature_tau: 1.0,
            nash_social_cost: 2.14,
            optimal_social_cost: 1.81,
            price_of_anarchy: 1.182,
            theoretical_poa_bound: 1.3333333333333333,
            iterations: 14,
            residual_norm: 1.2e-6,
            is_strict_contraction: true,
            poa_bound_satisfied: true,
        }
    }
}

impl MfgTelemetrySummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        num_resources: usize,
        temperature_tau: f64,
        nash_social_cost: f64,
        optimal_social_cost: f64,
        price_of_anarchy: f64,
        theoretical_poa_bound: f64,
        iterations: usize,
        residual_norm: f64,
        is_strict_contraction: bool,
        poa_bound_satisfied: bool,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            num_resources,
            temperature_tau,
            nash_social_cost,
            optimal_social_cost,
            price_of_anarchy,
            theoretical_poa_bound,
            iterations,
            residual_norm,
            is_strict_contraction,
            poa_bound_satisfied,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        self.temperature_tau.is_finite()
            && self.nash_social_cost.is_finite()
            && self.optimal_social_cost.is_finite()
            && self.price_of_anarchy.is_finite()
            && self.theoretical_poa_bound.is_finite()
            && self.residual_norm.is_finite()
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if !self.temperature_tau.is_finite() || self.temperature_tau <= 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "temperature_tau must be finite and > 0",
            ));
        }
        if !self.nash_social_cost.is_finite() || self.nash_social_cost < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "nash_social_cost must be finite and >= 0",
            ));
        }
        if !self.optimal_social_cost.is_finite() || self.optimal_social_cost < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "optimal_social_cost must be finite and >= 0",
            ));
        }
        if !self.price_of_anarchy.is_finite() || self.price_of_anarchy < 1.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "price_of_anarchy must be finite and >= 1",
            ));
        }
        if !self.theoretical_poa_bound.is_finite() || self.theoretical_poa_bound < 1.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "theoretical_poa_bound must be finite and >= 1",
            ));
        }
        if !self.residual_norm.is_finite() || self.residual_norm < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "residual_norm must be finite and >= 0",
            ));
        }
        Ok(())
    }
}

/// Phase 19 PAC-Bayes Policy Generalization & Variational KL Monitor telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PacBayesTelemetrySummary {
    pub sample_size: usize,
    pub delta: f64,
    pub empirical_risk: f64,
    pub kl_divergence: f64,
    pub mcallester_bound: f64,
    pub catoni_temperature_beta: f64,
    pub target_risk: f64,
    pub promotion_verdict: String,
    pub promotion_granted: bool,
}

impl Default for PacBayesTelemetrySummary {
    fn default() -> Self {
        Self {
            sample_size: 500,
            delta: 0.05,
            empirical_risk: 0.042,
            kl_divergence: 0.128,
            mcallester_bound: 0.098,
            catoni_temperature_beta: 12.4,
            target_risk: 0.10,
            promotion_verdict: "PromoteToCandidate".to_string(),
            promotion_granted: true,
        }
    }
}

impl PacBayesTelemetrySummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sample_size: usize,
        delta: f64,
        empirical_risk: f64,
        kl_divergence: f64,
        mcallester_bound: f64,
        catoni_temperature_beta: f64,
        target_risk: f64,
        promotion_verdict: impl Into<String>,
        promotion_granted: bool,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            sample_size,
            delta,
            empirical_risk,
            kl_divergence,
            mcallester_bound,
            catoni_temperature_beta,
            target_risk,
            promotion_verdict: promotion_verdict.into(),
            promotion_granted,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        self.delta.is_finite()
            && self.empirical_risk.is_finite()
            && self.kl_divergence.is_finite()
            && self.mcallester_bound.is_finite()
            && self.catoni_temperature_beta.is_finite()
            && self.target_risk.is_finite()
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if !self.delta.is_finite() || !(0.0..=1.0).contains(&self.delta) {
            return Err(DistillationContractError::InvalidMetricValue(
                "delta must be finite and in (0, 1]",
            ));
        }
        if !self.empirical_risk.is_finite() || self.empirical_risk < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "empirical_risk must be finite and >= 0",
            ));
        }
        if !self.kl_divergence.is_finite() || self.kl_divergence < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "kl_divergence must be finite and >= 0",
            ));
        }
        if !self.mcallester_bound.is_finite() || self.mcallester_bound < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "mcallester_bound must be finite and >= 0",
            ));
        }
        if !self.catoni_temperature_beta.is_finite() || self.catoni_temperature_beta <= 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "catoni_temperature_beta must be finite and > 0",
            ));
        }
        if !self.target_risk.is_finite() || self.target_risk < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "target_risk must be finite and >= 0",
            ));
        }
        Ok(())
    }
}

/// Phase 20 Tropical Max-Plus DAG Calculus & Karp Cycle Mean telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TropicalDagTelemetrySummary {
    pub workflow_nodes: usize,
    pub topology: String,
    pub critical_path_makespan: f64,
    pub karp_cycle_mean: f64,
    pub periodic_throughput: f64,
    pub sla_deadlock_threshold: f64,
    pub has_positive_cycle: bool,
    pub sla_compliant: bool,
}

impl Default for TropicalDagTelemetrySummary {
    fn default() -> Self {
        Self {
            workflow_nodes: 5,
            topology: "feedforward_dag".to_string(),
            critical_path_makespan: 14.2,
            karp_cycle_mean: 0.0,
            periodic_throughput: 0.0704,
            sla_deadlock_threshold: 100.0,
            has_positive_cycle: false,
            sla_compliant: true,
        }
    }
}

impl TropicalDagTelemetrySummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workflow_nodes: usize,
        topology: impl Into<String>,
        critical_path_makespan: f64,
        karp_cycle_mean: f64,
        periodic_throughput: f64,
        sla_deadlock_threshold: f64,
        has_positive_cycle: bool,
        sla_compliant: bool,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            workflow_nodes,
            topology: topology.into(),
            critical_path_makespan,
            karp_cycle_mean,
            periodic_throughput,
            sla_deadlock_threshold,
            has_positive_cycle,
            sla_compliant,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        self.critical_path_makespan.is_finite()
            && self.karp_cycle_mean.is_finite()
            && self.periodic_throughput.is_finite()
            && self.sla_deadlock_threshold.is_finite()
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if !self.critical_path_makespan.is_finite() || self.critical_path_makespan < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "critical_path_makespan must be finite and >= 0",
            ));
        }
        if !self.karp_cycle_mean.is_finite() {
            return Err(DistillationContractError::NonFiniteMetric(
                "karp_cycle_mean",
            ));
        }
        if !self.periodic_throughput.is_finite() || self.periodic_throughput < 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "periodic_throughput must be finite and >= 0",
            ));
        }
        if !self.sla_deadlock_threshold.is_finite() || self.sla_deadlock_threshold <= 0.0 {
            return Err(DistillationContractError::InvalidMetricValue(
                "sla_deadlock_threshold must be finite and > 0",
            ));
        }
        Ok(())
    }
}

/// Tamper-evident 12-engine mathematical assurance receipt telemetry summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssuranceReceiptSummary {
    pub run_id: String,
    pub sequence_number: u64,
    pub prev_receipt_sha256: Option<String>,
    pub all_invariants_verified: bool,
    pub phases_certified: usize,
    pub receipt_sha256: String,
    pub signature: Option<String>,
}

impl Default for AssuranceReceiptSummary {
    fn default() -> Self {
        Self {
            run_id: "pandora-assurance-live".to_string(),
            sequence_number: 1,
            prev_receipt_sha256: None,
            all_invariants_verified: true,
            phases_certified: 12,
            receipt_sha256:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            signature: None,
        }
    }
}

impl AssuranceReceiptSummary {
    pub fn new(
        run_id: impl Into<String>,
        sequence_number: u64,
        prev_receipt_sha256: Option<String>,
        all_invariants_verified: bool,
        phases_certified: usize,
        receipt_sha256: impl Into<String>,
        signature: Option<String>,
    ) -> Result<Self, DistillationContractError> {
        let summary = Self {
            run_id: run_id.into(),
            sequence_number,
            prev_receipt_sha256,
            all_invariants_verified,
            phases_certified,
            receipt_sha256: receipt_sha256.into(),
            signature,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn is_finite(&self) -> bool {
        true
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if self.receipt_sha256.is_empty() {
            return Err(DistillationContractError::InvalidMetricValue(
                "receipt_sha256 must not be empty",
            ));
        }
        Ok(())
    }
}

/// Persisted runtime and cross-session distillation telemetry snapshot surfaced in Cockpit & TUI.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistillationTelemetrySnapshot {
    pub pnn_hot_reloads: u32,
    pub lats_memo_hits: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_pnn_digest: Option<String>,
    pub dpo_pairs_total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_capsule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_capsule_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attestation_commitment: Option<String>,
    pub last_attestation_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasserstein_drift: Option<WassersteinTelemetrySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub martingale_canary: Option<MartingaleTelemetrySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conformal_calibration: Option<ConformalTelemetrySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectral_topology: Option<SpectralTopologySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adversarial_bandit: Option<AdversarialBanditSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mfg_congestion: Option<MfgTelemetrySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pac_bayes: Option<PacBayesTelemetrySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tropical_dag: Option<TropicalDagTelemetrySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assurance_receipt: Option<AssuranceReceiptSummary>,
    pub updated_at_unix: u64,
}

impl DistillationTelemetrySnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_runtime_counters(
        mut self,
        pnn_hot_reloads: u32,
        lats_memo_hits: u64,
        active_pnn_digest: Option<String>,
    ) -> Self {
        self.pnn_hot_reloads = pnn_hot_reloads;
        self.lats_memo_hits = lats_memo_hits;
        self.active_pnn_digest = active_pnn_digest;
        self
    }

    pub fn with_distillation_artifacts(
        mut self,
        dpo_pairs_total: usize,
        capsule_id: Option<String>,
        capsule_digest: Option<String>,
        attestation_commitment: Option<String>,
        attestation_verified: bool,
    ) -> Self {
        self.dpo_pairs_total = dpo_pairs_total;
        self.last_capsule_id = capsule_id;
        self.last_capsule_digest = capsule_digest;
        self.last_attestation_commitment = attestation_commitment;
        self.last_attestation_verified = attestation_verified;
        self
    }

    pub fn with_phase_metrics(
        mut self,
        wasserstein_drift: Option<WassersteinTelemetrySummary>,
        martingale_canary: Option<MartingaleTelemetrySummary>,
        updated_at_unix: u64,
    ) -> Self {
        self.wasserstein_drift = wasserstein_drift;
        self.martingale_canary = martingale_canary;
        self.updated_at_unix = updated_at_unix;
        self
    }

    pub fn with_phase15_17_metrics(
        mut self,
        conformal_calibration: Option<ConformalTelemetrySummary>,
        spectral_topology: Option<SpectralTopologySummary>,
        adversarial_bandit: Option<AdversarialBanditSummary>,
    ) -> Self {
        self.conformal_calibration = conformal_calibration;
        self.spectral_topology = spectral_topology;
        self.adversarial_bandit = adversarial_bandit;
        self
    }

    pub fn with_phase18_20_metrics(
        mut self,
        mfg_congestion: Option<MfgTelemetrySummary>,
        pac_bayes: Option<PacBayesTelemetrySummary>,
        tropical_dag: Option<TropicalDagTelemetrySummary>,
        assurance_receipt: Option<AssuranceReceiptSummary>,
    ) -> Self {
        self.mfg_congestion = mfg_congestion;
        self.pac_bayes = pac_bayes;
        self.tropical_dag = tropical_dag;
        self.assurance_receipt = assurance_receipt;
        self
    }

    pub fn is_finite(&self) -> bool {
        self.wasserstein_drift
            .as_ref()
            .is_none_or(WassersteinTelemetrySummary::is_finite)
            && self
                .martingale_canary
                .as_ref()
                .is_none_or(MartingaleTelemetrySummary::is_finite)
            && self
                .conformal_calibration
                .as_ref()
                .is_none_or(ConformalTelemetrySummary::is_finite)
            && self
                .spectral_topology
                .as_ref()
                .is_none_or(SpectralTopologySummary::is_finite)
            && self
                .adversarial_bandit
                .as_ref()
                .is_none_or(AdversarialBanditSummary::is_finite)
            && self
                .mfg_congestion
                .as_ref()
                .is_none_or(MfgTelemetrySummary::is_finite)
            && self
                .pac_bayes
                .as_ref()
                .is_none_or(PacBayesTelemetrySummary::is_finite)
            && self
                .tropical_dag
                .as_ref()
                .is_none_or(TropicalDagTelemetrySummary::is_finite)
            && self
                .assurance_receipt
                .as_ref()
                .is_none_or(AssuranceReceiptSummary::is_finite)
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if let Some(ref drift) = self.wasserstein_drift {
            drift.validate()?;
        }
        if let Some(ref canary) = self.martingale_canary {
            canary.validate()?;
        }
        if let Some(ref conformal) = self.conformal_calibration {
            conformal.validate()?;
        }
        if let Some(ref topology) = self.spectral_topology {
            topology.validate()?;
        }
        if let Some(ref bandit) = self.adversarial_bandit {
            bandit.validate()?;
        }
        if let Some(ref mfg) = self.mfg_congestion {
            mfg.validate()?;
        }
        if let Some(ref pac) = self.pac_bayes {
            pac.validate()?;
        }
        if let Some(ref dag) = self.tropical_dag {
            dag.validate()?;
        }
        if let Some(ref receipt) = self.assurance_receipt {
            receipt.validate()?;
        }
        Ok(())
    }

    pub fn load_from_file(path: &Path) -> Result<Self, DistillationContractError> {
        let raw =
            fs::read_to_string(path).map_err(|e| DistillationContractError::Io(e.to_string()))?;
        let snapshot: Self = serde_json::from_str(&raw)
            .map_err(|e| DistillationContractError::Serialization(e.to_string()))?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), DistillationContractError> {
        self.validate()?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| DistillationContractError::Io(e.to_string()))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| DistillationContractError::Serialization(e.to_string()))?;
        fs::write(path, json).map_err(|e| DistillationContractError::Io(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_snapshot_roundtrips_and_validates_finite_metrics() {
        let drift =
            WassersteinTelemetrySummary::new(0.0144, 0.12, 0.42, 0.08, 0.50, false, 12, 1e-7, 1e-8)
                .unwrap();
        let canary = MartingaleTelemetrySummary::new(
            12,
            0.91,
            1.04,
            0.96,
            20.0,
            -0.05,
            0.18,
            -0.23,
            0.13,
            0.12,
            "continue_monitoring",
        )
        .unwrap();
        let snap = DistillationTelemetrySnapshot::new()
            .with_runtime_counters(2, 17, Some("abc123".to_owned()))
            .with_distillation_artifacts(
                24,
                Some("capsule-1".to_owned()),
                Some("sha256:deadbeef".to_owned()),
                Some("commit123".to_owned()),
                true,
            )
            .with_phase_metrics(Some(drift), Some(canary), 1_700_000_000)
            .with_phase15_17_metrics(
                Some(ConformalTelemetrySummary::default()),
                Some(SpectralTopologySummary::default()),
                Some(AdversarialBanditSummary::default()),
            );

        assert!(snap.is_finite());
        assert!(snap.validate().is_ok());

        let json = serde_json::to_string(&snap).unwrap();
        let parsed: DistillationTelemetrySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, snap);
    }

    #[test]
    fn phase15_17_telemetry_summaries_validate_and_reject_invalids() {
        let conf = ConformalTelemetrySummary::default();
        assert!(conf.is_finite());
        assert!(conf.validate().is_ok());

        let topo = SpectralTopologySummary::default();
        assert!(topo.is_finite());
        assert!(topo.validate().is_ok());

        let bandit = AdversarialBanditSummary::default();
        assert!(bandit.is_finite());
        assert!(bandit.validate().is_ok());

        assert!(matches!(
            ConformalTelemetrySummary::new(
                10,
                1.5, // alpha > 1
                9,
                0.9,
                0.9,
                0.95,
                0.1,
                0.9,
                0.1,
                true,
                0.1,
                1,
                "certify_singleton",
            ),
            Err(DistillationContractError::InvalidMetricValue(_))
        ));

        assert!(matches!(
            SpectralTopologySummary::new(
                4,
                6,
                true,
                -1.0, // negative fiedler
                0.5,
                0.2,
                0.8,
                0.4,
                2,
                true,
                "connected_resilient",
            ),
            Err(DistillationContractError::InvalidMetricValue(_))
        ));

        assert!(matches!(
            AdversarialBanditSummary::new(
                3,
                100,
                0.1,
                0.0, // eta <= 0
                50.0,
                10.0,
                vec![0.5, 0.3, 0.2],
                vec![0.5, 0.3, 0.2],
                vec![50, 30, 20],
                vec![25.0, 15.0, 10.0],
            ),
            Err(DistillationContractError::InvalidMetricValue(_))
        ));
    }

    #[test]
    fn telemetry_validation_rejects_non_finite_and_out_of_range_metrics() {
        assert!(matches!(
            WassersteinTelemetrySummary::new(f64::NAN, 0.0, 0.0, 0.0, 0.5, false, 1, 0.0, 0.0,),
            Err(DistillationContractError::NonFiniteMetric(
                "sinkhorn_divergence"
            ))
        ));
        assert!(matches!(
            MartingaleTelemetrySummary::new(
                1,
                1.0,
                1.0,
                1.0,
                20.0,
                0.0,
                f64::INFINITY,
                -1.0,
                1.0,
                0.0,
                "continue_monitoring",
            ),
            Err(DistillationContractError::InvalidMetricValue(_))
        ));
    }

    #[test]
    fn phase18_20_and_assurance_telemetry_summaries_validate_and_reject_invalids() {
        let mfg = MfgTelemetrySummary::default();
        assert!(mfg.is_finite());
        assert!(mfg.validate().is_ok());

        let pac = PacBayesTelemetrySummary::default();
        assert!(pac.is_finite());
        assert!(pac.validate().is_ok());

        let dag = TropicalDagTelemetrySummary::default();
        assert!(dag.is_finite());
        assert!(dag.validate().is_ok());

        let receipt = AssuranceReceiptSummary::default();
        assert!(receipt.is_finite());
        assert!(receipt.validate().is_ok());

        assert!(matches!(
            MfgTelemetrySummary::new(
                3,
                -0.5, // tau <= 0
                2.14,
                1.81,
                1.182,
                1.3333333333333333,
                14,
                1.2e-6,
                true,
                true,
            ),
            Err(DistillationContractError::InvalidMetricValue(_))
        ));

        assert!(matches!(
            PacBayesTelemetrySummary::new(
                500,
                1.5, // delta > 1
                0.042,
                0.128,
                0.098,
                12.4,
                0.10,
                "PromoteToCandidate",
                true,
            ),
            Err(DistillationContractError::InvalidMetricValue(_))
        ));

        assert!(matches!(
            TropicalDagTelemetrySummary::new(
                5,
                "feedforward_dag",
                -1.0, // negative makespan
                0.0,
                0.0704,
                100.0,
                false,
                true,
            ),
            Err(DistillationContractError::InvalidMetricValue(_))
        ));

        assert!(matches!(
            AssuranceReceiptSummary::new(
                "test", 1, None, true, 12, "", // empty receipt hash
                None,
            ),
            Err(DistillationContractError::InvalidMetricValue(_))
        ));
    }
}
