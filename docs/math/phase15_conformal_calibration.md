# Phase 15: Conformal Prediction & Distribution-Free Uncertainty Calibration

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Runtime Module:** `crates/pandora-runtime/src/conformal_calibrator.rs`  
**CLI Surface:** `pandora runtime conformal`  

---

## 1. Split Conformal Prediction & Finite-Sample Exchangeable Coverage

Let $(X_1, Y_1), \dots, (X_n, Y_n), (X_{n+1}, Y_{n+1})$ be exchangeable pairs with non-conformity score function $s(X, Y) \in [0, \infty)$, and let $S_i = s(X_i, Y_i)$ for $i = 1, \dots, n+1$.

### Theorem 29 (Finite-Sample Marginal Coverage Bounds)

*Let $S_{(1)} \le S_{(2)} \le \dots \le S_{(n)}$ denote the order statistics of the $n$ calibration non-conformity scores. For target miscoverage rate $\alpha \in (0, 1)$, define the conformal rank*
$$k = \lceil (n + 1)(1 - \alpha) \rceil$$
*and empirical threshold $\widehat{q}_{1-\alpha} = S_{(k)}$ if $k \le n$ and $+\infty$ if $k > n$. Then the conformal prediction set $\widehat{C}_\alpha(X_{n+1}) = \{y : s(X_{n+1}, y) \le \widehat{q}_{1-\alpha}\}$ satisfies:*
$$\mathbb{P}\bigl(Y_{n+1} \in \widehat{C}_\alpha(X_{n+1})\bigr) \ge 1 - \alpha.$$
*Moreover, if $S_1, \dots, S_{n+1}$ are almost surely distinct (no ties),*
$$\mathbb{P}\bigl(Y_{n+1} \in \widehat{C}_\alpha(X_{n+1})\bigr) = \frac{\lceil (n + 1)(1 - \alpha) \rceil}{n + 1} \le 1 - \alpha + \frac{1}{n + 1}.$$

**Proof:**  
By exchangeability of $(S_1, \dots, S_{n+1})$, the rank of $S_{n+1}$ among $\{S_1, \dots, S_{n+1}\}$ (breaking ties conservatively in favor of larger rank for $S_{n+1}$) is stochastically dominated by a discrete uniform random variable $R \sim \mathrm{Uniform}\{1, 2, \dots, n+1\}$. Notice the deterministic equivalence of events:
$$S_{n+1} \le S_{(k)} \iff \text{Rank}(S_{n+1} \text{ in } \{S_1, \dots, S_{n+1}\}) \le k.$$
Therefore:
$$\mathbb{P}\bigl(S_{n+1} \le S_{(k)}\bigr) \ge \frac{k}{n + 1} = \frac{\lceil (n + 1)(1 - \alpha) \rceil}{n + 1} \ge 1 - \alpha.$$
When $S_1, \dots, S_{n+1}$ have no ties almost surely, the rank distribution is exactly uniform on $\{1, \dots, n+1\}$, and since $\lceil x \rceil < x + 1$,
$$\mathbb{P}\bigl(S_{n+1} \le S_{(k)}\bigr) = \frac{\lceil (n + 1)(1 - \alpha) \rceil}{n + 1} < \frac{(n + 1)(1 - \alpha) + 1}{n + 1} = 1 - \alpha + \frac{1}{n + 1}. \quad \blacksquare$$

---

## 2. Gibbs-Candès Adaptive Conformal Inference (ACI)

When an autonomous agent operates under non-stationary or adversarial distribution shift, exchangeability fails. Adaptive Conformal Inference updates the effective miscoverage parameter $\alpha_t$ online:
$$\alpha_{t+1} = \alpha_t + \gamma\bigl(\alpha - \mathrm{err}_t\bigr), \qquad \mathrm{err}_t = \mathbf{1}\bigl\{Y_t \notin \widehat{C}_{\alpha_t}(X_t)\bigr\}$$
with conventions $\widehat{C}_{\alpha_t}(x) = \mathcal{Y}$ ($\mathrm{err}_t = 0$) whenever $\alpha_t \le 0$ and $\widehat{C}_{\alpha_t}(x) = \emptyset$ ($\mathrm{err}_t = 1$) whenever $\alpha_t \ge 1$.

### Theorem 30 (Distribution-Free Long-Run Coverage under Arbitrary Shift)

*For any sequence of data $(X_1, Y_1), \dots, (X_T, Y_T)$ without any distributional or exchangeability assumptions, and for any step size $\gamma > 0$ with $\alpha_1 \in [0, 1]$, the ACI iterates satisfy $\alpha_t \in [-\gamma, 1 + \gamma]$ for all $t \ge 1$ and*
$$\left|\frac{1}{T}\sum_{t=1}^T \mathrm{err}_t - \alpha\right| \le \frac{\max(\alpha_1, 1 - \alpha_1) + \gamma}{T \gamma}.$$

**Proof:**  
First we show by induction that $\alpha_t \in [-\gamma, 1 + \gamma]$. Base case: $\alpha_1 \in [0, 1] \subset [-\gamma, 1 + \gamma]$. Suppose $\alpha_t \in [-\gamma, 1 + \gamma]$.
- If $\alpha_t \le 0$, then $\widehat{C}_{\alpha_t}(X_t) = \mathcal{Y}$, so $\mathrm{err}_t = 0$ and $\alpha_{t+1} = \alpha_t + \gamma \alpha > \alpha_t \ge -\gamma$.
- If $\alpha_t \ge 1$, then $\widehat{C}_{\alpha_t}(X_t) = \emptyset$, so $\mathrm{err}_t = 1$ and $\alpha_{t+1} = \alpha_t - \gamma(1 - \alpha) < \alpha_t \le 1 + \gamma$.
- If $\alpha_t \in (0, 1)$, then since $\alpha - \mathrm{err}_t \in [\alpha - 1, \alpha] \subset [-1, 1]$, we have $\alpha_{t+1} \in (-\gamma, 1 + \gamma)$.

Telescoping the recurrence $\alpha_{t+1} - \alpha_t = \gamma(\alpha - \mathrm{err}_t)$ from $t = 1$ to $T$:
$$\alpha_{T+1} - \alpha_1 = \gamma \sum_{t=1}^T (\alpha - \mathrm{err}_t) \implies \frac{1}{T}\sum_{t=1}^T \mathrm{err}_t - \alpha = \frac{\alpha_1 - \alpha_{T+1}}{T \gamma}.$$
Since $\alpha_{T+1} \in [-\gamma, 1 + \gamma]$, we have $|\alpha_1 - \alpha_{T+1}| \le \max(\alpha_1, 1 - \alpha_1) + \gamma$, yielding the stated bound. $\blacksquare$
