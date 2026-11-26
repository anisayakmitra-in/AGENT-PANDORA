#![forbid(unsafe_code)]

//! Phase 58, 59, 60 End-to-End Integration Tests (v4.1.0, Codename: THANATOS)
//!
//! Validates:
//! - Phase 58: Topological Quantum Field Theory (TQFT) & Chern-Simons Invariants (Theorems 146–148)
//! - Phase 59: Homomorphic Functional Encryption & Verifiable Inner-Product Decryption (Theorems 149–151)
//! - Phase 60: Distributed Symplectic Hamiltonian Neural ODEs & Energy-Conserving Swarm Dynamics (Theorems 152–154)
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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(200);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p58_59_60_{label}_{counter}"));
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
            "system_prompt": "You are AGENT-PANDORA Phase 58-60 test fixture."
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
fn test_phase58_swarm_tqft_knot_and_entropy_cli() {
    let env = TestEnv::new("p58");

    // 1. Swarm TQFT Knot
    let knot_res = env.run_success(&["swarm", "tqft-knot", "--knot", "trefoil", "-t", "1.5"]);
    assert_eq!(knot_res["jones"]["knot_name"], "Trefoil_3_1");
    assert!(knot_res["s3_partition_function"].as_f64().unwrap() > 0.0);
    assert_eq!(knot_res["theorems"].as_array().unwrap().len(), 2);

    // 2. Swarm TQFT Entropy
    let entropy_res = env.run_success(&[
        "swarm",
        "tqft-entropy",
        "--length",
        "10.0",
        "--alpha",
        "0.5",
    ]);
    assert!(
        entropy_res["topological_entanglement_entropy_gamma"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert_eq!(entropy_res["theorem_148_verified"], true);
}

#[test]
fn test_phase59_crypto_functional_encryption_cli() {
    let env = TestEnv::new("p59");

    // 1. Functional Encryption Keygen
    let keygen_res = env.run_success(&[
        "crypto",
        "fe-keygen",
        "--weights",
        "2,-3,4,1",
        "--dimension",
        "4",
    ]);
    assert_eq!(keygen_res["function_vector"].as_array().unwrap().len(), 4);
    assert_eq!(keygen_res["token_vector"].as_array().unwrap().len(), 4);
    assert_eq!(keygen_res["authority_id"], "authority-cluster-alpha");

    // 2. Functional Encryption Decrypt
    let decrypt_res = env.run_success(&[
        "crypto",
        "fe-decrypt",
        "--plaintext",
        "10,5,2,7",
        "--weights",
        "2,-3,4,1",
    ]);
    // Inner product: 10*2 + 5*(-3) + 2*4 + 7*1 = 20 - 15 + 8 + 7 = 20
    assert_eq!(decrypt_res["inner_product"], 20);
    assert_eq!(
        decrypt_res["function_weights_used"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(decrypt_res["theorem_149_ind_cpa_verified"], true);
    assert_eq!(decrypt_res["theorem_150_leakage_free"], true);
}

#[test]
fn test_phase60_cognitive_symplectic_ode_cli() {
    let env = TestEnv::new("p60");

    // 1. Symplectic Step
    let step_res = env.run_success(&[
        "cognitive",
        "symplectic-step",
        "--q",
        "1.0,0.0",
        "--p",
        "0.0,1.5",
        "--step-size",
        "0.02",
    ]);
    assert!((step_res["time_t"].as_f64().unwrap() - 0.02).abs() < 1e-6);
    assert_eq!(step_res["position_q"].as_array().unwrap().len(), 2);
    assert_eq!(step_res["momentum_p"].as_array().unwrap().len(), 2);
    assert!(step_res["total_hamiltonian"].as_f64().unwrap() > 0.0);

    // 2. Symplectic Energy Audit
    let audit_res = env.run_success(&[
        "cognitive",
        "energy-audit",
        "--steps",
        "50",
        "--q",
        "1.0,0.0",
        "--p",
        "0.0,1.5",
    ]);
    assert_eq!(audit_res["theorem_152_symplectic_verified"], true);
    assert_eq!(audit_res["theorem_153_shadow_verified"], true);
    assert_eq!(audit_res["theorem_154_energy_bounded"], true);
    assert_eq!(audit_res["volume_preservation_jacobian_det"], 1.0);
    assert!(audit_res["max_energy_drift"].as_f64().unwrap() < 0.05);
}

#[test]
fn test_cockpit_web_v410_routes_and_thanatos_overview() {
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

    // 1. Swarm TQFT Route
    let (code, val) = query_http(port, "/api/swarm/tqft");
    assert_eq!(code, 200);
    assert_eq!(val["jones_polynomial"]["knot_name"], "Trefoil_3_1");
    assert!(val["s3_partition_function"].as_f64().unwrap() > 0.0);

    // 2. Crypto Functional Encryption Route
    let (code, val) = query_http(port, "/api/crypto/functional-encryption");
    assert_eq!(code, 200);
    assert_eq!(val["plaintext_dimension"], 4);
    assert_eq!(val["functional_result"]["inner_product"], 20);

    // 3. Cognitive Symplectic ODE Route
    let (code, val) = query_http(port, "/api/cognitive/symplectic-ode");
    assert_eq!(code, 200);
    assert_eq!(val["audit"]["theorem_152_symplectic_verified"], true);

    // 4. Cockpit Overview Route
    let (code, overview) = query_http(port, "/api/cockpit/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.2");
    assert_eq!(overview["total_theorems_enforced"], 172);
    assert_eq!(overview["codename"], "THANATOS");

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
