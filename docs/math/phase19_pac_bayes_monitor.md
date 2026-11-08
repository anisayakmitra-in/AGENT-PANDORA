# Phase 19: PAC-Bayesian Generalization & Prompt/Policy Risk Certificates

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Runtime Module:** `crates/pandora-runtime/src/pac_bayes_monitor.rs`  
**CLI Surface:** `pandora runtime pac-bayes`  
**Safety Invariant:** `#![forbid(unsafe_code)]` Strictly Enforced  

---

## 1. Problem Formulation & Theoretical Foundations

In high-assurance autonomous agent operations, prompts, routing heuristics, and tool-dispatch policies are continuously optimized or adapted against execution traces. To prevent catastrophic overfitting and silent regressions, the runtime must certify generalization performance via finite-sample, distribution-free risk certificates.

Let $\mathcal{H}$ denote a space of candidate prompt configurations or routing policies. Let $\mathcal{Z}$ denote the domain of evaluation tasks / environment queries. The environment generates i.i.d. evaluation samples $S = (Z_1, \dots, Z_n) \sim \mathcal{D}^n$ according to an unknown data-generating distribution $\mathcal{D}$.

### 1.1 Loss Function & Risks
Let $\ell: \mathcal{H} \times \mathcal{Z} \to [0, 1]$ be a bounded loss metric (e.g. tool execution failure rate, test assertion violation rate, or normalized token budget overflow).
For any single policy $h \in \mathcal{H}$:
- **Empirical Risk:** $\hat{L}_n(h) \triangleq \frac{1}{n} \sum_{i=1}^n \ell(h, Z_i)$.
- **True Generalization Risk:** $L(h) \triangleq \mathbb{E}_{Z \sim \mathcal{D}}[\ell(h, Z)]$.

Let $P$ be a reference **prior probability measure** on $\mathcal{H}$, specified *prior* to observing sample $S$ (e.g. uniform prior over prompt variations, or centered at the production baseline).
Let $Q$ be an adapted **posterior probability measure** on $\mathcal{H}$, which may depend arbitrarily on the observed evaluation sample $S$.
The expected empirical and true risks under policy distribution $Q$ are:
$$\hat{L}_n(Q) \triangleq \mathbb{E}_{h \sim Q}[\hat{L}_n(h)], \qquad L(Q) \triangleq \mathbb{E}_{h \sim Q}[L(h)].$$

### 1.2 Binary Relative Entropy (Kullback-Leibler Divergence)
For $q, p \in (0, 1)$, the binary relative entropy function is defined as:
$$\mathrm{kl}(q \,\|\, p) \triangleq q \ln\left(\frac{q}{p}\right) + (1 - q)\ln\left(\frac{1 - q}{1 - p}\right),$$
extended continuously to the boundary with $\mathrm{kl}(0 \,\|\, 0) = \mathrm{kl}(1 \,\|\, 1) = 0$, $\mathrm{kl}(0 \,\|\, p) = \ln\frac{1}{1 - p}$, and $\mathrm{kl}(q \,\|\, p) = +\infty$ if $q > 0$ and $p = 0$.

---

## 2. Theorem 37: McAllester / Maurer Empirical-Bernstein PAC-Bayes KL-Divergence Risk Bound

### Statement
Let $P$ be any fixed prior distribution on $\mathcal{H}$ chosen independently of the training sample $S \sim \mathcal{D}^n$. For any user-specified failure probability $\delta \in (0, 1)$:
With probability at least $1 - \delta$ over the random sample draw $S \sim \mathcal{D}^n$, the following bound holds **simultaneously for all** posterior distributions $Q$ on $\mathcal{H}$ (including all data-dependent, fine-tuned, or actively learned distributions):
$$\mathrm{kl}\bigl(\hat{L}_n(Q) \,\|\, L(Q)\bigr) \le \frac{D_{\mathrm{KL}}(Q \,\|\, P) + \ln\left(\frac{2\sqrt{n}}{\delta}\right)}{n},$$
where $D_{\mathrm{KL}}(Q \,\|\, P) \triangleq \mathbb{E}_{h \sim Q}\left[\ln \frac{dQ}{dP}(h)\right]$ is the Kullback-Leibler divergence between posterior $Q$ and prior $P$.

Furthermore:
1. **Certified Upper Risk Bound:** Define the PAC-Bayes complexity budget:
   $$B(Q, \delta, n) \triangleq \frac{D_{\mathrm{KL}}(Q \,\|\, P) + \ln\left(\frac{2\sqrt{n}}{\delta}\right)}{n}.$$
   The guaranteed finite-sample upper risk certificate $L_{\max}(Q) \in [\hat{L}_n(Q), 1]$ is the unique solution $p \in [\hat{L}_n(Q), 1)$ to:
   $$\mathrm{kl}\bigl(\hat{L}_n(Q) \,\|\, p\bigr) = B(Q, \delta, n).$$
   It satisfies $\mathbb{P}\left(L(Q) \le L_{\max}(Q)\right) \ge 1 - \delta$.
2. **Pinsker Relaxation:** Applying Pinsker's inequality $\mathrm{kl}(q \,\|\, p) \ge 2(p - q)^2$ yields the classical sub-Gaussian bound:
   $$L(Q) \le \hat{L}_n(Q) + \sqrt{\frac{B(Q, \delta, n)}{2}}.$$
   The inverted $\mathrm{kl}$ root $L_{\max}(Q)$ is strictly tighter than the Pinsker bound, particularly when empirical risk $\hat{L}_n(Q)$ is small (approaching fast $O(1/n)$ convergence).

### Mathematical Proof

#### Step 1: Donsker-Varadhan Variational Representation of Relative Entropy
Let $(\mathcal{X}, \mathcal{F})$ be a measurable space. For any probability measures $Q \ll P$ and any bounded measurable function $\phi: \mathcal{X} \to \mathbb{R}$:
$$\mathbb{E}_{X \sim Q}[\phi(X)] \le D_{\mathrm{KL}}(Q \,\|\, P) + \ln \mathbb{E}_{X \sim P}\left[ e^{\phi(X)} \right].$$
*Proof of Step 1:* Define the Gibbs measure $G$ with density $\frac{dG}{dP}(x) = \frac{e^{\phi(x)}}{\mathbb{E}_P[e^\phi]}$. By non-negativity of the KL-divergence $D_{\mathrm{KL}}(Q \,\|\, G) \ge 0$:
$$0 \le \int \ln\left(\frac{dQ}{dG}\right) dQ = \int \ln\left(\frac{dQ}{dP} \cdot \frac{dP}{dG}\right) dQ = D_{\mathrm{KL}}(Q \,\|\, P) - \mathbb{E}_Q[\phi] + \ln \mathbb{E}_P[e^\phi].$$
Rearranging terms yields the variational inequality directly.

#### Step 2: Joint Convexity & Jensen's Inequality for Binary Relative Entropy
The function $(q, p) \mapsto \mathrm{kl}(q \,\|\, p)$ is strictly convex on $[0, 1] \times (0, 1)$.
By Jensen's inequality applied to the expectation over $h \sim Q$:
$$\mathrm{kl}\bigl(\hat{L}_n(Q) \,\|\, L(Q)\bigr) = \mathrm{kl}\left(\mathbb{E}_{h \sim Q}[\hat{L}_n(h)] \,\Big\|\, \mathbb{E}_{h \sim Q}[L(h)]\right) \le \mathbb{E}_{h \sim Q}\left[ \mathrm{kl}(\hat{L}_n(h) \,\|\, L(h)) \right].$$

#### Step 3: Applying Donsker-Varadhan Change of Measure
Multiply by $n$ and apply the Donsker-Varadhan inequality with $\phi(h) = n \cdot \mathrm{kl}(\hat{L}_n(h) \,\|\, L(h))$:
$$n \cdot \mathrm{kl}\bigl(\hat{L}_n(Q) \,\|\, L(Q)\bigr) \le n \, \mathbb{E}_{h \sim Q}\left[ \mathrm{kl}(\hat{L}_n(h) \,\|\, L(h)) \right] \le D_{\mathrm{KL}}(Q \,\|\, P) + \ln \mathbb{E}_{h \sim P}\left[ \exp\left( n \cdot \mathrm{kl}(\hat{L}_n(h) \,\|\, L(h)) \right) \right].$$

#### Step 4: Maurer's Moment Generating Function Bound for Bernoulli / Bounded Variables
For any fixed policy $h \in \mathcal{H}$, the random variable $n \hat{L}_n(h) = \sum_{i=1}^n \ell(h, Z_i)$ is the sum of $n$ independent random variables in $[0, 1]$ with expectation $p = L(h)$.
By Andreas Maurer's (2004) theorem on the moment generating function of relative entropy:
For any $p \in [0, 1]$ and $K \sim \operatorname{Binomial}(n, p)$:
$$\mathbb{E}\left[ \exp\left( n \cdot \mathrm{kl}\left(\frac{K}{n} \,\Big\|\, p\right) \right) \right] \le 2\sqrt{n}.$$
By Hoeffding's stochastic dominance reduction for bounded variables $\ell \in [0, 1]$, the same bound holds for general $[0, 1]$-valued i.i.d. sums:
$$\mathbb{E}_{S \sim \mathcal{D}^n}\left[ \exp\left( n \cdot \mathrm{kl}(\hat{L}_n(h) \,\|\, L(h)) \right) \right] \le 2\sqrt{n}.$$

#### Step 5: Fubini's Theorem & Markov's Tail Bound
Taking the expectation over the random sample $S \sim \mathcal{D}^n$ on the inner expectation:
$$\mathbb{E}_{S} \left[ \mathbb{E}_{h \sim P}\left[ \exp\left( n \cdot \mathrm{kl}(\hat{L}_n(h) \,\|\, L(h)) \right) \right] \right] = \mathbb{E}_{h \sim P} \left[ \mathbb{E}_{S}\left[ \exp\left( n \cdot \mathrm{kl}(\hat{L}_n(h) \,\|\, L(h)) \right) \right] \right] \le 2\sqrt{n}.$$
Define the non-negative random variable $\xi(S) \triangleq \mathbb{E}_{h \sim P}\left[ \exp\left( n \cdot \mathrm{kl}(\hat{L}_n(h) \,\|\, L(h)) \right) \right]$.
Since $\mathbb{E}_S[\xi(S)] \le 2\sqrt{n}$, by Markov's inequality:
$$\mathbb{P}_S\left( \xi(S) \ge \frac{2\sqrt{n}}{\delta} \right) \le \frac{\mathbb{E}_S[\xi(S)]}{2\sqrt{n}/\delta} \le \frac{2\sqrt{n}}{2\sqrt{n}/\delta} = \delta.$$
Thus, with probability at least $1 - \delta$:
$$\ln \xi(S) \le \ln\left(\frac{2\sqrt{n}}{\delta}\right).$$
Substituting this back into the inequality from Step 3:
$$n \cdot \mathrm{kl}\bigl(\hat{L}_n(Q) \,\|\, L(Q)\bigr) \le D_{\mathrm{KL}}(Q \,\|\, P) + \ln\left(\frac{2\sqrt{n}}{\delta}\right).$$
Dividing by $n$ completes the proof of the bound.

#### Step 6: Inversion of Binary Relative Entropy
For fixed $q \in [0, 1)$, the function $f(p) = \mathrm{kl}(q \,\|\, p)$ is strictly increasing on $[q, 1)$ with $f(q) = 0$ and $\lim_{p \to 1^-} f(p) = +\infty$.
Its first and second derivatives with respect to $p$ are:
$$f'(p) = \frac{\partial}{\partial p} \mathrm{kl}(q \,\|\, p) = \frac{p - q}{p(1 - p)} > 0 \quad \forall p \in (q, 1),$$
$$f''(p) = \frac{q}{p^2} + \frac{1 - q}{(1 - p)^2} > 0 \quad \forall p \in (0, 1).$$
Since $f$ is strictly convex and strictly increasing on $[q, 1)$, there exists a unique root $p^\star \in [q, 1)$ satisfying $f(p^\star) = B(Q, \delta, n)$.
This unique root is $L_{\max}(Q)$, and since $\mathrm{kl}(q \,\|\, L(Q)) \le B$, monotonicity requires $L(Q) \le L_{\max}(Q)$. $\blacksquare$

---

## 3. Theorem 38: Catoni Optimal Gibbs Posterior Temperature Selection & Finite-Sample Policy Promotion Gate

### Statement
1. **Gibbs Posterior Family:** Let $\mathcal{H} = \{h_1, \dots, h_M\}$ be a discrete hypothesis set with prior probabilities $P(h_j) = p_j > 0$. For inverse temperature parameter $\beta > 0$, define the Gibbs posterior distribution $Q_\beta$:
   $$Q_\beta(h_j) \triangleq \frac{p_j \exp(-\beta \hat{L}_n(h_j))}{Z_n(\beta)}, \quad Z_n(\beta) \triangleq \sum_{k=1}^M p_k \exp(-\beta \hat{L}_n(h_k)).$$
   The KL-divergence of $Q_\beta$ from $P$ has the exact analytical form:
   $$D_{\mathrm{KL}}(Q_\beta \,\|\, P) = -\beta \hat{L}_n(Q_\beta) - \ln Z_n(\beta).$$
2. **Catoni Optimal Temperature Selection:**
   For any $\beta \in (0, 2n)$, Catoni's PAC-Bayesian bound guarantees that with probability $\ge 1 - \delta$:
   $$L(Q_\beta) \le \frac{1}{1 - \frac{\beta}{2n}} \left( \hat{L}_n(Q_\beta) + \frac{D_{\mathrm{KL}}(Q_\beta \,\|\, P) + \ln(1/\delta)}{\beta} \right) = \frac{-\ln Z_n(\beta) + \ln(1/\delta)}{\beta \left(1 - \frac{\beta}{2n}\right)}.$$
   The optimal inverse temperature $\beta^\star$ minimizing the certified generalization risk is the unique solution of:
   $$\beta^\star = \arg\min_{\beta \in (0, 2n)} \left[ \frac{-\ln Z_n(\beta) + \ln(1/\delta)}{\beta \left(1 - \frac{\beta}{2n}\right)} \right].$$
3. **Finite-Sample Policy Promotion Gate:**
   Let $h_{\mathrm{base}}$ be the active production policy with certified risk $L_{\mathrm{base}}$.
   A candidate prompt / policy $Q_{\mathrm{cand}}$ is certified and admitted for production deployment if and only if:
   $$L_{\max}(Q_{\mathrm{cand}}) \le L_{\mathrm{base}} - \epsilon_{\mathrm{margin}},$$
   where $L_{\max}(Q_{\mathrm{cand}})$ is the exact inverted Maurer root from Theorem 37 at confidence $1 - \delta$, and $\epsilon_{\mathrm{margin}} \ge 0$ is the safety promotion margin.
   If $L_{\max}(Q_{\mathrm{cand}}) > L_{\mathrm{base}} - \epsilon_{\mathrm{margin}}$, promotion is deterministically rejected.

### Mathematical Proof

#### Part 1: Gibbs Posterior KL-Divergence
Expanding the definition of relative entropy:
$$D_{\mathrm{KL}}(Q_\beta \,\|\, P) = \sum_{j=1}^M Q_\beta(h_j) \ln\left(\frac{Q_\beta(h_j)}{p_j}\right) = \sum_{j=1}^M Q_\beta(h_j) \ln\left( \frac{\exp(-\beta \hat{L}_n(h_j))}{Z_n(\beta)} \right).$$
Using $\ln(u/v) = \ln u - \ln v$:
$$D_{\mathrm{KL}}(Q_\beta \,\|\, P) = \sum_{j=1}^M Q_\beta(h_j) \left( -\beta \hat{L}_n(h_j) - \ln Z_n(\beta) \right) = -\beta \sum_{j=1}^M Q_\beta(h_j) \hat{L}_n(h_j) - \ln Z_n(\beta) \sum_{j=1}^M Q_\beta(h_j).$$
Since $\sum_{j=1}^M Q_\beta(h_j) = 1$ and $\hat{L}_n(Q_\beta) = \sum_{j=1}^M Q_\beta(h_j) \hat{L}_n(h_j)$, this evaluates to:
$$D_{\mathrm{KL}}(Q_\beta \,\|\, P) = -\beta \hat{L}_n(Q_\beta) - \ln Z_n(\beta).$$

#### Part 2: Catoni PAC-Bayes Bound Derivation
For any single $h$ and loss bounded in $[0, 1]$, by Hoeffding's lemma on $[0, 1]$:
$$\ln \mathbb{E}_{Z}\left[\exp\left(\frac{\lambda}{n}(L(h) - \ell(h, Z))\right)\right] \le \frac{\lambda^2}{8 n^2}.$$
Summing over $n$ independent observations and applying Catoni's transform $\Phi(x) = \frac{1 - e^{-\lambda x}}{\lambda}$ (or directly bounding the moment generating function with $\lambda = \beta / n$):
For $\lambda \in (0, 2)$:
$$\mathbb{E}_S \left[ \exp\left( \beta \left( L(h) - \hat{L}_n(h) \right) - \frac{\beta^2}{2n} L(h) \right) \right] \le 1.$$
Applying Donsker-Varadhan to change measure from $P$ to $Q_\beta$:
$$\beta \left( 1 - \frac{\beta}{2n} \right) L(Q_\beta) - \beta \hat{L}_n(Q_\beta) \le D_{\mathrm{KL}}(Q_\beta \,\|\, P) + \ln(1/\delta).$$
Substituting $D_{\mathrm{KL}}(Q_\beta \,\|\, P) = -\beta \hat{L}_n(Q_\beta) - \ln Z_n(\beta)$:
$$\beta \left( 1 - \frac{\beta}{2n} \right) L(Q_\beta) \le -\ln Z_n(\beta) + \ln(1/\delta).$$
Dividing by $\beta \left(1 - \frac{\beta}{2n}\right) > 0$ yields Catoni's upper bound.
The function $\psi(\beta) = \frac{-\ln Z_n(\beta) + \ln(1/\delta)}{\beta(1 - \beta/(2n))}$ is strictly unimodal on $(0, 2n)$ because $-\ln Z_n(\beta)$ is concave (Free Energy of the Gibbs ensemble), guaranteeing a unique global minimum $\beta^\star$ computable via Golden section search.

#### Part 3: Policy Promotion Gate Soundness
By Theorem 37, the probability of false acceptance under the null hypothesis $H_0: L(Q_{\mathrm{cand}}) > L_{\mathrm{base}} - \epsilon_{\mathrm{margin}}$ is strictly bounded by:
$$\mathbb{P}_{H_0}\left( L_{\max}(Q_{\mathrm{cand}}) \le L_{\mathrm{base}} - \epsilon_{\mathrm{margin}} \right) \le \mathbb{P}\left( L_{\max}(Q_{\mathrm{cand}}) < L(Q_{\mathrm{cand}}) \right) \le \delta.$$
Thus, false promotion occurs with probability at most $\delta \ll 1$ (e.g. $\delta = 0.01$). $\blacksquare$

---

## 4. Algorithmic Specification: Newton-Raphson Inversion & Golden Section Search

```
Algorithm: Newton-Raphson Binary KL Inversion
Input:
  - q in [0, 1) (empirical risk)
  - B > 0 (PAC-Bayes complexity bound)
  - epsilon_tol = 1e-10, max_iter = 50
Output:
  - p* in [q, 1) satisfying kl(q || p*) = B

1. If q == 0.0:
     return 1.0 - exp(-B)
2. Initial guess via Pinsker lower bound:
     p_init <- min(0.999999, q + sqrt(B / 2.0))
3. p <- p_init
4. For iter = 1 to max_iter:
     f_val <- q * ln(q / p) + (1.0 - q) * ln((1.0 - q) / (1.0 - p)) - B
     If |f_val| < epsilon_tol:
          return p
     f_prime <- (p - q) / (p * (1.0 - p))
     p_next <- p - f_val / f_prime
     // Safeguard bracket in (q, 1.0)
     If p_next <= q or p_next >= 1.0:
          p_next <- 0.5 * (p + (if f_val < 0.0 { 1.0 } else { q }))
     p <- p_next
5. Return p
```

---

## 5. Rust Data Structures, Enums & Error Handling

```rust
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum PacBayesError {
    #[error("Invalid sample size: n must be >= 1, got {0}")]
    InvalidSampleSize(usize),
    #[error("Invalid confidence parameter: delta must be in (0, 1), got {0}")]
    InvalidConfidence(f64),
    #[error("Invalid empirical risk: must be in [0, 1], got {0}")]
    InvalidEmpiricalRisk(f64),
    #[error("Invalid KL divergence: must be non-negative, got {0}")]
    InvalidKlDivergence(f64),
    #[error("Root-finding failed to converge within {max_iter} iterations (last delta: {residual})")]
    RootFindingFailure { max_iter: usize, residual: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyPromotionVerdict {
    PromoteToProduction,
    RejectRegressed,
    InsufficientEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskCertificate {
    pub policy_id: String,
    pub sample_size_n: usize,
    pub delta: f64,
    pub empirical_risk: f64,
    pub kl_divergence_prior: f64,
    pub complexity_bound_b: f64,
    pub certified_risk_upper_bound: f64,
    pub pinsker_risk_upper_bound: f64,
    pub finite_sample_confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionGateResult {
    pub candidate_id: String,
    pub candidate_certificate: RiskCertificate,
    pub baseline_risk: f64,
    pub safety_margin: f64,
    pub risk_delta: f64,
    pub verdict: PolicyPromotionVerdict,
}
```

---

## 6. Concrete Numerical Test Cases & Verification Assertions

### Test Case 1: Fast Rate $O(1/n)$ on Low Empirical Risk
- **Inputs:** $n = 1000$, $\delta = 0.01$, Prior $P$ uniform over $M = 10$ candidates ($D_{\mathrm{KL}} = \ln 10 \approx 2.302585$).
- **Empirical Risk:** $\hat{L}_n(Q) = 0.01$ (10 failures out of 1000 trials).
- **Complexity Bound:**
  $$B = \frac{2.302585 + \ln\left(\frac{2\sqrt{1000}}{0.01}\right)}{1000} = \frac{2.302585 + \ln(6324.555)}{1000} = \frac{2.302585 + 8.7522}{1000} \approx 0.011055.$$
- **Exact Root $L_{\max}(Q)$:** Solving $\mathrm{kl}(0.01 \,\|\, p) = 0.011055$ yields $L_{\max}(Q) \approx 0.0336$ (3.36% certified max failure rate).
- **Pinsker Comparison:** $\hat{L} + \sqrt{B/2} = 0.01 + \sqrt{0.005527} \approx 0.01 + 0.0743 = 0.0843$ (8.43%).
- **Assertion:** $L_{\max}(Q) < 0.040$, demonstrating that Maurer's exact root is $>2.5\times$ tighter than Pinsker's relaxation.

### Test Case 2: Policy Promotion Gate Safety Decision
- **Production Baseline:** $L_{\mathrm{base}} = 0.050$, Safety Margin $\epsilon_{\mathrm{margin}} = 0.005$. Promotion threshold: $0.050 - 0.005 = 0.045$.
- **Candidate Policy 1:** $\hat{L}_n(Q_1) = 0.01 \implies L_{\max}(Q_1) = 0.0336 \le 0.045 \implies$ Verdict = `PromoteToProduction`.
- **Candidate Policy 2:** $\hat{L}_n(Q_2) = 0.03 \implies L_{\max}(Q_2) \approx 0.0631 > 0.045 \implies$ Verdict = `RejectRegressed`.
