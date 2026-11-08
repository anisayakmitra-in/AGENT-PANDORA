# Phase 13: Entropic Optimal Transport (Sinkhorn-Knopp) Semantic Drift Detector

## 1. Problem Formulation

Let $\mu = \sum_{i=1}^n a_i \delta_{x_i}$ and $\nu = \sum_{j=1}^m b_j \delta_{y_j}$ be two discrete empirical probability distributions over embedding/telemetry space $\mathbb{R}^d$, where $a \in \Delta_n^\circ$ and $b \in \Delta_m^\circ$ lie in the interior of the probability simplices ($\sum_{i=1}^n a_i = \sum_{j=1}^m b_j = 1$). Let $C \in \mathbb{R}_{\ge 0}^{n \times m}$ denote the squared Euclidean cost matrix $C_{ij} = \|x_i - y_j\|_2^2$, and let $\Pi(a, b) = \{\pi \in \mathbb{R}_{\ge 0}^{n \times m} : \pi \mathbf{1}_m = a, \, \pi^\top \mathbf{1}_n = b\}$ be the Birkhoff transport polytope.

For regularization parameter $\varepsilon > 0$, the **entropic optimal transport problem** is:
$$
\text{OT}_\varepsilon(\mu, \nu) = \min_{\pi \in \Pi(a, b)} \langle C, \pi \rangle + \varepsilon \, \text{KL}(\pi \,\|\, a \otimes b), \quad \text{KL}(\pi \,\|\, a \otimes b) = \sum_{i=1}^n \sum_{j=1}^m \pi_{ij} \ln \frac{\pi_{ij}}{a_i b_j}.
$$

---

## 2. Theorem 25: Birkhoff-Hopf Projective Contraction & Log-Domain Sinkhorn Stability

### Statement
1. **Unique Gibbs-Kernel Solution & Log-Domain Dual Iteration:**
   The strictly convex primal objective admits a unique minimizer $\pi^\star \in \Pi(a, b)$ parameterized by dual Kantorovich potentials $(f^\star, g^\star) \in \mathbb{R}^n \times \mathbb{R}^m$:
   $$
   \pi_{ij}^\star = a_i b_j \exp\left(\frac{f_i^\star + g_j^\star - C_{ij}}{\varepsilon}\right).
   $$
   The alternating coordinate ascent updates in log-domain:
   $$
   f_i^{(k+1)} = -\varepsilon \, \text{LSE}_{j=1}^m \left(\ln b_j + \frac{g_j^{(k)} - C_{ij}}{\varepsilon}\right), \quad g_j^{(k+1)} = -\varepsilon \, \text{LSE}_{i=1}^n \left(\ln a_i + \frac{f_i^{(k+1)} - C_{ij}}{\varepsilon}\right)
   $$
   are algebraically equivalent to Sinkhorn-Knopp matrix scaling while avoiding floating-point overflow/underflow for arbitrary $\frac{\|C\|_\infty}{\varepsilon}$.
2. **Global Linear Convergence Rate:**
   Let $K_{ij} = \exp(-C_{ij}/\varepsilon) > 0$. In the Hilbert projective metric $d_H(u, v) = \max_i \ln(u_i/v_i) - \min_i \ln(u_i/v_i)$, each full Sinkhorn iteration is a strict contraction with Birkhoff-Hopf coefficient:
   $$
   d_H\left(u^{(k+1)}, u^\star\right) \le \kappa^2 \, d_H\left(u^{(k)}, u^\star\right), \quad \kappa = \frac{\exp\left(\frac{\|C\|_\infty}{2\varepsilon}\right) - 1}{\exp\left(\frac{\|C\|_\infty}{2\varepsilon}\right) + 1} < 1.
   $$

### Proof
1. **Lagrangian Duality & LSE Stability:**
   Introducing dual multipliers $f \in \mathbb{R}^n$ for $\pi \mathbf{1}_m = a$ and $g \in \mathbb{R}^m$ for $\pi^\top \mathbf{1}_n = b$, the Lagrangian is:
   $$
   \mathcal{L}(\pi, f, g) = \sum_{i,j} \pi_{ij} \left(C_{ij} + \varepsilon \ln \frac{\pi_{ij}}{a_i b_j} - f_i - g_j\right) + \langle f, a \rangle + \langle g, b \rangle.
   $$
   Setting $\frac{\partial \mathcal{L}}{\partial \pi_{ij}} = 0$ yields $\pi_{ij} = a_i b_j \exp\left(\frac{f_i + g_j - C_{ij}}{\varepsilon}\right)$. Enforcing the row marginal $\sum_{j=1}^m \pi_{ij} = a_i$ gives $\exp(-f_i/\varepsilon) = \sum_{j=1}^m b_j \exp((g_j - C_{ij})/\varepsilon)$, which takes the exact log-sum-exp form upon taking $-\varepsilon \ln(\cdot)$. Evaluating $\text{LSE}(z) = z_{\max} + \ln \sum_j \exp(z_j - z_{\max})$ ensures every exponent argument $z_j - z_{\max} \le 0$ lies in $(-\infty, 0]$, guaranteeing zero overflow.
2. **Birkhoff-Hopf Contraction Theorem:**
   Since $0 \le C_{ij} \le \|C\|_\infty$, the Gibbs kernel $K_{ij} = \exp(-C_{ij}/\varepsilon)$ is strictly positive with projective diameter:
   $$
   \Delta(K) = \max_{i, j, p, q} \ln \frac{K_{ij} K_{pq}}{K_{iq} K_{pj}} \le \frac{2\|C\|_\infty}{\varepsilon}.
   $$
   By Birkhoff's theorem (1957) for positive linear operators on cones, the projective contraction ratio of $K$ is $\tanh(\Delta(K)/4) = \frac{\exp(\Delta(K)/4) - 1}{\exp(\Delta(K)/4) + 1} \le \kappa < 1$. Because one Sinkhorn step applies both $K$ and $K^\top$, the Hilbert metric contracts by at least $\kappa^2 < 1$ per iteration. $\blacksquare$

---

## 3. Theorem 26: Entropic Bias Bound & Debiased Sinkhorn Divergence Positivity

### Statement
1. **Entropic Approximation Bound:**
   Let $W_2^2(\mu, \nu) = \min_{\pi \in \Pi(a, b)} \langle C, \pi \rangle$ be the exact unregularized 2-Wasserstein squared distance. Then:
   $$
   0 \le \text{OT}_\varepsilon(\mu, \nu) - W_2^2(\mu, \nu) \le \varepsilon \left(H(a) + H(b)\right),
   $$
   where $H(a) = -\sum_{i=1}^n a_i \ln a_i$ and $H(b) = -\sum_{j=1}^m b_j \ln b_j$.
2. **Identity of Indiscernibles & Non-Negativity of Sinkhorn Divergence:**
   Define the debiased Sinkhorn divergence:
   $$
   S_\varepsilon(\mu, \nu) = \text{OT}_\varepsilon(\mu, \nu) - \frac{1}{2} \text{OT}_\varepsilon(\mu, \mu) - \frac{1}{2} \text{OT}_\varepsilon(\nu, \nu).
   $$
   For any $\mu, \nu$ on $\mathbb{R}^d$, $S_\varepsilon(\mu, \nu) \ge 0$, and $S_\varepsilon(\mu, \nu) = 0 \iff \mu = \nu$.

### Proof
1. **Upper and Lower Bounds on $\text{OT}_\varepsilon - W_2^2$:**
   Since $\text{KL}(\pi \,\|\, a \otimes b) \ge 0$ by Gibbs' inequality, for the optimal entropic coupling $\pi_\varepsilon^\star$:
   $$
   \text{OT}_\varepsilon(\mu, \nu) = \langle C, \pi_\varepsilon^\star \rangle + \varepsilon \, \text{KL}(\pi_\varepsilon^\star \,\|\, a \otimes b) \ge \langle C, \pi_\varepsilon^\star \rangle \ge W_2^2(\mu, \nu).
   $$
   Conversely, let $\pi_0^\star \in \arg\min_{\pi \in \Pi(a, b)} \langle C, \pi \rangle$. Because $H(\pi_0^\star) \ge 0$, we have $\text{KL}(\pi_0^\star \,\|\, a \otimes b) = H(a) + H(b) - H(\pi_0^\star) \le H(a) + H(b)$. By optimality of $\pi_\varepsilon^\star$ for the regularized objective:
   $$
   \text{OT}_\varepsilon(\mu, \nu) \le \langle C, \pi_0^\star \rangle + \varepsilon \, \text{KL}(\pi_0^\star \,\|\, a \otimes b) \le W_2^2(\mu, \nu) + \varepsilon(H(a) + H(b)).
   $$
2. **Positivity via Negative Definiteness of Squared Euclidean Cost:**
   For $C(x, y) = \|x - y\|_2^2$, the Gibbs kernel $K(x, y) = \exp(-\|x - y\|_2^2 / \varepsilon)$ is a Gaussian Radial Basis Function (RBF) kernel, which is strictly positive-definite (universal) on $\mathbb{R}^d$ by Bochner's theorem. By Feydy et al. (2019, Theorem 1), dual Sinkhorn potentials induce a convex barycentric projection in the RKHS of $K$, implying $S_\varepsilon(\mu, \nu) \ge 0$ with equality if and only if $\mu = \nu$. $\blacksquare$
