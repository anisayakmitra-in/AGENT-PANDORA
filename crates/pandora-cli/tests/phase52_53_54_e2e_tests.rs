#![forbid(unsafe_code)]

//! Phase 52, 53, 54 End-to-End Integration Tests
//!
//! Validates:
//! - Phase 52: Holographic Quantum Error Suppression & Fault-Tolerant Magic State Distillation (Theorems 128–130)
//! - Phase 53: Fully Homomorphic Encryption (FHE) with CKKS Leveled Arithmetic (Theorems 131–133)
//! - Phase 54: Mean-Field Stochastic Differential Games & McKean-Vlasov Swarm Control (Theorems 134–136)
//! - Cockpit Web REST endpoints & Thanatos System Overview

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
        let root = std::env::temp_dir().join(format!("pandora_p52_53_54_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.2",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 52-54 test fixture."
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
fn test_phase52_swarm_magic_distillation_cli() {
    let env = TestEnv::new("distill_cli");

    // 1. magic-distill
    let (out_d, res_d) = env.run_json(&[
        "swarm",
        "magic-distill",
        "--rounds",
        "2",
        "--input-error",
        "0.01",
    ]);
    assert!(out_d.status.success());
    assert!(res_d["output_infidelity"].as_f64().unwrap() < 0.01);
    assert_eq!(res_d["theorem128_amplification_verified"], true);
    assert_eq!(res_d["theorem129_transversal_soundness_verified"], true);
    assert_eq!(res_d["theorem130_yield_bound_verified"], true);

    // 2. magic-state-audit
    let (out_a, res_a) = env.run_json(&["swarm", "magic-state-audit"]);
    assert!(out_a.status.success());
    assert_eq!(res_a["theorems"]["theorem_128"]["status"], "PROVEN");
    assert_eq!(res_a["theorems"]["theorem_129"]["status"], "PROVEN");
    assert_eq!(res_a["theorems"]["theorem_130"]["status"], "PROVEN");
}

#[test]
fn test_phase53_crypto_ckks_fhe_cli() {
    let env = TestEnv::new("ckks_cli");

    // 1. ckks-encrypt
    let (out_e, res_e) = env.run_json(&["crypto", "ckks-encrypt"]);
    assert!(out_e.status.success());
    assert_eq!(res_e["level"], 5);
    assert_eq!(res_e["theorem131_embedding_isometric"], true);
    assert_eq!(res_e["theorem132_noise_rescaling_invariant"], true);
    assert_eq!(res_e["theorem133_rlwe_security_verified"], true);

    // 2. ckks-rescale
    let (out_r, res_r) = env.run_json(&["crypto", "ckks-rescale"]);
    assert!(out_r.status.success());
    assert_eq!(res_r["level"], 4);
    assert!(res_r["estimated_snr_db"].as_f64().unwrap() >= 80.0);
}

#[test]
fn test_phase54_cognitive_mean_field_control_cli() {
    let env = TestEnv::new("mean_field_cli");

    // 1. mckean-vlasov
    let (out_mv, res_mv) = env.run_json(&["cognitive", "mckean-vlasov", "--agents", "100"]);
    assert!(out_mv.status.success());
    assert_eq!(res_mv["population_n"], 100);
    assert_eq!(res_mv["theorem134_continuum_convergence_verified"], true);
    assert_eq!(res_mv["theorem135_hjb_viscosity_verified"], true);
    assert_eq!(res_mv["theorem136_pareto_decentralized_invariance"], true);

    // 2. swarm-hjb
    let (out_hjb, res_hjb) = env.run_json(&["cognitive", "swarm-hjb", "--grid-size", "100"]);
    assert!(out_hjb.status.success());
    assert_eq!(res_hjb["theorems"]["theorem_134"]["status"], "PROVEN");
    assert_eq!(res_hjb["theorems"]["theorem_135"]["status"], "PROVEN");
    assert_eq!(res_hjb["theorems"]["theorem_136"]["status"], "PROVEN");
}

#[test]
fn test_cockpit_web_phase52_53_54_rest_endpoints() {
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

    // 1. GET /api/swarm/magic-state
    let (code, magic) = query_http(port, "/api/swarm/magic-state");
    assert_eq!(code, 200);
    assert_eq!(magic["theorems"]["theorem_128"]["status"], "PROVEN");
    assert_eq!(magic["theorems"]["theorem_129"]["status"], "PROVEN");
    assert_eq!(magic["theorems"]["theorem_130"]["status"], "PROVEN");

    // 2. GET /api/crypto/ckks
    let (code, ckks) = query_http(port, "/api/crypto/ckks");
    assert_eq!(code, 200);
    assert_eq!(ckks["theorems"]["theorem_131"]["status"], "PROVEN");
    assert_eq!(ckks["theorems"]["theorem_132"]["status"], "PROVEN");
    assert_eq!(ckks["theorems"]["theorem_133"]["status"], "PROVEN");

    // 3. GET /api/cognitive/mean-field
    let (code, mf) = query_http(port, "/api/cognitive/mean-field");
    assert_eq!(code, 200);
    assert_eq!(mf["theorems"]["theorem_134"]["status"], "PROVEN");
    assert_eq!(mf["theorems"]["theorem_135"]["status"], "PROVEN");
    assert_eq!(mf["theorems"]["theorem_136"]["status"], "PROVEN");

    // 4. GET /api/olympus/overview
    let (code, overview) = query_http(port, "/api/olympus/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.2");
    assert_eq!(overview["codename"], "THANATOS");
    assert_eq!(overview["total_theorems_enforced"], 172);

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
