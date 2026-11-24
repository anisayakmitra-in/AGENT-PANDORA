#![forbid(unsafe_code)]

//! Integration test suite for Phase 15, Phase 16, and Phase 17 mathematical guarantees and CLI workflows.
//!
//! - Phase 15: Conformal Prediction & Distribution-Free Uncertainty Calibration (Theorems 29 & 30)
//! - Phase 16: Spectral Graph Partitioning & Algebraic Connectivity (Theorems 31 & 32)
//! - Phase 17: Exp3 Adversarial Multi-Armed Bandit Router (Theorems 33 & 34)

use pandora_runtime::adversarial_bandit::Exp3AdversarialBandit;
use pandora_runtime::conformal_calibrator::{
    AdaptiveConformalTracker, CandidateAction, ConformalGovernanceVerdict, SplitConformalCalibrator,
};
use pandora_runtime::spectral_topology::{SpectralTopologyAnalyzer, TopologyVerdict, WeightedEdge};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(850);

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory should have a canonical path");
        let root = temp_root.join(format!(
            "pandora-phase1517-test-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::create_dir_all(&data).expect("data dir should be created");
        Self {
            config: root.join("config.json"),
            data,
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env("PANDORA_PROVIDER_API_KEY", "test-api-key");
        cmd
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_phase15_split_conformal_theorems_29_and_30() {
    // 1. Theorem 29 Finite-Sample Exchangeable Coverage Guarantee
    // n = 99 calibration scores, alpha = 0.10
    // rank k = ceil((99 + 1) * 0.90) = 90
    let n = 99;
    let scores: Vec<f64> = (1..=n).map(|i| i as f64 / (n as f64)).collect();
    let cal =
        SplitConformalCalibrator::calibrate(&scores, 0.10).expect("calibration should succeed");
    let summary = cal.summary();

    assert_eq!(summary.sample_size, 99);
    assert_eq!(summary.order_statistic_rank, 90);
    // Lower bound: 1 - alpha = 0.90
    // Upper bound: 1 - alpha + 1/(n + 1) = 0.90 + 0.01 = 0.91
    assert!((summary.marginal_coverage_lower_bound - 0.90).abs() < 1e-12);
    assert!((summary.marginal_coverage_upper_bound - 0.91).abs() < 1e-12);

    // 2. Conformal Prediction Set Governance Verdicts
    let singleton = vec![
        CandidateAction {
            action_id: "allow_tool".to_owned(),
            nonconformity_score: 0.15,
        },
        CandidateAction {
            action_id: "reject_tool".to_owned(),
            nonconformity_score: 0.98,
        },
    ];
    let rep_singleton = cal.predict_set(&singleton).expect("predict singleton");
    assert_eq!(
        rep_singleton.verdict,
        ConformalGovernanceVerdict::CertifySingleton
    );
    assert_eq!(rep_singleton.prediction_set, vec!["allow_tool".to_owned()]);

    let ambiguous = vec![
        CandidateAction {
            action_id: "action_1".to_owned(),
            nonconformity_score: 0.40,
        },
        CandidateAction {
            action_id: "action_2".to_owned(),
            nonconformity_score: 0.50,
        },
    ];
    let rep_ambiguous = cal.predict_set(&ambiguous).expect("predict ambiguous");
    assert_eq!(
        rep_ambiguous.verdict,
        ConformalGovernanceVerdict::EscalateAmbiguous
    );
    assert_eq!(rep_ambiguous.set_size, 2);

    let empty = vec![CandidateAction {
        action_id: "risky_mutation".to_owned(),
        nonconformity_score: 0.99,
    }];
    let rep_empty = cal.predict_set(&empty).expect("predict empty");
    assert_eq!(rep_empty.verdict, ConformalGovernanceVerdict::AbstainEmpty);
    assert_eq!(rep_empty.set_size, 0);

    // 3. Theorem 30 Adaptive Conformal Inference (ACI) under distribution shift
    let mut aci = AdaptiveConformalTracker::new(0.10, 0.05).expect("aci init");
    for t in 0..300 {
        let q_t = cal.quantile_for_alpha(aci.status().current_alpha_t);
        // Non-stationary test score process
        let test_score = if t % 4 == 0 { 0.97 } else { 0.35 };
        aci.observe_step(test_score > q_t);
    }
    let status = aci.status();
    assert!(status.within_theoretical_bound);
    let dev = (status.empirical_miscoverage_rate - 0.10).abs();
    assert!(dev <= status.theoretical_error_bound + 1e-12);

    // 4. CLI Execution: pandora runtime conformal --json
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "runtime",
            "conformal",
            "--samples",
            "99",
            "--alpha",
            "0.10",
            "--gamma",
            "0.05",
            "--steps",
            "200",
            "--json",
        ])
        .output()
        .expect("CLI execution should succeed");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("valid JSON expected");
    assert_eq!(json["command"], "runtime conformal");
    assert_eq!(json["calibration"]["sample_size"], 99);
    assert_eq!(json["calibration"]["order_statistic_rank"], 90);
    assert_eq!(json["prediction"]["verdict"], "certify_singleton");
    assert_eq!(json["aci_status"]["within_theoretical_bound"], true);
}

#[test]
fn test_phase16_spectral_graph_topology_theorems_31_and_32() {
    // 1. Theorem 31 Courant-Fischer Variational Fiedler Value & Connectivity
    // Fully connected clique K_4 has algebraic connectivity lambda_2(L) = 4.0
    let mut edges = Vec::new();
    for u in 0..4 {
        for v in (u + 1)..4 {
            edges.push(WeightedEdge { u, v, weight: 1.0 });
        }
    }
    let analyzer = SpectralTopologyAnalyzer::new(4, edges, 0.20).expect("analyzer init");
    let report = analyzer.analyze();
    assert!((report.fiedler_value_unnormalized - 4.0).abs() < 1e-4);
    assert!((report.fiedler_value_normalized - (4.0 / 3.0)).abs() < 1e-4);
    assert_eq!(report.verdict, TopologyVerdict::ConnectedResilient);

    // 2. Theorem 32 Cheeger's Inequality nu_2 / 2 <= h(G) <= sqrt(2 * nu_2)
    assert!(report.cheeger_inequality_verified);
    assert!(report.sweep_cut_conductance + 1e-7 >= report.cheeger_lower_bound);
    assert!(report.sweep_cut_conductance <= report.cheeger_upper_bound + 1e-7);

    // 3. Barbell Graph with narrow bridge bottleneck
    let barbell_edges = vec![
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
        }, // narrow bridge
    ];
    let barbell_analyzer =
        SpectralTopologyAnalyzer::new(6, barbell_edges, 0.20).expect("barbell init");
    let barbell_report = barbell_analyzer.analyze();
    assert_eq!(barbell_report.verdict, TopologyVerdict::BottleneckWarning);
    assert!(barbell_report.sweep_cut_conductance < 0.20);
    // Bisection cleanly separates left and right triangles
    assert_eq!(barbell_report.partition_left.len(), 3);
    assert_eq!(barbell_report.partition_right.len(), 3);

    // 4. CLI Execution: pandora runtime topology --json
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "runtime",
            "topology",
            "--nodes",
            "6",
            "--bridge-weight",
            "0.05",
            "--threshold",
            "0.20",
            "--json",
        ])
        .output()
        .expect("CLI execution should succeed");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("valid JSON expected");
    assert_eq!(json["command"], "runtime topology");
    assert_eq!(json["node_count"], 6);
    assert_eq!(json["cheeger_inequality_verified"], true);
    assert_eq!(json["verdict"], "bottleneck_warning");
}

#[test]
fn test_phase17_exp3_adversarial_bandit_theorems_33_and_34() {
    let arm_ids = vec![
        "arm_a".to_owned(),
        "arm_b".to_owned(),
        "arm_c".to_owned(),
        "arm_d".to_owned(),
    ];
    let k = arm_ids.len();
    let horizon = 400;

    // 1. Theorem 34 Minimax Regret Bound 2 * sqrt((e - 1) * T * K * ln(K))
    let mut exp3 = Exp3AdversarialBandit::with_horizon(arm_ids, horizon).expect("bandit init");
    let theoretical_regret = 2.0
        * ((std::f64::consts::E - 1.0) * (horizon as f64) * (k as f64) * (k as f64).ln()).sqrt();
    assert!((exp3.theoretical_regret_bound(horizon) - theoretical_regret).abs() < 1e-8);

    // 2. Theorem 33 Unbiased Importance-Weighted Estimator & Sampling
    for t in 0..horizon {
        let u = ((t as f64 + 1.0) * 0.472_583_002).fract();
        let (arm, _arm_id, prob) = exp3.select_arm(u).expect("arm select");
        assert!(prob >= exp3.status().gamma / (k as f64));
        let reward = if arm == 0 { 0.90 } else { 0.20 };
        exp3.update_reward(arm, reward).expect("reward update");
    }

    let final_status = exp3.status();
    assert_eq!(final_status.rounds_elapsed, horizon);
    assert!((final_status.minimax_regret_upper_bound - theoretical_regret).abs() < 1e-8);
    assert!(final_status.cumulative_observed_reward > 0.0);
    // Probabilities sum to 1.0
    let prob_sum: f64 = final_status
        .arms
        .iter()
        .map(|a| a.sampling_probability)
        .sum();
    assert!((prob_sum - 1.0).abs() < 1e-10);
    // Arm 0 (best arm) received the largest sampling probability
    assert_eq!(final_status.arms[0].arm_id, "arm_a");
    assert!(final_status.arms[0].sampling_probability > final_status.arms[1].sampling_probability);

    // 3. CLI Execution: pandora runtime bandit --json
    let fixture = Fixture::new();
    let output = fixture
        .command(&[
            "runtime",
            "bandit",
            "--arms",
            "4",
            "--horizon",
            "200",
            "--gamma",
            "0.10",
            "--json",
        ])
        .output()
        .expect("CLI execution should succeed");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("valid JSON expected");
    assert_eq!(json["command"], "runtime bandit");
    assert_eq!(json["num_arms"], 4);
    assert_eq!(json["rounds_elapsed"], 200);
    assert_eq!(json["gamma"], 0.10);
    assert!(json["cumulative_observed_reward"].as_f64().unwrap() > 0.0);
    assert!(json["minimax_regret_upper_bound"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_phase15_17_cockpit_full_spectrum_12_panel_surfacing() {
    let fixture = Fixture::new();
    let output = fixture
        .command(&["cockpit", "status", "--panels", "12", "--json"])
        .output()
        .expect("cockpit command should succeed");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("valid JSON expected");

    assert_eq!(json["panel_count"], 12);
    assert!(json["snapshot_digest"].is_string());

    let panels = json["panels"].as_array().expect("panels array");
    assert_eq!(panels.len(), 12);

    // Check Panel 7: TropicalDag with spectral topology metrics
    let p7 = panels
        .iter()
        .find(|p| p["title"] == "[07:Tropical-DAG]")
        .expect("panel 7");
    assert!(p7["metrics"]["fiedler_value"].is_string());
    assert!(p7["metrics"]["sweep_cut_conductance"].is_string());
    assert!(p7["metrics"]["dag_tasks"].is_string());

    // Check Panel 8: BanditRouter with Exp3 routing metrics
    let p8 = panels
        .iter()
        .find(|p| p["title"] == "[08:Bandit-Router]")
        .expect("panel 8");
    assert!(p8["metrics"]["exp3_gamma"].is_string());
    assert!(p8["metrics"]["candidate_arms"].is_string());

    // Check Panel 9: DriftConformal with Phase 13, 14, and 15 metrics
    let p9 = panels
        .iter()
        .find(|p| p["title"] == "[09:Drift-Conformal]")
        .expect("panel 9");
    assert!(p9["metrics"]["conformal_quantile"].is_string());
    assert!(p9["metrics"]["conformal_coverage"].is_string());
    assert!(p9["metrics"]["conformal_verdict"].is_string());
    assert!(p9["metrics"]["sinkhorn_divergence"].is_string());
    assert!(p9["metrics"]["martingale_e_value"].is_string());
}
