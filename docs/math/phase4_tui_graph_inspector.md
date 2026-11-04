# Mathematical Foundations & Protocol Specification: Steerable TUI Visual Graph Inspector

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Version:** AGENT-PANDORA v3.0.4 (Phase 4)  
**Safety Invariant:** `#![forbid(unsafe_code)]` Strictly Enforced  

---

## 1. Executive Summary & Problem Formulation

In complex multi-agent execution runs and large codebases, agents produce intricate memory associations (`ContextVfs` and `SemanticMemory`) and multi-task dependencies (`TaskDag`). Understanding, debugging, and steering these topologies in real-time requires interactive visual projection onto low-overhead terminal interfaces (Ratatui/Crossterm) without external browser or GUI dependencies.

Phase 4 establishes the mathematical mechanics for:
1. **Force-Directed Graph Equilibrium (Fruchterman-Reingold / Eades model)**: Physical simulation of spring-electrical forces converging to minimal potential energy $E(\mathbf{x})$.
2. **Aspect-Ratio Compensated Character Cell Rasterization**: Geometric scaling mapping continuous Euclidean coordinates $\mathbf{x} \in \mathbb{R}^2$ to non-square discrete terminal character cells ($2:1$ aspect ratio).
3. **Interactive Viewport Affine Transforms**: Steerable camera mechanics (Pan, Zoom, Focus, Neighborhood Filtering) with invariant bounds.

---

## 2. Force-Directed Layout & Potential Energy Minimization

### 2.1 The Spring-Electrical Energy Functional

Let a graph be defined as $G = (V, E)$, with $|V| = n$ vertices and $|E| = m$ edges. Each vertex $u \in V$ is assigned a continuous coordinate $\mathbf{x}_u = (x_u, y_u) \in \mathbb{R}^2$.

The total potential energy $E(\mathbf{x})$ of the layout is defined by the Hamiltonian:
$$E(\mathbf{x}) = E_{\text{spring}}(\mathbf{x}) + E_{\text{repulsion}}(\mathbf{x})$$

$$E(\mathbf{x}) = \sum_{(u, v) \in E} \frac{k_a}{2} \|\mathbf{x}_u - \mathbf{x}_v\|^2 + \sum_{u \ne v} \frac{k_r}{\|\mathbf{x}_u - \mathbf{x}_v\|}$$

where:
- $k_a$ is the spring stiffness constant for connected nodes.
- $k_r$ is the electrical permittivity constant driving repulsion between all pairs of nodes.
- $k = C \sqrt{\frac{A}{|V|}}$ is the natural spring length, with area $A = \text{Width} \times \text{Height}$ and scaling constant $C \approx 0.75$.

### 2.2 Vector Field of Net Forces

The force $\mathbf{F}(u)$ acting on vertex $u$ is the negative gradient of the potential energy with respect to $\mathbf{x}_u$:
$$\mathbf{F}(u) = - \nabla_{\mathbf{x}_u} E(\mathbf{x}) = \mathbf{F}_a(u) + \mathbf{F}_r(u)$$

1. **Attractive Spring Force ($\mathbf{F}_a$)**:
   $$\mathbf{F}_a(u) = \sum_{v \in \mathcal{N}(u)} \frac{\|\mathbf{x}_v - \mathbf{x}_u\|^2}{k} \cdot \frac{\mathbf{x}_v - \mathbf{x}_u}{\|\mathbf{x}_v - \mathbf{x}_u\|}$$
2. **Repulsive Electrostatic Force ($\mathbf{F}_r$)**:
   $$\mathbf{F}_r(u) = - \sum_{v \in V \setminus \{u\}} \frac{k^2}{\|\mathbf{x}_v - \mathbf{x}_u\|^2} \cdot \frac{\mathbf{x}_v - \mathbf{x}_u}{\|\mathbf{x}_v - \mathbf{x}_u\|}$$

### 2.3 Theorem 6: Damped Convergence to Mechanical Equilibrium

**Theorem:**  
*Under velocity-damped numerical integration:*
$$\mathbf{v}_u(t + \Delta t) = \gamma \mathbf{v}_u(t) + \mathbf{F}(u) \Delta t$$
$$\mathbf{x}_u(t + \Delta t) = \mathbf{x}_u(t) + \mathbf{v}_u(t + \Delta t) \Delta t$$
*with damping parameter $\gamma \in (0, 1)$ (strictly $\gamma = 0.85$) and time step $\Delta t \le \sqrt{\frac{k}{\max_u \|\mathbf{F}(u)\|}}$:*
1. *The total kinetic energy $K(t) = \frac{1}{2} \sum_{u \in V} \|\mathbf{v}_u(t)\|^2$ strictly decays toward zero:*
   $$\lim_{t \to \infty} K(t) = 0$$
2. *The configuration $\mathbf{x}^* = \lim_{t \to \infty} \mathbf{x}(t)$ attains static equilibrium satisfying:*
   $$\forall u \in V, \quad \|\mathbf{F}(u)\| = 0$$

**Proof:**  
Consider the Lyapunov candidate function $V(\mathbf{x}, \mathbf{v}) = E(\mathbf{x}) + K(\mathbf{v}) = E(\mathbf{x}) + \frac{1}{2} \sum_u \|\mathbf{v}_u\|^2$.
Taking the time derivative along trajectories:
$$\frac{d}{dt} V(\mathbf{x}, \mathbf{v}) = \sum_{u \in V} \nabla_{\mathbf{x}_u} E(\mathbf{x}) \cdot \mathbf{v}_u + \sum_{u \in V} \mathbf{v}_u \cdot \frac{d\mathbf{v}_u}{dt}$$

From Newton's second law with damping $\frac{d\mathbf{v}_u}{dt} = \mathbf{F}(u) - (1 - \gamma)\mathbf{v}_u = -\nabla_{\mathbf{x}_u} E(\mathbf{x}) - (1 - \gamma)\mathbf{v}_u$:
$$\frac{d}{dt} V(\mathbf{x}, \mathbf{v}) = \sum_{u \in V} \nabla_{\mathbf{x}_u} E(\mathbf{x}) \cdot \mathbf{v}_u + \sum_{u \in V} \mathbf{v}_u \cdot \left[ -\nabla_{\mathbf{x}_u} E(\mathbf{x}) - (1 - \gamma)\mathbf{v}_u \right]$$
$$\frac{d}{dt} V(\mathbf{x}, \mathbf{v}) = - (1 - \gamma) \sum_{u \in V} \|\mathbf{v}_u\|^2 = - 2 (1 - \gamma) K(\mathbf{v}) \le 0$$

Since $0 < \gamma < 1$, $1 - \gamma > 0$, so $\frac{dV}{dt} \le 0$ with equality if and only if $\mathbf{v}_u = \mathbf{0}$ for all $u$.
By LaSalle's Invariance Principle, the system asymptotically converges to the largest invariant set where $\mathbf{v} = \mathbf{0}$. On this set, $\frac{d\mathbf{v}_u}{dt} = \mathbf{0} \implies \mathbf{F}(u) = \mathbf{0}$ for all $u \in V$.
$\blacksquare$

---

## 3. Terminal Grid Projection & Aspect Ratio Calibration

Terminal emulators display monospaced fonts where character cells typically have a height-to-width ratio of approximately $2.0:1$ (e.g., 8 pixels wide $\times$ 16 pixels high).

### 3.1 Affine Viewport Mapping

Let world coordinates be $\mathbf{x} = (x, y) \in [\min_x, \max_x] \times [\min_y, \max_y]$.
Given viewport camera parameters:
- Pan offset: $\mathbf{c} = (c_x, c_y) \in \mathbb{R}^2$
- Zoom factor: $z > 0$
- Terminal screen dimensions: $W_{\text{term}}$ columns, $H_{\text{term}}$ rows.

The world-to-screen projection $\Pi: \mathbb{R}^2 \to [0, W_{\text{term}}-1] \times [0, H_{\text{term}}-1]$ is given by:
$$\text{col} = \left\lfloor \frac{W_{\text{term}}}{2} + z \cdot s_x \cdot (x - c_x) \right\rfloor$$
$$\text{row} = \left\lfloor \frac{H_{\text{term}}}{2} + z \cdot s_y \cdot (y - c_y) \right\rfloor$$

To preserve circular symmetry (ensuring equal Euclidean distances in world space appear geometrically isotropic in character cells):
$$s_x = 2.0 \cdot s_y$$
with base scale $s_y = \frac{\min(W_{\text{term}}/2, H_{\text{term}})}{\max(\Delta x, \Delta y)}$.

---

## 4. Interactive Steerability Invariants

1. **Focus Centering**: Focusing node $u$ assigns camera offset $\mathbf{c} = \mathbf{x}_u$.
2. **Neighborhood Horizon $k$-Hop Filtering**:
   $$\mathcal{N}_k(u) = \{v \in V \mid \text{dist}_G(u, v) \le k\}$$
   Subgraph induction on $\mathcal{N}_k(u)$ isolates relevant subagent dependency clusters in $O(|V| + |E|)$ via breadth-first search (BFS).
3. **Bresenham Terminal Line Rasterization**: Directed edges $(u, v)$ render connecting lines and direction arrows `->`, `<-`, `^`, `v` across character cells without floating-point aliasing.
