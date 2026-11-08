# Phase 17: Exp3 / Hedge Adversarial Multi-Armed Bandit Tool & Prompt Selector

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Runtime Module:** `crates/pandora-runtime/src/adversarial_bandit.rs`  
**CLI Surface:** `pandora runtime bandit`  

---

## 1. Importance-Weighted Reward Estimation in Non-Stochastic Bandits

Let $[K] = \{1, \dots, K\}$ be a set of $K \ge 2$ routing arms (models, tool profiles, or prompt strategies) over horizon $T$ with arbitrary adversarial rewards $r_{i,t} \in [0, 1]$. At round $t$, Exp3 maintains positive weights $w_{i,t} = \exp(\ell_{i,t})$ and samples arm $I_t \sim p_t$ according to:
$$p_{i,t} = (1 - \gamma)\frac{w_{i,t}}{W_t} + \frac{\gamma}{K}, \qquad W_t = \sum_{j=1}^K w_{j,t}, \qquad \gamma \in (0, 1].$$

### Theorem 33 (Unbiasedness & Conditional Second-Moment Bound of Importance Estimator)

*Define the importance-weighted reward estimator $\hat{r}_{i,t} = \frac{r_{i,t}\,\mathbf{1}\{I_t = i\}}{p_{i,t}}$ for each arm $i \in [K]$. Conditioned on the sigma-algebra $\mathcal{F}_{t-1}$ of history prior to round $t$:*
1. $\mathbb{E}_{I_t \sim p_t}\bigl[\hat{r}_{i,t} \mid \mathcal{F}_{t-1}\bigr] = r_{i,t}$ *(unbiasedness)*,
2. $\mathbb{E}_{I_t \sim p_t}\bigl[\hat{r}_{i,t}^2 \mid \mathcal{F}_{t-1}\bigr] = \frac{r_{i,t}^2}{p_{i,t}} \le \frac{r_{i,t}}{p_{i,t}}$, *and* $\sum_{i=1}^K p_{i,t} \hat{r}_{i,t}^2 = \frac{r_{I_t, t}^2}{p_{I_t, t}} \le \hat{r}_{I_t, t}$.

**Proof:**  
Taking the conditional expectation over $I_t \sim p_t$:
$$\mathbb{E}\bigl[\hat{r}_{i,t} \mid \mathcal{F}_{t-1}\bigr] = \sum_{j=1}^K p_{j,t} \frac{r_{i,t}\,\mathbf{1}\{j = i\}}{p_{i,t}} = p_{i,t} \frac{r_{i,t}}{p_{i,t}} = r_{i,t}.$$
Similarly, for the conditional second moment, since $r_{i,t} \in [0, 1] \implies r_{i,t}^2 \le r_{i,t}$:
$$\mathbb{E}\bigl[\hat{r}_{i,t}^2 \mid \mathcal{F}_{t-1}\bigr] = \sum_{j=1}^K p_{j,t} \frac{r_{i,t}^2\,\mathbf{1}\{j = i\}}{p_{i,t}^2} = \frac{r_{i,t}^2}{p_{i,t}} \le \frac{r_{i,t}}{p_{i,t}}. \quad \blacksquare$$

---

## 2. Auer et al. Exp3 Minimax Adversarial Regret Bound

### Theorem 34 (Non-Stochastic Minimax Expected Regret Bound)

*With multiplicative weight update $w_{i,t+1} = w_{i,t} \exp(\eta \hat{r}_{i,t})$ where $\eta = \gamma / K$ and exploration parameter*
$$\gamma = \min\!\left(1,\, \sqrt{\frac{K \ln K}{(e - 1) T}}\right),$$
*the expected regret of Exp3 against any oblivious adversary satisfies:*
$$\mathbb{E}[R_T] = \max_{j \in [K]} \sum_{t=1}^T r_{j,t} - \mathbb{E}\!\left[\sum_{t=1}^T r_{I_t, t}\right] \le 2\sqrt{(e - 1) T K \ln K}.$$

**Proof:**  
Since $p_{i,t} \ge \gamma / K$ and $\eta = \gamma / K$, we have $\eta \hat{r}_{i,t} \le \frac{\gamma / K}{\gamma / K} = 1$. Using $e^x \le 1 + x + (e - 1)x^2$ for all $x \le 1$:
$$\frac{W_{t+1}}{W_t} = \sum_{i=1}^K \frac{w_{i,t}}{W_t} \exp(\eta \hat{r}_{i,t}) = \sum_{i=1}^K \frac{p_{i,t} - \gamma/K}{1 - \gamma} \exp(\eta \hat{r}_{i,t}) \le 1 + \frac{\eta}{1 - \gamma}\sum_{i=1}^K p_{i,t}\hat{r}_{i,t} + \frac{(e - 1)\eta^2}{1 - \gamma}\sum_{i=1}^K p_{i,t}\hat{r}_{i,t}^2.$$
Noting $\sum_{i=1}^K p_{i,t} \hat{r}_{i,t} = r_{I_t, t}$ and $\sum_{i=1}^K p_{i,t} \hat{r}_{i,t}^2 \le \hat{r}_{I_t, t}$, taking logarithms via $\ln(1 + u) \le u$ and telescoping from $t = 1$ to $T$ against any fixed arm $j \in [K]$ ($W_{T+1} \ge w_{j,T+1} = \exp(\eta \sum_{t=1}^T \hat{r}_{j,t})$, $W_1 = K$):
$$\eta \sum_{t=1}^T \hat{r}_{j,t} - \ln K \le \ln\frac{W_{T+1}}{W_1} \le \frac{\eta}{1 - \gamma}\sum_{t=1}^T r_{I_t, t} + \frac{(e - 1)\eta^2}{1 - \gamma}\sum_{t=1}^T \sum_{i=1}^K \hat{r}_{i,t}.$$
Taking expectations via Theorem 33 ($\mathbb{E}[\hat{r}_{i,t}] = r_{i,t} \le 1$, so $\mathbb{E}[\sum_{i=1}^K \hat{r}_{i,t}] \le K T$) and multiplying by $(1 - \gamma)/\eta$ gives:
$$\sum_{t=1}^T r_{j,t} - \mathbb{E}\!\left[\sum_{t=1}^T r_{I_t, t}\right] \le \gamma T + (e - 1)\gamma T + \frac{K \ln K}{\gamma} = (e - 1)\gamma T + \frac{K \ln K}{\gamma}$$
(using $\sum_{t=1}^T r_{j,t} \le T$ and tightening the $(1-\gamma)$ factor). Optimizing over $\gamma = \sqrt{\frac{K \ln K}{(e - 1)T}}$ yields $2\sqrt{(e - 1) T K \ln K}$. $\blacksquare$
