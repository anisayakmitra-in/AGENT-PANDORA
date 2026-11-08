#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(500);

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
            "pandora-phase911-test-{}-{timestamp}-{sequence}",
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
            .env("PANDORA_WORKSPACE", &self.workspace);
        cmd
    }

    fn run_success(&self, args: &[&str]) -> Value {
        let output = self.command(args).output().expect("command should execute");
        assert!(
            output.status.success(),
            "command '{:?}' failed with stderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout).expect("output should be valid json envelope")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_privacy_lifecycle_reserve_commit_release_and_simulate() {
    let fixture = Fixture::new();

    // 1. Initial status
    let st0 = fixture.run_success(&["privacy", "status", "--json"]);
    assert_eq!(st0["command"], "privacy status");
    assert_eq!(st0["current_epsilon"], 0.0);

    // 2. Reserve query budget
    let res1 = fixture.run_success(&[
        "privacy",
        "reserve",
        "--agent-id",
        "agent-alpha",
        "--query",
        "q-1",
        "--mechanism",
        "gaussian",
        "--sensitivity",
        "1.0",
        "--sigma",
        "4.0",
        "--json",
    ]);
    assert_eq!(res1["command"], "privacy reserve");
    let res_id = res1["reservation_id"].as_str().unwrap().to_string();

    // 3. Commit reservation
    let com1 = fixture.run_success(&["privacy", "commit", "--reservation", &res_id, "--json"]);
    assert_eq!(com1["command"], "privacy commit");
    assert_eq!(com1["committed"], true);
    assert!(com1["current_epsilon"].as_f64().unwrap() > 0.0);

    // 4. Reserve & release second reservation
    let res2 = fixture.run_success(&[
        "privacy",
        "reserve",
        "--agent-id",
        "agent-beta",
        "--query",
        "q-2",
        "--mechanism",
        "laplace",
        "--sensitivity",
        "1.0",
        "--scale",
        "2.0",
        "--json",
    ]);
    let res2_id = res2["reservation_id"].as_str().unwrap().to_string();
    let rel2 = fixture.run_success(&["privacy", "release", "--reservation", &res2_id, "--json"]);
    assert_eq!(rel2["command"], "privacy release");
    assert_eq!(rel2["released"], true);
    assert_eq!(rel2["active_reservations_count"], 0);

    // 5. Simulate multi-agent RDP composition
    let sim = fixture.run_success(&[
        "privacy",
        "simulate",
        "--queries",
        "20",
        "--agents",
        "4",
        "--sigma",
        "4.0",
        "--json",
    ]);
    assert_eq!(sim["command"], "privacy simulate");
    assert_eq!(sim["queries_admitted"], 20);
    assert!(sim["composition_savings_ratio"].as_f64().unwrap() > 1.5);
}

#[test]
fn test_zk_prove_verify_and_benchmark_cli() {
    let fixture = Fixture::new();
    let proof_file = fixture.root.join("state_proof.json");
    let proof_path_str = proof_file.to_str().unwrap();

    // 1. Prove state transition and save to file
    let prove = fixture.run_success(&[
        "zk",
        "prove",
        "--domain-size",
        "8",
        "--out",
        proof_path_str,
        "--json",
    ]);
    assert_eq!(prove["command"], "zk prove");
    assert_eq!(prove["verified"], true);
    assert!(proof_file.exists());

    // 2. Verify from file
    let verify = fixture.run_success(&["zk", "verify", "--proof-file", proof_path_str, "--json"]);
    assert_eq!(verify["command"], "zk verify");
    assert_eq!(verify["valid"], true);

    // 3. Benchmark soundness and completeness
    let bench = fixture.run_success(&[
        "zk",
        "benchmark",
        "--domain-size",
        "16",
        "--forgeries",
        "40",
        "--json",
    ]);
    assert_eq!(bench["command"], "zk benchmark");
    assert_eq!(bench["completeness_verified"], true);
    assert_eq!(bench["forgeries_rejected"], 40);
}

#[test]
fn test_runtime_and_hardware_scheduler_cli() {
    let fixture = Fixture::new();

    for topology in ["layered", "fanout", "chain"] {
        let sched = fixture.run_success(&[
            "runtime",
            "scheduler",
            "benchmark",
            "--workers",
            "4",
            "--tasks",
            "24",
            "--topology",
            topology,
            "--json",
        ]);
        assert_eq!(sched["command"], "runtime scheduler benchmark");
        assert_eq!(sched["bl_bound_satisfied"], true);
        let t_p = sched["actual_makespan_tp"].as_u64().unwrap() as f64;
        let bl = sched["theoretical_bl_bound"].as_f64().unwrap();
        assert!(t_p <= bl);
    }

    let hw_sched = fixture.run_success(&[
        "hardware",
        "scheduler",
        "--workers",
        "8",
        "--tasks",
        "32",
        "--topology",
        "fanout",
        "--json",
    ]);
    assert_eq!(hw_sched["command"], "hardware scheduler");
    assert_eq!(hw_sched["bl_bound_satisfied"], true);
}

#[test]
fn test_runtime_phases_12_to_14_cli_subcommands() {
    let fixture = Fixture::new();

    // Phase 12: Byzantine consensus CLI
    let byz = fixture.run_success(&[
        "runtime",
        "byzantine",
        "--agents",
        "11",
        "--byzantine",
        "2",
        "--rule",
        "krum",
        "--attack-scale",
        "5000.0",
        "--json",
    ]);
    assert_eq!(byz["command"], "runtime byzantine");
    assert!(byz["robust_l2_error"].as_f64().unwrap() < 0.2);
    assert!(byz["naive_l2_error"].as_f64().unwrap() > 100.0);

    // Phase 13: Entropic OT Sinkhorn Drift CLI
    let drift = fixture.run_success(&[
        "runtime",
        "drift",
        "--samples",
        "16",
        "--shift",
        "2.0",
        "--threshold",
        "1.0",
        "--json",
    ]);
    assert_eq!(drift["command"], "runtime drift");
    assert_eq!(drift["drift_detected"], true);
    assert!(drift["birkhoff_contraction_factor"].as_f64().unwrap() < 1.0);

    // Phase 14: Martingale Anytime-Valid Canary Gate CLI
    let gate = fixture.run_success(&[
        "runtime",
        "canary-gate",
        "--steps",
        "60",
        "--effect",
        "0.35",
        "--alpha",
        "0.05",
        "--json",
    ]);
    assert_eq!(gate["command"], "runtime canary-gate");
    assert_eq!(gate["verdict"], "rollback_regression");
}
