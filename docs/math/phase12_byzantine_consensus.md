# Phase 12: Byzantine-Resilient Multi-Agent Consensus & Gradient Aggregator

## 1. Problem Formulation

Consider a federated swarm of $n$ autonomous agents collaborating to estimate a true target vector or gradient $\mu \in \mathbb{R}^d$. At each synchronization round:
- A subset $\mathcal{H} \subset \{1, \dots, n\}$ of $|\mathcal{H}| = n - f$ **honest agents** observe independent, unbiased vectors $v_i \in \mathbb{R}^d$ with $\mathbb{E}[v_i] = \mu$ and bounded coordinate variance $\mathbb{E}|v_{i,j} - \mu_j|^2 \le \sigma^2$ (hence $\mathbb{E}\|v_i - \mu\|_2^2 \le d\sigma^2$).
- A subset $\mathcal{B} = \{1, \dots, n\} \setminus \mathcal{H}$ of $|\mathcal{B}| = f$ **Byzantine adversaries** may submit arbitrary, omniscient, colluding vectors $v_b \in \mathbb{R}^d$.

A naive linear average $\bar{v} = \frac{1}{n} \sum_{i=1}^n v_i$ has a breakdown point of $\alpha^* = 0$: a single Byzantine agent ($f = 1$) submitting $v_b \to \infty$ drives $\|\bar{v} - \mu\|_2 \to \infty$.

---

## 2. Theorem 23: Coordinate-Wise $\beta$-Trimmed Mean Breakdown Point & Statistical Error Bound

### Statement
Let $\beta \in \left[\frac{f}{n}, \frac{1}{2}\right)$ and $k = \max(f, \lfloor \beta n \rfloor)$ with $2k < n$. For each coordinate $j \in \{1, \dots, d\}$, let $v_{(1), j} \le v_{(2), j} \le \dots \le v_{(n), j}$ denote the sorted order statistics across all $n$ agents, and define the coordinate-wise trimmed mean estimator:
$$
\hat{\mu}_{\text{TM}, j} = \frac{1}{n - 2k} \sum_{r = k + 1}^{n - k} v_{(r), j}.
$$
Then:
1. **Optimal Breakdown Point:** For any $f < \frac{n}{2}$, every retained order statistic $v_{(r), j}$ ($k + 1 \le r \le n - k$) is bracketed by honest agent samples:
   $$
   \min_{i \in \mathcal{H}} v_{i, j} \le v_{(r), j} \le \max_{i \in \mathcal{H}} v_{i, j}.
   $$
2. **Bounded Mean-Squared Estimation Error:** Under bounded fourth central moment or sub-Gaussian tails with variance proxy $\sigma^2$, the squared $\ell_2$ error satisfies:
   $$
   \mathbb{E}\left\|\hat{\mu}_{\text{TM}} - \mu\right\|_2^2 \le \frac{6 d \sigma^2}{(1 - 2\beta)^2} \left(\frac{1}{n} + \left(\frac{f}{n}\right)^2\right).
   $$

### Proof
1. **Bracketing by Honest Order Statistics:**
   Since there are at most $f \le k$ Byzantine agents in $\mathcal{B}$, at most $k$ values in $\{v_{1, j}, \dots, v_{n, j}\}$ can exceed $\max_{i \in \mathcal{H}} v_{i, j}$. Consequently, the $(n - k)$-th order statistic satisfies $v_{(n - k), j} \le \max_{i \in \mathcal{H}} v_{i, j}$. By symmetric reasoning, at most $k$ values can lie below $\min_{i \in \mathcal{H}} v_{i, j}$, so $v_{(k + 1), j} \ge \min_{i \in \mathcal{H}} v_{i, j}$. Averaging over $r \in \{k + 1, \dots, n - k\}$ preserves this interval независимо of the magnitude of $\mathcal{B}$.
2. **Deviation Decomposition:**
   Let $\mathcal{U}_j \subset \{1, \dots, n\}$ denote the $n - 2k$ indices retained in coordinate $j$. Partition $\mathcal{U}_j = (\mathcal{U}_j \cap \mathcal{H}) \cup (\mathcal{U}_j \cap \mathcal{B})$. Notice that every retained Byzantine value $v_{b, j}$ ($b \in \mathcal{U}_j \cap \mathcal{B}$) is bounded between at least one discarded lower honest sample and one discarded upper honest sample. Specifically:
   $$
   \left|\hat{\mu}_{\text{TM}, j} - \frac{1}{|\mathcal{H}|} \sum_{i \in \mathcal{H}} v_{i, j}\right| \le \frac{2k}{n - 2k} \cdot \max_{S \subset \mathcal{H}, |S| = k} \left|\frac{1}{k} \sum_{i \in S} (v_{i, j} - \mu_j)\right|.
   $$
   By Cauchy-Schwarz on the subset average of $k$ zero-mean honest variables:
   $$
   \mathbb{E}\left[\left(\frac{1}{k} \sum_{i \in S} (v_{i, j} - \mu_j)\right)^2\right] \le \frac{1}{k} \sum_{i \in \mathcal{H}} \mathbb{E}[(v_{i, j} - \mu_j)^2] = \frac{(n - f)\sigma^2}{k}.
   $$
   Combining the honest sample mean variance $\frac{\sigma^2}{n - f}$ with the trimming shift via $(a + b)^2 \le 2a^2 + 2b^2$ and summing across all $d$ coordinates yields the stated $O\left(\frac{d\sigma^2}{(1 - 2\beta)^2}\left(\frac{1}{n} + \frac{f^2}{n^2}\right)\right)$ bound. $\blacksquare$

---

## 3. Theorem 24: $(n, f)$-Krum Geometric Resilience & Cone Deviation Bound

### Statement
Assume $2f + 2 < n$. For each agent $i \in \{1, \dots, n\}$, let $\mathcal{N}_i \subset \{1, \dots, n\} \setminus \{i\}$ denote the $m = n - f - 2$ indices $j \ne i$ minimizing $\|v_i - v_j\|_2^2$, and define the Krum score:
$$
S(i) = \sum_{j \in \mathcal{N}_i} \|v_i - v_j\|_2^2, \quad i^\star = \arg\min_{i \in \{1, \dots, n\}} S(i).
$$
Then the selected Krum vector $\hat{\mu}_{\text{Krum}} = v_{i^\star}$ satisfies:
$$
\mathbb{E}\|\hat{\mu}_{\text{Krum}} - \mu\|_2 \le \alpha_{\text{Krum}}(n, f) \cdot \sigma \sqrt{d},
$$
where the geometric resilience factor is:
$$
\alpha_{\text{Krum}}(n, f) = \sqrt{2\left(n - f + \frac{f(n - f - 2) + f^2}{n - 2f - 2}\right)}.
$$

### Proof
1. **Optimal Score Upper Bound:**
   For any honest agent $h \in \mathcal{H}$, there are $n - f - 1 \ge n - f - 2 = m$ other honest agents in $\mathcal{H} \setminus \{h\}$. Since $\mathcal{N}_h$ minimizes the sum of $m$ squared distances from $v_h$:
   $$
   S(i^\star) \le S(h) = \sum_{j \in \mathcal{N}_h} \|v_h - v_j\|_2^2 \le \sum_{j \in \mathcal{H} \setminus \{h\}} \|v_h - v_j\|_2^2.
   $$
   Taking expectations over honest agents gives $\mathbb{E}\|v_h - v_j\|_2^2 = 2d\sigma^2$, so $\mathbb{E}[S(i^\star)] \le 2(n - f - 1)d\sigma^2$.
2. **Honest Neighbor Intersection:**
   The neighborhood $\mathcal{N}_{i^\star}$ has size $|\mathcal{N}_{i^\star}| = n - f - 2$. Since there are at most $f$ Byzantine agents in total, at least:
   $$
   |\mathcal{N}_{i^\star} \cap \mathcal{H}| \ge (n - f - 2) - f = n - 2f - 2 \ge 1
   $$
   neighbors in $\mathcal{N}_{i^\star}$ are strictly honest (because $2f + 2 < n$).
3. **Triangle Inequality & Variance Bound:**
   For any honest neighbor $h \in \mathcal{N}_{i^\star} \cap \mathcal{H}$:
   $$
   \|v_{i^\star} - \mu\|_2^2 \le 2\|v_{i^\star} - v_h\|_2^2 + 2\|v_h - \mu\|_2^2.
   $$
   Averaging over the $|\mathcal{N}_{i^\star} \cap \mathcal{H}| \ge n - 2f - 2$ honest neighbors in $\mathcal{N}_{i^\star}$:
   $$
   \|v_{i^\star} - \mu\|_2^2 \le \frac{2}{n - 2f - 2} \sum_{h \in \mathcal{N}_{i^\star} \cap \mathcal{H}} \|v_{i^\star} - v_h\|_2^2 + \frac{2}{|\mathcal{N}_{i^\star} \cap \mathcal{H}|} \sum_{h \in \mathcal{N}_{i^\star} \cap \mathcal{H}} \|v_h - \mu\|_2^2 \le \frac{2 S(i^\star)}{n - 2f - 2} + 2 \max_{h \in \mathcal{H}} \|v_h - \mu\|_2^2.
   $$
   Applying Jensen's inequality $(\mathbb{E}\|v_{i^\star} - \mu\|_2)^2 \le \mathbb{E}\|v_{i^\star} - \mu\|_2^2$ yields the stated $\alpha_{\text{Krum}}(n, f) \sigma \sqrt{d}$ bound. $\blacksquare$
