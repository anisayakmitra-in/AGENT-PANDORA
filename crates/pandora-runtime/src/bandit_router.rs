//! Phase 6: Contextual Thompson Sampling and UCB1 Multi-Armed Bandit Model Router.
//!
//! Provides mathematically optimal arm selection across heterogeneous LLM endpoints,
//! balancing exploration and exploitation with proven regret bounds $O(\sqrt{K T \ln T})$.

#![forbid(unsafe_code)]

use crate::executors::ProviderCallMetrics;
use std::collections::BTreeMap;

/// Safe, zero-dependency deterministic pseudo-random number generator based on Xorshift64*.
#[derive(Clone, Debug)]
pub struct SafePrng {
    state: u64,
}

impl SafePrng {
    /// Creates a new PRNG with a non-zero 64-bit seed.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0xdead_beef_cafe_babe
            } else {
                seed
            },
        }
    }

    /// Generates the next pseudo-random 64-bit integer using Xorshift64*.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Generates a uniform float in the half-open interval `[0.0, 1.0)`.
    pub fn sample_uniform_f64(&mut self) -> f64 {
        let val = self.next_u64() >> 11;
        (val as f64) / ((1u64 << 53) as f64)
    }

    /// Generates a standard normal random variable $\mathcal{N}(0, 1)$ via Box-Muller transform.
    pub fn sample_standard_normal(&mut self) -> f64 {
        let mut u1 = self.sample_uniform_f64();
        while u1 <= 1e-15 {
            u1 = self.sample_uniform_f64();
        }
        let u2 = self.sample_uniform_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// Generates a Gamma-distributed random variable $\text{Gamma}(\alpha, 1.0)$
    /// using Marsaglia and Tsang's rejection method (2000).
    pub fn sample_gamma(&mut self, shape: f64) -> f64 {
        if shape < 1.0 {
            // For shape < 1, use Johnk's identity: Gamma(a, 1) = Gamma(a + 1, 1) * U^(1/a)
            let g = self.sample_gamma(shape + 1.0);
            let mut u = self.sample_uniform_f64();
            while u <= 1e-15 {
                u = self.sample_uniform_f64();
            }
            return g * u.powf(1.0 / shape);
        }

        let d = shape - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();

        loop {
            let z = self.sample_standard_normal();
            let v = 1.0 + c * z;
            if v <= 0.0 {
                continue;
            }
            let v3 = v * v * v;
            let mut u = self.sample_uniform_f64();
            while u <= 1e-15 {
                u = self.sample_uniform_f64();
            }

            // Quick squeeze test
            if u < 1.0 - 0.0331 * z * z * z * z {
                return d * v3;
            }
            // Slower log test
            if u.ln() < 0.5 * z * z + d * (1.0 - v3 + v3.ln()) {
                return d * v3;
            }
        }
    }

    /// Generates a Beta-distributed random variable $\text{Beta}(\alpha, \beta)$
    /// using the standard relation $X / (X + Y)$ where $X \sim \text{Gamma}(\alpha, 1)$
    /// and $Y \sim \text{Gamma}(\beta, 1)$.
    pub fn sample_beta(&mut self, alpha: f64, beta: f64) -> f64 {
        let safe_alpha = if alpha <= 0.0 { 1.0 } else { alpha };
        let safe_beta = if beta <= 0.0 { 1.0 } else { beta };

        let x = self.sample_gamma(safe_alpha);
        let y = self.sample_gamma(safe_beta);
        let sum = x + y;

        if sum <= 1e-15 {
            0.5
        } else {
            (x / sum).clamp(0.0, 1.0)
        }
    }
}

/// Multi-Armed Bandit strategy selector.
#[derive(Clone, Debug, PartialEq)]
pub enum BanditStrategy {
    /// Upper Confidence Bound 1 with exploration coefficient $c$ (default $c = \sqrt{2} \approx 1.414$).
    Ucb1 { c: f64 },
    /// Bayesian Thompson Sampling with conjugate Beta posterior updates.
    ThompsonSampling,
    /// $\epsilon$-greedy exploration with probability $\epsilon \in [0, 1]$.
    EpsilonGreedy { epsilon: f64 },
}

impl Default for BanditStrategy {
    fn default() -> Self {
        Self::Ucb1 {
            c: std::f64::consts::SQRT_2,
        }
    }
}

/// Represents a single candidate LLM provider/model arm in the bandit router.
#[derive(Clone, Debug, PartialEq)]
pub struct BanditArm {
    id: String,
    model_id: String,
    provider_id: String,
    pull_count: u64,
    total_reward: f64,
    alpha: f64,
    beta: f64,
    feature_weights: Vec<f64>,
}

impl BanditArm {
    /// Creates a new bandit arm with uninformative Beta(1.0, 1.0) uniform prior.
    pub fn new(
        id: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            model_id: model_id.into(),
            provider_id: provider_id.into(),
            pull_count: 0,
            total_reward: 0.0,
            alpha: 1.0,
            beta: 1.0,
            feature_weights: Vec::new(),
        }
    }

    /// Initializes contextual linear weights for a given feature dimension.
    pub fn with_context_dim(mut self, dim: usize) -> Self {
        self.feature_weights = vec![0.0; dim];
        self
    }

    /// Returns the unique arm identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the associated model identifier.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Returns the associated provider identifier.
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// Returns the number of times this arm has been pulled.
    pub const fn pull_count(&self) -> u64 {
        self.pull_count
    }

    /// Returns the cumulative reward accrued by this arm.
    pub const fn total_reward(&self) -> f64 {
        self.total_reward
    }

    /// Returns the sample mean empirical reward $\hat{\mu}_a$.
    pub fn mean_reward(&self) -> f64 {
        if self.pull_count == 0 {
            0.0
        } else {
            self.total_reward / (self.pull_count as f64)
        }
    }

    /// Returns the current Beta distribution shape parameter $\alpha$.
    pub const fn alpha(&self) -> f64 {
        self.alpha
    }

    /// Returns the current Beta distribution shape parameter $\beta$.
    pub const fn beta(&self) -> f64 {
        self.beta
    }

    /// Returns the feature weights for contextual routing.
    pub fn feature_weights(&self) -> &[f64] {
        &self.feature_weights
    }

    /// Updates arm state with an observed scalar reward $r \in [0.0, 1.0]$.
    pub fn update(&mut self, reward: f64, context_features: Option<&[f64]>) {
        let clamped = reward.clamp(0.0, 1.0);
        self.pull_count += 1;
        self.total_reward += clamped;

        // Conjugate Beta posterior update
        self.alpha += clamped;
        self.beta += 1.0 - clamped;

        // Contextual gradient update if feature vectors are provided
        if let Some(features) = context_features.filter(|f| f.len() == self.feature_weights.len()) {
            let current_pred: f64 = features
                .iter()
                .zip(self.feature_weights.iter())
                .map(|(x, w)| x * w)
                .sum();
            let error = clamped - current_pred;
            let lr = 1.0 / (self.pull_count as f64).sqrt();
            for (w, &x) in self.feature_weights.iter_mut().zip(features.iter()) {
                *w += lr * error * x;
            }
        }
    }
}

/// Contextual attributes of the incoming inference request used for routing decisions.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutingContext {
    pub query_length: usize,
    pub task_complexity: f64,
    pub cache_affinity: f64,
    pub max_latency_ms: f64,
    pub features: Vec<f64>,
}

impl RoutingContext {
    /// Creates a new routing context with explicitly specified parameters.
    pub fn new(
        query_length: usize,
        task_complexity: f64,
        cache_affinity: f64,
        max_latency_ms: f64,
    ) -> Self {
        let norm_len = (query_length as f64 / 4096.0).min(1.0);
        let norm_comp = task_complexity.clamp(0.0, 1.0);
        let norm_cache = cache_affinity.clamp(0.0, 1.0);
        let norm_lat = (max_latency_ms / 10000.0).min(1.0);

        let features = vec![1.0, norm_len, norm_comp, norm_cache, norm_lat];

        Self {
            query_length,
            task_complexity: norm_comp,
            cache_affinity: norm_cache,
            max_latency_ms,
            features,
        }
    }

    /// Synthesizes a default routing context from an input prompt string.
    pub fn default_for_prompt(prompt: &str) -> Self {
        let query_length = prompt.len();
        // Heuristic complexity: presence of code syntax or high length
        let has_code = prompt.contains('{') || prompt.contains("fn ") || prompt.contains("def ");
        let task_complexity = if has_code { 0.8 } else { 0.3 };
        Self::new(query_length, task_complexity, 0.5, 5000.0)
    }

    /// Returns the normalized numerical feature vector for contextual bandits.
    pub fn to_features(&self) -> &[f64] {
        &self.features
    }
}

/// Reward synthesis and weights for multi-armed bandit routing.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutingReward {
    pub weight_success: f64,
    pub weight_latency: f64,
    pub weight_cost: f64,
}

impl Default for RoutingReward {
    fn default() -> Self {
        Self {
            weight_success: 0.50,
            weight_latency: 0.30,
            weight_cost: 0.20,
        }
    }
}

impl RoutingReward {
    /// Computes scalar reward $r \in [0.0, 1.0]$ from raw execution parameters.
    pub fn calculate(
        &self,
        succeeded: bool,
        elapsed_ms: f64,
        max_latency_ms: f64,
        total_tokens: f64,
        max_tokens: f64,
    ) -> f64 {
        let succ_score = if succeeded { 1.0 } else { 0.0 };
        let lat_ratio = if max_latency_ms > 0.0 {
            (1.0 - (elapsed_ms / max_latency_ms)).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let cost_ratio = if max_tokens > 0.0 {
            (1.0 - (total_tokens / max_tokens)).clamp(0.0, 1.0)
        } else {
            0.5
        };

        (self.weight_success * succ_score
            + self.weight_latency * lat_ratio
            + self.weight_cost * cost_ratio)
            .clamp(0.0, 1.0)
    }

    /// Computes reward from a `ProviderCallMetrics` instance.
    pub fn from_metrics(
        &self,
        metrics: &ProviderCallMetrics,
        max_latency_ms: f64,
        max_tokens: f64,
    ) -> f64 {
        let total_tokens = (metrics.input_tokens() + metrics.output_tokens()) as f64;
        self.calculate(
            metrics.succeeded(),
            metrics.elapsed_ms() as f64,
            max_latency_ms,
            total_tokens,
            max_tokens,
        )
    }
}

/// Multi-Armed Bandit model router for intelligent LLM provider selection.
#[derive(Clone, Debug)]
pub struct BanditModelRouter {
    arms: BTreeMap<String, BanditArm>,
    strategy: BanditStrategy,
    total_pulls: u64,
    prng: SafePrng,
    reward_calculator: RoutingReward,
}

impl BanditModelRouter {
    /// Creates a new router with the specified strategy and default PRNG seed.
    pub fn new(strategy: BanditStrategy) -> Self {
        Self::with_seed(strategy, 0x1234_5678_9abc_def0)
    }

    /// Creates a new router with a deterministic seed for reproducible exploration.
    pub fn with_seed(strategy: BanditStrategy, seed: u64) -> Self {
        Self {
            arms: BTreeMap::new(),
            strategy,
            total_pulls: 0,
            prng: SafePrng::new(seed),
            reward_calculator: RoutingReward::default(),
        }
    }

    /// Registers a candidate bandit arm.
    pub fn register_arm(&mut self, arm: BanditArm) {
        self.arms.insert(arm.id().to_string(), arm);
    }

    /// Returns a reference to a registered arm.
    pub fn get_arm(&self, id: &str) -> Option<&BanditArm> {
        self.arms.get(id)
    }

    /// Returns a mutable reference to a registered arm.
    pub fn get_arm_mut(&mut self, id: &str) -> Option<&mut BanditArm> {
        self.arms.get_mut(id)
    }

    /// Iterates over all registered arms.
    pub fn arms(&self) -> impl Iterator<Item = &BanditArm> {
        self.arms.values()
    }

    /// Returns the number of registered arms.
    pub fn arm_count(&self) -> usize {
        self.arms.len()
    }

    /// Returns the total number of arm pulls executed.
    pub const fn total_pulls(&self) -> u64 {
        self.total_pulls
    }

    /// Returns the current strategy.
    pub const fn strategy(&self) -> &BanditStrategy {
        &self.strategy
    }

    /// Updates the bandit exploration/exploitation strategy.
    pub fn set_strategy(&mut self, strategy: BanditStrategy) {
        self.strategy = strategy;
    }

    /// Selects the optimal arm for the given context based on the active strategy.
    pub fn select_arm(&mut self, context: &RoutingContext) -> Option<String> {
        if self.arms.is_empty() {
            return None;
        }

        // Under UCB1, ensure every arm is sampled at least once initially
        if matches!(self.strategy, BanditStrategy::Ucb1 { .. }) {
            for (id, arm) in &self.arms {
                if arm.pull_count() == 0 {
                    return Some(id.clone());
                }
            }
        }

        match self.strategy {
            BanditStrategy::Ucb1 { c } => self.select_ucb1(c),
            BanditStrategy::ThompsonSampling => self.select_thompson(context),
            BanditStrategy::EpsilonGreedy { epsilon } => self.select_epsilon_greedy(epsilon),
        }
    }

    fn select_ucb1(&self, c: f64) -> Option<String> {
        let t = (self.total_pulls.max(1)) as f64;
        let ln_t = t.ln();

        let mut best_arm = None;
        let mut best_score = f64::NEG_INFINITY;

        for (id, arm) in &self.arms {
            let n = arm.pull_count() as f64;
            let score = if n == 0.0 {
                f64::INFINITY
            } else {
                arm.mean_reward() + c * (2.0 * ln_t / n).sqrt()
            };

            if score > best_score {
                best_score = score;
                best_arm = Some(id.clone());
            }
        }

        best_arm
    }

    fn select_thompson(&mut self, context: &RoutingContext) -> Option<String> {
        let mut best_arm = None;
        let mut best_sample = f64::NEG_INFINITY;

        let features = context.to_features();

        for (id, arm) in &self.arms {
            let base_beta_sample = self.prng.sample_beta(arm.alpha(), arm.beta());
            let context_bonus: f64 = if arm.feature_weights().len() == features.len() {
                arm.feature_weights()
                    .iter()
                    .zip(features.iter())
                    .map(|(w, x)| w * x)
                    .sum()
            } else {
                0.0
            };

            let total_sample = (base_beta_sample + context_bonus).clamp(0.0, 1.0);

            if total_sample > best_sample {
                best_sample = total_sample;
                best_arm = Some(id.clone());
            }
        }

        best_arm
    }

    fn select_epsilon_greedy(&mut self, epsilon: f64) -> Option<String> {
        let arm_ids: Vec<String> = self.arms.keys().cloned().collect();
        if arm_ids.is_empty() {
            return None;
        }

        let roll = self.prng.sample_uniform_f64();
        if roll < epsilon {
            // Explore uniformly at random
            let idx = (self.prng.next_u64() as usize) % arm_ids.len();
            Some(arm_ids[idx].clone())
        } else {
            // Exploit highest empirical mean
            let mut best_arm = None;
            let mut best_mean = f64::NEG_INFINITY;
            for (id, arm) in &self.arms {
                let mean = arm.mean_reward();
                if mean > best_mean {
                    best_mean = mean;
                    best_arm = Some(id.clone());
                }
            }
            best_arm
        }
    }

    /// Records an observed scalar reward for the given arm.
    pub fn record_reward(&mut self, arm_id: &str, reward: f64) {
        if let Some(arm) = self.arms.get_mut(arm_id) {
            arm.update(reward, None);
            self.total_pulls += 1;
        }
    }

    /// Records an observed contextual reward with associated context feature vectors.
    pub fn record_contextual_reward(
        &mut self,
        arm_id: &str,
        reward: f64,
        context: &RoutingContext,
    ) {
        if let Some(arm) = self.arms.get_mut(arm_id) {
            arm.update(reward, Some(context.to_features()));
            self.total_pulls += 1;
        }
    }

    /// Synthesizes and records a reward derived directly from `ProviderCallMetrics`.
    pub fn record_metrics_reward(
        &mut self,
        arm_id: &str,
        metrics: &ProviderCallMetrics,
        max_latency_ms: f64,
        max_tokens: f64,
    ) -> f64 {
        let reward = self
            .reward_calculator
            .from_metrics(metrics, max_latency_ms, max_tokens);
        self.record_reward(arm_id, reward);
        reward
    }

    /// Calculates cumulative empirical regret relative to an optimal expected reward $\mu^*$:
    /// $$R(T) = T \cdot \mu^* - \sum_{t=1}^T r_t$$
    pub fn cumulative_regret(&self, optimal_mean: f64) -> f64 {
        let total_accumulated_reward: f64 = self.arms.values().map(|a| a.total_reward()).sum();
        let expected_optimal = optimal_mean * (self.total_pulls as f64);
        (expected_optimal - total_accumulated_reward).max(0.0)
    }

    /// Computes the theoretical minimax regret bound:
    /// $$\mathbb{E}[R(T)] \le 2 \sqrt{8 K T \ln T} + O(K) = O(\sqrt{K T \ln T})$$
    pub fn theoretical_minimax_regret(k: usize, t: u64) -> f64 {
        if k == 0 || t <= 1 {
            return 0.0;
        }
        let k_f = k as f64;
        let t_f = t as f64;
        let ln_t = t_f.ln().max(1.0);
        2.0 * (8.0 * k_f * t_f * ln_t).sqrt() + (k_f * (1.0 + std::f64::consts::PI.powi(2) / 3.0))
    }
}
