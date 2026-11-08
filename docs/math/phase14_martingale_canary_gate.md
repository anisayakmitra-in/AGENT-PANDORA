# Phase 14: Martingale Anytime-Valid Canary Gate with e-Processes & Confidence Sequences

## 1. Problem Formulation

In continuous canary rollout evaluation, an autonomous agent sequentially observes bounded performance differences $X_1, X_2, \dots, X_t \in [-1, 1]$ adapted to a filtration $(\mathcal{F}_t)_{t \ge 0}$, where $X_t = L_{\text{canary}, t} - L_{\text{baseline}, t} - \delta_{\text{tol}}$.
- **Null Hypothesis (Safety / Non-Regression):** $H_0 : \mathbb{E}[X_t \mid \mathcal{F}_{t-1}] \le 0$ for all $t \ge 1$.
- **Classical Peeking Fallacy:** Repeatedly evaluating a fixed-horizon $z$-test or $t$-test at every step $t = 1, 2, \dots$ inflates the Type-I false-alarm probability to $100\%$ as $t \to \infty$ by the Law of the Iterated Logarithm.

We resolve this via **non-negative test supermartingales ($e$-processes)** and **time-uniform confidence sequences**.

---

## 2. Theorem 27: Ville's Maximal Inequality & Anytime-Valid e-Process Control

### Statement
Let $(\lambda_t)_{t \ge 1}$ be a predictable sequence ($\lambda_t \in [0, 1)$ is $\mathcal{F}_{t-1}$-measurable), and define the multiplicative wealth process ($e$-process):
$$
E_0 = 1, \quad E_t = \prod_{i=1}^t (1 + \lambda_i X_i).
$$
Under $H_0 : \mathbb{E}[X_t \mid \mathcal{F}_{t-1}] \le 0$ with $X_t \ge -1$:
1. $(E_t)_{t \ge 0}$ is a non-negative supermartingale with $\mathbb{E}[E_t \mid \mathcal{F}_{t-1}] \le E_{t-1}$.
2. For any significance level $\alpha \in (0, 1)$ and any (possibly unbounded or data-dependent) stopping time $\tau$:
   $$
   \Pr_{H_0}\left(\exists t \ge 1 : E_t \ge \frac{1}{\alpha}\right) \le \alpha,
   $$
   and $p_t = \min\left(1, \left(\max_{1 \le s \le t} E_s\right)^{-1}\right)$ is an anytime-valid $p$-value satisfying $\Pr_{H_0}(\exists t \ge 1 : p_t \le \alpha) \le \alpha$.

### Proof
1. **Non-Negativity & Supermartingale Property:**
   Since $\lambda_t \in [0, 1)$ and $X_t \ge -1$, each factor satisfies $1 + \lambda_t X_t \ge 1 - \lambda_t > 0$, so $E_t > 0$ almost surely for all $t \ge 0$. Because $E_{t-1}$ and $\lambda_t$ are $\mathcal{F}_{t-1}$-measurable, linearity of conditional expectation gives:
   $$
   \mathbb{E}[E_t \mid \mathcal{F}_{t-1}] = E_{t-1} \left(1 + \lambda_t \mathbb{E}[X_t \mid \mathcal{F}_{t-1}]\right) \le E_{t-1}(1 + 0) = E_{t-1}.
   $$
2. **Ville's Maximal Inequality (1939):**
   Define the first-passage stopping time $\tau = \inf\{t \ge 1 : E_t \ge 1/\alpha\}$ (with $\inf \emptyset = \infty$). For any finite horizon $T \ge 1$, the stopped process $E_{t \wedge \tau}$ is a non-negative supermartingale. By Doob's Optional Stopping Theorem:
   $$
   1 = E_0 \ge \mathbb{E}[E_{T \wedge \tau}] \ge \mathbb{E}\left[E_\tau \, \mathbb{I}\{\tau \le T\}\right] \ge \frac{1}{\alpha} \Pr(\tau \le T).
   $$
   Rearranging yields $\Pr\left(\max_{1 \le t \le T} E_t \ge \frac{1}{\alpha}\right) = \Pr(\tau \le T) \le \alpha$. By monotone continuity of probability measures as $T \to \infty$:
   $$
   \Pr_{H_0}\left(\exists t \ge 1 : E_t \ge \frac{1}{\alpha}\right) = \lim_{T \to \infty} \Pr(\tau \le T) \le \alpha. \quad \blacksquare
   $$

---

## 3. Theorem 28: Time-Uniform Gaussian-Mixture Confidence Sequence Boundary

### Statement
Suppose $D_t = X_t - \mu$ is conditionally $\sigma$-sub-Gaussian given $\mathcal{F}_{t-1}$:
$$
\mathbb{E}\left[\exp(\eta D_t) \mid \mathcal{F}_{t-1}\right] \le \exp\left(\frac{\eta^2 \sigma^2}{2}\right) \quad \forall \eta \in \mathbb{R}.
$$
For any precision parameter $\rho > 0$ and confidence level $1 - \alpha \in (0, 1)$, define the time-uniform radius:
$$
r_t(\alpha) = \sigma \sqrt{\frac{2(t \rho^2 + 1)}{t^2 \rho^2} \ln\left(\frac{\sqrt{t \rho^2 + 1}}{\alpha}\right)}.
$$
Then the empirical mean $\hat{\mu}_t = \frac{1}{t} \sum_{i=1}^t X_i$ satisfies the simultaneous time-uniform coverage guarantee:
$$
\Pr\left(\forall t \ge 1 : |\hat{\mu}_t - \mu| \le r_t(\alpha)\right) \ge 1 - \alpha,
$$
with asymptotic width $r_t(\alpha) = O\left(\sigma \sqrt{\frac{\ln t}{t}}\right)$ as $t \to \infty$.

### Proof
1. **Exponential Supermartingale Family:**
   Let $S_t = \sum_{i=1}^t (X_i - \mu) = t(\hat{\mu}_t - \mu)$. By the conditional $\sigma$-sub-Gaussian assumption, for every fixed $\eta \in \mathbb{R}$, the process:
   $$
   M_t(\eta) = \exp\left(\eta S_t - \frac{t \eta^2 \sigma^2}{2}\right)
   $$
   is a non-negative supermartingale with $M_0(\eta) = 1$.
2. **Gaussian Mixture Integration (Method of Mixtures, Robbins 1970 / Howard et al. 2021):**
   Integrate $M_t(\eta)$ against a zero-mean Gaussian prior $\eta \sim \mathcal{N}(0, \rho^2 / \sigma^2)$ with density $\phi_\rho(\eta) = \frac{\sigma}{\sqrt{2\pi \rho^2}} \exp\left(-\frac{\eta^2 \sigma^2}{2\rho^2}\right)$. By Tonelli's theorem, the mixture process $\bar{M}_t = \int_{-\infty}^\infty M_t(\eta) \phi_\rho(\eta) \, d\eta$ is also a non-negative supermartingale with $\bar{M}_0 = 1$. Completing the square in the exponent yields the closed form:
   $$
   \bar{M}_t = \frac{1}{\sqrt{t \rho^2 + 1}} \exp\left(\frac{\rho^2 S_t^2}{2 \sigma^2 (t \rho^2 + 1)}\right).
   $$
3. **Application of Ville's Inequality:**
   By Theorem 27 applied to $\bar{M}_t$:
   $$
   \Pr\left(\exists t \ge 1 : \bar{M}_t \ge \frac{1}{\alpha}\right) \le \alpha.
   $$
   Solving $\bar{M}_t < \frac{1}{\alpha}$ for $|S_t| = t |\hat{\mu}_t - \mu|$ gives:
   $$
   \frac{\rho^2 t^2 (\hat{\mu}_t - \mu)^2}{2 \sigma^2 (t \rho^2 + 1)} < \ln\left(\frac{\sqrt{t \rho^2 + 1}}{\alpha}\right) \iff |\hat{\mu}_t - \mu| < \sigma \sqrt{\frac{2(t \rho^2 + 1)}{t^2 \rho^2} \ln\left(\frac{\sqrt{t \rho^2 + 1}}{\alpha}\right)} = r_t(\alpha). \quad \blacksquare
   $$
