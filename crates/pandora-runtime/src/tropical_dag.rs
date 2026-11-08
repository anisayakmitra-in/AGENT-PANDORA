#![forbid(unsafe_code)]

//! Phase 20: Tropical / Max-Plus Algebra DAG Critical-Path & Latency Calculus
//!
//! Implements the idempotent Max-Plus semiring $(\mathbb{R} \cup \{-\infty\}, \oplus, \otimes)$,
//! Kleene star $A^\star$ convergence, workflow DAG critical-path makespan stabilization,
//! algebraic positive-cycle detection in $O(n^3)$, and Karp's maximum cycle mean theorem
//! $\lambda(G)$ for periodic pipeline throughput and SLA deadlock prevention.
//!
//! ## Mathematical Guarantees
//!
//! 1. **Theorem 39 (Max-Plus Semiring Kleene Star $A^\star$ Convergence & Makespan):**
//!    In the Max-Plus semiring $\mathbb{T}_{\max} = (\mathbb{R} \cup \{-\infty\}, \oplus, \otimes)$
//!    with operations:
//!    $$a \oplus b \triangleq \max(a, b), \qquad a \otimes b \triangleq a + b,$$
//!    zero element $\mathbb{O} = -\infty$, and unit element $\mathbb{I} = 0$, matrix powers
//!    $(A^k)_{ij}$ compute the longest path from node $i$ to node $j$ traversing exactly $k$ edges.
//!    - The Kleene star series $A^\star \triangleq \bigoplus_{k=0}^\infty A^k$ converges with
//!      finite entries if and only if digraph $G$ contains no directed cycle of positive weight
//!      ($w(C) \le 0$).
//!    - When $G$ is a Directed Acyclic Graph (DAG), $A^k = [-\infty]$ for all $k \ge n$, and the
//!      series terminates at step $n - 1$:
//!      $$A^\star = \bigoplus_{k=0}^{n-1} A^k = I \oplus A \oplus A^2 \oplus \dots \oplus A^{n-1}.$$
//!      The entry $(A^\star)_{ij}$ is the exact critical path latency from task $i$ to task $j$,
//!      and the overall workflow makespan is $\max_{i, j} (A^\star)_{ij}$.
//!    - A positive cycle exists if and only if $\exists i \in [n], 1 \le k \le n: (A^k)_{ii} > 0$.
//!
//! 2. **Theorem 40 (Karp's Maximum Cycle Mean & SLA Deadlock Condition):**
//!    For recurrent workflow digraph $G$ with adjacency matrix $A \in \mathbb{T}_{\max}^{n \times n}$,
//!    the maximum cycle mean $\lambda(G) \triangleq \max_{C} \frac{w(C)}{|C|}$ is the unique tropical
//!    eigenvalue satisfying $A \otimes \mathbf{v} = \lambda(G) \otimes \mathbf{v}$, and is given
//!    by Karp's exact minimax characterization:
//!    $$\lambda(G) = \max_{v \in V: F_n(v) > -\infty} \min_{0 \le k < n, F_k(v) > -\infty} \frac{F_n(v) - F_k(v)}{n - k}, \quad F_k(v) \triangleq \max_{u \in V} (A^k)_{uv}.$$
//!    The maximum sustainable periodic pipeline throughput is:
//!    $$\Theta_{\max} = \frac{1}{\max(\lambda(G), 10^{-12})},$$
//!    and the pipeline is certified SLA compliant and deadlock-free if and only if
//!    $$\lambda(G) \le T_{\mathrm{SLA}}.$$

use serde::{Deserialize, Serialize};
use std::fmt;

/// Negative infinity constant representing the zero element $\mathbb{O}$ of the Max-Plus semiring.
pub const NEG_INFINITY: f64 = f64::NEG_INFINITY;

/// Errors returned by the tropical algebra DAG engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TropicalError {
    /// Matrix dimensions cannot be empty.
    EmptyMatrix,
    /// Dimensions of matrices do not match for addition or multiplication.
    DimensionMismatch { expected: usize, actual: usize },
    /// Node index is out of matrix bounds.
    NodeOutOfBounds { node: usize, size: usize },
    /// Edge weight is invalid (e.g. NaN or positive infinity).
    InvalidWeight(f64),
    /// Positive cycle detected in workflow DAG (causes infinite latency accumulation).
    PositiveCycleDetected { node: usize, cycle_weight: f64 },
    /// Graph contains no cycles or is completely disconnected for cycle mean evaluation.
    GraphAcyclicOrUnreachable,
    /// SLA threshold must be non-negative.
    InvalidSlaThreshold(f64),
}

impl fmt::Display for TropicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMatrix => write!(f, "tropical matrix cannot be empty (size 0)"),
            Self::DimensionMismatch { expected, actual } => {
                write!(f, "dimension mismatch: expected {expected}, got {actual}")
            }
            Self::NodeOutOfBounds { node, size } => {
                write!(
                    f,
                    "node index {node} out of bounds for matrix of size {size}"
                )
            }
            Self::InvalidWeight(w) => write!(f, "invalid tropical weight {w}"),
            Self::PositiveCycleDetected { node, cycle_weight } => {
                write!(
                    f,
                    "positive cycle detected at node {node} with weight +{cycle_weight:.4} (violates DAG convergence)"
                )
            }
            Self::GraphAcyclicOrUnreachable => {
                write!(f, "graph is acyclic or contains no reachable closed cycles")
            }
            Self::InvalidSlaThreshold(t) => {
                write!(f, "SLA threshold ({t}) must be non-negative")
            }
        }
    }
}

impl std::error::Error for TropicalError {}

/// Dense Max-Plus matrix $A \in (\mathbb{R} \cup \{-\infty\})^{n \times n}$.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaxPlusMatrix {
    /// Dimension $n$ of the square matrix.
    pub size: usize,
    /// Row-major matrix elements ($A_{ij} = \text{data}[i][j]$).
    pub data: Vec<Vec<f64>>,
}

/// Type alias for compatibility with `PROJECT.md`.
pub type TropicalMatrix = MaxPlusMatrix;

impl MaxPlusMatrix {
    /// Create a zero matrix $\mathbb{O}_{n \times n}$ where all elements are $-\infty$.
    pub fn zero(size: usize) -> Result<Self, TropicalError> {
        if size == 0 {
            return Err(TropicalError::EmptyMatrix);
        }
        Ok(Self {
            size,
            data: vec![vec![NEG_INFINITY; size]; size],
        })
    }

    /// Create an identity matrix $I_n$ where $I_{ii} = 0$ and $I_{ij} = -\infty$ for $i \ne j$.
    pub fn identity(size: usize) -> Result<Self, TropicalError> {
        if size == 0 {
            return Err(TropicalError::EmptyMatrix);
        }
        let mut mat = vec![vec![NEG_INFINITY; size]; size];
        for (i, row) in mat.iter_mut().enumerate().take(size) {
            row[i] = 0.0;
        }
        Ok(Self { size, data: mat })
    }

    /// Create a matrix from raw 2D vector data, validating dimensions.
    pub fn from_data(data: Vec<Vec<f64>>) -> Result<Self, TropicalError> {
        let size = data.len();
        if size == 0 {
            return Err(TropicalError::EmptyMatrix);
        }
        for row in &data {
            if row.len() != size {
                return Err(TropicalError::DimensionMismatch {
                    expected: size,
                    actual: row.len(),
                });
            }
            for &val in row {
                if val.is_nan() || val == f64::INFINITY {
                    return Err(TropicalError::InvalidWeight(val));
                }
            }
        }
        Ok(Self { size, data })
    }

    /// Get element $A_{ij}$.
    #[inline]
    pub fn get(&self, i: usize, j: usize) -> Result<f64, TropicalError> {
        if i >= self.size || j >= self.size {
            return Err(TropicalError::NodeOutOfBounds {
                node: i.max(j),
                size: self.size,
            });
        }
        Ok(self.data[i][j])
    }

    /// Set element $A_{ij}$.
    #[inline]
    pub fn set(&mut self, i: usize, j: usize, val: f64) -> Result<(), TropicalError> {
        if i >= self.size || j >= self.size {
            return Err(TropicalError::NodeOutOfBounds {
                node: i.max(j),
                size: self.size,
            });
        }
        if val.is_nan() || val == f64::INFINITY {
            return Err(TropicalError::InvalidWeight(val));
        }
        self.data[i][j] = val;
        Ok(())
    }

    /// Max-Plus addition: $(A \oplus B)_{ij} = \max(A_{ij}, B_{ij})$.
    pub fn tropical_add(&self, other: &Self) -> Result<Self, TropicalError> {
        if self.size != other.size {
            return Err(TropicalError::DimensionMismatch {
                expected: self.size,
                actual: other.size,
            });
        }
        let mut result = vec![vec![NEG_INFINITY; self.size]; self.size];
        for (i, row) in result.iter_mut().enumerate() {
            for (j, val) in row.iter_mut().enumerate() {
                *val = self.data[i][j].max(other.data[i][j]);
            }
        }
        Ok(Self {
            size: self.size,
            data: result,
        })
    }

    /// Max-Plus multiplication: $(A \otimes B)_{ij} = \bigoplus_{k=0}^{n-1} (A_{ik} + B_{kj})$.
    pub fn tropical_mul(&self, other: &Self) -> Result<Self, TropicalError> {
        if self.size != other.size {
            return Err(TropicalError::DimensionMismatch {
                expected: self.size,
                actual: other.size,
            });
        }
        let n = self.size;
        let mut result = vec![vec![NEG_INFINITY; n]; n];
        for (i, row) in result.iter_mut().enumerate() {
            for (j, val) in row.iter_mut().enumerate() {
                let mut max_sum = NEG_INFINITY;
                for k in 0..n {
                    let a_ik = self.data[i][k];
                    let b_kj = other.data[k][j];
                    if a_ik > NEG_INFINITY && b_kj > NEG_INFINITY {
                        let sum = a_ik + b_kj;
                        if sum > max_sum {
                            max_sum = sum;
                        }
                    }
                }
                *val = max_sum;
            }
        }
        Ok(Self {
            size: n,
            data: result,
        })
    }
}

/// Result of Kleene Star closure $A^\star$ and DAG makespan calculus (Theorem 39).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KleeneStarResult {
    /// Kleene star matrix $A^\star = \bigoplus_{k=0}^{n-1} A^k$.
    pub closure: MaxPlusMatrix,
    /// Critical path makespan $\max_{i, j} (A^\star)_{ij}$.
    pub critical_path_makespan: f64,
    /// Whether any directed positive cycle was detected.
    pub has_positive_cycle: bool,
    /// Number of matrix multiplication steps until stabilization.
    pub convergence_steps: usize,
    /// Individual matrix powers $A^0, A^1, \dots, A^n$.
    pub matrix_powers: Vec<MaxPlusMatrix>,
}

/// Result of Karp's maximum cycle mean theorem $\lambda(G)$ and SLA deadlock guard (Theorem 40).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KarpCycleMeanResult {
    /// Maximum cycle mean $\lambda(G) = \max_C \frac{w(C)}{|C|}$.
    pub maximum_cycle_mean: f64,
    /// Maximum sustainable pipeline throughput $\Theta_{\max} = 1 / \lambda(G)$.
    pub asymptotic_throughput: f64,
    /// Configured SLA cycle threshold $T_{\mathrm{SLA}}$.
    pub sla_threshold: f64,
    /// SLA compliance flag: $\lambda(G) \le T_{\mathrm{SLA}}$.
    pub sla_satisfied: bool,
    /// Vertex index achieving the maximum cycle mean.
    pub critical_vertex: usize,
}

/// Tropical DAG Critical-Path & Latency Calculus Engine.
#[derive(Debug, Clone, Default)]
pub struct TropicalDagScheduler;

/// Type alias for compatibility with `PROJECT.md`.
pub type TropicalDagEngine = TropicalDagScheduler;

impl TropicalDagScheduler {
    /// Create a new tropical scheduler engine.
    pub fn new() -> Self {
        Self
    }

    /// Compute Kleene star closure $A^\star = \bigoplus_{k=0}^{n-1} A^k$,
    /// detect positive cycles, and determine critical path makespan (Theorem 39).
    pub fn compute_kleene_star(
        &self,
        adjacency: &MaxPlusMatrix,
    ) -> Result<KleeneStarResult, TropicalError> {
        let n = adjacency.size;
        if n == 0 {
            return Err(TropicalError::EmptyMatrix);
        }

        let id = MaxPlusMatrix::identity(n)?;
        let mut powers = Vec::with_capacity(n + 1);
        powers.push(id.clone()); // A^0

        let current = adjacency.clone();
        powers.push(current.clone()); // A^1

        let mut closure = id.tropical_add(&current)?;
        let mut has_positive_cycle = false;
        let mut cycle_node = 0;
        let mut cycle_weight = 0.0;

        // Check A^1 for positive self-loops
        for i in 0..n {
            if current.data[i][i] > 1e-9 {
                has_positive_cycle = true;
                cycle_node = i;
                cycle_weight = current.data[i][i];
                break;
            }
        }

        let mut convergence_steps = 1;
        for k in 2..=n {
            let next_power = powers[k - 1].tropical_mul(adjacency)?;
            // Check for positive cycle of length k
            for i in 0..n {
                if next_power.data[i][i] > 1e-9 {
                    has_positive_cycle = true;
                    cycle_node = i;
                    cycle_weight = next_power.data[i][i];
                    break;
                }
            }

            if k < n {
                closure = closure.tropical_add(&next_power)?;
            }
            powers.push(next_power);
            convergence_steps = k;
            if has_positive_cycle {
                break;
            }
        }

        if has_positive_cycle {
            return Err(TropicalError::PositiveCycleDetected {
                node: cycle_node,
                cycle_weight,
            });
        }

        // Compute critical-path makespan: max_{i, j} (A^\star)_{ij}
        let mut makespan = 0.0_f64;
        for i in 0..n {
            for j in 0..n {
                let val = closure.data[i][j];
                if val > makespan {
                    makespan = val;
                }
            }
        }

        Ok(KleeneStarResult {
            closure,
            critical_path_makespan: makespan,
            has_positive_cycle: false,
            convergence_steps,
            matrix_powers: powers,
        })
    }

    /// Compute Karp's maximum cycle mean $\lambda(G) = \max_v \min_{k<n} \frac{F_n(v) - F_k(v)}{n - k}$
    /// where $F_k(v) = \max_u (A^k)_{uv}$ for periodic pipeline throughput and SLA deadlock verification (Theorem 40).
    pub fn compute_karp_maximum_cycle_mean(
        &self,
        adjacency: &MaxPlusMatrix,
    ) -> Result<KarpCycleMeanResult, TropicalError> {
        let n = adjacency.size;
        if n == 0 {
            return Err(TropicalError::EmptyMatrix);
        }

        // Compute powers A^0 through A^n
        let id = MaxPlusMatrix::identity(n)?;
        let mut powers = Vec::with_capacity(n + 1);
        powers.push(id); // A^0

        let current = adjacency.clone();
        powers.push(current.clone()); // A^1

        for _ in 2..=n {
            let next_power = powers.last().unwrap().tropical_mul(adjacency)?;
            powers.push(next_power);
        }

        // Karp's formula using all-predecessors path table: F_k(v) = max_{u} (A^k)_{uv}
        // This computes the maximum weight of any walk of length k ending at v,
        // guaranteeing finite entries across all cycle periodicities L regardless of
        // whether L divides n or whether the graph contains disconnected components.
        let mut max_lambda = NEG_INFINITY;
        let mut critical_vertex = 0;
        let mut found_any_cycle = false;

        // Precompute F_k(v) = max_{u} (A^k)_{uv} for k in 0..=n, v in 0..n
        let mut f = vec![vec![NEG_INFINITY; n]; n + 1];
        for (k, power_k) in powers.iter().enumerate().take(n + 1) {
            for (v, f_k_v) in f[k].iter_mut().enumerate().take(n) {
                let mut max_u = NEG_INFINITY;
                for u in 0..n {
                    if power_k.data[u][v] > max_u {
                        max_u = power_k.data[u][v];
                    }
                }
                *f_k_v = max_u;
            }
        }

        for (v, &f_n_v) in f[n].iter().enumerate().take(n) {
            if f_n_v <= NEG_INFINITY {
                continue;
            }

            let mut min_diff = f64::INFINITY;
            let mut valid_k_found = false;

            for (k, f_k) in f.iter().enumerate().take(n) {
                let f_k_v = f_k[v];
                if f_k_v > NEG_INFINITY {
                    let diff = (f_n_v - f_k_v) / ((n - k) as f64);
                    if diff < min_diff {
                        min_diff = diff;
                    }
                    valid_k_found = true;
                }
            }

            if valid_k_found && min_diff > NEG_INFINITY {
                found_any_cycle = true;
                if min_diff > max_lambda {
                    max_lambda = min_diff;
                    critical_vertex = v;
                }
            }
        }

        if !found_any_cycle || max_lambda <= NEG_INFINITY {
            return Err(TropicalError::GraphAcyclicOrUnreachable);
        }

        // Asymptotic throughput: 1 / lambda(G)
        let asymptotic_throughput = if max_lambda > 1e-12 {
            1.0 / max_lambda
        } else {
            f64::INFINITY
        };

        // Default SLA threshold: 50.0 ms
        let sla_threshold = 50.0;
        let sla_satisfied = max_lambda <= sla_threshold;

        Ok(KarpCycleMeanResult {
            maximum_cycle_mean: max_lambda,
            asymptotic_throughput,
            sla_threshold,
            sla_satisfied,
            critical_vertex,
        })
    }

    /// Evaluate SLA deadlock compliance against a specific threshold.
    pub fn check_sla_compliance(
        &self,
        adjacency: &MaxPlusMatrix,
        sla_threshold: f64,
    ) -> Result<KarpCycleMeanResult, TropicalError> {
        if sla_threshold < 0.0 || sla_threshold.is_nan() {
            return Err(TropicalError::InvalidSlaThreshold(sla_threshold));
        }
        let mut result = self.compute_karp_maximum_cycle_mean(adjacency)?;
        result.sla_threshold = sla_threshold;
        result.sla_satisfied = result.maximum_cycle_mean <= sla_threshold;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tropical_semiring_algebraic_properties() {
        // Identity and zero matrices
        let id = MaxPlusMatrix::identity(3).unwrap();
        assert_eq!(id.data[0][0], 0.0);
        assert_eq!(id.data[0][1], NEG_INFINITY);

        let zero = MaxPlusMatrix::zero(3).unwrap();
        assert_eq!(zero.data[0][0], NEG_INFINITY);

        // A (x) I == A
        let mut a = MaxPlusMatrix::zero(3).unwrap();
        a.set(0, 1, 5.0).unwrap();
        a.set(1, 2, 3.0).unwrap();

        let a_mul_id = a.tropical_mul(&id).unwrap();
        assert_eq!(a_mul_id.data[0][1], 5.0);
        assert_eq!(a_mul_id.data[1][2], 3.0);
        assert_eq!(a_mul_id.data[0][0], NEG_INFINITY);
    }

    #[test]
    fn test_theorem_39_dag_kleene_star_makespan() {
        // Linear pipeline DAG: 0 --(10)--> 1 --(15)--> 2 --(20)--> 3
        let mut adj = MaxPlusMatrix::zero(4).unwrap();
        adj.set(0, 1, 10.0).unwrap();
        adj.set(1, 2, 15.0).unwrap();
        adj.set(2, 3, 20.0).unwrap();

        let scheduler = TropicalDagScheduler::new();
        let star = scheduler.compute_kleene_star(&adj).expect("kleene star");

        assert!(!star.has_positive_cycle);
        // Critical path 0 -> 3: 10 + 15 + 20 = 45
        assert_eq!(star.closure.data[0][3], 45.0);
        assert_eq!(star.critical_path_makespan, 45.0);

        // 0 -> 2: 10 + 15 = 25
        assert_eq!(star.closure.data[0][2], 25.0);
    }

    #[test]
    fn test_theorem_39_positive_cycle_detection() {
        // Digraph with positive cycle: 1 -> 2 (weight 5), 2 -> 1 (weight 2) => cycle weight 7 > 0
        let mut adj = MaxPlusMatrix::zero(3).unwrap();
        adj.set(0, 1, 4.0).unwrap();
        adj.set(1, 2, 5.0).unwrap();
        adj.set(2, 1, 2.0).unwrap();

        let scheduler = TropicalDagScheduler::new();
        let err = scheduler.compute_kleene_star(&adj).unwrap_err();
        match err {
            TropicalError::PositiveCycleDetected { cycle_weight, .. } => {
                assert!(cycle_weight > 0.0);
            }
            _ => panic!("expected positive cycle error"),
        }
    }

    #[test]
    fn test_theorem_40_karp_maximum_cycle_mean() {
        // 3-node graph with cycles:
        // Cycle 1: node 0 self loop with weight 3.0 (mean = 3.0)
        // Cycle 2: node 1 self loop with weight 5.0 (mean = 5.0)
        // Cycle 3: node 2 self loop with weight 4.0 (mean = 4.0)
        let mut adj = MaxPlusMatrix::zero(3).unwrap();
        adj.set(0, 0, 3.0).unwrap();
        adj.set(1, 1, 5.0).unwrap();
        adj.set(2, 2, 4.0).unwrap();
        adj.set(0, 1, 1.0).unwrap();
        adj.set(1, 2, 1.0).unwrap();

        let scheduler = TropicalDagScheduler::new();
        let karp = scheduler
            .compute_karp_maximum_cycle_mean(&adj)
            .expect("karp result");

        // Maximum cycle mean should be 5.0
        assert!((karp.maximum_cycle_mean - 5.0).abs() < 1e-6);
        // Throughput = 1 / 5.0 = 0.20
        assert!((karp.asymptotic_throughput - 0.20).abs() < 1e-6);
        assert_eq!(karp.critical_vertex, 1);
        assert!(karp.sla_satisfied); // 5.0 <= 50.0 SLA
    }
}
