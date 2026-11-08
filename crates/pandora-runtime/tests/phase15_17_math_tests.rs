#![forbid(unsafe_code)]

use pandora_runtime::adversarial_bandit::Exp3AdversarialBandit;
use pandora_runtime::assurance_receipt::UnifiedAssuranceReceipt;
use pandora_runtime::conformal_calibrator::{
    AdaptiveConformalTracker, CandidateAction, ConformalGovernanceVerdict, SplitConformalCalibrator,
};
use pandora_runtime::spectral_topology::{SpectralTopologyAnalyzer, TopologyVerdict, WeightedEdge};

#[test]
fn theorem_29_30_conformal_calibration_and_aci_shift_resilience() {
    let scores: Vec<f64> = (1..=99).map(|i| i as f64 / 100.0).collect();
    let calibrator = SplitConformalCalibrator::calibrate(&scores, 0.10).unwrap();
    let summary = calibrator.summary();
    assert_eq!(summary.order_statistic_rank, 90);
    assert!((summary.quantile_threshold - 0.90).abs() < 1e-12);
    assert!(summary.marginal_coverage_lower_bound >= 0.90 - 1e-12);

    let report = calibrator
        .predict_set(&[
            CandidateAction {
                action_id: "safe_op".to_owned(),
                nonconformity_score: 0.35,
            },
            CandidateAction {
                action_id: "risky_op".to_owned(),
                nonconformity_score: 0.95,
            },
        ])
        .unwrap();
    assert_eq!(report.verdict, ConformalGovernanceVerdict::CertifySingleton);
    assert_eq!(report.prediction_set, vec!["safe_op".to_owned()]);

    let mut aci = AdaptiveConformalTracker::new(0.10, 0.04).unwrap();
    for t in 0..400 {
        let q_t = calibrator.quantile_for_alpha(aci.status().current_alpha_t);
        let test_s = if t % 4 == 0 { 0.96 } else { 0.40 };
        aci.observe_step(test_s > q_t);
    }
    let aci_status = aci.status();
    assert!(aci_status.within_theoretical_bound);
}

#[test]
fn theorem_31_32_spectral_topology_fiedler_and_cheeger_inequality() {
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
            weight: 0.08,
        },
    ];
    let analyzer = SpectralTopologyAnalyzer::new(6, edges, 0.25).unwrap();
    let report = analyzer.analyze();
    assert!(report.fiedler_value_unnormalized > 0.0);
    assert!(report.cheeger_inequality_verified);
    assert!(report.sweep_cut_conductance >= report.cheeger_lower_bound - 1e-7);
    assert!(report.sweep_cut_conductance <= report.cheeger_upper_bound + 1e-7);
    assert_eq!(report.verdict, TopologyVerdict::BottleneckWarning);
}

#[test]
fn theorem_33_34_exp3_adversarial_bandit_regret_bound() {
    let arms = vec![
        "arm_alpha".to_owned(),
        "arm_beta".to_owned(),
        "arm_gamma".to_owned(),
        "arm_delta".to_owned(),
    ];
    let horizon = 500usize;
    let mut bandit = Exp3AdversarialBandit::with_horizon(arms, horizon).unwrap();
    let mut best_arm_sum = [0.0_f64; 4];

    for t in 0..horizon {
        let u = ((t as f64 + 1.0) * 0.372_583_002).fract();
        let (chosen, _, p_chosen) = bandit.select_arm(u).unwrap();
        assert!(p_chosen >= bandit.status().gamma / 4.0 - 1e-12);

        let rewards = [0.20, 0.30, if t % 8 == 0 { 0.30 } else { 0.92 }, 0.40];
        for (acc, &r) in best_arm_sum.iter_mut().zip(rewards.iter()) {
            *acc += r;
        }
        bandit.update_reward(chosen, rewards[chosen]).unwrap();
    }

    let status = bandit.status();
    let best_fixed = best_arm_sum.into_iter().fold(0.0_f64, f64::max);
    let realized_regret = best_fixed - status.cumulative_observed_reward;
    assert!(realized_regret <= status.minimax_regret_upper_bound);
}

#[test]
fn unified_assurance_pipeline_certifies_phases_9_through_17() {
    let receipt = UnifiedAssuranceReceipt::generate("integration-suite-v306").unwrap();
    assert!(receipt.all_invariants_verified);
    assert_eq!(receipt.phases_certified, 9);
    assert_eq!(receipt.certificates.len(), 9);
    for (idx, cert) in receipt.certificates.iter().enumerate() {
        assert_eq!(cert.phase as usize, 9 + idx);
        assert!(cert.invariant_verified);
    }
}
