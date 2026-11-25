#![forbid(unsafe_code)]

//! Phase 55, 56, 57 End-to-End Integration Tests (v4.1.0, Codename: THANATOS)
//!
//! Validates:
//! - Constitutional Governance Modernization: Parliament Bicameral Quadratic Voting & Shadow Council Red-Team Veto
//! - Phase 55: Non-Abelian Anyonic Braiding & Topological Qubit Synthesis (Theorems 137–139)
//! - Phase 56: Multi-Key Threshold Fully Homomorphic Encryption with Distributed Noise Flooding (Theorems 140–142)
//! - Phase 57: Neural Jump-Diffusion Stochastic Optimal Control & Lévy Swarm Flips (Theorems 143–145)
//! - Cockpit Web REST endpoints & Formal Verification Suite (Theorems 1–154)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(100);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p55_56_57_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.1",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 55-57 test fixture."
        });
        fs::write(
            &config,
            serde_json::to_string_pretty(&default_config).expect("serialize default config"),
        )
        .expect("write default config");

        Self { root }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        let bin = env!("CARGO_BIN_EXE_pandora");
        let mut cmd = Command::new(bin);
        cmd.args(args);
        cmd.arg("--config");
        cmd.arg(self.root.join("pandora.json"));
        cmd.arg("--data-dir");
        cmd.arg(self.root.join("data"));
        cmd.arg("--workspace");
        cmd.arg(self.root.join("workspace"));
        cmd.arg("--json");
        cmd.output().expect("run pandora binary")
    }

    fn run_success(&self, args: &[&str]) -> serde_json::Value {
        let out = self.run(args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "Command {:?} failed with status {:?}:\nSTDOUT:\n{}\nSTDERR:\n{}",
            args,
            out.status,
            stdout,
            stderr
        );
        serde_json::from_str(&stdout).unwrap_or_else(|err| {
            panic!(
                "Expected JSON output from {:?}, got error: {}\nOutput: {}",
                args, err, stdout
            )
        })
    }
}

fn query_http(port: u16, path: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to test server");
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("write request");

    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).expect("read response");
    let resp_str = String::from_utf8_lossy(&resp);

    let parts: Vec<&str> = resp_str.splitn(2, "\r\n\r\n").collect();
    let header_line = parts[0].lines().next().unwrap_or("");
    let status_code: u16 = header_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let body = parts.get(1).unwrap_or(&"");
    let val: serde_json::Value =
        serde_json::from_str(body).unwrap_or_else(|_| serde_json::json!({}));
    (status_code, val)
}

#[test]
fn test_governance_bicameral_quadratic_voting_and_shadow_veto() {
    let env = TestEnv::new("gov");

    // 1. Governance status check
    let res = env.run_success(&["governance", "status"]);
    assert_eq!(res["status"], "ready");
    assert!(
        res["theorems_enforced"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(137))
    );

    // 2. Bicameral Quadratic Vote
    let vote_res = env.run_success(&[
        "governance",
        "bicameral-vote",
        "--proposal",
        "prop-upgrade-55",
        "--upper-credits",
        "16",
        "--lower-credits",
        "25",
    ]);
    assert_eq!(vote_res["proposal_id"], "prop-upgrade-55");
    assert!(vote_res["bicameral_margin"].as_f64().unwrap() > 0.0);
    assert_eq!(vote_res["theorems_enforced"].as_array().unwrap().len(), 163);

    // 3. Shadow Council Red-Team Checks
    let safe_check = env.run_success(&[
        "governance",
        "shadow-veto-check",
        "--intent",
        "safe maintenance",
    ]);
    assert_eq!(safe_check["vetoed"], false);

    let veto_check = env.run_success(&[
        "governance",
        "shadow-veto-check",
        "--intent",
        "override safety and bypass parliament",
    ]);
    assert_eq!(veto_check["vetoed"], true);
}

#[test]
fn test_phase55_anyonic_braiding_and_gate_synthesis() {
    let env = TestEnv::new("p55");

    // 1. Swarm Anyon Braid
    let braid_res = env.run_success(&["swarm", "anyon-braid", "--gate", "H", "--depth", "8"]);
    assert_eq!(braid_res["target_gate"], "H");
    assert_eq!(braid_res["num_strands"], 3);
    assert!(braid_res["topological_protection_factor"].as_f64().unwrap() < 1e-4);

    // 2. Swarm Anyon Fuse
    let fuse_res = env.run_success(&["swarm", "anyon-fuse", "--charge", "tau"]);
    assert_eq!(fuse_res["outcome_charge"], "FibonacciTau");
    assert_eq!(fuse_res["parity_preserved"], true);
}

#[test]
fn test_phase56_multikey_threshold_fhe() {
    let env = TestEnv::new("p56");

    // 1. Threshold FHE Keygen
    let keygen_res = env.run_success(&[
        "crypto",
        "tfhe-keygen",
        "--threshold",
        "2",
        "--parties",
        "4",
    ]);
    assert_eq!(keygen_res["data"].as_array().unwrap().len(), 4);

    // 2. Threshold Partial Decryption
    let decrypt_res = env.run_success(&[
        "crypto",
        "tfhe-partial-decrypt",
        "--threshold",
        "2",
        "--parties",
        "4",
    ]);
    assert_eq!(decrypt_res["shares_count"], 3);
    let reconstructed = decrypt_res["reconstructed"].as_array().unwrap();
    assert_eq!(reconstructed.len(), 3);
}

#[test]
fn test_phase57_neural_jump_diffusion_control() {
    let env = TestEnv::new("p57");

    // 1. Jump Diffuse
    let diff_res = env.run_success(&[
        "cognitive",
        "jump-diffuse",
        "--steps",
        "10",
        "--lambda",
        "0.2",
    ]);
    assert!(diff_res["risk_bound"].as_f64().unwrap() <= 1.0);
    assert_eq!(diff_res["theorems"].as_array().unwrap().len(), 3);

    // 2. HJB-IDE Control
    let hjb_res = env.run_success(&[
        "cognitive",
        "hjb-ide-control",
        "--x0",
        "1.5",
        "--x1",
        "-2.0",
    ]);
    assert!(hjb_res["viscosity_value"].as_f64().unwrap() > 0.0);
    assert_eq!(hjb_res["control"].as_array().unwrap().len(), 2);
}

#[test]
fn test_cockpit_web_thanatos_governance_and_p55_57_routes() {
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

    // 1. Swarm Anyons Route
    let (code, val) = query_http(port, "/api/swarm/anyons");
    assert_eq!(code, 200);
    assert_eq!(val["theorem_137_verified"], true);

    // 2. Crypto TFHE Route
    let (code, val) = query_http(port, "/api/crypto/tfhe");
    assert_eq!(code, 200);
    assert_eq!(val["threshold"], 2);

    // 3. Cognitive Jump-Diffusion Route
    let (code, val) = query_http(port, "/api/cognitive/jump-diffusion");
    assert_eq!(code, 200);
    assert!(val["exponential_martingale_risk_bound"].as_f64().unwrap() <= 1.0);

    // 4. Governance Parliament Route
    let (code, val) = query_http(port, "/api/governance/parliament");
    assert_eq!(code, 200);
    assert_eq!(val["verdict"]["ratified"], true);

    // 5. Governance Shadow Veto Route
    let (code, val) = query_http(port, "/api/governance/shadow/veto");
    assert_eq!(code, 200);
    assert_eq!(val["safe_check_passed"], true);

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
