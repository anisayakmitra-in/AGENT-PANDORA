#![forbid(unsafe_code)]

//! Phase 43, 44, 45 End-to-End Integration Tests
//!
//! Validates:
//! - Phase 43: Recursive ZK Rollups & STARK Proofs (Theorems 101–103)
//! - Phase 44: Autonomous Self-Healing Kernel & Invariant Synthesis (Theorems 104–106)
//! - Phase 45: Neuromorphic Spiking Context VFS & Temporal Memory Decay (Theorems 107–109)
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
        let root = std::env::temp_dir().join(format!("pandora_p43_44_45_{label}_{counter}"));
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
            "system_prompt": "You are AGENT-PANDORA Phase 43-45 test fixture."
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
fn test_phase43_crypto_zk_rollup_and_verify_cli() {
    let env = TestEnv::new("p43_zk");

    // 1. Rollup aggregation
    let (_out, res) = env.run_json(&["crypto", "zk-rollup", "--batch-size", "4"]);
    assert_eq!(res["batch_size"], 4);
    assert_eq!(res["soundness_bits"], 128);
    assert_eq!(res["theorem_101_succinctness"], true);
    assert_eq!(res["theorem_102_prime_field_complete"], true);
    assert_eq!(res["theorem_103_soundness_verified"], true);

    // 2. Rollup verification
    let (_out_v, res_v) = env.run_json(&[
        "crypto",
        "zk-verify",
        "--proof",
        res["proof_stark_hex"].as_str().unwrap(),
    ]);
    assert_eq!(res_v["valid"], true);
    assert_eq!(res_v["soundness_verified"], true);
}

#[test]
fn test_phase44_governance_heal_and_invariant_check_cli() {
    let env = TestEnv::new("p44_heal");

    // 1. Heal synthesis
    let (_out, res) = env.run_json(&["governance", "heal", "--subsystem", "crypto"]);
    assert_eq!(res["target_subsystem"], "crypto");
    assert_eq!(res["bisimulation_verified"], true);
    assert_eq!(res["hot_patch_safe"], true);

    // 2. Invariant check
    let (_out_inv, res_inv) =
        env.run_json(&["governance", "invariant-check", "--subsystem", "crypto"]);
    assert_eq!(res_inv["theorem_104_bisimulation_equivalent"], true);
    assert_eq!(res_inv["theorem_105_invariant_rederived"], true);
    assert_eq!(res_inv["theorem_106_zero_downtime_preserved"], true);
}

#[test]
fn test_phase45_memory_spike_train_and_temporal_decay_cli() {
    let env = TestEnv::new("p45_spike");

    // 1. Spike train injection
    let (_out, res) = env.run_json(&[
        "memory",
        "spike-train",
        "--uri",
        "pandora://system/core_context",
        "--current",
        "20.0",
    ]);
    assert_eq!(res["uri"], "pandora://system/core_context");
    assert_eq!(res["fired"], true);
    assert_eq!(res["audit"]["theorem_107_lif_dynamics_verified"], true);
    assert_eq!(res["audit"]["theorem_108_stdp_convergence_verified"], true);

    // 2. Temporal decay audit
    let (_out_d, res_d) = env.run_json(&["memory", "temporal-decay"]);
    assert!(
        res_d["audit"]["temporal_retention_fidelity"]
            .as_f64()
            .unwrap()
            >= 0.95
    );
    assert_eq!(res_d["audit"]["theorem_109_retention_bound_verified"], true);
}

#[test]
fn test_cockpit_web_phase43_44_45_rest_endpoints() {
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

    // 1. Test /api/crypto/rollup
    let (code, rollup) = query_http(port, "/api/crypto/rollup");
    assert_eq!(code, 200);
    assert_eq!(rollup["batch_size"], 2);
    assert_eq!(rollup["theorem_101_succinctness"], true);
    assert_eq!(rollup["theorem_102_prime_field_complete"], true);
    assert_eq!(rollup["theorem_103_soundness_verified"], true);

    // 2. Test /api/governance/healing
    let (code, healing) = query_http(port, "/api/governance/healing");
    assert_eq!(code, 200);
    assert_eq!(healing["theorem_104_bisimulation_equivalent"], true);
    assert_eq!(healing["theorem_105_invariant_rederived"], true);
    assert_eq!(healing["theorem_106_zero_downtime_preserved"], true);

    // 3. Test /api/memory/neuromorphic
    let (code, neuro) = query_http(port, "/api/memory/neuromorphic");
    assert_eq!(code, 200);
    assert_eq!(neuro["theorem_107_lif_dynamics_verified"], true);
    assert_eq!(neuro["theorem_108_stdp_convergence_verified"], true);
    assert_eq!(neuro["theorem_109_retention_bound_verified"], true);

    // 4. Test /api/olympus/overview
    let (code, overview) = query_http(port, "/api/olympus/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.8");
    assert_eq!(overview["codename"], "THANATOS");
    assert_eq!(overview["total_theorems_enforced"], 226);

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
