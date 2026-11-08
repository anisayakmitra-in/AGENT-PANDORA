#![forbid(unsafe_code)]

use pandora_runtime::bandit_router::SafePrng;
use pandora_runtime::byzantine_aggregator::{
    AgentVectorUpdate, AggregationRule, ByzantineAggregator, ByzantineError,
};
use pandora_runtime::martingale_gate::{CanaryGateVerdict, MartingaleCanaryGate};
use pandora_runtime::wasserstein_drift::{EmpiricalDistribution, SinkhornDriftDetector};

#[test]
fn test_theorem_23_trimmed_mean_breakdown_and_infinite_adversary_immunity() {
    let n = 15;
    let f = 4; // 2f = 8 < 15
    let d = 6;
    let true_mu: Vec<f64> = (0..d).map(|j| (j as f64) * 10.0 + 5.0).collect();

    let mut updates = Vec::with_capacity(n);
    for i in 0..n {
        if i < n - f {
            let honest: Vec<f64> = (0..d)
                .map(|j| true_mu[j] + (((i as isize) - 5) as f64) * 0.01)
                .collect();
            updates.push(AgentVectorUpdate::new(format!("honest_{i}"), honest));
        } else {
            // Extreme omniscient Byzantine attack (10^9 magnitude)
            let byz: Vec<f64> = (0..d).map(|j| 1e9 * ((j + 1) as f64)).collect();
            updates.push(AgentVectorUpdate::new(format!("byz_{i}"), byz));
        }
    }

    let tm_agg = ByzantineAggregator::new(f, AggregationRule::TrimmedMean { beta: 0.3 }).unwrap();
    let res = tm_agg.aggregate(&updates).unwrap();

    let robust_err = ByzantineAggregator::l2_distance(&res.consensus_vector, &true_mu);
    let naive_err = ByzantineAggregator::l2_distance(&res.naive_mean_vector, &true_mu);

    assert!(
        robust_err < 0.2,
        "Trimmed mean error {robust_err} must remain tightly bounded despite 1e9 Byzantine attack"
    );
    assert!(
        naive_err > 1e8,
        "Naive mean error {naive_err} is catastrophically corrupted"
    );

    // Verify breakdown condition 2f >= n fails closed
    let invalid_agg =
        ByzantineAggregator::new(8, AggregationRule::TrimmedMean { beta: 0.2 }).unwrap();
    assert!(matches!(
        invalid_agg.aggregate(&updates),
        Err(ByzantineError::ByzantineQuorumViolated { .. })
    ));
}

#[test]
fn test_theorem_24_multi_krum_cone_deviation_and_outlier_identification() {
    let n = 11;
    let f = 3; // 2f + 2 = 8 < 11
    let d = 8;
    let true_mu: Vec<f64> = vec![3.125; d];

    let mut updates = Vec::with_capacity(n);
    for i in 0..(n - f) {
        let honest: Vec<f64> = (0..d)
            .map(|j| true_mu[j] + (((i + j) % 3) as f64 - 1.0) * 0.05)
            .collect();
        updates.push(AgentVectorUpdate::new(format!("honest_{i}"), honest));
    }
    for i in (n - f)..n {
        let byz: Vec<f64> = vec![-50_000.0 * ((i + 1) as f64); d];
        updates.push(AgentVectorUpdate::new(format!("byzantine_{i}"), byz));
    }

    let krum_agg =
        ByzantineAggregator::new(f, AggregationRule::MultiKrum { selection_q: 5 }).unwrap();
    let res = krum_agg.aggregate(&updates).unwrap();

    // Every selected agent must be honest
    for sel in &res.selected_agents {
        assert!(
            sel.starts_with("honest_"),
            "Multi-Krum selected a Byzantine agent: {sel}"
        );
    }
    // Every suspected outlier must be Byzantine
    assert_eq!(res.suspected_outliers.len(), f);
    for out in &res.suspected_outliers {
        assert!(
            out.starts_with("byzantine_"),
            "Expected Byzantine outlier, got {out}"
        );
    }

    let robust_err = ByzantineAggregator::l2_distance(&res.consensus_vector, &true_mu);
    let theo_bound = res.theoretical_deviation_factor * 0.05 * (d as f64).sqrt();
    assert!(
        robust_err <= theo_bound,
        "Krum error {robust_err} exceeded Theorem 24 bound {theo_bound}"
    );
}

#[test]
fn test_theorems_25_and_26_sinkhorn_contraction_debiased_positivity_and_exact_shift() {
    let d = 4;
    let n = 12;
    let shift = 2.5;

    let mut pts_mu = Vec::with_capacity(n);
    let mut pts_nu = Vec::with_capacity(n);
    for i in 0..n {
        let base: Vec<f64> = (0..d).map(|j| ((i + j) as f64) * 0.05).collect();
        // Translate every point by uniform vector of L2 norm = `shift`
        let delta = shift / (d as f64).sqrt();
        let shifted: Vec<f64> = base.iter().map(|&x| x + delta).collect();
        pts_mu.push(base);
        pts_nu.push(shifted);
    }

    let mu = EmpiricalDistribution::uniform(pts_mu).unwrap();
    let nu = EmpiricalDistribution::uniform(pts_nu).unwrap();

    let detector = SinkhornDriftDetector::new(0.5, 500, 1e-10, 1.0).unwrap();

    // 1. Identity of indiscernibles: S_eps(mu, mu) == 0
    let self_report = detector.evaluate_drift(&mu, &mu).unwrap();
    assert!(
        self_report.sinkhorn_divergence.abs() < 1e-9,
        "Debiased Sinkhorn divergence S_eps(mu, mu) must be 0, got {}",
        self_report.sinkhorn_divergence
    );
    assert!(!self_report.drift_detected);

    // 2. Shifted distribution: W_2(mu, nu) recovers the exact translation norm `shift`
    let shift_report = detector.evaluate_drift(&mu, &nu).unwrap();
    assert!(
        shift_report.birkhoff_contraction_factor < 1.0,
        "Birkhoff contraction factor must be strictly < 1.0"
    );
    assert!(
        shift_report.marginal_conservation_error < 1e-6,
        "Marginal conservation error too large: {}",
        shift_report.marginal_conservation_error
    );
    assert!(
        (shift_report.wasserstein_2_distance - shift).abs() < 0.08,
        "Expected W_2 distance ~ {shift}, got {}",
        shift_report.wasserstein_2_distance
    );
    assert!(shift_report.drift_detected);
}

#[test]
fn test_theorems_27_and_28_ville_maximal_inequality_and_confidence_sequence() {
    let alpha = 0.05;
    let trials = 200;
    let horizon = 100;
    let mut false_alarms = 0usize;
    let mut rng = SafePrng::new(0x1234_5678_9abc_def0);

    // Under H_0: X_t in {-0.5, +0.5} with mean 0 (fair coin)
    for _ in 0..trials {
        let mut gate = MartingaleCanaryGate::new(alpha, 0.5, 15).unwrap();
        let mut rejected = false;
        for _ in 0..horizon {
            let x_t = if rng.sample_uniform_f64() < 0.5 {
                0.5
            } else {
                -0.5
            };
            let snap = gate.observe(x_t).unwrap();
            if snap.e_value >= snap.ville_rejection_threshold {
                rejected = true;
                break;
            }
        }
        if rejected {
            false_alarms += 1;
        }
    }

    let empirical_type1 = (false_alarms as f64) / (trials as f64);
    assert!(
        empirical_type1 <= alpha + 0.02,
        "Empirical Type-I error {empirical_type1} violated Ville's inequality bound alpha={alpha}"
    );

    // Under H_1 (severe canary regression X_t = +0.4): gate must trigger RollbackRegression
    let mut reg_gate = MartingaleCanaryGate::new(0.05, 0.5, 10).unwrap();
    for _ in 0..50 {
        reg_gate.observe(0.4).unwrap();
    }
    let reg_snap = reg_gate.snapshot().unwrap();
    assert_eq!(reg_snap.verdict, CanaryGateVerdict::RollbackRegression);
    assert!(reg_snap.e_value >= reg_snap.ville_rejection_threshold);

    // Under strict canary improvement (X_t = -0.45): gate must trigger PromoteCanary
    let mut prom_gate = MartingaleCanaryGate::new(0.05, 0.5, 10).unwrap();
    for _ in 0..60 {
        prom_gate.observe(-0.45).unwrap();
    }
    let prom_snap = prom_gate.snapshot().unwrap();
    assert_eq!(prom_snap.verdict, CanaryGateVerdict::PromoteCanary);
    assert!(prom_snap.upper_confidence_bound < 0.0);
}
