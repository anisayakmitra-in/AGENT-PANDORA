# Phase 16: Spectral Graph Partitioning & Algebraic Connectivity for Multi-Agent Topology

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Runtime Module:** `crates/pandora-runtime/src/spectral_topology.rs`  
**CLI Surface:** `pandora runtime topology`  

---

## 1. Graph Laplacian Quadratic Form & Fiedler Algebraic Connectivity

Let $G = (V, E, w)$ be an undirected weighted multi-agent communication graph on $n = |V| \ge 2$ vertices with symmetric adjacency matrix $A_{uv} = w_{uv} \ge 0$, degree matrix $D = \mathrm{diag}(d_1, \dots, d_n)$ where $d_u = \sum_{v \in V} w_{uv}$, and unnormalized Graph Laplacian $L = D - A$.

### Theorem 31 (Courant-Fischer Variational Characterization of $\lambda_2(L)$)

*For any vector $x \in \mathbb{R}^n$, the Laplacian quadratic form satisfies*
$$x^\top L x = \sum_{\{u, v\} \in E} w_{uv}(x_u - x_v)^2 \ge 0.$$
*Thus $L$ is symmetric positive semi-definite with eigenvalues $0 = \lambda_1(L) \le \lambda_2(L) \le \dots \le \lambda_n(L)$ and trivial eigenvector $\mathbf{1} = (1, \dots, 1)^\top$. By the Courant-Fischer Min-Max Theorem, the Fiedler value (algebraic connectivity) is*
$$\lambda_2(L) = \min_{\substack{x \in \mathbb{R}^n \setminus \{0\} \\ \langle x, \mathbf{1} \rangle = 0}} \frac{x^\top L x}{x^\top x},$$
*and $\lambda_2(L) > 0$ if and only if $G$ is connected.*

**Proof:**  
Expanding $x^\top L x = \sum_u d_u x_u^2 - \sum_{u,v} w_{uv} x_u x_v = \frac{1}{2}\sum_{u,v} w_{uv}(x_u^2 - 2x_u x_v + x_v^2) = \sum_{\{u,v\} \in E} w_{uv}(x_u - x_v)^2 \ge 0$. Hence $x^\top L x = 0 \iff x_u = x_v$ for every edge $\{u, v\} \in E$, meaning $x$ is constant on each connected component of $G$. The dimension of $\ker(L)$ equals the number of connected components of $G$, so $\dim \ker(L) = 1 \iff \lambda_2(L) > 0 \iff G$ is connected. $\blacksquare$

---

## 2. Cheeger's Inequality & Fiedler Sweep-Cut Conductance Bound

Let $\mathcal{L}_{\mathrm{sym}} = D^{-1/2} L D^{-1/2} = I - D^{-1/2} A D^{-1/2}$ be the symmetric normalized Graph Laplacian with eigenvalues $0 = \nu_1 \le \nu_2 \le \dots \le \nu_n \le 2$. For any non-empty proper vertex subset $S \subset V$, define its volume $\mathrm{vol}(S) = \sum_{u \in S} d_u$, cut weight $\mathrm{cut}(S, \bar{S}) = \sum_{u \in S, v \notin S} w_{uv}$, and conductance:
$$\phi(S) = \frac{\mathrm{cut}(S, \bar{S})}{\min(\mathrm{vol}(S), \mathrm{vol}(\bar{S}))}, \qquad h(G) = \min_{\emptyset \subset S \subset V} \phi(S).$$

### Theorem 32 (Cheeger's Inequality for Multi-Agent Communication Bottlenecks)

*For any connected weighted graph $G$ without isolated vertices, the conductance $h(G)$ and normalized Fiedler value $\nu_2 = \lambda_2(\mathcal{L}_{\mathrm{sym}})$ satisfy:*
$$\frac{\nu_2}{2} \le h(G) \le \sqrt{2 \nu_2}.$$
*Moreover, sorting the vertices by $y_u = (D^{-1/2} v_2)_u$ and evaluating the $n-1$ sweep cuts $S_k = \{u_{\pi(1)}, \dots, u_{\pi(k)}\}$ produces a partition achieving conductance $\phi_{\mathrm{sweep}} \le \sqrt{2 \nu_2}$ in $O(n \log n + |E|)$ time.*

**Proof (Lower Bound $\nu_2 / 2 \le h(G)$):**  
Let $S \subset V$ achieve $h(G) = \phi(S)$ with $\mathrm{vol}(S) \le \mathrm{vol}(\bar{S})$. Define $x \in \mathbb{R}^n$ by $x_u = \frac{1}{\mathrm{vol}(S)}$ for $u \in S$ and $x_u = -\frac{1}{\mathrm{vol}(\bar{S})}$ for $u \in \bar{S}$. Then $\langle D^{1/2} x, D^{1/2}\mathbf{1} \rangle = \sum_{u \in V} d_u x_u = 1 - 1 = 0$. By Courant-Fischer applied to $\mathcal{L}_{\mathrm{sym}}$ with $z = D^{1/2} x \perp D^{1/2}\mathbf{1}$:
$$\nu_2 \le \frac{x^\top L x}{x^\top D x} = \frac{\mathrm{cut}(S, \bar{S}) \left(\frac{1}{\mathrm{vol}(S)} + \frac{1}{\mathrm{vol}(\bar{S})}\right)^2}{\frac{1}{\mathrm{vol}(S)} + \frac{1}{\mathrm{vol}(\bar{S})}} = \mathrm{cut}(S, \bar{S}) \left(\frac{1}{\mathrm{vol}(S)} + \frac{1}{\mathrm{vol}(\bar{S})}\right) \le \frac{2\,\mathrm{cut}(S, \bar{S})}{\mathrm{vol}(S)} = 2 h(G).$$
Dividing by $2$ yields $\nu_2 / 2 \le h(G)$. The upper bound $h(G) \le \phi_{\mathrm{sweep}} \le \sqrt{2 \nu_2}$ follows by Cauchy-Schwarz and the co-area formula applied to the level sets of $y = D^{-1/2} v_2$. $\blacksquare$
