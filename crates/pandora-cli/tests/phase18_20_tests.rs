#![forbid(unsafe_code)]

//! Integration test suite for Phase 18, Phase 19, Phase 20, and 12-Engine Unified Assurance Receipt.
//!
//! - Phase 18: Mean-Field Game (MFG) Resource Allocation & Congestion Control (Theorems 35 & 36)
//! - Phase 19: PAC-Bayes Policy Generalization & Variational KL Monitor (Theorems 37 & 38)
//! - Phase 20: Tropical Max-Plus DAG Calculus & Karp Cycle Mean (Theorems 39 & 40)
//! - 12-Engine Unified Mathematical Assurance Receipt Chain (Phases 9-20, Theorems 17-40)
//! - Mission-Control Cockpit 12-Panel Telemetry Surfacing

use pandora_runtime::assurance_receipt::UnifiedAssuranceReceipt;
use pandora_runtime::mean_field_game::{
    MeanFieldGame, MfgResourceConfig, THEORETICAL_POA_UPPER_BOUND,
};
use pandora_runtime::pac_bayes_monitor::{
    PacBayesConfig, PacBayesMonitor, PolicyPromotionVerdict, binary_kl,
};
use pandora_runtime::tropical_dag::{MaxPlusMatrix, TropicalDagScheduler};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1820);

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
            "pandora-phase1820-test-{}-{timestamp}-{sequence}",
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
fn test_phase18_mean_field_game_theorems_35_and_36() {
    let fixture = Fixture::new();

    // 1. Theorem 35: Banach Fixed-Point Contraction Guarantee
    // When kappa = beta_max / (2 * tau) < 1, F is a strict contraction on simplex Delta^K.
    let resources = vec![
        MfgResourceConfig::new("gpu_node_1", 1.0, 1.2).expect("res 1"),
        MfgResourceConfig::new("gpu_node_2", 1.5, 0.8).expect("res 2"),
        MfgResourceConfig::new("gpu_node_3", 2.0, 0.5).expect("res 3"),
    ];
    let temperature_tau = 2.0; // beta_max = 1.2, kappa = 1.2 / 4.0 = 0.30 < 1.0
    let mfg = MeanFieldGame::with_damping(resources, temperature_tau, 0.5).expect("mfg engine");

    let status = mfg.status().expect("status");
    assert!(status.fixed_point.is_strict_contraction);
    assert!(status.contraction_modulus < 1.0);
    assert!((status.contraction_modulus - 0.30).abs() < 1e-6);

    let eq = mfg.compute_equilibrium(100, 1e-8).expect("equilibrium");
    assert!(eq.is_strict_contraction);
    assert!(eq.residual_norm < 1e-8);
    // Simplex sum check: sum(m*) = 1.0
    let sum_m: f64 = eq.equilibrium_distribution.iter().sum();
    assert!((sum_m - 1.0).abs() < 1e-6);

    // 2. Theorem 36: Price of Anarchy Bound PoA <= 4/3
    let poa = mfg.compute_price_of_anarchy().expect("poa");
    assert!(poa.bound_satisfied);
    assert!(poa.price_of_anarchy <= THEORETICAL_POA_UPPER_BOUND + 1e-6);
    assert!(poa.price_of_anarchy >= 1.0);
    assert!(poa.nash_social_cost >= poa.optimal_social_cost - 1e-6);

    // 3. CLI execution: pandora runtime mfg --resources 3 --temperature 2.0 --json
    let out = fixture
        .command(&[
            "runtime",
            "mfg",
            "--resources",
            "3",
            "--temperature",
            "2.0",
            "--json",
        ])
        .output()
        .expect("CLI execution should run");

    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).expect("valid JSON response");
    assert_eq!(json["command"], "runtime mfg");
    assert_eq!(json["num_resources"], 3);
    assert_eq!(json["fixed_point"]["is_strict_contraction"], true);
    assert_eq!(json["poa"]["bound_satisfied"], true);
    let poa_ratio = json["poa"]["price_of_anarchy"].as_f64().expect("poa float");
    assert!(poa_ratio <= 1.333334);
}

#[test]
fn test_phase19_pac_bayes_monitor_theorems_37_and_38() {
    let fixture = Fixture::new();

    // 1. Theorem 37: PAC-Bayes Risk Inversion and binary KL
    let p = 0.05;
    let q = 0.12;
    let kl = binary_kl(p, q);
    assert!(kl > 0.0);
    assert!(kl.is_finite());

    let monitor = PacBayesMonitor::with_config(PacBayesConfig {
        delta: 0.05,
        target_risk: 0.10,
        risk_margin: 0.01,
    })
    .expect("monitor");

    let cert = monitor
        .certify_risk(0.04, 500, 0.15, 0.05)
        .expect("certify");
    assert!(cert.maurer_risk_bound >= 0.04);
    assert!(cert.bound_verified);

    // 2. Theorem 38: Catoni Optimal Gibbs Posterior & Policy Promotion Gate
    let losses = vec![0.02, 0.05, 0.08, 0.14];
    let report = monitor.run_promotion_gate(&losses, 400).expect("gate");
    assert!(report.optimal_temperature_beta > 0.0);
    assert_eq!(report.gibbs_posterior.len(), 4);
    let posterior_sum: f64 = report.gibbs_posterior.iter().sum();
    assert!((posterior_sum - 1.0).abs() < 1e-6);
    // Lower loss hypotheses receive strictly higher Gibbs posterior weight
    assert!(report.gibbs_posterior[0] > report.gibbs_posterior[1]);
    assert!(report.gibbs_posterior[1] > report.gibbs_posterior[2]);
    assert!(report.gibbs_posterior[2] > report.gibbs_posterior[3]);
    assert_eq!(report.verdict, PolicyPromotionVerdict::PromoteToProduction);

    // 3. CLI execution: pandora runtime pac-bayes --samples 300 --delta 0.05 --target-risk 0.15 --json
    let out = fixture
        .command(&[
            "runtime",
            "pac-bayes",
            "--samples",
            "300",
            "--delta",
            "0.05",
            "--target-risk",
            "0.15",
            "--json",
        ])
        .output()
        .expect("CLI execution should run");

    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).expect("valid JSON response");
    assert_eq!(json["command"], "runtime pac-bayes");
    assert_eq!(json["verdict"], "PromoteToProduction");
    assert_eq!(json["target_risk"], 0.15);
    assert!(
        json["certified_risk_bound"]
            .as_f64()
            .expect("certified bound")
            < 0.15
    );
}

#[test]
fn test_phase20_tropical_dag_theorems_39_and_40() {
    let fixture = Fixture::new();

    // 1. Theorem 39: Tropical Max-Plus Makespan & Kleene Star Closure
    let scheduler = TropicalDagScheduler::new();
    let mut adj = MaxPlusMatrix::zero(4).expect("zero matrix");
    adj.set(0, 1, 10.0).expect("edge 0->1");
    adj.set(1, 2, 15.0).expect("edge 1->2");
    adj.set(2, 3, 20.0).expect("edge 2->3");

    let star = scheduler.compute_kleene_star(&adj).expect("kleene star");
    assert!(!star.has_positive_cycle);
    assert_eq!(star.closure.size, 4);
    // Pipeline: 10 + 15 + 20 = 45.0
    assert_eq!(star.critical_path_makespan, 45.0);
    assert_eq!(star.closure.data[0][3], 45.0);

    // 2. Theorem 40: Karp Cycle Mean & Deadlock Cycle Detection
    let mut cyclic = MaxPlusMatrix::zero(3).expect("cyclic matrix");
    cyclic.set(0, 0, 3.0).expect("loop 0");
    cyclic.set(1, 1, 5.0).expect("loop 1");
    cyclic.set(2, 2, 4.0).expect("loop 2");
    let karp = scheduler
        .compute_karp_maximum_cycle_mean(&cyclic)
        .expect("karp result");
    assert!((karp.maximum_cycle_mean - 5.0).abs() < 1e-6);
    assert!((karp.asymptotic_throughput - 0.20).abs() < 1e-6);
    assert_eq!(karp.critical_vertex, 1);
    assert!(karp.sla_satisfied); // 5.0 <= 50.0 SLA

    // 3. CLI execution: pandora runtime tropical --nodes 4 --topology pipeline --json
    let out = fixture
        .command(&[
            "runtime",
            "tropical",
            "--nodes",
            "4",
            "--topology",
            "pipeline",
            "--sla-threshold",
            "50.0",
            "--json",
        ])
        .output()
        .expect("CLI execution should run");

    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).expect("valid JSON response");
    assert_eq!(json["command"], "runtime tropical");
    assert_eq!(json["nodes"], 4);
    assert_eq!(json["has_positive_cycle"], false);
    assert_eq!(json["sla_satisfied"], true);
    assert_eq!(json["critical_path_makespan"], 60.0);
}

#[test]
fn test_unified_mathematical_assurance_receipt_12_engines() {
    let fixture = Fixture::new();

    // 1. Generate chained UnifiedAssuranceReceipt spanning all 12 engines (Theorems 17-40)
    let receipt1 = UnifiedAssuranceReceipt::generate_chained("pandora-cert-run-1", 1, None)
        .expect("receipt 1");
    assert_eq!(receipt1.phases_certified, 12);
    assert_eq!(receipt1.certificates.len(), 12);
    assert!(receipt1.all_invariants_verified);
    assert!(receipt1.verify_hash_integrity());
    assert_eq!(receipt1.sequence_number, 1);
    assert!(receipt1.prev_receipt_sha256.is_none());

    // 2. Cryptographic Hash Chain: Receipt 2 binds to Receipt 1
    let mut receipt2 = UnifiedAssuranceReceipt::generate_chained(
        "pandora-cert-run-2",
        2,
        Some(receipt1.receipt_sha256.clone()),
    )
    .expect("receipt 2");
    assert_eq!(receipt2.phases_certified, 12);
    assert!(receipt2.all_invariants_verified);
    assert_eq!(receipt2.sequence_number, 2);
    assert_eq!(
        receipt2.prev_receipt_sha256,
        Some(receipt1.receipt_sha256.clone())
    );
    assert!(receipt2.verify_hash_integrity());

    // 3. Ed25519 / HMAC signing
    receipt2.sign_with_key("master-assurance-key");
    assert!(receipt2.signature.is_some());
    assert!(receipt2.signature.as_ref().unwrap().starts_with("sig:"));

    // 4. CLI execution: pandora runtime assurance --run-id test-assurance-cli --json
    let out = fixture
        .command(&[
            "runtime",
            "assurance",
            "--run-id",
            "test-assurance-cli",
            "--json",
        ])
        .output()
        .expect("CLI execution should run");

    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).expect("valid JSON response");
    assert_eq!(json["command"], "runtime assurance");
    assert_eq!(json["run_id"], "test-assurance-cli");
    assert_eq!(json["phases_certified"], 12);
    assert_eq!(json["all_invariants_verified"], true);
    assert!(
        json["receipt_sha256"]
            .as_str()
            .expect("receipt digest")
            .starts_with("sha256:")
    );
}

#[test]
fn test_cockpit_12_panel_status_surfacing() {
    let fixture = Fixture::new();

    let out = fixture
        .command(&["cockpit", "status", "--panels", "12", "--json"])
        .output()
        .expect("cockpit status should run");

    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).expect("valid JSON response");
    assert_eq!(json["command"], "cockpit status");
    assert_eq!(json["layout"], "twelve_panel");
    assert_eq!(json["panel_count"], 12);

    let panels = json["panels"].as_array().expect("panels array");
    assert_eq!(panels.len(), 12);

    // Verify key mathematical and assurance panels are present
    let panel_titles: Vec<&str> = panels.iter().filter_map(|p| p["title"].as_str()).collect();

    assert!(panel_titles.contains(&"[01:Daemon-Guard]"));
    assert!(panel_titles.contains(&"[02:Assurance-12]"));
    assert!(panel_titles.contains(&"[03:Genes-Evolution]"));
    assert!(panel_titles.contains(&"[06:Swarm-MFG-BFT]"));
    assert!(panel_titles.contains(&"[07:Tropical-DAG]"));
    assert!(panel_titles.contains(&"[08:Bandit-Router]"));
    assert!(panel_titles.contains(&"[09:Drift-Conformal]"));
}
