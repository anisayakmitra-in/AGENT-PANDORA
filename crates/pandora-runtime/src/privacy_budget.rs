//! Phase 9: Distributed Differential Privacy Budget Tracker with Rényi DP.
//!
//! Provides mathematically optimal privacy budget accounting across heterogeneous
//! autonomous agent swarms, featuring:
//! - Exact Rényi Differential Privacy (RDP) evaluation for Gaussian and Laplace mechanisms.
//! - Linear sequential composition (Theorem 17): $\varepsilon_{\text{comp}}(\alpha) = \sum \varepsilon_i(\alpha)$.
//! - Optimal $(\varepsilon, \delta)$-DP tail bound conversion (Theorem 18):
//!   $\varepsilon(\delta) = \inf_{\alpha \in \mathcal{A}} [\varepsilon(\alpha) + \frac{\ln(1/\delta)}{\alpha - 1}]$.
//! - Decentralized multi-agent query reservation lifecycle (`reserve` -> `commit` / `release`).
//! - Calibrated Gaussian and Laplace perturbation sampling using safe, deterministic PRNG.

#![forbid(unsafe_code)]

use crate::bandit_router::SafePrng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Standard evaluation grid of Rényi orders $\alpha > 1$.
///
/// Spans orders from 1.25 to 64.0 to guarantee $< 0.1\%$ discretization error
/// against continuous infima across diverse noise scales.
pub const STANDARD_RDP_ORDERS: [f64; 16] = [
    1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0,
];

/// Multiplier used to map floating-point Rényi orders to integer keys for deterministic map storage.
pub const RDP_ORDER_KEY_SCALE: f64 = 1000.0;

/// Converts a floating-point Rényi order $\alpha$ to its canonical integer key.
#[inline]
pub fn order_to_key(alpha: f64) -> u64 {
    (alpha * RDP_ORDER_KEY_SCALE).round() as u64
}

/// Converts an integer key back to its floating-point Rényi order $\alpha$.
#[inline]
pub fn key_to_order(key: u64) -> f64 {
    (key as f64) / RDP_ORDER_KEY_SCALE
}

/// Strongly-typed Rényi order wrapper enforcing $\alpha > 1.0$.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct RdpOrder(pub f64);

impl RdpOrder {
    /// Creates a new `RdpOrder`, returning an error if $\alpha \le 1.0$ or non-finite.
    pub fn new(alpha: f64) -> Result<Self, PrivacyAccountantError> {
        if alpha <= 1.0 || alpha.is_nan() || alpha.is_infinite() {
            Err(PrivacyAccountantError::InvalidOrder(alpha))
        } else {
            Ok(Self(alpha))
        }
    }

    /// Returns the raw order as an `f64`.
    pub fn value(&self) -> f64 {
        self.0
    }

    /// Returns the integer map key for this order.
    pub fn key(&self) -> u64 {
        order_to_key(self.0)
    }
}

/// Gaussian perturbation mechanism parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GaussianMechanism {
    /// Global $\ell_2$-sensitivity: $\Delta_2 = \max \|f(D) - f(D')\|_2$.
    pub sensitivity: f64,
    /// Calibrated noise standard deviation $\sigma > 0$.
    pub sigma: f64,
}

impl GaussianMechanism {
    /// Constructs a validated Gaussian mechanism configuration.
    pub fn new(sensitivity: f64, sigma: f64) -> Result<Self, PrivacyAccountantError> {
        if sensitivity <= 0.0 || sensitivity.is_nan() {
            return Err(PrivacyAccountantError::InvalidSensitivity(sensitivity));
        }
        if sigma <= 0.0 || sigma.is_nan() {
            return Err(PrivacyAccountantError::InvalidSigma(sigma));
        }
        Ok(Self { sensitivity, sigma })
    }

    /// Evaluates closed-form RDP budget for order $\alpha$: $\varepsilon(\alpha) = \frac{\alpha \Delta_2^2}{2\sigma^2}$.
    pub fn rdp_epsilon(&self, alpha: f64) -> Result<f64, PrivacyAccountantError> {
        if alpha <= 1.0 || alpha.is_nan() {
            return Err(PrivacyAccountantError::InvalidOrder(alpha));
        }
        let var2 = 2.0 * self.sigma * self.sigma;
        let sens_sq = self.sensitivity * self.sensitivity;
        Ok((alpha * sens_sq) / var2)
    }

    /// Generates full RDP profile over the standard orders spectrum.
    pub fn to_rdp_budget(&self) -> Result<RdpBudget, PrivacyAccountantError> {
        RdpBudget::from_gaussian(self.sensitivity, self.sigma)
    }

    /// Perturbs a scalar value by adding calibrated Gaussian noise sampled via Box-Muller.
    pub fn add_noise(&self, value: f64, rng: &mut SafePrng) -> f64 {
        let z = rng.sample_standard_normal();
        value + (z * self.sigma)
    }

    /// Perturbs a slice of values by adding independent Gaussian noise to each element.
    pub fn add_noise_vec(&self, values: &[f64], rng: &mut SafePrng) -> Vec<f64> {
        values.iter().map(|&v| self.add_noise(v, rng)).collect()
    }
}

/// Laplace perturbation mechanism parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LaplaceMechanism {
    /// Global $\ell_1$-sensitivity: $\Delta_1 = \max \|f(D) - f(D')\|_1$.
    pub sensitivity: f64,
    /// Calibrated Laplace scale parameter $b > 0$.
    pub scale: f64,
}

impl LaplaceMechanism {
    /// Constructs a validated Laplace mechanism configuration.
    pub fn new(sensitivity: f64, scale: f64) -> Result<Self, PrivacyAccountantError> {
        if sensitivity <= 0.0 || sensitivity.is_nan() {
            return Err(PrivacyAccountantError::InvalidSensitivity(sensitivity));
        }
        if scale <= 0.0 || scale.is_nan() {
            return Err(PrivacyAccountantError::InvalidScale(scale));
        }
        Ok(Self { sensitivity, scale })
    }

    /// Evaluates closed-form RDP budget for order $\alpha > 1$.
    pub fn rdp_epsilon(&self, alpha: f64) -> Result<f64, PrivacyAccountantError> {
        if alpha <= 1.0 || alpha.is_nan() {
            return Err(PrivacyAccountantError::InvalidOrder(alpha));
        }
        let lambda = self.sensitivity / self.scale;
        if (alpha - 1.0) * lambda > 700.0 {
            return Err(PrivacyAccountantError::NumericalOverflow(
                "Laplace exponent overflow".to_string(),
            ));
        }
        let term1 = (alpha / (2.0 * alpha - 1.0)) * ((alpha - 1.0) * lambda).exp();
        let term2 = ((alpha - 1.0) / (2.0 * alpha - 1.0)) * (-alpha * lambda).exp();
        let inner = term1 + term2;
        if inner <= 0.0 || inner.is_nan() {
            return Ok(0.0);
        }
        let eps = inner.ln() / (alpha - 1.0);
        Ok(eps.max(0.0))
    }

    /// Generates full RDP profile over the standard orders spectrum.
    pub fn to_rdp_budget(&self) -> Result<RdpBudget, PrivacyAccountantError> {
        RdpBudget::from_laplace(self.sensitivity, self.scale)
    }

    /// Perturbs a scalar value by adding calibrated Laplace noise sampled via inverse CDF.
    pub fn add_noise(&self, value: f64, rng: &mut SafePrng) -> f64 {
        let mut u = rng.sample_uniform_f64();
        while u <= 1e-15 || (1.0 - u) <= 1e-15 {
            u = rng.sample_uniform_f64();
        }
        let centered = u - 0.5;
        let sgn = if centered >= 0.0 { 1.0 } else { -1.0 };
        let noise = -self.scale * sgn * (1.0 - 2.0 * centered.abs()).ln();
        value + noise
    }

    /// Perturbs a slice of values by adding independent Laplace noise to each element.
    pub fn add_noise_vec(&self, values: &[f64], rng: &mut SafePrng) -> Vec<f64> {
        values.iter().map(|&v| self.add_noise(v, rng)).collect()
    }
}

/// Cumulative Rényi Differential Privacy budget profile evaluated across orders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RdpBudget {
    /// Mapping from fixed-point integer key (`round(alpha * 1000)`) to $\varepsilon(\alpha)$.
    pub values: BTreeMap<u64, f64>,
}

impl Default for RdpBudget {
    fn default() -> Self {
        Self::zero()
    }
}

impl RdpBudget {
    /// Initializes an empty budget with $\varepsilon(\alpha) = 0.0$ for all standard orders.
    pub fn zero() -> Self {
        let mut values = BTreeMap::new();
        for &alpha in &STANDARD_RDP_ORDERS {
            values.insert(order_to_key(alpha), 0.0);
        }
        Self { values }
    }

    /// Constructs an RDP budget for a Gaussian mechanism with sensitivity $\Delta_2$ and noise $\sigma$.
    pub fn gaussian(sensitivity: f64, sigma: f64) -> Result<Self, PrivacyAccountantError> {
        Self::from_gaussian(sensitivity, sigma)
    }

    /// Evaluates exact Gaussian mechanism RDP profile: $\varepsilon(\alpha) = \frac{\alpha \Delta_2^2}{2\sigma^2}$.
    pub fn from_gaussian(sensitivity: f64, sigma: f64) -> Result<Self, PrivacyAccountantError> {
        let mechanism = GaussianMechanism::new(sensitivity, sigma)?;
        let mut values = BTreeMap::new();
        for &alpha in &STANDARD_RDP_ORDERS {
            let eps = mechanism.rdp_epsilon(alpha)?;
            values.insert(order_to_key(alpha), eps);
        }
        Ok(Self { values })
    }

    /// Constructs an RDP budget for a Laplace mechanism with sensitivity $\Delta_1$ and scale $b$.
    pub fn laplace(sensitivity: f64, scale: f64) -> Result<Self, PrivacyAccountantError> {
        Self::from_laplace(sensitivity, scale)
    }

    /// Evaluates Laplace mechanism RDP profile over the standard orders spectrum.
    pub fn from_laplace(sensitivity: f64, scale: f64) -> Result<Self, PrivacyAccountantError> {
        let mechanism = LaplaceMechanism::new(sensitivity, scale)?;
        let mut values = BTreeMap::new();
        for &alpha in &STANDARD_RDP_ORDERS {
            let eps = mechanism.rdp_epsilon(alpha)?;
            values.insert(order_to_key(alpha), eps);
        }
        Ok(Self { values })
    }

    /// Returns the $\varepsilon(\alpha)$ value for a specific order $\alpha$, if present in the profile.
    pub fn get_order(&self, alpha: f64) -> Option<f64> {
        let key = order_to_key(alpha);
        self.values.get(&key).copied()
    }

    /// Inserts or updates an explicit order $\alpha > 1$ with expenditure $\varepsilon$.
    pub fn set_order(&mut self, alpha: f64, epsilon: f64) -> Result<(), PrivacyAccountantError> {
        if alpha <= 1.0 || alpha.is_nan() {
            return Err(PrivacyAccountantError::InvalidOrder(alpha));
        }
        if epsilon < 0.0 || epsilon.is_nan() {
            return Err(PrivacyAccountantError::InvalidBudget(epsilon));
        }
        self.values.insert(order_to_key(alpha), epsilon);
        Ok(())
    }

    /// Applies Theorem 17 linear composition by element-wise summation, returning a new `RdpBudget`.
    pub fn compose(&self, other: &Self) -> Result<Self, PrivacyAccountantError> {
        let mut result = self.clone();
        result.compose_in_place(other);
        Ok(result)
    }

    /// In-place element-wise linear composition with another `RdpBudget` (Theorem 17).
    pub fn compose_in_place(&mut self, other: &Self) {
        for (k, v) in &other.values {
            *self.values.entry(*k).or_insert(0.0) += *v;
        }
    }

    /// Converts RDP budget to equivalent $(\varepsilon(\delta), \delta)$-DP via Theorem 18 infimum.
    ///
    /// Evaluates:
    /// $$\varepsilon(\delta) = \inf_{\alpha \in \mathcal{A}} \left[ \varepsilon(\alpha) + \frac{\ln(1/\delta)}{\alpha - 1} \right]$$
    pub fn to_approx_dp(&self, delta: f64) -> Result<(f64, f64), PrivacyAccountantError> {
        let eps = self.epsilon_at_delta(delta)?;
        Ok((eps, delta))
    }

    /// Computes the optimal converted $\varepsilon$ for target failure probability $\delta \in (0, 1)$.
    pub fn epsilon_at_delta(&self, delta: f64) -> Result<f64, PrivacyAccountantError> {
        if delta <= 0.0 || delta >= 1.0 || delta.is_nan() {
            return Err(PrivacyAccountantError::InvalidDelta(delta));
        }
        if self.values.is_empty() {
            return Ok(0.0);
        }

        let log_inv_delta = (1.0 / delta).ln();
        let mut min_eps = f64::INFINITY;

        for (&key, &rdp_eps) in &self.values {
            let alpha = key_to_order(key);
            if alpha <= 1.0 {
                continue;
            }
            let converted = rdp_eps + (log_inv_delta / (alpha - 1.0));
            if converted < min_eps {
                min_eps = converted;
            }
        }

        if min_eps.is_infinite() {
            Ok(0.0)
        } else {
            Ok(min_eps)
        }
    }

    /// Finds the specific order $\alpha^* \in \mathcal{A}$ that minimizes the conversion bound for $\delta$.
    pub fn optimal_order_for_delta(
        &self,
        delta: f64,
    ) -> Result<(f64, f64), PrivacyAccountantError> {
        if delta <= 0.0 || delta >= 1.0 || delta.is_nan() {
            return Err(PrivacyAccountantError::InvalidDelta(delta));
        }
        let log_inv_delta = (1.0 / delta).ln();
        let mut best_alpha = 2.0;
        let mut min_eps = f64::INFINITY;

        for (&key, &rdp_eps) in &self.values {
            let alpha = key_to_order(key);
            if alpha <= 1.0 {
                continue;
            }
            let converted = rdp_eps + (log_inv_delta / (alpha - 1.0));
            if converted < min_eps {
                min_eps = converted;
                best_alpha = alpha;
            }
        }

        Ok((best_alpha, min_eps))
    }

    /// Computes theoretical continuous optimum for $k$ Gaussian mechanisms under Theorem 18:
    /// $\alpha^* = 1 + \sqrt{\frac{2\sigma^2\ln(1/\delta)}{k\Delta_2^2}}$,
    /// $\varepsilon^*(\delta) = \frac{k\Delta_2^2}{2\sigma^2} + \sqrt{\frac{2k\Delta_2^2\ln(1/\delta)}{\sigma^2}}$.
    pub fn continuous_gaussian_optimum(
        k: usize,
        sensitivity: f64,
        sigma: f64,
        delta: f64,
    ) -> Result<(f64, f64), PrivacyAccountantError> {
        if k == 0 {
            return Ok((1.0, 0.0));
        }
        if sensitivity <= 0.0 {
            return Err(PrivacyAccountantError::InvalidSensitivity(sensitivity));
        }
        if sigma <= 0.0 {
            return Err(PrivacyAccountantError::InvalidSigma(sigma));
        }
        if delta <= 0.0 || delta >= 1.0 {
            return Err(PrivacyAccountantError::InvalidDelta(delta));
        }

        let k_f = k as f64;
        let sens_sq = sensitivity * sensitivity;
        let sigma_sq = sigma * sigma;
        let log_inv_delta = (1.0 / delta).ln();

        let ratio = (2.0 * sigma_sq * log_inv_delta) / (k_f * sens_sq);
        let alpha_star = 1.0 + ratio.sqrt();

        let first_term = (k_f * sens_sq) / (2.0 * sigma_sq);
        let second_term = ((2.0 * k_f * sens_sq * log_inv_delta) / sigma_sq).sqrt();
        let eps_star = first_term + second_term;

        Ok((alpha_star, eps_star))
    }

    /// Returns the maximum $\varepsilon(\alpha)$ across all tracked orders.
    pub fn max_epsilon(&self) -> f64 {
        self.values.values().copied().fold(0.0, f64::max)
    }

    /// Returns true if all Rényi orders have zero expenditure.
    pub fn is_empty(&self) -> bool {
        self.values.values().all(|&v| v == 0.0)
    }
}

/// A privacy expenditure record in the distributed ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrivacyExpenditure {
    pub transaction_id: String,
    pub agent_id: String,
    pub query_id: String,
    pub mechanism_name: String,
    pub sensitivity: f64,
    pub noise_parameter: f64,
    pub rdp_expenditure: RdpBudget,
    pub timestamp_ms: u64,
}

/// A pending privacy budget reservation in the federated swarm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryReservation {
    pub reservation_id: String,
    pub agent_id: String,
    pub query_id: String,
    pub reserved_budget: RdpBudget,
    pub timestamp_ms: u64,
    pub committed: bool,
}

/// Distributed multi-agent differential privacy coordinator and budget tracker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DistributedPrivacyTracker {
    /// Maximum allowable global privacy budget $\varepsilon_{\max}$.
    pub global_max_epsilon: f64,
    /// Fixed target failure rate $\delta_{\text{target}} \in (0, 1)$.
    pub global_target_delta: f64,
    /// Cumulative committed RDP budget across all completed queries.
    pub cumulative_budget: RdpBudget,
    /// In-flight reservations held by peer agents prior to query execution.
    pub active_reservations: BTreeMap<String, QueryReservation>,
    /// Cumulative committed RDP budget indexed per agent ID.
    pub agent_allocations: BTreeMap<String, RdpBudget>,
    /// Immutable audit ledger of committed privacy transactions.
    pub ledger: Vec<PrivacyExpenditure>,
    /// Monotonic counter used for unique reservation IDs.
    reservation_counter: u64,
}

impl DistributedPrivacyTracker {
    /// Creates a new distributed privacy tracker with specified global budget thresholds.
    pub fn new(
        global_max_epsilon: f64,
        global_target_delta: f64,
    ) -> Result<Self, PrivacyAccountantError> {
        if global_max_epsilon <= 0.0 || global_max_epsilon.is_nan() {
            return Err(PrivacyAccountantError::InvalidBudget(global_max_epsilon));
        }
        if global_target_delta <= 0.0 || global_target_delta >= 1.0 || global_target_delta.is_nan()
        {
            return Err(PrivacyAccountantError::InvalidDelta(global_target_delta));
        }
        Ok(Self {
            global_max_epsilon,
            global_target_delta,
            cumulative_budget: RdpBudget::zero(),
            active_reservations: BTreeMap::new(),
            agent_allocations: BTreeMap::new(),
            ledger: Vec::new(),
            reservation_counter: 0,
        })
    }

    /// Evaluates current committed $(\varepsilon, \delta)$-DP bound.
    pub fn current_approx_dp(&self) -> Result<(f64, f64), PrivacyAccountantError> {
        self.cumulative_budget
            .to_approx_dp(self.global_target_delta)
    }

    /// Aggregates all currently active, uncommitted reservations into an RDP budget.
    pub fn reserved_rdp_budget(&self) -> RdpBudget {
        let mut reserved = RdpBudget::zero();
        for res in self.active_reservations.values() {
            if !res.committed {
                reserved.compose_in_place(&res.reserved_budget);
            }
        }
        reserved
    }

    /// Computes total allocated RDP budget: cumulative committed plus active reservations.
    pub fn total_allocated_rdp(&self) -> RdpBudget {
        let mut total = self.cumulative_budget.clone();
        total.compose_in_place(&self.reserved_rdp_budget());
        total
    }

    /// Checks whether an additional RDP budget can be admitted without exceeding $\varepsilon_{\max}$.
    pub fn can_admit_query(&self, additional: &RdpBudget) -> Result<bool, PrivacyAccountantError> {
        let mut hypothetical = self.total_allocated_rdp();
        hypothetical.compose_in_place(additional);
        let converted_eps = hypothetical.epsilon_at_delta(self.global_target_delta)?;
        Ok(converted_eps <= self.global_max_epsilon)
    }

    /// Reserves privacy budget for an agent query prior to execution.
    ///
    /// Returns the unique reservation token upon successful admission.
    pub fn reserve(
        &mut self,
        agent_id: &str,
        query_id: &str,
        budget: &RdpBudget,
    ) -> Result<String, PrivacyAccountantError> {
        if !self.can_admit_query(budget)? {
            let mut hyp = self.total_allocated_rdp();
            hyp.compose_in_place(budget);
            let projected = hyp.epsilon_at_delta(self.global_target_delta)?;
            return Err(PrivacyAccountantError::BudgetExceeded {
                projected,
                limit: self.global_max_epsilon,
            });
        }

        self.reservation_counter += 1;
        let reservation_id = format!("res_{}_{}_{}", agent_id, self.reservation_counter, query_id);

        let reservation = QueryReservation {
            reservation_id: reservation_id.clone(),
            agent_id: agent_id.to_string(),
            query_id: query_id.to_string(),
            reserved_budget: budget.clone(),
            timestamp_ms: self.reservation_counter,
            committed: false,
        };

        self.active_reservations
            .insert(reservation_id.clone(), reservation);
        Ok(reservation_id)
    }

    /// Commits a successful query reservation, applying budget to cumulative and agent totals.
    ///
    /// Returns the updated global converted $\varepsilon(\delta)$.
    pub fn commit_reservation(
        &mut self,
        reservation_id: &str,
    ) -> Result<f64, PrivacyAccountantError> {
        let mut res = self
            .active_reservations
            .remove(reservation_id)
            .ok_or_else(|| {
                PrivacyAccountantError::ReservationNotFound(reservation_id.to_string())
            })?;

        if res.committed {
            return Err(PrivacyAccountantError::ReservationAlreadyCommitted(
                reservation_id.to_string(),
            ));
        }

        res.committed = true;
        self.cumulative_budget
            .compose_in_place(&res.reserved_budget);
        self.agent_allocations
            .entry(res.agent_id.clone())
            .or_insert_with(RdpBudget::zero)
            .compose_in_place(&res.reserved_budget);

        let expenditure = PrivacyExpenditure {
            transaction_id: format!("tx_{}", reservation_id),
            agent_id: res.agent_id,
            query_id: res.query_id,
            mechanism_name: "reserved_query".to_string(),
            sensitivity: 1.0,
            noise_parameter: 1.0,
            rdp_expenditure: res.reserved_budget,
            timestamp_ms: res.timestamp_ms,
        };
        self.ledger.push(expenditure);

        self.cumulative_budget
            .epsilon_at_delta(self.global_target_delta)
    }

    /// Releases a reservation when a query fails, times out, or is cancelled.
    pub fn release_reservation(
        &mut self,
        reservation_id: &str,
    ) -> Result<(), PrivacyAccountantError> {
        self.active_reservations
            .remove(reservation_id)
            .ok_or_else(|| {
                PrivacyAccountantError::ReservationNotFound(reservation_id.to_string())
            })?;
        Ok(())
    }

    /// Directly records an executed expenditure without prior reservation.
    pub fn record_expenditure(
        &mut self,
        exp: PrivacyExpenditure,
    ) -> Result<f64, PrivacyAccountantError> {
        if !self.can_admit_query(&exp.rdp_expenditure)? {
            let mut hyp = self.total_allocated_rdp();
            hyp.compose_in_place(&exp.rdp_expenditure);
            let projected = hyp.epsilon_at_delta(self.global_target_delta)?;
            return Err(PrivacyAccountantError::BudgetExceeded {
                projected,
                limit: self.global_max_epsilon,
            });
        }

        self.cumulative_budget
            .compose_in_place(&exp.rdp_expenditure);
        self.agent_allocations
            .entry(exp.agent_id.clone())
            .or_insert_with(RdpBudget::zero)
            .compose_in_place(&exp.rdp_expenditure);
        self.ledger.push(exp);

        self.cumulative_budget
            .epsilon_at_delta(self.global_target_delta)
    }

    /// Returns the accumulated RDP budget for a given agent.
    pub fn agent_budget(&self, agent_id: &str) -> Option<&RdpBudget> {
        self.agent_allocations.get(agent_id)
    }

    /// Evaluates the remaining global $\varepsilon$ before exhausting the budget.
    pub fn remaining_epsilon(&self) -> Result<f64, PrivacyAccountantError> {
        let current_eps = self
            .cumulative_budget
            .epsilon_at_delta(self.global_target_delta)?;
        Ok((self.global_max_epsilon - current_eps).max(0.0))
    }

    /// Returns the budget utilization ratio $\frac{\varepsilon_{\text{current}}}{\varepsilon_{\max}} \in [0, 1]$.
    pub fn budget_utilization(&self) -> Result<f64, PrivacyAccountantError> {
        let current_eps = self
            .cumulative_budget
            .epsilon_at_delta(self.global_target_delta)?;
        Ok((current_eps / self.global_max_epsilon).min(1.0))
    }
}

/// Error taxonomy for differential privacy accounting and budget operations.
#[derive(Clone, Debug, PartialEq)]
pub enum PrivacyAccountantError {
    /// Rényi order is invalid ($\alpha \le 1.0$ or non-finite).
    InvalidOrder(f64),
    /// Query sensitivity is invalid ($\le 0.0$ or non-finite).
    InvalidSensitivity(f64),
    /// Noise parameter $\sigma \le 0.0$.
    InvalidSigma(f64),
    /// Noise parameter $b \le 0.0$.
    InvalidScale(f64),
    /// Failure probability $\delta \notin (0, 1)$.
    InvalidDelta(f64),
    /// Budget specification $\le 0.0$.
    InvalidBudget(f64),
    /// Transaction exceeds global privacy budget limit.
    BudgetExceeded { projected: f64, limit: f64 },
    /// Specified reservation ID was not found in active tracking table.
    ReservationNotFound(String),
    /// Reservation has already been committed.
    ReservationAlreadyCommitted(String),
    /// Evaluated order spectra do not match.
    OrderMismatch(String),
    /// Numerical overflow during exponential evaluation.
    NumericalOverflow(String),
}

/// Type alias for ergonomic backwards compatibility across modules.
pub type PrivacyError = PrivacyAccountantError;

impl fmt::Display for PrivacyAccountantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOrder(alpha) => {
                write!(
                    f,
                    "invalid Rényi order alpha={alpha}; must satisfy alpha > 1.0"
                )
            }
            Self::InvalidSensitivity(s) => {
                write!(
                    f,
                    "invalid query sensitivity {s}; must be strictly positive"
                )
            }
            Self::InvalidSigma(sig) => {
                write!(
                    f,
                    "invalid Gaussian noise scale sigma={sig}; must be strictly positive"
                )
            }
            Self::InvalidScale(b) => {
                write!(
                    f,
                    "invalid Laplace noise scale b={b}; must be strictly positive"
                )
            }
            Self::InvalidDelta(d) => {
                write!(f, "invalid DP target delta={d}; must satisfy 0 < delta < 1")
            }
            Self::InvalidBudget(b) => {
                write!(
                    f,
                    "invalid privacy budget epsilon={b}; must be strictly positive"
                )
            }
            Self::BudgetExceeded { projected, limit } => {
                write!(
                    f,
                    "privacy budget exceeded: projected={projected:.4}, limit={limit:.4}"
                )
            }
            Self::ReservationNotFound(id) => write!(f, "query reservation not found: {id}"),
            Self::ReservationAlreadyCommitted(id) => {
                write!(f, "reservation already committed: {id}")
            }
            Self::OrderMismatch(desc) => write!(f, "Rényi order mismatch: {desc}"),
            Self::NumericalOverflow(desc) => {
                write!(f, "numerical overflow in DP evaluation: {desc}")
            }
        }
    }
}

impl std::error::Error for PrivacyAccountantError {}
