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

    pub fn is_finite(&self) -> bool {
        self.wasserstein_drift
            .as_ref()
            .is_none_or(WassersteinTelemetrySummary::is_finite)
            && self
                .martingale_canary
                .as_ref()
                .is_none_or(MartingaleTelemetrySummary::is_finite)
    }

    pub fn validate(&self) -> Result<(), DistillationContractError> {
        if let Some(ref drift) = self.wasserstein_drift {
            drift.validate()?;
        }
        if let Some(ref canary) = self.martingale_canary {
            canary.validate()?;
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
            .with_phase_metrics(Some(drift), Some(canary), 1_700_000_000);

        assert!(snap.is_finite());
        assert!(snap.validate().is_ok());

        let json = serde_json::to_string(&snap).unwrap();
        let parsed: DistillationTelemetrySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, snap);
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
}
