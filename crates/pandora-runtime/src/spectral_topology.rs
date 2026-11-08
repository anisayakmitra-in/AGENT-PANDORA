#![forbid(unsafe_code)]

//! Phase 16: Spectral Graph Partitioning & Algebraic Connectivity for Multi-Agent Topology
//!
//! Certifies multi-agent communication resilience, bottleneck risk, and optimal
//! spectral bisection using the unnormalized Graph Laplacian $L = D - A$ and
//! symmetric normalized Graph Laplacian $\mathcal{L}_{\mathrm{sym}} = I - D^{-1/2} A D^{-1/2}$.
//!
//! ## Mathematical Guarantees
//!
//! 1. **Theorem 31 (Courant-Fischer Variational Fiedler Value & Connectivity):**
//!    For an undirected weighted graph $G = (V, E, w)$ with $n = |V| \ge 2$ and Laplacian
//!    $L = D - A$, the quadratic form satisfies
//!    $$x^\top L x = \sum_{\{u, v\} \in E} w_{uv}(x_u - x_v)^2 \ge 0.$$
//!    The second-smallest eigenvalue (Fiedler value / algebraic connectivity) is
//!    $$\lambda_2(L) = \min_{\substack{x \in \mathbb{R}^n \setminus \{0\} \\ \langle x, \mathbf{1} \rangle = 0}} \frac{x^\top L x}{x^\top x},$$
//!    and $\lambda_2(L) > 0$ if and only if $G$ is connected.
//!
//! 2. **Theorem 32 (Cheeger's Inequality & Fiedler Sweep-Cut Conductance Bound):**
//!    Let $\mathcal{L}_{\mathrm{sym}} = D^{-1/2} L D^{-1/2}$ have second-smallest eigenvalue
//!    $\nu_2 = \lambda_2(\mathcal{L}_{\mathrm{sym}})$, and let the graph conductance (Cheeger constant) be
//!    $$h(G) = \min_{\emptyset \subset S \subset V} \phi(S), \qquad \phi(S) = \frac{\mathrm{cut}(S, V \setminus S)}{\min(\mathrm{vol}(S), \mathrm{vol}(V \setminus S))}.$$
//!    Then Cheeger's Inequality holds:
//!    $$\frac{\nu_2}{2} \le h(G) \le \sqrt{2 \nu_2},$$
//!    and the Fiedler sweep cut over the sorted entries of $D^{-1/2} v_2$ achieves conductance at most $\sqrt{2 \nu_2}$.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Errors returned by the spectral topology analyzer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpectralTopologyError {
    /// Graph must contain at least 2 nodes for spectral bisection and Fiedler analysis.
    InsufficientNodes { node_count: usize },
    /// Edge references an out-of-bounds node index or has an invalid weight.
    InvalidEdge {
        u: usize,
        v: usize,
        weight: f64,
        reason: String,
    },
}

impl fmt::Display for SpectralTopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientNodes { node_count } => {
                write!(
                    f,
                    "spectral topology requires at least 2 nodes, got {node_count}"
                )
            }
            Self::InvalidEdge {
                u,
                v,
                weight,
                reason,
            } => {
                write!(f, "invalid edge ({u}, {v}, weight={weight}): {reason}")
            }
        }
    }
}

impl std::error::Error for SpectralTopologyError {}

/// Weighted undirected edge between two agents in the topology.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WeightedEdge {
    pub u: usize,
    pub v: usize,
    pub weight: f64,
}

/// Topological resilience classification based on $\lambda_2(L)$ and Cheeger conductance $h(G)$.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyVerdict {
    /// $\lambda_2(L) > 0$ and Cheeger conductance exceeds bottleneck threshold.
    ConnectedResilient,
    /// Graph is connected ($\lambda_2(L) > 0$) but exhibits a narrow communication bottleneck.
    BottleneckWarning,
    /// $\lambda_2(L) \approx 0$: Graph is disconnected into multiple isolated components.
    DisconnectedPartition,
}

/// Full spectral report including Fiedler value, Fiedler vector, Cheeger bounds, and bisection cut.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpectralTopologyReport {
    pub node_count: usize,
    pub edge_count: usize,
    pub fiedler_value_unnormalized: f64,
    pub fiedler_value_normalized: f64,
    pub fiedler_vector: Vec<f64>,
    pub sweep_cut_conductance: f64,
    pub cheeger_lower_bound: f64,
    pub cheeger_upper_bound: f64,
    pub cheeger_inequality_verified: bool,
    pub partition_left: Vec<usize>,
    pub partition_right: Vec<usize>,
    pub verdict: TopologyVerdict,
}

/// Spectral Multi-Agent Topology Analyzer implementing Theorems 31 and 32.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectralTopologyAnalyzer {
    node_count: usize,
    edges: Vec<WeightedEdge>,
    bottleneck_conductance_threshold: f64,
}

impl SpectralTopologyAnalyzer {
    /// Construct a spectral topology analyzer for `node_count` agents and weighted undirected `edges`.
    pub fn new(
        node_count: usize,
        edges: Vec<WeightedEdge>,
        bottleneck_conductance_threshold: f64,
    ) -> Result<Self, SpectralTopologyError> {
        if node_count < 2 {
            return Err(SpectralTopologyError::InsufficientNodes { node_count });
        }
        for edge in &edges {
            if edge.u >= node_count || edge.v >= node_count {
                return Err(SpectralTopologyError::InvalidEdge {
                    u: edge.u,
                    v: edge.v,
                    weight: edge.weight,
                    reason: format!("node index out of bounds for node_count={node_count}"),
                });
            }
            if !edge.weight.is_finite() || edge.weight <= 0.0 {
                return Err(SpectralTopologyError::InvalidEdge {
                    u: edge.u,
                    v: edge.v,
                    weight: edge.weight,
                    reason: "edge weight must be finite and strictly positive".to_owned(),
                });
            }
        }
        Ok(Self {
            node_count,
            edges,
            bottleneck_conductance_threshold: bottleneck_conductance_threshold.clamp(0.01, 0.99),
        })
    }

    /// Build the symmetric adjacency matrix $A$ and degree vector $d$.
    fn build_adjacency_and_degrees(&self) -> (Vec<Vec<f64>>, Vec<f64>) {
        let n = self.node_count;
        let mut adj = vec![vec![0.0_f64; n]; n];
        let mut deg = vec![0.0_f64; n];
        for e in &self.edges {
            if e.u == e.v {
                continue;
            }
            adj[e.u][e.v] += e.weight;
            adj[e.v][e.u] += e.weight;
            deg[e.u] += e.weight;
            deg[e.v] += e.weight;
        }
        (adj, deg)
    }

    /// Check graph connectivity via breadth-first traversal.
    fn is_connected(&self, adj: &[Vec<f64>]) -> bool {
        let n = self.node_count;
        let mut visited = vec![false; n];
        let mut queue = std::collections::VecDeque::new();
        visited[0] = true;
        queue.push_back(0);
        let mut count = 1usize;

        while let Some(u) = queue.pop_front() {
            for (v, &w) in adj[u].iter().enumerate() {
                if w > 0.0 && !visited[v] {
                    visited[v] = true;
                    count += 1;
                    queue.push_back(v);
                }
            }
        }
        count == n
    }

    /// Compute the smallest eigenvalue/eigenvector of a symmetric positive semi-definite matrix $M$
    /// in the subspace orthogonal to `nullspace_vec` using deflated shifted power iteration.
    fn smallest_orthogonal_eigenpair(
        matrix: &[Vec<f64>],
        nullspace_vec: &[f64],
        gershgorin_bound: f64,
    ) -> (f64, Vec<f64>) {
        let n = matrix.len();
        let mut u0 = nullspace_vec.to_vec();
        normalize(&mut u0);

        // Deterministic initial vector orthogonal to u0
        let mut v: Vec<f64> = (0..n)
            .map(|i| ((i as f64 + 1.0) * 1.372_583_002).sin())
            .collect();
        project_out(&mut v, &u0);
        if normalize(&mut v) < 1e-12 {
            for (i, val) in v.iter_mut().enumerate() {
                *val = if i == 0 { 1.0 } else { -1.0 / (n - 1) as f64 };
            }
            project_out(&mut v, &u0);
            normalize(&mut v);
        }

        let shift = gershgorin_bound.max(1.0) * 1.05;
        let mut next_v = vec![0.0_f64; n];

        for _ in 0..400 {
            // next_v = (shift * I - matrix) * v
            for (i, out_i) in next_v.iter_mut().enumerate() {
                let mut dot = 0.0_f64;
                for (j, &m_ij) in matrix[i].iter().enumerate() {
                    dot += m_ij * v[j];
                }
                *out_i = shift * v[i] - dot;
            }
            project_out(&mut next_v, &u0);
            if normalize(&mut next_v) < 1e-15 {
                break;
            }
            let diff: f64 = v
                .iter()
                .zip(next_v.iter())
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt();
            v.copy_from_slice(&next_v);
            if diff < 1e-11 {
                break;
            }
        }

        // Rayleigh quotient v^T M v
        let mut rayleigh = 0.0_f64;
        for (i, row) in matrix.iter().enumerate() {
            let mut row_dot = 0.0_f64;
            for (j, &m_ij) in row.iter().enumerate() {
                row_dot += m_ij * v[j];
            }
            rayleigh += v[i] * row_dot;
        }
        (rayleigh.max(0.0), v)
    }

    /// Execute full spectral analysis (Theorems 31 and 32).
    pub fn analyze(&self) -> SpectralTopologyReport {
        let n = self.node_count;
        let (adj, deg) = self.build_adjacency_and_degrees();
        let connected = self.is_connected(&adj);

        if !connected {
            // Find connected component of node 0 for partition_left
            let mut visited = vec![false; n];
            let mut queue = std::collections::VecDeque::new();
            visited[0] = true;
            queue.push_back(0);
            while let Some(u) = queue.pop_front() {
                for (v, &w) in adj[u].iter().enumerate() {
                    if w > 0.0 && !visited[v] {
                        visited[v] = true;
                        queue.push_back(v);
                    }
                }
            }
            let mut left = Vec::new();
            let mut right = Vec::new();
            for (i, &vis) in visited.iter().enumerate() {
                if vis {
                    left.push(i);
                } else {
                    right.push(i);
                }
            }
            let fiedler_vec: Vec<f64> = visited
                .iter()
                .map(|&vis| if vis { -1.0 } else { 1.0 })
                .collect();
            return SpectralTopologyReport {
                node_count: n,
                edge_count: self.edges.len(),
                fiedler_value_unnormalized: 0.0,
                fiedler_value_normalized: 0.0,
                fiedler_vector: fiedler_vec,
                sweep_cut_conductance: 0.0,
                cheeger_lower_bound: 0.0,
                cheeger_upper_bound: 0.0,
                cheeger_inequality_verified: true,
                partition_left: left,
                partition_right: right,
                verdict: TopologyVerdict::DisconnectedPartition,
            };
        }

        // 1. Unnormalized Laplacian L = D - A
        let mut lap = vec![vec![0.0_f64; n]; n];
        let mut max_deg = 0.0_f64;
        for i in 0..n {
            lap[i][i] = deg[i];
            if deg[i] > max_deg {
                max_deg = deg[i];
            }
            for j in 0..n {
                if i != j {
                    lap[i][j] = -adj[i][j];
                }
            }
        }
        let ones = vec![1.0_f64; n];
        let (lambda2_unnorm, fiedler_vec) =
            Self::smallest_orthogonal_eigenpair(&lap, &ones, 2.0 * max_deg);

        // 2. Normalized Laplacian L_sym = I - D^{-1/2} A D^{-1/2}
        let mut lap_sym = vec![vec![0.0_f64; n]; n];
        let sqrt_deg: Vec<f64> = deg.iter().map(|d| d.sqrt()).collect();
        for i in 0..n {
            lap_sym[i][i] = 1.0;
            for j in 0..n {
                if i != j && sqrt_deg[i] > 0.0 && sqrt_deg[j] > 0.0 {
                    lap_sym[i][j] = -adj[i][j] / (sqrt_deg[i] * sqrt_deg[j]);
                }
            }
        }
        let (nu2_raw, norm_fiedler_vec) =
            Self::smallest_orthogonal_eigenpair(&lap_sym, &sqrt_deg, 2.0);
        let nu2 = nu2_raw.clamp(0.0, 2.0);

        // 3. Fiedler Sweep Cut on scaled embedding y_i = norm_fiedler_vec[i] / sqrt(d_i)
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            let ya = norm_fiedler_vec[a] / sqrt_deg[a].max(1e-12);
            let yb = norm_fiedler_vec[b] / sqrt_deg[b].max(1e-12);
            ya.total_cmp(&yb)
        });

        let total_vol: f64 = deg.iter().sum();
        let mut in_left = vec![false; n];
        let mut vol_s = 0.0_f64;
        let mut cut_s = 0.0_f64;
        let mut best_conductance = f64::INFINITY;
        let mut best_k = 1usize;

        for (k_idx, &u) in order.iter().enumerate().take(n - 1) {
            in_left[u] = true;
            vol_s += deg[u];
            for (v, &w) in adj[u].iter().enumerate() {
                if w > 0.0 {
                    if in_left[v] {
                        cut_s -= w;
                    } else {
                        cut_s += w;
                    }
                }
            }
            let denom = vol_s.min(total_vol - vol_s);
            if denom > 0.0 {
                let phi = (cut_s.max(0.0)) / denom;
                if phi < best_conductance {
                    best_conductance = phi;
                    best_k = k_idx + 1;
                }
            }
        }

        let partition_left: Vec<usize> = order[..best_k].to_vec();
        let partition_right: Vec<usize> = order[best_k..].to_vec();

        // Theorem 32: nu2 / 2 <= h(G) <= phi_sweep <= sqrt(2 * nu2)
        let cheeger_lower = 0.5 * nu2;
        let cheeger_upper = (2.0 * nu2).sqrt();
        let cheeger_verified =
            best_conductance + 1e-7 >= cheeger_lower && best_conductance <= cheeger_upper + 1e-7;

        let verdict = if best_conductance < self.bottleneck_conductance_threshold {
            TopologyVerdict::BottleneckWarning
        } else {
            TopologyVerdict::ConnectedResilient
        };

        SpectralTopologyReport {
            node_count: n,
            edge_count: self.edges.len(),
            fiedler_value_unnormalized: lambda2_unnorm,
            fiedler_value_normalized: nu2,
            fiedler_vector: fiedler_vec,
            sweep_cut_conductance: best_conductance,
            cheeger_lower_bound: cheeger_lower,
            cheeger_upper_bound: cheeger_upper,
            cheeger_inequality_verified: cheeger_verified,
            partition_left,
            partition_right,
            verdict,
        }
    }
}

fn project_out(v: &mut [f64], unit_u: &[f64]) {
    let dot: f64 = v.iter().zip(unit_u.iter()).map(|(a, b)| a * b).sum();
    for (vi, &ui) in v.iter_mut().zip(unit_u.iter()) {
        *vi -= dot * ui;
    }
}

fn normalize(v: &mut [f64]) -> f64 {
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 1e-15 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_graph_k4_fiedler_value_and_cheeger_bound() {
        // For K_4 with unit weights, L has eigenvalues 0, 4, 4, 4 -> lambda_2(L) = 4.0
        // L_sym has eigenvalues 0, 4/3, 4/3, 4/3 -> nu_2 = 4/3
        let mut edges = Vec::new();
        for u in 0..4 {
            for v in (u + 1)..4 {
                edges.push(WeightedEdge { u, v, weight: 1.0 });
            }
        }
        let analyzer = SpectralTopologyAnalyzer::new(4, edges, 0.25).unwrap();
        let report = analyzer.analyze();
        assert!((report.fiedler_value_unnormalized - 4.0).abs() < 1e-4);
        assert!((report.fiedler_value_normalized - (4.0 / 3.0)).abs() < 1e-4);
        assert!(report.cheeger_inequality_verified);
        assert_eq!(report.verdict, TopologyVerdict::ConnectedResilient);
    }

    #[test]
    fn barbell_graph_detects_narrow_bottleneck() {
        // Two triangles (0,1,2) and (3,4,5) connected by a weak bridge (2,3) of weight 0.05
        let edges = vec![
            WeightedEdge {
                u: 0,
                v: 1,
                weight: 1.0,
            },
            WeightedEdge {
                u: 1,
                v: 2,
                weight: 1.0,
            },
            WeightedEdge {
                u: 0,
                v: 2,
                weight: 1.0,
            },
            WeightedEdge {
                u: 3,
                v: 4,
                weight: 1.0,
            },
            WeightedEdge {
                u: 4,
                v: 5,
                weight: 1.0,
            },
            WeightedEdge {
                u: 3,
                v: 5,
                weight: 1.0,
            },
            WeightedEdge {
                u: 2,
                v: 3,
                weight: 0.05,
            },
        ];
        let analyzer = SpectralTopologyAnalyzer::new(6, edges, 0.20).unwrap();
        let report = analyzer.analyze();
        assert!(report.fiedler_value_unnormalized > 0.0);
        assert!(report.cheeger_inequality_verified);
        assert_eq!(report.verdict, TopologyVerdict::BottleneckWarning);
        assert_eq!(report.partition_left.len(), 3);
        assert_eq!(report.partition_right.len(), 3);
    }

    #[test]
    fn disconnected_graph_has_zero_fiedler_value() {
        let edges = vec![
            WeightedEdge {
                u: 0,
                v: 1,
                weight: 1.0,
            },
            WeightedEdge {
                u: 2,
                v: 3,
                weight: 1.0,
            },
        ];
        let analyzer = SpectralTopologyAnalyzer::new(4, edges, 0.20).unwrap();
        let report = analyzer.analyze();
        assert_eq!(report.fiedler_value_unnormalized, 0.0);
        assert_eq!(report.verdict, TopologyVerdict::DisconnectedPartition);
    }
}
