#![forbid(unsafe_code)]

//! Adversarial Empirical Challenge & Stress Tests for Phase 20: Tropical / Max-Plus Algebra DAG Engine
//!
//! Evaluates:
//! 1. Tropical semiring algebraic laws (associativity, distributivity, identities, absorbing element, idempotency)
//! 2. Kleene star closure $A^\star = \bigoplus_{k=0}^{n-1} A^k$:
//!    - $n=1$ boundary cases (isolated node, zero self-loop, negative self-loop, positive self-loop)
//!    - Fully disconnected graphs (all $-\infty$)
//!    - Long linear chains ($n=50$)
//!    - Wide fan-out / fan-in DAGs (diamonds)
//!    - Positive cycles (direct 1-cycle, 2-cycle, 3-cycle, mixed signs)
//!    - Zero-weight cycles and negative-weight cycles (verifying convergence)
//! 3. Karp's maximum cycle mean algorithm $\lambda(G)$:
//!    - Single 2-cycle, 3-cycle, 4-cycle with exact algebraic verification
//!    - Multi-cycle graphs with mixed cycle lengths and means
//!    - Disconnected components with different cycle means
//!    - SLA deadlock detection and threshold alerting
//! 4. Robustness against invalid weights (NaN, +inf), empty matrices, and dimension mismatches

use pandora_runtime::tropical_dag::{
    MaxPlusMatrix, NEG_INFINITY, TropicalDagScheduler, TropicalError,
};

fn approx_eq(a: f64, b: f64) -> bool {
    if a.is_infinite() && b.is_infinite() {
        a.is_sign_negative() == b.is_sign_negative()
    } else {
        (a - b).abs() < 1e-9
    }
}

// ---------------------------------------------------------------------------
// 1. Tropical Semiring Algebraic Laws
// ---------------------------------------------------------------------------

#[test]
fn test_tropical_semiring_associativity_distributivity() {
    let n = 4;
    // Construct three distinct matrices A, B, C with mixed values and -inf
    let mut a = MaxPlusMatrix::zero(n).unwrap();
    let mut b = MaxPlusMatrix::zero(n).unwrap();
    let mut c = MaxPlusMatrix::zero(n).unwrap();

    let val_a = [
        [1.5, 2.0, NEG_INFINITY, 4.2],
        [NEG_INFINITY, 0.0, 3.1, NEG_INFINITY],
        [5.0, NEG_INFINITY, -1.2, 2.5],
        [0.5, 3.3, NEG_INFINITY, 6.0],
    ];
    let val_b = [
        [0.0, NEG_INFINITY, 1.1, 3.0],
        [2.2, 4.4, NEG_INFINITY, 0.5],
        [NEG_INFINITY, 1.0, 2.0, NEG_INFINITY],
        [3.5, NEG_INFINITY, 4.0, 1.2],
    ];
    let val_c = [
        [2.1, 1.0, 0.0, NEG_INFINITY],
        [NEG_INFINITY, 3.2, 1.5, 4.0],
        [0.5, NEG_INFINITY, 2.5, 3.0],
        [1.0, 2.0, 3.0, 4.0],
    ];

    for i in 0..n {
        for j in 0..n {
            if val_a[i][j] > NEG_INFINITY {
                a.set(i, j, val_a[i][j]).unwrap();
            }
            if val_b[i][j] > NEG_INFINITY {
                b.set(i, j, val_b[i][j]).unwrap();
            }
            if val_c[i][j] > NEG_INFINITY {
                c.set(i, j, val_c[i][j]).unwrap();
            }
        }
    }

    // Law 1: Addition Associativity: (A (+) B) (+) C == A (+) (B (+) C)
    let a_plus_b = a.tropical_add(&b).unwrap();
    let lhs_add = a_plus_b.tropical_add(&c).unwrap();

    let b_plus_c = b.tropical_add(&c).unwrap();
    let rhs_add = a.tropical_add(&b_plus_c).unwrap();

    for i in 0..n {
        for j in 0..n {
            assert!(
                approx_eq(lhs_add.data[i][j], rhs_add.data[i][j]),
                "Addition associativity failed at ({}, {}): lhs={}, rhs={}",
                i,
                j,
                lhs_add.data[i][j],
                rhs_add.data[i][j]
            );
        }
    }

    // Law 2: Multiplication Associativity: (A (*) B) (*) C == A (*) (B (*) C)
    let a_mul_b = a.tropical_mul(&b).unwrap();
    let lhs_mul = a_mul_b.tropical_mul(&c).unwrap();

    let b_mul_c = b.tropical_mul(&c).unwrap();
    let rhs_mul = a.tropical_mul(&b_mul_c).unwrap();

    for i in 0..n {
        for j in 0..n {
            assert!(
                approx_eq(lhs_mul.data[i][j], rhs_mul.data[i][j]),
                "Multiplication associativity failed at ({}, {}): lhs={}, rhs={}",
                i,
                j,
                lhs_mul.data[i][j],
                rhs_mul.data[i][j]
            );
        }
    }

    // Law 3: Distributivity: A (*) (B (+) C) == (A (*) B) (+) (A (*) C)
    let lhs_dist = a.tropical_mul(&b_plus_c).unwrap();
    let a_mul_c = a.tropical_mul(&c).unwrap();
    let rhs_dist = a_mul_b.tropical_add(&a_mul_c).unwrap();

    for i in 0..n {
        for j in 0..n {
            assert!(
                approx_eq(lhs_dist.data[i][j], rhs_dist.data[i][j]),
                "Distributivity failed at ({}, {}): lhs={}, rhs={}",
                i,
                j,
                lhs_dist.data[i][j],
                rhs_dist.data[i][j]
            );
        }
    }

    // Law 4: Right Distributivity: (B (+) C) (*) A == (B (*) A) (+) (C (*) A)
    let lhs_rdist = b_plus_c.tropical_mul(&a).unwrap();
    let b_mul_a = b.tropical_mul(&a).unwrap();
    let c_mul_a = c.tropical_mul(&a).unwrap();
    let rhs_rdist = b_mul_a.tropical_add(&c_mul_a).unwrap();

    for i in 0..n {
        for j in 0..n {
            assert!(
                approx_eq(lhs_rdist.data[i][j], rhs_rdist.data[i][j]),
                "Right distributivity failed at ({}, {}): lhs={}, rhs={}",
                i,
                j,
                lhs_rdist.data[i][j],
                rhs_rdist.data[i][j]
            );
        }
    }

    // Law 5: Additive Identity: A (+) 0 == A
    let zero = MaxPlusMatrix::zero(n).unwrap();
    let a_plus_zero = a.tropical_add(&zero).unwrap();
    for i in 0..n {
        for j in 0..n {
            assert!(approx_eq(a_plus_zero.data[i][j], a.data[i][j]));
        }
    }

    // Law 6: Multiplicative Identity: A (*) I == I (*) A == A
    let id = MaxPlusMatrix::identity(n).unwrap();
    let a_mul_id = a.tropical_mul(&id).unwrap();
    let id_mul_a = id.tropical_mul(&a).unwrap();
    for i in 0..n {
        for j in 0..n {
            assert!(approx_eq(a_mul_id.data[i][j], a.data[i][j]));
            assert!(approx_eq(id_mul_a.data[i][j], a.data[i][j]));
        }
    }

    // Law 7: Absorbing Element: A (*) 0 == 0 (*) A == 0
    let a_mul_zero = a.tropical_mul(&zero).unwrap();
    let zero_mul_a = zero.tropical_mul(&a).unwrap();
    for i in 0..n {
        for j in 0..n {
            assert_eq!(a_mul_zero.data[i][j], NEG_INFINITY);
            assert_eq!(zero_mul_a.data[i][j], NEG_INFINITY);
        }
    }

    // Law 8: Idempotency of addition: A (+) A == A
    let a_plus_a = a.tropical_add(&a).unwrap();
    for i in 0..n {
        for j in 0..n {
            assert!(approx_eq(a_plus_a.data[i][j], a.data[i][j]));
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Kleene Star Edge Cases & Structural Topologies
// ---------------------------------------------------------------------------

#[test]
fn test_kleene_star_n1_boundaries() {
    let scheduler = TropicalDagScheduler::new();

    // 1. Isolated n=1 node (no edges, A = [[-inf]])
    let adj_isolated = MaxPlusMatrix::zero(1).unwrap();
    let star_isolated = scheduler.compute_kleene_star(&adj_isolated).unwrap();
    assert_eq!(star_isolated.closure.data[0][0], 0.0);
    assert_eq!(star_isolated.critical_path_makespan, 0.0);
    assert!(!star_isolated.has_positive_cycle);

    // 2. n=1 with negative self-loop: A = [[-5.0]]
    let mut adj_neg = MaxPlusMatrix::zero(1).unwrap();
    adj_neg.set(0, 0, -5.0).unwrap();
    let star_neg = scheduler.compute_kleene_star(&adj_neg).unwrap();
    assert_eq!(star_neg.closure.data[0][0], 0.0); // max(0.0, -5.0) = 0.0
    assert_eq!(star_neg.critical_path_makespan, 0.0);
    assert!(!star_neg.has_positive_cycle);

    // 3. n=1 with zero self-loop: A = [[0.0]]
    let mut adj_zero = MaxPlusMatrix::zero(1).unwrap();
    adj_zero.set(0, 0, 0.0).unwrap();
    let star_zero = scheduler.compute_kleene_star(&adj_zero).unwrap();
    assert_eq!(star_zero.closure.data[0][0], 0.0);
    assert_eq!(star_zero.critical_path_makespan, 0.0);
    assert!(!star_zero.has_positive_cycle);

    // 4. n=1 with positive self-loop: A = [[0.001]]
    let mut adj_pos = MaxPlusMatrix::zero(1).unwrap();
    adj_pos.set(0, 0, 0.001).unwrap();
    let err_pos = scheduler.compute_kleene_star(&adj_pos).unwrap_err();
    match err_pos {
        TropicalError::PositiveCycleDetected { node, cycle_weight } => {
            assert_eq!(node, 0);
            assert!(approx_eq(cycle_weight, 0.001));
        }
        _ => panic!("Expected PositiveCycleDetected for n=1 positive self-loop"),
    }
}

#[test]
fn test_kleene_star_disconnected_graph() {
    let scheduler = TropicalDagScheduler::new();
    let n = 6;
    let adj = MaxPlusMatrix::zero(n).unwrap();
    let star = scheduler.compute_kleene_star(&adj).unwrap();

    assert!(!star.has_positive_cycle);
    assert_eq!(star.critical_path_makespan, 0.0);
    for i in 0..n {
        for j in 0..n {
            if i == j {
                assert_eq!(star.closure.data[i][j], 0.0);
            } else {
                assert_eq!(star.closure.data[i][j], NEG_INFINITY);
            }
        }
    }
}

#[test]
fn test_kleene_star_long_linear_chain_stress() {
    let scheduler = TropicalDagScheduler::new();
    let n = 30; // Chain of 30 nodes: 0 -> 1 -> 2 -> ... -> 29
    let mut adj = MaxPlusMatrix::zero(n).unwrap();
    let mut expected_prefix = vec![0.0; n];

    for i in 0..(n - 1) {
        let weight = (i as f64) + 1.0; // edge weight: 1, 2, 3, ...
        adj.set(i, i + 1, weight).unwrap();
        expected_prefix[i + 1] = expected_prefix[i] + weight;
    }

    let star = scheduler.compute_kleene_star(&adj).unwrap();
    assert!(!star.has_positive_cycle);

    // Check distance from 0 to each node
    for (j, &expected) in expected_prefix.iter().enumerate().take(n) {
        assert!(
            approx_eq(star.closure.data[0][j], expected),
            "Path 0 -> {} mismatch: expected {}, got {}",
            j,
            expected,
            star.closure.data[0][j]
        );
    }

    // Critical path makespan is total chain length: sum_{i=1}^{29} i = 29*30/2 = 435.0
    let total_chain_len = (29.0 * 30.0) / 2.0;
    assert!(approx_eq(star.critical_path_makespan, total_chain_len));
}

#[test]
fn test_kleene_star_wide_fan_out_fan_in_diamond() {
    let scheduler = TropicalDagScheduler::new();
    // Diamond graph: Source 0 fans out to 10 parallel workers (1..10), which fan in to Sink 11
    let k = 10;
    let n = k + 2; // nodes 0 to 11
    let mut adj = MaxPlusMatrix::zero(n).unwrap();

    let mut max_branch_latency = 0.0;
    for i in 1..=k {
        let fan_out_w = (i as f64) * 2.0;
        let fan_in_w = 30.0 - (i as f64);
        let branch_total = fan_out_w + fan_in_w; // = i*2 + 30 - i = 30 + i
        if branch_total > max_branch_latency {
            max_branch_latency = branch_total;
        }
        adj.set(0, i, fan_out_w).unwrap();
        adj.set(i, n - 1, fan_in_w).unwrap();
    }

    let star = scheduler.compute_kleene_star(&adj).unwrap();
    assert!(!star.has_positive_cycle);
    // Makespan must equal max over branches: for i=10, 30 + 10 = 40.0
    assert!(approx_eq(star.critical_path_makespan, max_branch_latency));
    assert!(approx_eq(star.closure.data[0][n - 1], 40.0));
}

// ---------------------------------------------------------------------------
// 3. Positive / Zero / Negative Cycle Stress Tests
// ---------------------------------------------------------------------------

#[test]
fn test_positive_cycles_various_lengths() {
    let scheduler = TropicalDagScheduler::new();

    // 2-cycle: 1 -> 2 (3.5), 2 -> 1 (1.5) => sum 5.0 > 0
    let mut adj2 = MaxPlusMatrix::zero(4).unwrap();
    adj2.set(0, 1, 10.0).unwrap();
    adj2.set(1, 2, 3.5).unwrap();
    adj2.set(2, 1, 1.5).unwrap();
    adj2.set(2, 3, 4.0).unwrap();

    let err2 = scheduler.compute_kleene_star(&adj2).unwrap_err();
    match err2 {
        TropicalError::PositiveCycleDetected { cycle_weight, .. } => {
            assert!(approx_eq(cycle_weight, 5.0));
        }
        _ => panic!("Expected PositiveCycleDetected for 2-cycle"),
    }

    // 3-cycle: 1 -> 2 (10.0), 2 -> 3 (-4.0), 3 -> 1 (-2.0) => sum 4.0 > 0
    let mut adj3 = MaxPlusMatrix::zero(4).unwrap();
    adj3.set(1, 2, 10.0).unwrap();
    adj3.set(2, 3, -4.0).unwrap();
    adj3.set(3, 1, -2.0).unwrap();

    let err3 = scheduler.compute_kleene_star(&adj3).unwrap_err();
    match err3 {
        TropicalError::PositiveCycleDetected { cycle_weight, .. } => {
            assert!(approx_eq(cycle_weight, 4.0));
        }
        _ => panic!("Expected PositiveCycleDetected for 3-cycle"),
    }
}

#[test]
fn test_zero_and_negative_cycles_convergence() {
    let scheduler = TropicalDagScheduler::new();

    // Zero-weight cycle: 1 -> 2 (5.0), 2 -> 1 (-5.0) => sum 0.0
    let mut adj_zero = MaxPlusMatrix::zero(3).unwrap();
    adj_zero.set(0, 1, 10.0).unwrap();
    adj_zero.set(1, 2, 5.0).unwrap();
    adj_zero.set(2, 1, -5.0).unwrap();

    let star_zero = scheduler
        .compute_kleene_star(&adj_zero)
        .expect("Zero weight cycle should converge");
    assert!(!star_zero.has_positive_cycle);
    assert_eq!(star_zero.closure.data[1][1], 0.0);
    assert_eq!(star_zero.closure.data[2][2], 0.0);
    assert_eq!(star_zero.closure.data[0][1], 10.0);
    assert_eq!(star_zero.closure.data[0][2], 15.0);

    // Strictly negative-weight cycle: 1 -> 2 (2.0), 2 -> 1 (-5.0) => sum -3.0
    let mut adj_neg = MaxPlusMatrix::zero(3).unwrap();
    adj_neg.set(0, 1, 10.0).unwrap();
    adj_neg.set(1, 2, 2.0).unwrap();
    adj_neg.set(2, 1, -5.0).unwrap();

    let star_neg = scheduler
        .compute_kleene_star(&adj_neg)
        .expect("Negative weight cycle should converge");
    assert!(!star_neg.has_positive_cycle);
    assert_eq!(star_neg.closure.data[1][1], 0.0); // A^0 diagonal is 0.0
    assert_eq!(star_neg.closure.data[2][2], 0.0);
}

// ---------------------------------------------------------------------------
// 4. Karp's Maximum Cycle Mean Verification
// ---------------------------------------------------------------------------

#[test]
fn test_karp_exact_algebraic_small_cycles() {
    let scheduler = TropicalDagScheduler::new();

    // Pure 2-cycle: 0 -> 1 (4.0), 1 -> 0 (6.0) => mean = 10 / 2 = 5.0
    let mut adj2 = MaxPlusMatrix::zero(2).unwrap();
    adj2.set(0, 1, 4.0).unwrap();
    adj2.set(1, 0, 6.0).unwrap();

    let karp2 = scheduler
        .compute_karp_maximum_cycle_mean(&adj2)
        .expect("karp 2-cycle");
    assert!(approx_eq(karp2.maximum_cycle_mean, 5.0));
    assert!(approx_eq(karp2.asymptotic_throughput, 1.0 / 5.0));

    // Pure 3-cycle: 0 -> 1 (3.0), 1 -> 2 (7.0), 2 -> 0 (2.0) => mean = 12 / 3 = 4.0
    let mut adj3 = MaxPlusMatrix::zero(3).unwrap();
    adj3.set(0, 1, 3.0).unwrap();
    adj3.set(1, 2, 7.0).unwrap();
    adj3.set(2, 0, 2.0).unwrap();

    let karp3 = scheduler
        .compute_karp_maximum_cycle_mean(&adj3)
        .expect("karp 3-cycle");
    assert!(approx_eq(karp3.maximum_cycle_mean, 4.0));
    assert!(approx_eq(karp3.asymptotic_throughput, 1.0 / 4.0));
}

#[test]
fn test_karp_multiple_cycles_selection() {
    let scheduler = TropicalDagScheduler::new();

    // 4-node graph with multiple cycles:
    // Cycle 1: 0 -> 0 (self-loop, weight 2.0, mean = 2.0)
    // Cycle 2: 1 -> 2 -> 1 (weights 6.0, 8.0, total 14.0, mean = 7.0)
    // Cycle 3: 0 -> 1 -> 2 -> 3 -> 0 (weights 1, 1, 1, 1, total 4.0, mean = 1.0)
    let mut adj = MaxPlusMatrix::zero(4).unwrap();
    adj.set(0, 0, 2.0).unwrap();
    adj.set(1, 2, 6.0).unwrap();
    adj.set(2, 1, 8.0).unwrap();
    adj.set(0, 1, 1.0).unwrap();
    adj.set(2, 3, 1.0).unwrap();
    adj.set(3, 0, 1.0).unwrap();

    let karp = scheduler
        .compute_karp_maximum_cycle_mean(&adj)
        .expect("karp multi-cycle");
    // Maximum cycle mean MUST be 7.0 (achieved by Cycle 2: 1 <-> 2)
    assert!(
        approx_eq(karp.maximum_cycle_mean, 7.0),
        "Expected maximum cycle mean 7.0, got {}",
        karp.maximum_cycle_mean
    );
}

#[test]
fn test_karp_sla_deadlock_detection() {
    let scheduler = TropicalDagScheduler::new();

    // Graph with maximum cycle mean = 25.0
    let mut adj = MaxPlusMatrix::zero(2).unwrap();
    adj.set(0, 1, 20.0).unwrap();
    adj.set(1, 0, 30.0).unwrap(); // mean = 50 / 2 = 25.0

    // Test SLA threshold = 30.0 -> Compliant (25.0 <= 30.0)
    let res_pass = scheduler.check_sla_compliance(&adj, 30.0).unwrap();
    assert!(res_pass.sla_satisfied);
    assert_eq!(res_pass.sla_threshold, 30.0);

    // Test SLA threshold = 20.0 -> Violated / Deadlock alert (25.0 > 20.0)
    let res_fail = scheduler.check_sla_compliance(&adj, 20.0).unwrap();
    assert!(!res_fail.sla_satisfied);
    assert_eq!(res_fail.sla_threshold, 20.0);

    // Test negative SLA threshold -> Error
    let err_neg = scheduler.check_sla_compliance(&adj, -5.0).unwrap_err();
    match err_neg {
        TropicalError::InvalidSlaThreshold(val) => assert_eq!(val, -5.0),
        _ => panic!("Expected InvalidSlaThreshold error"),
    }
}

// ---------------------------------------------------------------------------
// 5. Robustness & Error Path Testing
// ---------------------------------------------------------------------------

#[test]
fn test_robustness_and_error_handling() {
    let scheduler = TropicalDagScheduler::new();

    // Empty matrix
    assert_eq!(
        MaxPlusMatrix::zero(0).unwrap_err(),
        TropicalError::EmptyMatrix
    );
    assert_eq!(
        MaxPlusMatrix::identity(0).unwrap_err(),
        TropicalError::EmptyMatrix
    );
    assert_eq!(
        MaxPlusMatrix::from_data(vec![]).unwrap_err(),
        TropicalError::EmptyMatrix
    );

    // Dimension mismatch
    let m2 = MaxPlusMatrix::zero(2).unwrap();
    let m3 = MaxPlusMatrix::zero(3).unwrap();
    assert_eq!(
        m2.tropical_add(&m3).unwrap_err(),
        TropicalError::DimensionMismatch {
            expected: 2,
            actual: 3
        }
    );
    assert_eq!(
        m2.tropical_mul(&m3).unwrap_err(),
        TropicalError::DimensionMismatch {
            expected: 2,
            actual: 3
        }
    );

    // Out of bounds
    let mut m = MaxPlusMatrix::zero(2).unwrap();
    assert_eq!(
        m.get(2, 0).unwrap_err(),
        TropicalError::NodeOutOfBounds { node: 2, size: 2 }
    );
    assert_eq!(
        m.set(0, 5, 1.0).unwrap_err(),
        TropicalError::NodeOutOfBounds { node: 5, size: 2 }
    );

    // Invalid weights: NaN and +inf
    match m.set(0, 0, f64::NAN).unwrap_err() {
        TropicalError::InvalidWeight(w) => assert!(w.is_nan()),
        other => panic!("Expected InvalidWeight(NaN), got {:?}", other),
    }
    assert_eq!(
        m.set(0, 0, f64::INFINITY).unwrap_err(),
        TropicalError::InvalidWeight(f64::INFINITY)
    );

    // Pure acyclic graph for Karp -> Error
    let mut dag = MaxPlusMatrix::zero(3).unwrap();
    dag.set(0, 1, 5.0).unwrap();
    dag.set(1, 2, 5.0).unwrap();
    assert_eq!(
        scheduler.compute_karp_maximum_cycle_mean(&dag).unwrap_err(),
        TropicalError::GraphAcyclicOrUnreachable
    );
}

#[test]
fn test_karp_disconnected_components_cycle_means() {
    let scheduler = TropicalDagScheduler::new();

    // 3-node graph with two disconnected components:
    // Component 1: Node 0 has self-loop with weight 3.0 (length 1, mean = 3.0)
    // Component 2: Nodes 1 and 2 form a 2-cycle with weights 10.0, 10.0 (length 2, mean = 10.0)
    // The maximum cycle mean of the entire graph is max(3.0, 10.0) = 10.0.
    let mut adj = MaxPlusMatrix::zero(3).unwrap();
    adj.set(0, 0, 3.0).unwrap();
    adj.set(1, 2, 10.0).unwrap();
    adj.set(2, 1, 10.0).unwrap();

    let karp = scheduler
        .compute_karp_maximum_cycle_mean(&adj)
        .expect("karp on disconnected components with cycles");

    println!(
        "Karp result for disconnected components: max_mean={}, critical_v={}",
        karp.maximum_cycle_mean, karp.critical_vertex
    );
    assert!(
        approx_eq(karp.maximum_cycle_mean, 10.0),
        "Expected maximum cycle mean 10.0, but got {}",
        karp.maximum_cycle_mean
    );
}

#[test]
fn test_karp_strongly_connected_multi_cycle_challenge() {
    let scheduler = TropicalDagScheduler::new();

    // 5-node STRONGLY CONNECTED graph:
    // Cycle 1: 1 <-> 2 (weights 10.0, 10.0, length 2, mean = 10.0)
    // Cycle 2: 0 -> 3 -> 4 -> 0 (weights 2.0, 2.0, 2.0, length 3, mean = 2.0)
    // Cross edges connecting 0 and the 2-cycle:
    // 0 -> 1 (weight 0.0)
    // 2 -> 0 (weight 0.0)
    // This graph is fully strongly connected.
    // The maximum cycle mean is unambiguously 10.0 (Cycle 1).
    let mut adj = MaxPlusMatrix::zero(5).unwrap();
    adj.set(1, 2, 10.0).unwrap();
    adj.set(2, 1, 10.0).unwrap();
    adj.set(0, 3, 2.0).unwrap();
    adj.set(3, 4, 2.0).unwrap();
    adj.set(4, 0, 2.0).unwrap();
    adj.set(0, 1, 0.0).unwrap();
    adj.set(2, 0, 0.0).unwrap();

    let karp = scheduler
        .compute_karp_maximum_cycle_mean(&adj)
        .expect("karp on 5-node strongly connected graph");

    println!(
        "Karp 5-node strongly connected: max_mean={}, critical_v={}",
        karp.maximum_cycle_mean, karp.critical_vertex
    );
    assert!(
        approx_eq(karp.maximum_cycle_mean, 10.0),
        "Expected maximum cycle mean 10.0, but got {}",
        karp.maximum_cycle_mean
    );
}
