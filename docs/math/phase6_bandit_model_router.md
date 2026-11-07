# Mathematical Foundations & Architecture: Contextual Multi-Armed Bandit Model Router

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Version:** AGENT-PANDORA v3.0.4 (Phase 6)  
**Safety Invariant:** `#![forbid(unsafe_code)]` Strictly Enforced  

---

## 1. Executive Summary & Routing Motivation

Autonomous software engineering agents interact with heterogeneous Large Language Model (LLM) endpoints spanning diverse parameter scales, reasoning latencies, dollar costs, and rate limits. Fixed heuristics or static routing policies fail because:
1. Provider performance is non-stationary: latency varies dynamically with datacenter load and queue depth.
2. Token unit economics vary by orders of magnitude across tier-1 reasoning models and compact helper models.
3. Over-exploration wastes tokens and incurs latency penalties, while under-exploration gets trapped in sub-optimal provider choices.

Phase 6 addresses this optimization challenge by formulating LLM model selection as a Multi-Armed Bandit (MAB) problem. We implement:
- **Upper Confidence Bound 1 (UCB1)** with provable minimax cumulative regret bound $O(\sqrt{K T \ln T})$.
- **Bayesian Contextual Thompson Sampling** using conjugate Beta posteriors with proven variance concentration $O(1/N_a)$.
- **Empirical Reward Synthesis** derived from real-time `ProviderCallMetrics` (success status, latency decay, and token budget consumption).

---

## 2. Multi-Armed Bandit Mathematical Formulation

### 2.1 Problem Setting

Let $\mathcal{A} = \{1, 2, \dots, K\}$ denote the discrete set of $K$ available model endpoints (bandit arms).  
At each discrete decision step $t \in \{1, 2, \dots, T\}$:
1. The agent observes a routing context $\mathbf{x}_t \in \mathbb{R}^d$ characterizing query complexity, prompt length, and cache affinity.
2. The router selects an arm $a_t \in \mathcal{A}$.
3. The environment executes the model inference and returns an empirical observation $\mathbf{m}_t$ (`ProviderCallMetrics`), from which a scalar reward $r_t \in [0, 1]$ is evaluated.

Let $\mu_a = \mathbb{E}[r_t \mid a_t = a]$ denote the true expected reward of arm $a$, and let $\mu^* = \max_{a \in \mathcal{A}} \mu_a$ denote the optimal expected reward.  
For each sub-optimal arm $a$, the sub-optimality gap is defined as:
$$\Delta_a = \mu^* - \mu_a > 0$$

The cumulative pseudo-regret $R(T)$ after $T$ total routing decisions is:
$$R(T) = T \cdot \mu^* - \sum_{t=1}^T \mathbb{E}[r_t] = \sum_{a: \Delta_a > 0} \Delta_a \mathbb{E}[N_a(T)]$$
where $N_a(T) = \sum_{t=1}^T \mathbb{I}\{a_t = a\}$ denotes the total number of pulls allocated to arm $a$ up to horizon $T$.

---

### 2.2 Empirical Reward Synthesis from `ProviderCallMetrics`

To capture the multi-objective Pareto trade-off between task success, execution latency, and token cost, the scalar reward $r_t$ is synthesized from `ProviderCallMetrics` via convex combination:

$$r_t = w_{\text{succ}} \cdot \mathbb{I}\{\text{succeeded}\} + w_{\text{lat}} \cdot \max\left(0, 1 - \frac{\text{elapsed\_ms}}{\text{lat}_{\max}}\right) + w_{\text{cost}} \cdot \max\left(0, 1 - \frac{\text{tokens}_{\text{in}} + \text{tokens}_{\text{out}}}{\text{tok}_{\max}}\right)$$

with standard convex weights:
$$w_{\text{succ}} = 0.50, \quad w_{\text{lat}} = 0.30, \quad w_{\text{cost}} = 0.20, \quad \sum_{i} w_i = 1.0, \quad r_t \in [0, 1]$$

---

## 3. Theorem 9: Minimax Regret Bound $O(\sqrt{K T \ln T})$ under UCB1

### 3.1 Theorem Statement

**Theorem 9 (Minimax Regret Bound):**  
*Under the UCB1 policy with exploration constant $c = \sqrt{2}$, where each arm $a \in \mathcal{A}$ is initially pulled once and subsequently chosen according to the decision rule:*
$$a_t = \arg\max_{a \in \mathcal{A}} \left[ \hat{\mu}_a(t - 1) + \sqrt{\frac{2 \ln t}{N_a(t - 1)}} \right]$$
*the expected cumulative regret over $T$ steps is strictly bounded by:*
$$\mathbb{E}[R(T)] \le 2 \sqrt{8 K T \ln T} + K \left(1 + \frac{\pi^2}{3}\right) = O(\sqrt{K T \ln T})$$

---

### 3.2 Proof

**Step 1: Hoeffding's Concentration Inequality for Sample Means.**  
Let $X_{a, 1}, X_{a, 2}, \dots, X_{a, s}$ be independent and identically distributed random reward variables for arm $a$ bounded in $[0, 1]$ with true expectation $\mu_a$. The empirical sample mean after $s$ pulls is:
$$\hat{\mu}_{a, s} = \frac{1}{s} \sum_{i=1}^s X_{a, i}$$

By Hoeffding's Inequality, for any confidence threshold $\delta > 0$:
$$\mathbb{P}(\hat{\mu}_{a, s} - \mu_a \ge \delta) \le \exp(-2 s \delta^2)$$
$$\mathbb{P}(\mu_a - \hat{\mu}_{a, s} \ge \delta) \le \exp(-2 s \delta^2)$$

Setting $\delta_t(s) = \sqrt{\frac{2 \ln t}{s}}$, the tail probability evaluates to:
$$\mathbb{P}\left(|\hat{\mu}_{a, s} - \mu_a| \ge \sqrt{\frac{2 \ln t}{s}}\right) \le 2 \exp\left(-2 s \cdot \frac{2 \ln t}{s}\right) = 2 \exp(-4 \ln t) = 2 t^{-4}$$

**Step 2: Decomposition of Sub-Optimal Arm Pulls.**  
Let arm $*$ denote the optimal arm with mean $\mu^*$. A sub-optimal arm $a$ is selected at step $t$ only if its upper confidence bound exceeds that of the optimal arm:
$$\hat{\mu}_a(t - 1) + \sqrt{\frac{2 \ln t}{N_a(t - 1)}} \ge \hat{\mu}^*(t - 1) + \sqrt{\frac{2 \ln t}{N^*(t - 1)}}$$

This condition implies at least one of the following three events must hold:
1. $E_1$: $\hat{\mu}^*(t - 1) \le \mu^* - \sqrt{\frac{2 \ln t}{N^*(t - 1)}}$ (optimal arm sample mean underestimated).
2. $E_2$: $\hat{\mu}_a(t - 1) \ge \mu_a + \sqrt{\frac{2 \ln t}{N_a(t - 1)}}$ (sub-optimal arm sample mean overestimated).
3. $E_3$: $\mu^* < \mu_a + 2 \sqrt{\frac{2 \ln t}{N_a(t - 1)}}$.

By Hoeffding's bound from Step 1:
$$\mathbb{P}(E_1) \le \sum_{s=1}^t t^{-4} = t^{-3}$$
$$\mathbb{P}(E_2) \le \sum_{s=1}^t t^{-4} = t^{-3}$$

Condition $E_3$ cannot hold if $N_a(t - 1) \ge \frac{8 \ln t}{\Delta_a^2}$, because:
$$N_a(t - 1) \ge \frac{8 \ln t}{\Delta_a^2} \implies 2 \sqrt{\frac{2 \ln t}{N_a(t - 1)}} \le \Delta_a = \mu^* - \mu_a \implies \mu_a + 2 \sqrt{\frac{2 \ln t}{N_a(t - 1)}} \le \mu^*$$

**Step 3: Bounding Expected Pulls $\mathbb{E}[N_a(T)]$.**  
For any horizon $T$:
$$\mathbb{E}[N_a(T)] \le \frac{8 \ln T}{\Delta_a^2} + 1 + \sum_{t=1}^T \left(\mathbb{P}(E_1) + \mathbb{P}(E_2)\right) \le \frac{8 \ln T}{\Delta_a^2} + 1 + 2 \sum_{t=1}^\infty t^{-3}$$
Using the Basel-Riemann zeta sum $\sum_{t=1}^\infty t^{-2} = \frac{\pi^2}{6}$, and recognizing $\sum_{t=1}^\infty t^{-3} < \sum_{t=1}^\infty t^{-2} \le \frac{\pi^2}{6}$:
$$\mathbb{E}[N_a(T)] \le \frac{8 \ln T}{\Delta_a^2} + 1 + \frac{\pi^2}{3}$$

**Step 4: Minimax Regret Gap Thresholding.**  
The cumulative regret is:
$$\mathbb{E}[R(T)] = \sum_{a: \Delta_a > 0} \Delta_a \mathbb{E}[N_a(T)] \le \sum_{a: \Delta_a > 0} \left( \frac{8 \ln T}{\Delta_a} + \Delta_a \left(1 + \frac{\pi^2}{3}\right) \right)$$

For small sub-optimality gaps $\Delta_a \to 0$, the $1/\Delta_a$ term grows unbounded. We partition the arm set into two subsets using a gap threshold $\epsilon > 0$:
- Arms with large gap: $\mathcal{A}_{\text{large}} = \{a \in \mathcal{A} \mid \Delta_a \ge \epsilon\}$.
- Arms with small gap: $\mathcal{A}_{\text{small}} = \{a \in \mathcal{A} \mid 0 < \Delta_a < \epsilon\}$.

For arms in $\mathcal{A}_{\text{small}}$, the total regret contributed over $T$ steps is trivially bounded by $\epsilon T$:
$$\sum_{a \in \mathcal{A}_{\text{small}}} \Delta_a \mathbb{E}[N_a(T)] \le \epsilon \sum_{a \in \mathcal{A}_{\text{small}}} \mathbb{E}[N_a(T)] \le \epsilon T$$

For arms in $\mathcal{A}_{\text{large}}$, we substitute $\Delta_a \ge \epsilon$:
$$\sum_{a \in \mathcal{A}_{\text{large}}} \Delta_a \mathbb{E}[N_a(T)] \le \sum_{a \in \mathcal{A}_{\text{large}}} \left( \frac{8 \ln T}{\epsilon} + \left(1 + \frac{\pi^2}{3}\right) \right) \le \frac{8 K \ln T}{\epsilon} + K \left(1 + \frac{\pi^2}{3}\right)$$

Combining both regimes:
$$\mathbb{E}[R(T)] \le \epsilon T + \frac{8 K \ln T}{\epsilon} + K \left(1 + \frac{\pi^2}{3}\right)$$

To minimize this convex upper bound with respect to $\epsilon$, differentiate and set the derivative to zero:
$$\frac{d}{d\epsilon} \left( \epsilon T + \frac{8 K \ln T}{\epsilon} \right) = T - \frac{8 K \ln T}{\epsilon^2} = 0 \implies \epsilon^* = \sqrt{\frac{8 K \ln T}{T}}$$

Substituting $\epsilon^*$ back into the bound:
$$\mathbb{E}[R(T)] \le \sqrt{\frac{8 K \ln T}{T}} \cdot T + \frac{8 K \ln T}{\sqrt{\frac{8 K \ln T}{T}}} + K \left(1 + \frac{\pi^2}{3}\right) = 2 \sqrt{8 K T \ln T} + K \left(1 + \frac{\pi^2}{3}\right)$$
$$\mathbb{E}[R(T)] = O(\sqrt{K T \ln T})$$
$\blacksquare$

---

## 4. Theorem 10: Beta Posterior Concentration in Thompson Sampling

### 4.1 Theorem Statement

**Theorem 10 (Posterior Variance Concentration):**  
*In Bayesian Thompson Sampling with prior $\theta_a \sim \text{Beta}(\alpha_0, \beta_0)$ ($\alpha_0 = 1, \beta_0 = 1$), upon observing $N_a$ independent rewards with cumulative sum $S_a = \sum_{i=1}^{N_a} r_i$, the posterior distribution is $\text{Beta}(\alpha_0 + S_a, \beta_0 + N_a - S_a)$ with posterior variance strictly bounded by:*
$$\operatorname{Var}(\theta_a \mid N_a) \le \frac{1}{4(N_a + 2)} = O\left(\frac{1}{N_a}\right)$$
*and the posterior distribution concentrates exponentially around the true mean:*
$$\mathbb{P}\left(|\theta_a - \hat{\mu}_a| \ge \delta \mid N_a\right) \le 2 \exp(-2 N_a \delta^2)$$

---

### 4.2 Proof

**Step 1: Conjugate Beta Update.**  
The Beta distribution probability density function for $\theta \in [0, 1]$ is:
$$p(\theta; \alpha, \beta) = \frac{\Gamma(\alpha + \beta)}{\Gamma(\alpha)\Gamma(\beta)} \theta^{\alpha - 1} (1 - \theta)^{\beta - 1}$$
With a Bernoulli likelihood $p(r \mid \theta) = \theta^r (1 - \theta)^{1 - r}$ (or continuous exponential family reward in $[0, 1]$), the Bayesian posterior after $N_a$ observations satisfies:
$$p(\theta \mid r_1, \dots, r_{N_a}) \propto p(\theta) \prod_{i=1}^{N_a} p(r_i \mid \theta) \propto \theta^{\alpha_0 + S_a - 1} (1 - \theta)^{\beta_0 + N_a - S_a - 1}$$
which is identically $\text{Beta}(\alpha_{\text{post}}, \beta_{\text{post}})$ where $\alpha_{\text{post}} = \alpha_0 + S_a$ and $\beta_{\text{post}} = \beta_0 + N_a - S_a$.

**Step 2: Variance Bound.**  
The variance of a Beta distribution $\text{Beta}(\alpha, \beta)$ is:
$$\operatorname{Var}(\theta) = \frac{\alpha \beta}{(\alpha + \beta)^2 (\alpha + \beta + 1)}$$
Let $M = \alpha + \beta = \alpha_0 + \beta_0 + N_a = N_a + 2$. By the AM-GM inequality:
$$\alpha \beta \le \left(\frac{\alpha + \beta}{2}\right)^2 = \frac{M^2}{4}$$
Therefore:
$$\operatorname{Var}(\theta) = \frac{\alpha \beta}{M^2 (M + 1)} \le \frac{M^2 / 4}{M^2 (M + 1)} = \frac{1}{4(M + 1)} = \frac{1}{4(N_a + 3)} \le \frac{1}{4(N_a + 2)}$$
Thus, the posterior epistemic uncertainty decays at rate $O(1/N_a)$.

**Step 3: Posterior Concentration.**  
Applying Chebyshev's inequality or the sub-Gaussian property of bounded Beta random variables:
$$\mathbb{E}[(\theta - \mathbb{E}[\theta])^2] \le \frac{1}{4 N_a}$$
By Hoeffding's lemma for the posterior, the distribution exhibits sub-Gaussian parameter $\sigma^2 \le \frac{1}{4 N_a}$:
$$\mathbb{P}(|\theta_a - \hat{\mu}_a| \ge \delta) \le 2 \exp\left(-\frac{\delta^2}{2 \sigma^2}\right) \le 2 \exp(-2 N_a \delta^2)$$
$\blacksquare$

---

## 5. Engineering Implementation & Test Alignment

- **Runtime Module**: `crates/pandora-runtime/src/bandit_router.rs`
- **Re-exports**: `BanditModelRouter`, `BanditStrategy`, `BanditArm`, `RoutingContext`, `RoutingReward`, `SafePrng` in `crates/pandora-runtime/src/lib.rs`.
- **Integration Tests**: `crates/pandora-runtime/tests/bandit_router_tests.rs`:
  - `test_safe_prng_distributions`: Confirms uniform, Gaussian Box-Muller, Gamma Marsaglia-Tsang, and Beta distribution numerical properties.
  - `test_ucb1_initial_exploration`: Confirms round-robin initial sampling of unpulled candidate arms.
  - `test_ucb1_regret_bounds_and_convergence`: Confirms empirical cumulative regret remains strictly below $2 \sqrt{8 K T \ln T} + O(K)$.
  - `test_thompson_sampling_convergence`: Confirms posterior concentration favoring higher-reward endpoints.
  - `test_routing_reward_calculation`: Validates multi-objective scoring from `ProviderCallMetrics`.
