#![forbid(unsafe_code)]

//! Phase 17: Exp3 / Hedge Adversarial Multi-Armed Bandit Tool & Prompt Selector
//!
//! Implements the log-domain stabilized Exp3 (Exponential-weight algorithm for
//! Exploration and Exploitation, Auer, Cesa-Bianchi, Freund & Schapire 2002) for
//! non-stationary and adversarial tool/model/prompt routing.
//!
//! ## Mathematical Guarantees
//!
//! 1. **Theorem 33 (Unbiased Importance-Weighted Reward Estimator & Variance Bound):**
//!    At round $t$, let $p_t = (p_{1,t}, \dots, p_{K,t}) \in \Delta_{K-1}$ be the mixed
//!    sampling distribution
//!    $$p_{i,t} = (1 - \gamma)\frac{w_{i,t}}{\sum_{j=1}^K w_{j,t}} + \frac{\gamma}{K}, \qquad \gamma \in (0, 1]$$
//!    and for observed reward $r_{I_t, t} \in [0, 1]$ define
//!    $$\hat{r}_{i,t} = \frac{r_{i,t}\,\mathbf{1}\{I_t = i\}}{p_{i,t}}.$$
//!    Then conditioned on history $\mathcal{F}_{t-1}$:
//!    $$\mathbb{E}_{I_t \sim p_t}\bigl[\hat{r}_{i,t} \mid \mathcal{F}_{t-1}\bigr] = \sum_{j=1}^K p_{j,t} \frac{r_{i,t}\mathbf{1}\{j=i\}}{p_{i,t}} = r_{i,t},$$
//!    and the conditional second moment satisfies
//!    $$\mathbb{E}_{I_t \sim p_t}\bigl[\hat{r}_{i,t}^2 \mid \mathcal{F}_{t-1}\bigr] = \frac{r_{i,t}^2}{p_{i,t}} \le \frac{r_{i,t}}{p_{i,t}} \le \frac{K}{\gamma}.$$
//!
//! 2. **Theorem 34 (Auer et al. Exp3 Non-Stochastic Minimax Regret Bound):**
//!    With log-weight update $\ln w_{i,t+1} = \ln w_{i,t} + \eta \hat{r}_{i,t}$ where $\eta = \gamma / K$
//!    and optimal exploration parameter
//!    $$\gamma = \min\!\left(1,\, \sqrt{\frac{K \ln K}{(e - 1) T}}\right),$$
//!    for any oblivious adversarial sequence of rewards $\{r_{i,t}\}_{i \in [K], t \in [T]} \in [0, 1]^{K \times T}$:
//!    $$\mathbb{E}[R_T] = \max_{i \in [K]} \sum_{t=1}^T r_{i,t} - \mathbb{E}\!\left[\sum_{t=1}^T r_{I_t, t}\right] \le 2\sqrt{(e - 1) T K \ln K}.$$

use serde::{Deserialize, Serialize};
use std::fmt;

/// Errors returned by the adversarial bandit engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AdversarialBanditError {
    /// Bandit must have at least 2 arms.
    InsufficientArms { count: usize },
    /// Exploration parameter $\gamma$ must lie in $(0, 1]$.
    InvalidGamma { gamma: f64 },
    /// Arm index is out of bounds.
    ArmIndexOutOfBounds { arm_index: usize, num_arms: usize },
    /// Observed reward $r_t$ must lie in $[0, 1]$.
    InvalidReward { reward: f64 },
    /// Sampling quantile $u$ must lie in $[0, 1)$.
    InvalidUniformSample { u: f64 },
}

impl fmt::Display for AdversarialBanditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientArms { count } => {
                write!(
                    f,
                    "Exp3 adversarial bandit requires at least 2 arms, got {count}"
                )
            }
            Self::InvalidGamma { gamma } => {
                write!(f, "exploration parameter gamma ({gamma}) must be in (0, 1]")
            }
            Self::ArmIndexOutOfBounds {
                arm_index,
                num_arms,
            } => {
                write!(
                    f,
                    "arm index {arm_index} out of bounds for {num_arms} arm(s)"
                )
            }
            Self::InvalidReward { reward } => {
                write!(f, "observed reward ({reward}) must be finite and in [0, 1]")
            }
            Self::InvalidUniformSample { u } => {
                write!(f, "uniform sample u ({u}) must be finite and in [0, 1)")
            }
        }
    }
}

impl std::error::Error for AdversarialBanditError {}

/// Snapshot of a single arm's state in the Exp3 adversarial bandit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp3ArmState {
    pub arm_index: usize,
    pub arm_id: String,
    pub log_weight: f64,
    pub normalized_weight: f64,
    pub sampling_probability: f64,
    pub pulls: usize,
    pub cumulative_estimated_reward: f64,
}

/// Telemetry and regret certificate for the Exp3 adversarial bandit (Theorems 33 and 34).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp3BanditStatus {
    pub num_arms: usize,
    pub gamma: f64,
    pub eta: f64,
    pub rounds_elapsed: usize,
    pub cumulative_observed_reward: f64,
    pub max_estimated_arm_reward: f64,
    pub minimax_regret_upper_bound: f64,
    pub arms: Vec<Exp3ArmState>,
}

/// Log-domain stabilized Exp3 Adversarial Multi-Armed Bandit implementing Theorems 33 and 34.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exp3AdversarialBandit {
    arm_ids: Vec<String>,
    log_weights: Vec<f64>,
    pulls: Vec<usize>,
    cumulative_estimated_rewards: Vec<f64>,
    gamma: f64,
    eta: f64,
    rounds_elapsed: usize,
    cumulative_observed_reward: f64,
}

impl Exp3AdversarialBandit {
    /// Construct an Exp3 bandit with explicit exploration parameter $\gamma \in (0, 1]$.
    pub fn new(arm_ids: Vec<String>, gamma: f64) -> Result<Self, AdversarialBanditError> {
        let k = arm_ids.len();
        if k < 2 {
            return Err(AdversarialBanditError::InsufficientArms { count: k });
        }
        if !gamma.is_finite() || gamma <= 0.0 || gamma > 1.0 {
            return Err(AdversarialBanditError::InvalidGamma { gamma });
        }
        let eta = gamma / (k as f64);
        Ok(Self {
            arm_ids,
            log_weights: vec![0.0; k],
            pulls: vec![0; k],
            cumulative_estimated_rewards: vec![0.0; k],
            gamma,
            eta,
            rounds_elapsed: 0,
            cumulative_observed_reward: 0.0,
        })
    }

    /// Construct an Exp3 bandit with horizon-optimal exploration parameter
    /// $\gamma = \min\bigl(1, \sqrt{\frac{K \ln K}{(e - 1) T}}\bigr)$ from Theorem 34.
    pub fn with_horizon(
        arm_ids: Vec<String>,
        horizon_t: usize,
    ) -> Result<Self, AdversarialBanditError> {
        let k = arm_ids.len();
        if k < 2 {
            return Err(AdversarialBanditError::InsufficientArms { count: k });
        }
        let t = horizon_t.max(1) as f64;
        let k_f = k as f64;
        let e_minus_1 = std::f64::consts::E - 1.0;
        let optimal_gamma = ((k_f * k_f.ln()) / (e_minus_1 * t)).sqrt().clamp(1e-6, 1.0);
        Self::new(arm_ids, optimal_gamma)
    }

    /// Compute normalized weights $w_{i,t} / \sum_j w_{j,t}$ in log-domain using the max-trick.
    pub fn normalized_weights(&self) -> Vec<f64> {
        let max_lw = self
            .log_weights
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let exp_weights: Vec<f64> = self
            .log_weights
            .iter()
            .map(|&lw| (lw - max_lw).exp())
            .collect();
        let sum: f64 = exp_weights.iter().sum();
        exp_weights.into_iter().map(|w| w / sum).collect()
    }

    /// Compute mixed sampling probabilities $p_{i,t} = (1 - \gamma) \tilde{w}_{i,t} + \gamma / K$.
    pub fn sampling_probabilities(&self) -> Vec<f64> {
        let k_f = self.arm_ids.len() as f64;
        let norm_w = self.normalized_weights();
        norm_w
            .into_iter()
            .map(|w| (1.0 - self.gamma) * w + self.gamma / k_f)
            .collect()
    }

    /// Select an arm given a uniform random quantile $u \in [0, 1)$.
    pub fn select_arm(&self, u: f64) -> Result<(usize, String, f64), AdversarialBanditError> {
        if !u.is_finite() || !(0.0..1.0).contains(&u) {
            return Err(AdversarialBanditError::InvalidUniformSample { u });
        }
        let probs = self.sampling_probabilities();
        let mut cdf = 0.0_f64;
        for (idx, &p) in probs.iter().enumerate() {
            cdf += p;
            if u < cdf || idx + 1 == probs.len() {
                return Ok((idx, self.arm_ids[idx].clone(), p));
            }
        }
        let last = probs.len() - 1;
        Ok((last, self.arm_ids[last].clone(), probs[last]))
    }

    /// Update the Exp3 log-weights given the pulled `arm_index` and observed reward $r_t \in [0, 1]$.
    /// Returns the importance-weighted unbiased estimate $\hat{r}_{i,t} = r_t / p_{i,t}$.
    pub fn update_reward(
        &mut self,
        arm_index: usize,
        reward: f64,
    ) -> Result<f64, AdversarialBanditError> {
        let k = self.arm_ids.len();
        if arm_index >= k {
            return Err(AdversarialBanditError::ArmIndexOutOfBounds {
                arm_index,
                num_arms: k,
            });
        }
        if !reward.is_finite() || !(0.0..=1.0).contains(&reward) {
            return Err(AdversarialBanditError::InvalidReward { reward });
        }

        let probs = self.sampling_probabilities();
        let p_i = probs[arm_index];
        // Theorem 33: unbiased importance-weighted reward estimator
        let estimated_reward = reward / p_i;

        self.log_weights[arm_index] += self.eta * estimated_reward;
        // Shift log-weights so max is 0.0 to preserve numerical precision indefinitely
        let max_lw = self
            .log_weights
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        for lw in &mut self.log_weights {
            *lw -= max_lw;
        }

        self.pulls[arm_index] += 1;
        self.cumulative_estimated_rewards[arm_index] += estimated_reward;
        self.rounds_elapsed += 1;
        self.cumulative_observed_reward += reward;

        Ok(estimated_reward)
    }

    /// Compute the theoretical minimax expected regret upper bound $2\sqrt{(e-1) T K \ln K}$ (Theorem 34).
    pub fn theoretical_regret_bound(&self, horizon_t: usize) -> f64 {
        let k_f = self.arm_ids.len() as f64;
        let t_f = horizon_t.max(1) as f64;
        let e_minus_1 = std::f64::consts::E - 1.0;
        2.0 * (e_minus_1 * t_f * k_f * k_f.ln()).sqrt()
    }

    /// Return the full status and regret certificate of the bandit.
    pub fn status(&self) -> Exp3BanditStatus {
        let norm_w = self.normalized_weights();
        let probs = self.sampling_probabilities();
        let arms: Vec<Exp3ArmState> = (0..self.arm_ids.len())
            .map(|i| Exp3ArmState {
                arm_index: i,
                arm_id: self.arm_ids[i].clone(),
                log_weight: self.log_weights[i],
                normalized_weight: norm_w[i],
                sampling_probability: probs[i],
                pulls: self.pulls[i],
                cumulative_estimated_reward: self.cumulative_estimated_rewards[i],
            })
            .collect();

        let max_est = self
            .cumulative_estimated_rewards
            .iter()
            .copied()
            .fold(0.0_f64, f64::max);

        Exp3BanditStatus {
            num_arms: self.arm_ids.len(),
            gamma: self.gamma,
            eta: self.eta,
            rounds_elapsed: self.rounds_elapsed,
            cumulative_observed_reward: self.cumulative_observed_reward,
            max_estimated_arm_reward: max_est,
            minimax_regret_upper_bound: self.theoretical_regret_bound(self.rounds_elapsed.max(1)),
            arms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp3_probabilities_sum_to_one_and_respect_exploration_floor() {
        let arms = vec![
            "provider_a".to_owned(),
            "provider_b".to_owned(),
            "provider_c".to_owned(),
        ];
        let mut bandit = Exp3AdversarialBandit::new(arms, 0.15).unwrap();
        bandit.update_reward(0, 1.0).unwrap();
        bandit.update_reward(0, 1.0).unwrap();

        let probs = bandit.sampling_probabilities();
        let sum: f64 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12);
        for &p in &probs {
            // Every arm has probability >= gamma / K = 0.15 / 3 = 0.05
            assert!(p >= 0.05 - 1e-12);
        }
    }

    #[test]
    fn exp3_sublinear_regret_against_adversarial_schedule() {
        let arms = vec![
            "fast_model".to_owned(),
            "accurate_model".to_owned(),
            "fallback_model".to_owned(),
        ];
        let horizon = 600usize;
        let mut bandit = Exp3AdversarialBandit::with_horizon(arms, horizon).unwrap();
        let mut best_arm_cum = [0.0_f64; 3];

        // Deterministic quasi-random Halton/golden-ratio sequence for reproducible simulation
        for t in 0..horizon {
            let u = ((t as f64 + 1.0) * 0.372_583_002).fract();
            let (chosen, _, _) = bandit.select_arm(u).unwrap();

            // Arm 1 is consistently superior (0.90) with periodic adversarial dips
            let rewards = [0.25, if t % 10 == 0 { 0.40 } else { 0.90 }, 0.35];
            for k in 0..3 {
                best_arm_cum[k] += rewards[k];
            }
            bandit.update_reward(chosen, rewards[chosen]).unwrap();
        }

        let status = bandit.status();
        let best_fixed = best_arm_cum.into_iter().fold(0.0_f64, f64::max);
        let realized_regret = best_fixed - status.cumulative_observed_reward;
        assert!(
            realized_regret <= status.minimax_regret_upper_bound,
            "realized regret {realized_regret} exceeded Theorem 34 bound {}",
            status.minimax_regret_upper_bound
        );
    }
}
