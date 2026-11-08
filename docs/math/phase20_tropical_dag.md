# Phase 20: Tropical / Max-Plus Algebra DAG Critical-Path & Latency Calculus

**Author:** ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>  
**Runtime Module:** `crates/pandora-runtime/src/tropical_dag.rs`  
**CLI Surface:** `pandora runtime tropical`  
**Safety Invariant:** `#![forbid(unsafe_code)]` Strictly Enforced  

---

## 1. Problem Formulation & The Max-Plus Semiring

Complex multi-agent orchestration pipelines execute dependent tasks subject to concurrency barriers, synchronization joins, and cyclic feedback loops (such as iterative refinement, verification cycles, and streaming pipeline stages).

Let $G = (V, E, W)$ be a directed execution graph on $n = |V|$ agent task nodes, with edges $E \subseteq V \times V$ and edge latency weights $W_{ij} \in \mathbb{R}$ indicating the elapsed duration from task $i$ completing until task $j$ may commence.

### 1.1 The Max-Plus (Tropical) Semiring
The Max-Plus semiring is the algebraic structure:
$$\mathbb{T}_{\max} \triangleq \left( \mathbb{R}_{\max}, \oplus, \otimes \right), \quad \mathbb{R}_{\max} \triangleq \mathbb{R} \cup \{-\infty\},$$
equipped with two binary operations:
1. **Tropical Addition ($\oplus$):** Represents synchronization maximization / join:
   $$a \oplus b \triangleq \max(a, b), \quad \text{identity element } \mathbb{0} \triangleq -\infty.$$
   It is commutative, associative, and idempotent ($a \oplus a = a$).
2. **Tropical Multiplication ($\otimes$):** Represents causal delay composition:
   $$a \otimes b \triangleq a + b, \quad \text{identity element } \mathbb{1} \triangleq 0.$$
   It is commutative, associative, and distributes over $\oplus$:
   $$a \otimes (b \oplus c) = a + \max(b, c) = \max(a + b, a + c) = (a \otimes b) \oplus (a \otimes c).$$
3. **Absorbing Element:**
   $$a \otimes \mathbb{0} = a + (-\infty) = -\infty = \mathbb{0} \quad \forall a \in \mathbb{R}_{\max}.$$

### 1.2 Matrix Algebra over $\mathbb{T}_{\max}^{n \times n}$
For matrices $A, B \in \mathbb{T}_{\max}^{n \times n}$:
$$(A \oplus B)_{ij} = \max\left(A_{ij}, B_{ij}\right),$$
$$(A \otimes B)_{ij} = \bigoplus_{k=1}^n \left( A_{ik} \otimes B_{kj} \right) = \max_{1 \le k \le n} \left( A_{ik} + B_{kj} \right).$$
The tropical identity matrix $I \in \mathbb{T}_{\max}^{n \times n}$ has $I_{ii} = 0$ ($\mathbb{1}$) and $I_{ij} = -\infty$ ($\mathbb{0}$) for $i \ne j$.
Matrix power $A^k = A \otimes A^{k-1}$ ($A^0 = I$) computes the maximum weight of any directed path from node $i$ to node $j$ traversing **exactly $k$ edges**:
$$(A^k)_{ij} = \max_{p \in \mathcal{P}_k(i, j)} \sum_{e \in p} w_e.$$

---

## 2. Theorem 39: Max-Plus Semiring Kleene Star $A^*$ Convergence & Positive-Cycle Detection for Multi-Agent Workflow DAGs

### Statement
Let $A \in \mathbb{T}_{\max}^{n \times n}$ be the adjacency weight matrix of a workflow graph $G$ on $n$ tasks:
$$A_{ij} = \begin{cases} w_{ij} & \text{if } (i, j) \in E, \\ -\infty & \text{otherwise}. \end{cases}$$
The **Kleene Star** (all-pairs longest path matrix / closure operator) is defined as the formal power series:
$$A^\star \triangleq \bigoplus_{k=0}^\infty A^k = I \oplus A \oplus A^2 \oplus A^3 \oplus \dots$$
Then:
1. **Convergence Condition & Stabilization:** The Kleene star series $A^\star$ converges in $\mathbb{T}_{\max}^{n \times n}$ (all entries are finite or $-\infty$, with zero $+\infty$) if and only if $G$ contains **no directed cycle of strictly positive weight**:
   $$\forall \text{ directed cycles } C \text{ in } G, \quad w(C) \triangleq \sum_{e \in C} w_e \le 0.$$
   When this holds, the series terminates and stabilizes at degree $n - 1$:
   $$A^\star = \bigoplus_{k=0}^{n-1} A^k = I \oplus A \oplus A^2 \oplus \dots \oplus A^{n-1}.$$
2. **DAG Nilpotency & Critical Path:**
   If $G$ is a Directed Acyclic Graph (DAG), then $A$ is topologically nilpotent:
   $$A^k = [-\infty]_{n \times n} \quad \forall k \ge n.$$
   For all $i, j \in [n]$, $(A^\star)_{ij}$ is the exact critical path latency from task $i$ to task $j$.
   The makespan of the workflow is:
   $$\text{Makespan}(G) = \max_{i, j \in [n]} (A^\star)_{ij}.$$
3. **Algebraic Positive-Cycle / Deadlock Detection:**
   $G$ contains a cycle of strictly positive weight if and only if:
   $$\exists i \in [n], \; 1 \le k \le n \quad \text{s.t.} \quad (A^k)_{ii} > 0.$$
   Moreover, the tropical Floyd-Warshall algorithm evaluates $A^\star$ and certifies acyclicity / absence of positive cycles in $O(n^3)$ operations.

### Mathematical Proof

#### Part 1: Path Interpretation of Tropical Matrix Powers
We establish by induction that $(A^k)_{ij}$ is the maximum weight of any directed path from $i$ to $j$ with exactly $k$ edges.
- *Base case $k=1$:* $(A^1)_{ij} = A_{ij}$, which is $w_{ij}$ if an edge exists and $-\infty$ otherwise.
- *Inductive step:* Suppose $(A^{k-1})_{im}$ is the maximum weight of a path of length $k-1$ from $i$ to $m$. Then:
  $$(A^k)_{ij} = (A^{k-1} \otimes A)_{ij} = \max_{1 \le m \le n} \left( (A^{k-1})_{im} + A_{mj} \right).$$
  Any path of length $k$ from $i$ to $j$ consists of a path of length $k-1$ from $i$ to some intermediate node $m$, followed by the final edge $(m, j)$. Maximizing over all $m \in [n]$ selects the path of length $k$ with maximal total weight.

#### Part 2: Cycle Decomposition for Long Paths ($k \ge n$)
Let $p = (v_0, v_1, \dots, v_k)$ be a directed path from $i = v_0$ to $j = v_k$ of length $k \ge n$.
The path contains $k + 1 \ge n + 1$ vertices.
By the Dirichlet Pigeonhole Principle, since $|V| = n$, at least one vertex must appear at least twice in the sequence:
$$\exists 0 \le r < s \le k \quad \text{such that} \quad v_r = v_s.$$
Thus, the subpath $C = (v_r, v_{r+1}, \dots, v_s)$ forms a directed cycle.
We can excise cycle $C$ from path $p$, leaving a strictly shorter path $p \setminus C$ from $i$ to $j$.
By repeating this decomposition, any path $p$ of length $k \ge n$ can be decomposed into:
1. An elementary simple path $p_{\mathrm{simple}}$ from $i$ to $j$ of length $|p_{\mathrm{simple}}| \le n - 1$.
2. A finite collection of simple directed cycles $C_1, \dots, C_m$ ($m \ge 1$):
   $$w(p) = w(p_{\mathrm{simple}}) + \sum_{\ell=1}^m w(C_\ell).$$

#### Part 3: Stabilization under Non-Positive Cycle Weights
Suppose every directed cycle in $G$ satisfies $w(C) \le 0$. Then $\sum_{\ell=1}^m w(C_\ell) \le 0$, which implies:
$$w(p) \le w(p_{\mathrm{simple}}).$$
Since $p_{\mathrm{simple}}$ has length at most $n - 1$:
$$w(p_{\mathrm{simple}}) \le \max_{0 \le s \le n-1} (A^s)_{ij}.$$
Taking the maximum over all paths of length $k \ge n$ from $i$ to $j$:
$$(A^k)_{ij} \le \bigoplus_{s=0}^{n-1} (A^s)_{ij}.$$
Therefore, for any upper index $M \ge n - 1$:
$$\bigoplus_{k=0}^M A^k = \bigoplus_{s=0}^{n-1} A^s.$$
Taking the limit as $M \to \infty$, the Kleene star series stabilizes:
$$A^\star = \bigoplus_{s=0}^{n-1} A^s.$$
Every entry $(A^\star)_{ij}$ is either finite (the length of the longest simple path) or $-\infty$ (if no path exists).

#### Part 4: Divergence under Positive Cycles
Conversely, suppose there exists a directed cycle $C$ with $w(C) > 0$ passing through vertex $v$.
Construct a closed walk from $v$ to $v$ traversing cycle $C$ repeatedly $m$ times, giving length $k = m |C|$ and weight:
$$w(p_m) = m \cdot w(C).$$
As $m \to \infty$, $(A^{m|C|})_{vv} \ge m \cdot w(C) \to +\infty$.
Hence the diagonal entry $(A^\star)_{vv}$ diverges to $+\infty$.
Therefore, $A^\star$ converges if and only if no cycle has positive weight. Furthermore, a positive cycle exists if and only if $(A^k)_{ii} > 0$ for some $1 \le k \le n$. $\blacksquare$

---

## 3. Theorem 40: Karp's Maximum Cycle Mean Theorem for Periodic Pipeline Throughput & SLA Deadlock Prevention

### Statement
Consider a recurrent or periodic multi-agent workflow where agents execute iteratively in cycles, modeled by the tropical dynamic recurrence:
$$\mathbf{x}(t+1) = A \otimes \mathbf{x}(t), \quad t = 0, 1, 2, \dots$$
where $x_i(t)$ represents the initiation timestamp of task $i$ at pipeline iteration $t$.
Let $G = (V, E, W)$ be the strongly connected dependency digraph with matrix $A \in \mathbb{T}_{\max}^{n \times n}$.
The **maximum cycle mean** $\lambda(G)$ is defined as:
$$\lambda(G) \triangleq \max_{\text{directed cycles } C \text{ in } G} \frac{w(C)}{|C|} = \max_{C} \frac{\sum_{e \in C} w_e}{|C|}.$$

Then:
1. **Karp's Minimax Formula:** For any vertex $i \in V$ in strongly connected digraphs:
   $$\lambda(G) = \max_{i \in V} \min_{0 \le k < n} \frac{(A^n)_{ii} - (A^k)_{ii}}{n - k}.$$
   More generally, across arbitrary directed graphs (including disconnected components and cycles of lengths not dividing $n$), connecting a virtual super-source $s^\star$ with zero-weight edges to all vertices defines the universal all-predecessors path table:
   $$F_k(v) \triangleq \max_{u \in V} (A^k)_{uv}$$
   representing the maximum weight of any walk of length $k$ terminating at $v$. Then Karp's maximum cycle mean is given by:
   $$\lambda(G) = \max_{v \in V: F_n(v) > -\infty} \min_{0 \le k < n, F_k(v) > -\infty} \frac{F_n(v) - F_k(v)}{n - k}.$$
2. **Tropical Eigenvalue Characterization:** $\lambda(G)$ is the unique tropical eigenvalue of matrix $A$:
   $$\exists \mathbf{v} \in \mathbb{R}^n \quad \text{s.t.} \quad A \otimes \mathbf{v} = \lambda(G) \otimes \mathbf{v} \iff \max_{1 \le j \le n} \left( A_{ij} + v_j \right) = \lambda(G) + v_i \quad \forall i \in [n].$$
3. **Pipeline Throughput & SLA Deadlock Prevention:**
   - The maximum sustainable throughput of the periodic multi-agent pipeline is:
     $$\Theta_{\max} = \frac{1}{\lambda(G)} \quad \text{iterations / second}.$$
   - Given a maximum allowable SLA period $T_{\mathrm{SLA}}$, the multi-agent pipeline is certified **deadlock-free and bounded** if and only if:
     $$\lambda(G) \le T_{\mathrm{SLA}}.$$
   - If $\lambda(G) > T_{\mathrm{SLA}}$, pipeline queues diverge at rate $\lambda(G) - T_{\mathrm{SLA}} > 0$ per iteration, inducing unbounded latency cascades.

### Mathematical Proof

#### Step 1: Normalization via Shifted Matrix $B$
Let $\lambda \triangleq \lambda(G)$. Define the shifted matrix $B \in \mathbb{T}_{\max}^{n \times n}$ by subtracting $\lambda$ from every edge weight:
$$B_{ij} \triangleq A_{ij} - \lambda.$$
For any directed cycle $C$ in $G$:
$$w_B(C) = \sum_{e \in C} (w_e - \lambda) = w_A(C) - |C| \lambda.$$
By definition $\lambda = \max_C \frac{w_A(C)}{|C|}$, so $\frac{w_A(C)}{|C|} \le \lambda \implies w_B(C) \le 0$ for all cycles $C$.
Moreover, for any critical cycle $C^\star$ achieving the maximum cycle mean, $\frac{w_A(C^\star)}{|C^\star|} = \lambda$, so:
$$w_B(C^\star) = 0.$$
Thus, in graph $G_B$, **no directed cycle has positive weight**, and **at least one cycle has weight exactly zero**.

#### Step 2: Relation Between Powers of $A$ and $B$
Any path $p$ of length $k$ in $G_B$ has weight $w_B(p) = w_A(p) - k \lambda$. Therefore:
$$(B^k)_{ij} = (A^k)_{ij} - k \lambda \iff (A^k)_{ij} = (B^k)_{ij} + k \lambda.$$
In terms of distances from the virtual super-source, letting $F_k^B(v) \triangleq \max_u (B^k)_{uv}$:
$$F_k(v) = F_k^B(v) + k \lambda.$$

#### Step 3: Upper Bounding Paths in $G_B$
Since $G_B$ has no positive cycles, by Theorem 39 all paths of length $k \ge n$ have weight at most that of their elementary subpath (length $< n$).
Therefore, for every vertex $v \in V$:
$$\max_{k \ge 0} F_k^B(v) = \max_{0 \le k < n} F_k^B(v).$$
In particular, for length $n$:
$$F_n^B(v) \le \max_{0 \le k < n} F_k^B(v).$$
Choose $k^\star(v) \in \{0, \dots, n-1\}$ that achieves this maximum: $F_{k^\star(v)}^B(v) = \max_{0 \le k < n} F_k^B(v)$.
Then:
$$F_n^B(v) \le F_{k^\star(v)}^B(v).$$
Substituting $F_m^B(v) = F_m(v) - m \lambda$:
$$F_n(v) - n \lambda \le F_{k^\star(v)}(v) - k^\star(v) \lambda.$$
Rearranging terms:
$$F_n(v) - F_{k^\star(v)}(v) \le (n - k^\star(v)) \lambda \implies \lambda \ge \frac{F_n(v) - F_{k^\star(v)}(v)}{n - k^\star(v)}.$$
Since $k^\star(v) < n$, the minimum over all $k \in \{0, \dots, n-1\}$ is bounded by:
$$\min_{0 \le k < n} \frac{F_n(v) - F_k(v)}{n - k} \le \frac{F_n(v) - F_{k^\star(v)}(v)}{n - k^\star(v)} \le \lambda.$$
Because this holds for every vertex $v \in V$:
$$\max_{v \in V} \min_{0 \le k < n} \frac{F_n(v) - F_k(v)}{n - k} \le \lambda.$$

#### Step 4: Achieving Equality on the Critical 0-Weight Cycle
Let $u$ be any vertex lying on the critical cycle $C^\star$ where $w_B(C^\star) = 0$.
Let $L = |C^\star| \in \{1, \dots, n\}$ be the cycle length.
Traversing $C^\star$ produces paths in $G_B$ with identical weight $F_{k + m L}^B(u) = F_k^B(u)$ for all integers $m \ge 0$.
Therefore, a path of length $n$ extending from the virtual super-source around $C^\star$ achieves the global maximum weight:
$$F_n^B(u) = \max_{0 \le k < n} F_k^B(u).$$
Hence, for this vertex $u$, there exists $k \in \{0, \dots, n-1\}$ such that:
$$F_n^B(u) = F_k^B(u) \iff F_n(u) - n \lambda = F_k(u) - k \lambda \iff \frac{F_n(u) - F_k(u)}{n - k} = \lambda.$$
Since for all other $j < n$, $F_n^B(u) \ge F_j^B(u) \implies \frac{F_n(u) - F_j(u)}{n - j} \ge \lambda$, the minimum over $k$ for this vertex $u$ is exactly $\lambda$:
$$\min_{0 \le k < n} \frac{F_n(u) - F_k(u)}{n - k} = \lambda.$$
Taking the maximum over all $v \in V$ yields exact equality:
$$\max_{v \in V} \min_{0 \le k < n} \frac{F_n(v) - F_k(v)}{n - k} = \lambda = \lambda(G).$$
This proves Karp's theorem in full. $\blacksquare$

---

## 4. Algorithmic Specification: Tropical Dynamic Programming & Karp Solver

```
Algorithm: Karp's Maximum Cycle Mean Algorithm (Universal Multi-Component Form)
Input:
  - Directed graph G = (V, E, W) with n = |V| vertices
  - Adjacency weight matrix A in (R U {-inf})^(n x n)
Output:
  - lambda(G) in R (maximum cycle mean)
  - Critical cycle nodes

1. Initialize 2D table F[k][v] for k in 0..=n, v in 0..n-1 with -infinity
2. For all v in 0..n-1:
     F[0][v] <- 0.0  (virtual super-source s* with 0-weight edges)
3. For k = 1 to n:
     For v = 0 to n - 1:
       For u = 0 to n - 1:
         If F[k - 1][u] > -infinity and A[u][v] > -infinity:
           F[k][v] <- max(F[k][v], F[k - 1][u] + A[u][v])

4. lambda_star <- -infinity
5. For v = 0 to n - 1:
     If F[n][v] <= -infinity:
       Continue
     min_ratio <- +infinity
     For k = 0 to n - 1:
       If F[k][v] > -infinity:
         ratio <- (F[n][v] - F[k][v]) / (n - k)
         min_ratio <- min(min_ratio, ratio)
     If min_ratio < +infinity and min_ratio > lambda_star:
       lambda_star <- min_ratio

6. If lambda_star <= -infinity:
     Return GraphAcyclicOrUnreachable
   Return lambda_star

Complexity: Step 4 performs $n$ matrix-vector tropical products in $O(n \cdot |E|) \le O(n^3)$ operations. Step 6 evaluates the minimax formula in $O(n^2)$ time. Total space complexity is $O(n^2)$.

---

## 5. Rust Data Structures, Enums & Error Handling

```rust
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum TropicalError {
    #[error("Invalid vertex count: must have n >= 1, got {0}")]
    InvalidVertexCount(usize),
    #[error("Matrix dimension mismatch: expected {expected}x{expected}, got {rows}x{cols}")]
    DimensionMismatch { expected: usize, rows: usize, cols: usize },
    #[error("Positive cycle detected in DAG: task {vertex} has positive return weight {weight}")]
    PositiveCycleDetected { vertex: usize, weight: f64 },
    #[error("Graph is not strongly connected or unreachable from source {source}")]
    GraphUnreachable { source: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TropicalSlaVerdict {
    SlaCompliant,
    SlaViolatedDeadlockRisk,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriticalPathResult {
    pub num_tasks: usize,
    pub is_acyclic: bool,
    pub critical_path_makespan: f64,
    pub critical_path_nodes: Vec<usize>,
    pub kleene_star_matrix: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CycleMeanResult {
    pub maximum_cycle_mean: f64,
    pub maximum_throughput: f64,
    pub sla_period_limit: f64,
    pub verdict: TropicalSlaVerdict,
}
```

---

## 6. Concrete Numerical Test Cases & Verification Assertions

### Test Case 1: 3-Node Workflow DAG Critical Path & Kleene Star
- **Nodes:** $V = \{0, 1, 2\}$.
- **Edges:** $(0, 1)$ with latency $3.0$; $(1, 2)$ with latency $4.0$; $(0, 2)$ with latency $5.0$.
- **Adjacency Matrix:**
  $$A = \begin{pmatrix} -\infty & 3.0 & 5.0 \\ -\infty & -\infty & 4.0 \\ -\infty & -\infty & -\infty \end{pmatrix}.$$
- **Powers:**
  - $A^1_{01} = 3.0$, $A^1_{12} = 4.0$, $A^1_{02} = 5.0$.
  - $A^2_{02} = A_{01} + A_{12} = 3.0 + 4.0 = 7.0$. All other entries $-\infty$.
  - $A^3 = [-\infty]_{3 \times 3}$ (nilpotent at $k \ge n$).
- **Kleene Star $A^\star = I \oplus A \oplus A^2$:**
  $$(A^\star)_{02} = \max(5.0, 7.0) = 7.0.$$
- **Assertion:** Makespan is $7.0$ (via path $0 \to 1 \to 2$), not $5.0$.

### Test Case 2: Karp Cycle Mean on Cyclic 3-Node Feedback Pipeline
- **Cycle 1:** $0 \to 1 \to 0$ with weights $w(0, 1) = 4.0$, $w(1, 0) = 2.0$. Length $2$, total weight $6.0 \implies \text{Mean} = 3.0$.
- **Cycle 2:** $1 \to 2 \to 1$ with weights $w(1, 2) = 5.0$, $w(2, 1) = 5.0$. Length $2$, total weight $10.0 \implies \text{Mean} = 5.0$.
- **Cycle 3:** $0 \to 1 \to 2 \to 0$ with weights $w(0, 1) = 4.0, w(1, 2) = 5.0, w(2, 0) = 3.0$. Length $3$, total weight $12.0 \implies \text{Mean} = 4.0$.
- **Maximum Cycle Mean:** $\lambda(G) = \max(3.0, 5.0, 4.0) = 5.0$.
- **Throughput:** $\Theta_{\max} = 1 / 5.0 = 0.20$ iterations / unit time.
- **SLA Check:** For $T_{\mathrm{SLA}} = 6.0$, $\lambda(G) = 5.0 \le 6.0 \implies$ `SlaCompliant`.
- **Assertion:** Karp minimax formula returns exactly $\lambda(G) = 5.00000$.
