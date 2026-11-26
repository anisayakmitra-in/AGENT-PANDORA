#![forbid(unsafe_code)]

//! Phase 49, 50, 51 End-to-End Integration Tests
//!
//! Validates:
//! - Phase 49: Topological Quantum Error Correction & Surface Code Stabilizers (Theorems 119–121)
//! - Phase 50: Zero-Knowledge PlonK Polynomial Commitments & KZG Schemes (Theorems 122–124)
//! - Phase 51: Multi-Agent Mechanism Design & VCG Truthful Equilibrium (Theorems 125–127)
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
        let root = std::env::temp_dir().join(format!("pandora_p49_50_51_{label}_{counter}"));
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
            "system_prompt": "You are AGENT-PANDORA Phase 49-51 test fixture."
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
        let stdout_str = String::from_utf8_lossy(&out.stdout);
        let parsed = serde_json::from_str::<serde_json::Value>(&stdout_str).unwrap_or_else(|e| {
            panic!(
                "Failed to parse JSON stdout: {e}. Output was:\n{}\nStderr:\n{}",
                stdout_str,
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out, parsed)
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
fn test_phase49_swarm_topological_qec_cli() {
    let env = TestEnv::new("qec_cli");

    // 1. qec-syndrome
    let (out_syn, res_syn) = env.run_json(&["swarm", "qec-syndrome", "--distance", "5"]);
    assert!(out_syn.status.success());
    assert_eq!(res_syn["distance"], 5);
    assert!(res_syn["syndrome_hash"].as_str().unwrap().len() >= 32);

    // 2. qec-decode
    let (out_dec, res_dec) = env.run_json(&["swarm", "qec-decode", "--distance", "5"]);
    assert!(out_dec.status.success());
    assert_eq!(res_dec["distance"], 5);
    assert_eq!(res_dec["below_fault_tolerance_threshold"], true);
    assert_eq!(res_dec["theorem119_fault_tolerance_verified"], true);
    assert_eq!(res_dec["theorem120_commutativity_verified"], true);
    assert_eq!(res_dec["theorem121_mwpm_optimality_verified"], true);
}

#[test]
fn test_phase50_crypto_kzg_commitment_cli() {
    let env = TestEnv::new("kzg_cli");

    // 1. kzg-commit
    let (out_c, res_c) = env.run_json(&["crypto", "kzg-commit"]);
    assert!(out_c.status.success());
    assert_eq!(res_c["degree"], 2);
    assert!(res_c["commitment_hex"].as_str().unwrap().starts_with("04"));

    // 2. kzg-verify
    let (out_v, res_v) = env.run_json(&["crypto", "kzg-verify", "--point", "7"]);
    assert!(out_v.status.success());
    assert_eq!(res_v["evaluation_point_z"], 7);
    assert_eq!(res_v["pairing_verified"], true);
    assert_eq!(res_v["theorem122_binding_verified"], true);
    assert_eq!(res_v["theorem124_pairing_soundness_verified"], true);
}

#[test]
fn test_phase51_cognitive_game_mechanism_cli() {
    let env = TestEnv::new("mechanism_cli");

    // 1. vcg-auction
    let (out_vcg, res_vcg) = env.run_json(&["cognitive", "vcg-auction"]);
    assert!(out_vcg.status.success());
    assert_eq!(res_vcg["winning_agent_id"], "agent_alpha");
    assert_eq!(res_vcg["social_welfare"], 120.0);
    assert_eq!(res_vcg["vcg_payment"], 95.0);
    assert_eq!(res_vcg["theorem125_dsic_verified"], true);
    assert_eq!(res_vcg["theorem126_correlated_equilibrium_verified"], true);

    // 2. shapley-credit
    let (out_shapley, res_shapley) = env.run_json(&["cognitive", "shapley-credit"]);
    assert!(out_shapley.status.success());
    assert_eq!(res_shapley["data"].as_array().unwrap().len(), 3);
}

#[test]
fn test_cockpit_web_phase49_50_51_rest_endpoints() {
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

    // 1. GET /api/swarm/qec
    let (code, qec) = query_http(port, "/api/swarm/qec");
    assert_eq!(code, 200);
    assert_eq!(qec["theorems"]["theorem_119"]["status"], "PROVEN");
    assert_eq!(qec["theorems"]["theorem_120"]["status"], "PROVEN");
    assert_eq!(qec["theorems"]["theorem_121"]["status"], "PROVEN");

    // 2. GET /api/crypto/kzg
    let (code, kzg) = query_http(port, "/api/crypto/kzg");
    assert_eq!(code, 200);
    assert_eq!(kzg["theorems"]["theorem_122"]["status"], "PROVEN");
    assert_eq!(kzg["theorems"]["theorem_123"]["status"], "PROVEN");
    assert_eq!(kzg["theorems"]["theorem_124"]["status"], "PROVEN");

    // 3. GET /api/cognitive/mechanism
    let (code, mech) = query_http(port, "/api/cognitive/mechanism");
    assert_eq!(code, 200);
    assert_eq!(mech["theorems"]["theorem_125"]["status"], "PROVEN");
    assert_eq!(mech["theorems"]["theorem_126"]["status"], "PROVEN");
    assert_eq!(mech["theorems"]["theorem_127"]["status"], "PROVEN");

    // 4. GET /api/olympus/overview
    let (code, overview) = query_http(port, "/api/olympus/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.6");
    assert_eq!(overview["codename"], "THANATOS");
    assert_eq!(overview["total_theorems_enforced"], 208);

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
