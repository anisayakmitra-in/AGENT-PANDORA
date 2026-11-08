#![forbid(unsafe_code)]

//! Formal Mathematical Verification Test Suite: Phases 18–20 (Theorems 35–40)
//!
//! Validates:
//! - Theorem 35: Discrete McKean-Vlasov fixed-point contraction & geometric convergence
//! - Theorem 36: Price-of-Anarchy (PoA <= 4/3) Roughgarden (1, 1/4)-smoothness bound
//! - Theorem 37: Maurer empirical-Bernstein PAC-Bayes KL inversion vs Pinsker relaxation
//! - Theorem 38: Catoni optimal Gibbs posterior temperature & finite-sample promotion gate
//! - Theorem 39: Max-Plus semiring Kleene star A* convergence & algebraic cycle detection
//! - Theorem 40: Karp's maximum cycle mean lambda(G) & SLA periodic pipeline throughput

use pandora_runtime::mean_field_game::{
    MeanFieldGame, MfgResourceConfig, THEORETICAL_POA_UPPER_BOUND,
};
use pandora_runtime::pac_bayes_monitor::{
    PacBayesConfig, PacBayesMonitor, PolicyPromotionVerdict, binary_kl,
};
use pandora_runtime::tropical_dag::{MaxPlusMatrix, TropicalDagScheduler, TropicalError};

#[test]
fn theorem_35_36_mfg_contraction_and_poa_smoothness() {
    // 1. Theorem 35 Contraction: tau = 4.0, beta_max = 2.0 => kappa = 2.0 / (2 * 4.0) = 0.25 < 1.0
    let resources = vec![
        MfgResourceConfig::new("cluster_alpha", 0.5, 2.0).unwrap(),
        MfgResourceConfig::new("cluster_beta", 1.0, 1.5).unwrap(),
        MfgResourceConfig::new("cluster_gamma", 1.5, 1.0).unwrap(),
    ];
    let mfg = MeanFieldGame::new(resources, 4.0).unwrap();
    assert!(mfg.is_strict_contraction());
    assert!((mfg.contraction_modulus() - 0.25).abs() < 1e-6);

    let eq = mfg.compute_equilibrium(200, 1e-9).expect("equilibrium");
    assert!(eq.residual_norm < 1e-9);
    assert!(eq.is_strict_contraction);
    assert!(eq.iterations <= 30);

    // Sum-to-one invariant on simplex Delta_K
    let sum: f64 = eq.equilibrium_distribution.iter().sum();
    assert!((sum - 1.0).abs() < 1e-6);

    // Fixed point identity Phi(m*) = m*
    let response = mfg.best_response(&eq.equilibrium_distribution).unwrap();
    for (m_k, phi_k) in eq.equilibrium_distribution.iter().zip(response.iter()) {
        assert!((m_k - phi_k).abs() < 1e-6);
    }

    // 2. Theorem 36: Price of Anarchy on Pigou network
    // Latencies: c1(x) = x, c2(x) = 1
    let pigou = vec![
        MfgResourceConfig::new("pigou_congested", 0.0, 1.0).unwrap(),
        MfgResourceConfig::new("pigou_uncongested", 1.0, 0.0).unwrap(),
    ];
    let pigou_game = MeanFieldGame::with_damping(pigou, 0.02, 0.05).unwrap();
    let poa_report = pigou_game.compute_price_of_anarchy().expect("poa report");

    assert!(poa_report.bound_satisfied);
    assert!(poa_report.price_of_anarchy <= THEORETICAL_POA_UPPER_BOUND + 1e-3);
    assert!(poa_report.price_of_anarchy >= 1.0 - 1e-6);
    // Social optimum must achieve m1 = 0.5, m2 = 0.5 with cost 0.75
    assert!((poa_report.optimal_social_cost - 0.75).abs() < 1e-2);
}

#[test]
fn theorem_37_38_pac_bayes_risk_certificates_and_promotion_gate() {
    let monitor = PacBayesMonitor::new();

    // 1. Theorem 37: Maurer vs Pinsker
    let n = 2000;
    let delta = 0.01;
    let empirical_risk = 0.02;
    let kl = 1.5;

    let cert = monitor
        .certify_risk(empirical_risk, n, kl, delta)
        .expect("pac-bayes cert");

    assert!(cert.bound_verified);
    // Maurer bound must be strictly tighter than Pinsker sub-Gaussian bound
    assert!(cert.maurer_risk_bound < cert.pinsker_risk_bound);
    assert!(cert.maurer_risk_bound >= empirical_risk);

    // Verify binary kl equation holds at Maurer root: kl(L_hat || L_maurer) == Delta
    let kl_at_root = binary_kl(empirical_risk, cert.maurer_risk_bound);
    assert!((kl_at_root - cert.complexity_delta).abs() < 1e-6);

    // 2. Theorem 38: Catoni Gibbs temperature & promotion gate
    let candidate_losses = vec![0.01, 0.04, 0.10, 0.30];
    let gate_monitor = PacBayesMonitor::with_config(PacBayesConfig {
        delta: 0.05,
        target_risk: 0.08,
        risk_margin: 0.01,
    })
    .unwrap();

    let gate = gate_monitor
        .run_promotion_gate(&candidate_losses, 1500)
        .expect("gate run");

    assert_eq!(gate.verdict, PolicyPromotionVerdict::PromoteToProduction);
    assert!(gate.certified_risk_bound <= 0.08);
    assert!(gate.risk_margin > 0.0);
    assert!(gate.optimal_temperature_beta > 0.0);
}

#[test]
fn theorem_39_40_tropical_dag_kleene_star_and_karp_cycle_mean() {
    let scheduler = TropicalDagScheduler::new();

    // 1. Theorem 39: DAG Critical Path Makespan
    // 5-task DAG: 0 -> 1 (12ms), 0 -> 2 (8ms), 1 -> 3 (15ms), 2 -> 3 (20ms), 3 -> 4 (10ms)
    // Longest path: 0 -> 2 -> 3 -> 4 = 8 + 20 + 10 = 38 ms
    let mut dag = MaxPlusMatrix::zero(5).unwrap();
    dag.set(0, 1, 12.0).unwrap();
    dag.set(0, 2, 8.0).unwrap();
    dag.set(1, 3, 15.0).unwrap();
    dag.set(2, 3, 20.0).unwrap();
    dag.set(3, 4, 10.0).unwrap();

    let star = scheduler.compute_kleene_star(&dag).expect("kleene star");
    assert!(!star.has_positive_cycle);
    assert_eq!(star.critical_path_makespan, 38.0);
    assert_eq!(star.closure.data[0][4], 38.0);
    assert_eq!(star.closure.data[0][3], 28.0);

    // 2. Theorem 39: Positive Cycle Detection
    let mut cyclic = MaxPlusMatrix::zero(3).unwrap();
    cyclic.set(0, 1, 10.0).unwrap();
    cyclic.set(1, 2, 10.0).unwrap();
    cyclic.set(2, 0, 5.0).unwrap(); // Cycle 0 -> 1 -> 2 -> 0 with weight 25 > 0

    let err = scheduler.compute_kleene_star(&cyclic).unwrap_err();
    match err {
        TropicalError::PositiveCycleDetected { cycle_weight, .. } => {
            assert!(cycle_weight >= 25.0);
        }
        _ => panic!("expected positive cycle error"),
    }

    // 3. Theorem 40: Karp Maximum Cycle Mean
    // 3-node graph with cycles of mean weights 3.0, 5.0, 4.0
    let mut karp_graph = MaxPlusMatrix::zero(3).unwrap();
    karp_graph.set(0, 0, 3.0).unwrap();
    karp_graph.set(1, 1, 5.0).unwrap();
    karp_graph.set(2, 2, 4.0).unwrap();

    let karp_res = scheduler
        .compute_karp_maximum_cycle_mean(&karp_graph)
        .expect("karp cycle mean");

    assert!((karp_res.maximum_cycle_mean - 5.0).abs() < 1e-6);
    assert!((karp_res.asymptotic_throughput - 0.20).abs() < 1e-6);
    assert!(karp_res.sla_satisfied);

    // SLA violation check
    let sla_res = scheduler
        .check_sla_compliance(&karp_graph, 4.5)
        .expect("sla check");
    assert!(!sla_res.sla_satisfied); // 5.0 > 4.5 => SLA violated, deadlock risk
}
