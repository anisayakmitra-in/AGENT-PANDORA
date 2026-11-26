#![forbid(unsafe_code)]

//! Phase 46, 47, 48 End-to-End Integration Tests
//!
//! Validates:
//! - Phase 46: Quantum Annealing Graph Partitioning & QUBO Scheduling (Theorems 110–112)
//! - Phase 47: Homomorphic Federated Model Distillation (Theorems 113–115)
//! - Phase 48: Causal Counterfactual Policy Inference & Do-Calculus (Theorems 116–118)
//! - Cockpit Web REST endpoints & Olympus System Overview

use pandora_runtime::cockpit::{CockpitController, CockpitLayout};
use pandora_runtime::cockpit_web::CockpitWebServer;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p46_47_48_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.3",
            "active_model": "olympus-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 46-48 test fixture."
        });
        fs::write(
            &config,
            serde_json::to_string_pretty(&default_config).expect("serialize default config"),
        )
        .expect("write default config");

        Self { root }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", self.root.join("pandora.json"))
            .env("PANDORA_DATA_DIR", self.root.join("data"))
            .env("PANDORA_WORKSPACE", self.root.join("workspace"));
        cmd.output().expect("execute pandora binary")
    }

    fn run_json(&self, args: &[&str]) -> (std::process::Output, serde_json::Value) {
        let mut full_args = args.to_vec();
        full_args.push("--json");
        let out = self.run(&full_args);
        assert!(
            out.status.success(),
            "command failed: {:?}\nstdout: {}\nstderr: {}",
            full_args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let val: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("parse stdout as json");
        (out, val)
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn query_http(port: u16, path: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to cockpit server");
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("write request");

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read response");
    let resp = String::from_utf8_lossy(&buf);

    let mut lines = resp.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);

    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("");
    let val: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    (code, val)
}

#[test]
fn test_phase46_swarm_quantum_anneal_cli() {
    let env = TestEnv::new("p46_anneal");

    // 1. Swarm anneal
    let (_out, res) = env.run_json(&["swarm", "anneal", "--clusters", "2"]);
    assert_eq!(res["theorem_110_qubo_invariant_verified"], true);
    assert_eq!(res["theorem_111_quantum_tunneling_converged"], true);
    assert_eq!(res["theorem_112_pareto_makespan_bounded"], true);

    // 2. QUBO schedule alias
    let (_out_q, res_q) = env.run_json(&["swarm", "qubo-schedule", "--clusters", "3"]);
    assert!(res_q["makespan_estimate"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_phase47_crypto_fed_aggregate_cli() {
    let env = TestEnv::new("p47_fed");

    // 1. Fed aggregate
    let (_out, res) = env.run_json(&["crypto", "fed-aggregate"]);
    assert_eq!(res["round_id"], 1);
    assert_eq!(res["workers_contributed"], 2);
    assert_eq!(res["theorem_113_ind_cpa_preserved"], true);
    assert_eq!(res["theorem_114_poisoning_rejected"], true);
    assert_eq!(res["theorem_115_zero_leakage_verified"], true);

    // 2. Fed verify
    let (_out_v, res_v) = env.run_json(&["crypto", "fed-verify"]);
    assert_eq!(res_v["theorem_113_ind_cpa_preserved"], true);
    assert_eq!(res_v["theorem_115_zero_leakage_verified"], true);
}

#[test]
fn test_phase48_cognitive_causal_do_calculus_cli() {
    let env = TestEnv::new("p48_causal");

    // 1. Do-calculus intervention
    let (_out, res) = env.run_json(&[
        "cognitive",
        "do-calculus",
        "--treatment",
        "policy_action",
        "--value",
        "safe_explore",
        "--outcome",
        "agent_reward",
    ]);
    assert_eq!(res["treatment_variable"], "policy_action");
    assert_eq!(res["treatment_value"], "safe_explore");
    assert_eq!(res["outcome_variable"], "agent_reward");
    assert!(res["average_treatment_effect"].as_f64().unwrap() > 0.0);
    assert_eq!(res["theorem_116_identifiability_verified"], true);
    assert_eq!(res["theorem_117_counterfactual_sound"], true);
    assert_eq!(res["theorem_118_confounder_bounded"], true);

    // 2. Counterfactual alias
    let (_out_cf, res_cf) = env.run_json(&["cognitive", "counterfactual"]);
    assert_eq!(res_cf["theorem_116_identifiability_verified"], true);
}

#[test]
fn test_cockpit_web_phase46_47_48_rest_endpoints() {
    let controller = Arc::new(std::sync::Mutex::new(CockpitController::new(
        CockpitLayout::TwelvePanel,
    )));
    let server =
        CockpitWebServer::new(0, "127.0.0.1", controller, None).expect("bind cockpit server");
    let port = server.port();
    let shutdown_handle = server.shutdown_handle();

    let server_thread = thread::spawn(move || {
        let _ = server.run();
    });
    thread::sleep(Duration::from_millis(100));

    // 1. Test /api/swarm/anneal
    let (code, anneal) = query_http(port, "/api/swarm/anneal");
    assert_eq!(code, 200);
    assert_eq!(anneal["theorem_110_qubo_invariant_verified"], true);
    assert_eq!(anneal["theorem_111_quantum_tunneling_converged"], true);
    assert_eq!(anneal["theorem_112_pareto_makespan_bounded"], true);

    // 2. Test /api/crypto/federated
    let (code, fed) = query_http(port, "/api/crypto/federated");
    assert_eq!(code, 200);
    assert_eq!(fed["theorem_113_ind_cpa_preserved"], true);
    assert_eq!(fed["theorem_114_poisoning_rejected"], true);
    assert_eq!(fed["theorem_115_zero_leakage_verified"], true);

    // 3. Test /api/cognitive/causal
    let (code, causal) = query_http(port, "/api/cognitive/causal");
    assert_eq!(code, 200);
    assert_eq!(causal["theorem_116_identifiability_verified"], true);
    assert_eq!(causal["theorem_117_counterfactual_sound"], true);
    assert_eq!(causal["theorem_118_confounder_bounded"], true);

    // 4. Test /api/olympus/overview
    let (code, overview) = query_http(port, "/api/olympus/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.3");
    assert_eq!(overview["codename"], "THANATOS");
    assert_eq!(overview["total_theorems_enforced"], 181);

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
