//! Phase 6: Contextual Thompson Sampling and UCB1 Multi-Armed Bandit Model Router.
//!
//! Provides mathematically optimal arm selection across heterogeneous LLM endpoints,
//! balancing exploration and exploitation with proven regret bounds $O(\sqrt{K T \ln T})$.

#![forbid(unsafe_code)]

use crate::executors::ProviderCallMetrics;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

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

    /// Returns the 7-dimensional normalized feature vector matching LinTS router dimensions.
    pub fn to_7d_features(&self) -> [f64; TASK_COMPLEXITY_DIM] {
        let norm_len = (self.query_length as f64 / 4096.0).min(1.0);
        let norm_comp = self.task_complexity.clamp(0.0, 1.0);
        let norm_cache = self.cache_affinity.clamp(0.0, 1.0);
        let norm_lat = (self.max_latency_ms / 10000.0).min(1.0);
        [
            1.0,
            norm_len,
            norm_comp,
            norm_cache,
            norm_lat,
            (norm_len * norm_comp).min(1.0),
            (norm_comp * (1.0 - norm_lat)).clamp(0.0, 1.0),
        ]
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
    lints_states: Vec<LinTsArmState>,
    lints_v_scale: f64,
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
            lints_states: Vec::new(),
            lints_v_scale: 0.2,
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

    /// Creates a contextual LinTS bandit router with a given number of candidate arms and feature dimension.
    pub fn new_contextual_lints(
        num_arms: usize,
        dim: usize,
        lambda: f64,
        v_scale: f64,
        seed: u64,
    ) -> Self {
        let mut router = Self::with_seed(BanditStrategy::ThompsonSampling, seed);
        for a in 0..num_arms {
            let arm_id = format!("arm_{a}");
            router.register_arm(BanditArm::new(&arm_id, format!("model_{a}"), "local"));
            router.lints_states.push(LinTsArmState::new(dim, lambda));
        }
        router.lints_v_scale = v_scale;
        router
    }

    /// Selects an arm index via LinTS multivariate Gaussian posterior sampling.
    pub fn select_arm_lints(&mut self, context: &[f64]) -> usize {
        if self.lints_states.is_empty() {
            return 0;
        }
        let mut best_arm = 0;
        let mut best_score = f64::NEG_INFINITY;
        for (idx, state) in self.lints_states.iter().enumerate() {
            let sample_theta = state
                .sample_theta(&mut self.prng, self.lints_v_scale)
                .unwrap_or_else(|_| vec![0.0; state.dim]);
            let score: f64 = context
                .iter()
                .zip(sample_theta.iter())
                .map(|(x, t)| x * t)
                .sum();
            if score > best_score {
                best_score = score;
                best_arm = idx;
            }
        }
        best_arm
    }

    /// Updates the LinTS state for the specified arm index with the observed scalar reward.
    pub fn update_lints(&mut self, arm_idx: usize, context: &[f64], reward: f64) {
        if let Some(state) = self.lints_states.get_mut(arm_idx) {
            state.update(context, reward);
            self.total_pulls += 1;
        }
        let arm_id = format!("arm_{arm_idx}");
        if let Some(arm) = self.arms.get_mut(&arm_id) {
            arm.update(reward, None);
        }
    }
}

// ============================================================================
// CONTEXTUAL THOMPSON SAMPLING (LinTS) & SPECULATIVE DRAFTING ROLE ALLOCATION
// ============================================================================

pub const TASK_COMPLEXITY_DIM: usize = 7;

/// Error type for bandit matrix operations and routing errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BanditError {
    MatrixNotPositiveDefinite,
    DimensionMismatch { expected: usize, found: usize },
    ArmNotFound(String),
    NoArmAvailable(String),
}

impl std::fmt::Display for BanditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MatrixNotPositiveDefinite => write!(f, "matrix is not positive definite"),
            Self::DimensionMismatch { expected, found } => {
                write!(f, "dimension mismatch: expected {expected}, found {found}")
            }
            Self::ArmNotFound(id) => write!(f, "arm not found: {id}"),
            Self::NoArmAvailable(role) => write!(f, "no arm available for role: {role}"),
        }
    }
}

impl std::error::Error for BanditError {}

/// Normalized 7-dimensional task complexity feature vector for LinTS routing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskComplexityFeatures {
    pub line_count: usize,
    pub max_ast_depth: usize,
    pub language_complexity: f64,
    pub dependency_count: usize,
    pub error_count: usize,
    pub instruction_length: usize,
}

impl TaskComplexityFeatures {
    pub fn new(
        line_count: usize,
        max_ast_depth: usize,
        language_complexity: f64,
        dependency_count: usize,
        error_count: usize,
        instruction_length: usize,
    ) -> Self {
        Self {
            line_count,
            max_ast_depth,
            language_complexity: language_complexity.clamp(0.0, 1.0),
            dependency_count,
            error_count,
            instruction_length,
        }
    }

    /// Automatically extracts complexity features from target source code and user instruction.
    pub fn extract(target_path: &Path, source: &str, instruction: &str) -> Self {
        let line_count = source.lines().count();

        // Approximate AST depth via maximum brace/indentation nesting
        let mut depth = 0usize;
        let mut max_depth = 0usize;
        for ch in source.chars() {
            match ch {
                '{' | '(' | '[' => {
                    depth = depth.saturating_add(1);
                    if depth > max_depth {
                        max_depth = depth;
                    }
                }
                '}' | ')' | ']' => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }

        let ext = target_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let language_complexity = match ext {
            "rs" | "c" | "cpp" => 0.90,
            "ts" | "tsx" | "go" => 0.65,
            "py" | "js" => 0.45,
            "json" | "yaml" | "toml" | "md" => 0.15,
            _ => 0.50,
        };

        let dependency_count = source
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                trimmed.starts_with("use ")
                    || trimmed.starts_with("import ")
                    || trimmed.starts_with("#include")
                    || trimmed.starts_with("require(")
            })
            .count();

        let error_count = if instruction.contains("error:") || instruction.contains("failed") {
            1
        } else {
            0
        };
        let instruction_length = instruction.len();

        Self::new(
            line_count,
            max_depth,
            language_complexity,
            dependency_count,
            error_count,
            instruction_length,
        )
    }

    /// Converts normalized features into a fixed-size array in R^7 with bias x_0 = 1.0.
    pub fn to_feature_vector(&self) -> [f64; TASK_COMPLEXITY_DIM] {
        [
            1.0,                                                // x0: Intercept / Bias
            (self.line_count as f64 / 1000.0).min(1.0),         // x1: Line count
            (self.max_ast_depth as f64 / 16.0).min(1.0),        // x2: AST depth
            self.language_complexity,                           // x3: Language complexity
            (self.dependency_count as f64 / 30.0).min(1.0),     // x4: Dependencies
            (self.error_count as f64 / 10.0).min(1.0),          // x5: Errors
            (self.instruction_length as f64 / 1024.0).min(1.0), // x6: Instruction length
        ]
    }

    /// Converts normalized features into a vector in R^7.
    pub fn to_vector(&self) -> Vec<f64> {
        self.to_feature_vector().to_vec()
    }
}

/// Cognitive role capability of a bandit model arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoleCapability {
    Drafter,
    Critic,
    PureFrontier,
}

/// Linear Thompson Sampling state for a single arm using Cholesky factorized Gram matrix.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinTsArmState {
    pub dim: usize,
    pub lambda: f64,
    pub v_matrix: Vec<f64>, // Row-major d x d precision matrix
    pub b_vector: Vec<f64>, // d-dimensional response vector
    pub pulls: u64,
    pub total_reward: f64,
}

impl LinTsArmState {
    pub fn new(dim: usize, lambda: f64) -> Self {
        let safe_lambda = lambda.max(1e-4);
        let mut v_matrix = vec![0.0; dim * dim];
        for i in 0..dim {
            v_matrix[i * dim + i] = safe_lambda;
        }
        Self {
            dim,
            lambda: safe_lambda,
            v_matrix,
            b_vector: vec![0.0; dim],
            pulls: 0,
            total_reward: 0.0,
        }
    }

    /// Updates precision matrix V <- V + x x^T and response vector b <- b + r x.
    pub fn update(&mut self, context: &[f64], reward: f64) {
        if context.len() != self.dim {
            return;
        }
        let r = reward.clamp(0.0, 1.0);
        self.pulls += 1;
        self.total_reward += r;

        // V_a += x * x^T
        for i in 0..self.dim {
            for j in 0..self.dim {
                self.v_matrix[i * self.dim + j] += context[i] * context[j];
            }
            self.b_vector[i] += r * context[i];
        }
    }

    /// Computes Cholesky factor L (lower triangular) such that V = L L^T.
    pub fn cholesky(&self) -> Result<Vec<f64>, BanditError> {
        let d = self.dim;
        let mut l = vec![0.0; d * d];

        for j in 0..d {
            let mut diag_sum = 0.0;
            for k in 0..j {
                let l_jk = l[j * d + k];
                diag_sum += l_jk * l_jk;
            }
            let val = self.v_matrix[j * d + j] - diag_sum;
            if val <= 1e-12 {
                return Err(BanditError::MatrixNotPositiveDefinite);
            }
            let l_jj = val.sqrt();
            l[j * d + j] = l_jj;

            for i in (j + 1)..d {
                let mut off_sum = 0.0;
                for k in 0..j {
                    off_sum += l[i * d + k] * l[j * d + k];
                }
                l[i * d + j] = (self.v_matrix[i * d + j] - off_sum) / l_jj;
            }
        }
        Ok(l)
    }

    /// Forward solve L y = b for lower triangular L.
    pub fn solve_l(l: &[f64], d: usize, b: &[f64]) -> Vec<f64> {
        let mut y = vec![0.0; d];
        for i in 0..d {
            let mut sum = 0.0;
            for k in 0..i {
                sum += l[i * d + k] * y[k];
            }
            y[i] = (b[i] - sum) / l[i * d + i];
        }
        y
    }

    /// Back solve L^T w = y for upper triangular L^T.
    pub fn solve_lt(l: &[f64], d: usize, y: &[f64]) -> Vec<f64> {
        let mut w = vec![0.0; d];
        for i in (0..d).rev() {
            let mut sum = 0.0;
            for k in (i + 1)..d {
                sum += l[k * d + i] * w[k];
            }
            w[i] = (y[i] - sum) / l[i * d + i];
        }
        w
    }

    /// Computes the MAP ridge regression parameter \hat{\theta} = V^{-1} b.
    pub fn mean_theta(&self) -> Result<Vec<f64>, BanditError> {
        let l = self.cholesky()?;
        let y = Self::solve_l(&l, self.dim, &self.b_vector);
        Ok(Self::solve_lt(&l, self.dim, &y))
    }

    /// Draws Thompson sample \tilde{\theta} ~ N(\hat{\theta}, v^2 V^{-1}).
    pub fn sample_theta(&self, prng: &mut SafePrng, v_scale: f64) -> Result<Vec<f64>, BanditError> {
        let l = self.cholesky()?;
        let y = Self::solve_l(&l, self.dim, &self.b_vector);
        let theta_hat = Self::solve_lt(&l, self.dim, &y);

        let mut z = vec![0.0; self.dim];
        for val in &mut z {
            *val = prng.sample_standard_normal();
        }
        let w = Self::solve_lt(&l, self.dim, &z);

        let mut sample = vec![0.0; self.dim];
        for i in 0..self.dim {
            sample[i] = theta_hat[i] + v_scale * w[i];
        }
        Ok(sample)
    }

    /// Evaluates predicted expected reward x^T \hat{\theta}.
    pub fn predict_reward(&self, context: &[f64]) -> Result<f64, BanditError> {
        let theta = self.mean_theta()?;
        let dot: f64 = context.iter().zip(theta.iter()).map(|(x, t)| x * t).sum();
        Ok(dot.clamp(0.0, 1.0))
    }

    /// Computes posterior epistemic variance \sigma^2(x) = v^2 x^T V^{-1} x.
    pub fn posterior_variance(&self, context: &[f64], v_scale: f64) -> Result<f64, BanditError> {
        let l = self.cholesky()?;
        let u = Self::solve_l(&l, self.dim, context);
        let norm_sq: f64 = u.iter().map(|val| val * val).sum();
        Ok(v_scale * v_scale * norm_sq)
    }
}

/// A candidate model arm with assigned role capability.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BanditRoleArm {
    pub arm_id: String,
    pub model_id: String,
    pub provider_id: String,
    pub role: RoleCapability,
    pub state: LinTsArmState,
}

impl BanditRoleArm {
    pub fn new(
        arm_id: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        role: RoleCapability,
        dim: usize,
        lambda: f64,
    ) -> Self {
        Self {
            arm_id: arm_id.into(),
            model_id: model_id.into(),
            provider_id: provider_id.into(),
            role,
            state: LinTsArmState::new(dim, lambda),
        }
    }
}

/// Final routing decision produced by the contextual Thompson sampling engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SpeculativeRoutingPlan {
    PureFrontier {
        frontier_model: String,
        frontier_provider: String,
        predicted_reward: f64,
        reason: String,
    },
    SpeculativeDrafting {
        drafter_model: String,
        drafter_provider: String,
        critic_model: String,
        critic_provider: String,
        predicted_drafter_reward: f64,
        predicted_critic_reward: f64,
        predicted_speedup: f64,
        acceptance_threshold: f64,
    },
}

/// Execution outcome used to synthesize multi-objective reward updates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftOutcome {
    pub accepted: bool,
    pub has_residual: bool,
    pub drafter_elapsed_ms: f64,
    pub critic_elapsed_ms: f64,
    pub frontier_fallback_elapsed_ms: Option<f64>,
    pub baseline_frontier_elapsed_ms: f64,
    pub tokens_used: usize,
    pub baseline_tokens: usize,
}

impl DraftOutcome {
    /// Multi-objective reward synthesis:
    /// r = 0.50 * r_acc + 0.30 * r_speed + 0.20 * r_cost
    pub fn calculate_reward(&self) -> f64 {
        let r_acc = if self.accepted {
            if self.has_residual { 0.80 } else { 1.00 }
        } else {
            0.0
        };

        let total_time = self.drafter_elapsed_ms
            + self.critic_elapsed_ms
            + self.frontier_fallback_elapsed_ms.unwrap_or(0.0);
        let r_speed = if total_time > 0.0 && self.baseline_frontier_elapsed_ms > 0.0 {
            (self.baseline_frontier_elapsed_ms / total_time * 0.5).clamp(0.0, 1.0)
        } else {
            0.5
        };

        let r_cost = if self.baseline_tokens > 0 {
            (1.0 - (self.tokens_used as f64 / self.baseline_tokens as f64)).clamp(0.0, 1.0)
        } else {
            0.5
        };

        (0.50 * r_acc + 0.30 * r_speed + 0.20 * r_cost).clamp(0.0, 1.0)
    }
}

/// Contextual Thompson Sampling Router coordinating drafter, critic, and pure frontier role allocation.
#[derive(Clone, Debug)]
pub struct ContextualThompsonSamplingRouter {
    arms: BTreeMap<String, BanditRoleArm>,
    dim: usize,
    lambda: f64,
    v_scale: f64,
    total_rounds: u64,
    prng: SafePrng,
    drafter_latency_baseline_ms: f64,
    critic_latency_baseline_ms: f64,
    frontier_latency_baseline_ms: f64,
}

impl ContextualThompsonSamplingRouter {
    pub fn new(dim: usize, lambda: f64, v_scale: f64) -> Self {
        Self::with_seed(dim, lambda, v_scale, 0x1337_c0de_cafe_babe)
    }

    pub fn with_seed(dim: usize, lambda: f64, v_scale: f64, seed: u64) -> Self {
        Self {
            arms: BTreeMap::new(),
            dim,
            lambda,
            v_scale,
            total_rounds: 0,
            prng: SafePrng::new(seed),
            drafter_latency_baseline_ms: 200.0,
            critic_latency_baseline_ms: 700.0,
            frontier_latency_baseline_ms: 2200.0,
        }
    }

    pub fn register_arm(&mut self, arm: BanditRoleArm) {
        self.arms.insert(arm.arm_id.clone(), arm);
    }

    pub fn get_arm(&self, id: &str) -> Option<&BanditRoleArm> {
        self.arms.get(id)
    }

    pub fn get_arm_mut(&mut self, id: &str) -> Option<&mut BanditRoleArm> {
        self.arms.get_mut(id)
    }

    pub fn arms(&self) -> impl Iterator<Item = &BanditRoleArm> {
        self.arms.values()
    }

    pub fn arm_count(&self) -> usize {
        self.arms.len()
    }

    pub fn total_rounds(&self) -> u64 {
        self.total_rounds
    }

    pub const fn dim(&self) -> usize {
        self.dim
    }

    pub const fn lambda(&self) -> f64 {
        self.lambda
    }

    pub const fn v_scale(&self) -> f64 {
        self.v_scale
    }

    /// Selects optimal arm for a specified role capability via sampled Thompson payoff.
    pub fn select_arm_for_role(
        &mut self,
        role: RoleCapability,
        context: &[f64],
    ) -> Option<(String, f64)> {
        let mut best_arm = None;
        let mut best_sample = f64::NEG_INFINITY;

        for (id, arm) in &self.arms {
            if arm.role != role {
                continue;
            }
            let sample_theta = arm.state.sample_theta(&mut self.prng, self.v_scale).ok()?;
            let payoff: f64 = context
                .iter()
                .zip(sample_theta.iter())
                .map(|(x, t)| x * t)
                .sum();

            if payoff > best_sample {
                best_sample = payoff;
                best_arm = Some(id.clone());
            }
        }

        best_arm.map(|id| (id, best_sample))
    }

    /// Evaluates task complexity, samples candidates, and returns optimal routing plan.
    pub fn select_routing_plan(
        &mut self,
        features: &TaskComplexityFeatures,
    ) -> SpeculativeRoutingPlan {
        let x = features.to_vector();

        let td = self.drafter_latency_baseline_ms;
        let tc = self.critic_latency_baseline_ms;
        let tf = self.frontier_latency_baseline_ms;
        let break_even_threshold = (td + tc) / tf; // ~0.409
        let safety_margin = 0.091;
        let dynamic_threshold = break_even_threshold + safety_margin; // 0.50

        // 1. Sample best Drafter
        let best_drafter = self.select_arm_for_role(RoleCapability::Drafter, &x);
        // 2. Sample best Critic
        let best_critic = self.select_arm_for_role(RoleCapability::Critic, &x);
        // 3. Sample best Pure Frontier
        let best_frontier = self.select_arm_for_role(RoleCapability::PureFrontier, &x);

        if let (Some((draft_id, _draft_score)), Some((critic_id, _critic_score))) =
            (best_drafter, best_critic)
        {
            let drafter_opt = self.arms.get(&draft_id);
            let critic_opt = self.arms.get(&critic_id);
            if let (Some(drafter_arm), Some(critic_arm)) = (drafter_opt, critic_opt) {
                let pred_draft_reward = drafter_arm.state.predict_reward(&x).unwrap_or(0.5);

                // Compute expected speedup factor S(x) = t_f / (t_d + t_c + (1 - alpha) * t_f)
                let exp_spec_latency = td + tc + (1.0 - pred_draft_reward) * tf;
                let speedup = tf / exp_spec_latency.max(10.0);

                if pred_draft_reward >= dynamic_threshold && speedup > 1.05 {
                    let pred_critic_reward = critic_arm.state.predict_reward(&x).unwrap_or(0.5);

                    return SpeculativeRoutingPlan::SpeculativeDrafting {
                        drafter_model: drafter_arm.model_id.clone(),
                        drafter_provider: drafter_arm.provider_id.clone(),
                        critic_model: critic_arm.model_id.clone(),
                        critic_provider: critic_arm.provider_id.clone(),
                        predicted_drafter_reward: pred_draft_reward,
                        predicted_critic_reward: pred_critic_reward,
                        predicted_speedup: speedup,
                        acceptance_threshold: dynamic_threshold,
                    };
                }
            }
        }

        // Fall back to Pure Frontier
        let (front_id, front_score) =
            best_frontier.unwrap_or_else(|| ("arm_frontier_default".to_string(), 0.70));
        let front_arm = self.arms.get(&front_id);

        SpeculativeRoutingPlan::PureFrontier {
            frontier_model: front_arm
                .map(|a| a.model_id.clone())
                .unwrap_or_else(|| "claude-3-7-sonnet".to_string()),
            frontier_provider: front_arm
                .map(|a| a.provider_id.clone())
                .unwrap_or_else(|| "anthropic".to_string()),
            predicted_reward: front_score,
            reason: format!(
                "Task complexity ({:.2}) exceeded drafter threshold ({:.2}) or speedup <= 1.05",
                features.language_complexity, dynamic_threshold
            ),
        }
    }

    /// Records outcome and updates posterior precision and response vectors.
    pub fn record_outcome(&mut self, arm_id: &str, context: &[f64], reward: f64) {
        if let Some(arm) = self.arms.get_mut(arm_id) {
            arm.state.update(context, reward);
            self.total_rounds += 1;
        }
    }

    /// Calculates cumulative empirical regret relative to an optimal expected reward \mu^*:
    /// R(T) = T * \mu^* - \sum r_t
    pub fn cumulative_regret(&self, optimal_mean: f64) -> f64 {
        let total_accumulated_reward: f64 = self.arms.values().map(|a| a.state.total_reward).sum();
        let expected_optimal = optimal_mean * (self.total_rounds as f64);
        (expected_optimal - total_accumulated_reward).max(0.0)
    }

    /// Computes the theoretical regret bound: R(T) < 2.5 * d * sqrt(T * ln(T))
    pub fn theoretical_regret_bound(d: usize, rounds: u64) -> f64 {
        if rounds <= 1 {
            return 0.0;
        }
        2.5 * (d as f64) * ((rounds as f64) * (rounds as f64).ln()).sqrt()
    }
}

/// Offline Tropical Polyhedral AST complexity prior for model routing.
///
/// Loads `.pandora/model_tpan.pnn` (if present) and evaluates 16-dim AST/task
/// feature vectors with zero floating-point multiplications to produce a
/// calibrated complexity score in `[0.0, 1.0]`. Falls back to `0.5` when no
/// `.pnn` artifact exists on disk.
#[derive(Debug, Clone)]
pub struct TropicalComplexityPrior {
    network: Option<crate::tropical_nn::TropicalNetwork>,
}

impl Default for TropicalComplexityPrior {
    fn default() -> Self {
        Self::from_default_path()
    }
}

impl TropicalComplexityPrior {
    /// Loads a `.pnn` model from `.pandora/model_tpan.pnn` if present.
    pub fn from_default_path() -> Self {
        Self::from_pnn_path(Path::new(".pandora/model_tpan.pnn"))
    }

    /// Loads a `.pnn` model from an explicit path; falls back gracefully if absent.
    pub fn from_pnn_path(path: &Path) -> Self {
        let network = crate::tropical_nn::TropicalNetwork::load_from_file(path).ok();
        Self { network }
    }

    /// Constructs from an in-memory `TropicalNetwork`.
    pub fn with_network(network: Option<crate::tropical_nn::TropicalNetwork>) -> Self {
        Self { network }
    }

    /// Evaluates a 16-dimensional AST feature vector and squashes the tropical
    /// output into `[0.0, 1.0]` via logistic sigmoid.
    pub fn score_complexity(&self, ast_features: &[f32; 16]) -> f64 {
        let Some(ref net) = self.network else {
            return 0.5;
        };
        match net.forward(ast_features) {
            Ok(out) => {
                let raw = out.first().copied().unwrap_or(0.0) as f64;
                1.0 / (1.0 + (-raw).exp())
            }
            Err(_) => 0.5,
        }
    }

    /// Extracts structural AST features directly from `source` and scores its complexity in `[0.0, 1.0]`.
    pub fn score_source(&self, source: &str) -> f64 {
        let features = crate::tropical_nn::extract_ast_features(source);
        self.score_complexity(&features)
    }
}

#[cfg(test)]
mod tropical_prior_tests {
    use super::TropicalComplexityPrior;
    use crate::tropical_nn::{TropicalLayer, TropicalNetwork};
    use std::path::Path;

    #[test]
    fn test_tropical_complexity_prior_fallback_and_scoring() {
        let missing = TropicalComplexityPrior::from_pnn_path(Path::new("nonexistent_tpan.pnn"));
        assert_eq!(missing.score_complexity(&[0.0; 16]), 0.5);

        let layer = TropicalLayer::new(16, 1, vec![0.2; 16], None).unwrap();
        let net = TropicalNetwork::new("tpan_prior_test", "ast", vec![layer]).unwrap();
        let prior = TropicalComplexityPrior::with_network(Some(net));
        let score = prior.score_complexity(&[1.0; 16]);
        assert!(score > 0.5 && score <= 1.0);
    }
}
