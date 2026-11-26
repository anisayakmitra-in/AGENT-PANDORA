#![forbid(unsafe_code)]

//! Phase 61, 62, 63 End-to-End Integration Tests (v4.1.1, Codename: THANATOS)
//!
//! Validates:
//! - Phase 61: Non-Commutative Quantum Geometric Tensor & Berry Curvature Swarm Manifolds (Theorems 155–157)
//! - Phase 62: Verifiable Post-Quantum Blind Multi-Party Computation & Threshold Oblivious Transfer (Theorems 158–160)
//! - Phase 63: Infinite-Horizon Mean-Field Game (MFG) Master Equation & Viscosity Equilibrium (Theorems 161–163)
//! - Cockpit Web REST endpoints & Formal Verification Suite (Theorems 1–163)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(300);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p61_62_63_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.3",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 61-63 test fixture."
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
fn test_phase61_swarm_quantum_geometry_cli() {
    let env = TestEnv::new("p61");

    // 1. Berry Curvature
    let berry_res = env.run_success(&[
        "swarm",
        "berry-curvature",
        "--kx",
        "0.5",
        "--ky",
        "-0.5",
        "--grid",
        "16",
    ]);
    assert!(
        berry_res["tensor"]["berry_curvature_f_xy"]
            .as_f64()
            .is_some()
    );
    assert_eq!(
        berry_res["chern_number"]["theorem_156_chern_quantized"],
        true
    );
    assert_eq!(berry_res["theorems"].as_array().unwrap().len(), 2);

    // 2. Quantum Metric
    let metric_res = env.run_success(&[
        "swarm",
        "quantum-metric",
        "--kx",
        "0.5",
        "--ky",
        "-0.5",
        "--radius",
        "1.0",
    ]);
    assert_eq!(
        metric_res["fubini_study_metric"].as_array().unwrap().len(),
        3
    );
    assert!(metric_res["gauge_invariant_ds2"].as_f64().unwrap() > 0.0);
    assert_eq!(
        metric_res["holonomy"]["theorem_157_unitarity_verified"],
        true
    );
}

#[test]
fn test_phase62_crypto_blind_mpc_cli() {
    let env = TestEnv::new("p62");

    // 1. Oblivious Transfer Branch 0
    let ot0_res = env.run_success(&[
        "crypto", "blind-ot", "--choice", "0", "--m0", "100", "--m1", "200",
    ]);
    assert_eq!(ot0_res["chosen_branch"], 0);
    assert_eq!(ot0_res["transferred_value"], 100);

    // 2. Oblivious Transfer Branch 1
    let ot1_res = env.run_success(&[
        "crypto", "blind-ot", "--choice", "1", "--m0", "100", "--m1", "200",
    ]);
    assert_eq!(ot1_res["chosen_branch"], 1);
    assert_eq!(ot1_res["transferred_value"], 200);

    // 3. Blind Multiplication
    let mult_res = env.run_success(&["crypto", "mpc-multiply", "--x", "45", "--y", "80"]);
    assert_eq!(mult_res["computed_product"], 3600);
    assert_eq!(mult_res["theorem_159_soundness_verified"], true);
    assert_eq!(mult_res["theorem_160_secrecy_verified"], true);
}

#[test]
fn test_phase63_cognitive_mean_field_master_cli() {
    let env = TestEnv::new("p63");

    // 1. Solve Master Equation
    let solve_res = env.run_success(&[
        "cognitive",
        "mfg-master-solve",
        "--time",
        "2.0",
        "--position",
        "1.0",
        "--mean",
        "1.5",
        "--variance",
        "0.5",
    ]);
    assert!(solve_res["value_u"].as_f64().unwrap() > 0.0);
    assert_eq!(solve_res["theorem_161_viscosity_unique"], true);
    assert_eq!(solve_res["theorem_162_chain_rule_verified"], true);
    assert!(solve_res["theorem_163_nash_error_bound"].as_f64().unwrap() < 0.5);

    // 2. Lions Derivative
    let lions_res = env.run_success(&[
        "cognitive",
        "lions-derivative",
        "--y",
        "2.5",
        "--mean",
        "2.0",
        "--variance",
        "1.0",
    ]);
    assert_eq!(lions_res["test_point_y"], 2.5);
    assert!(lions_res["functional_value"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_cockpit_web_v411_routes_and_thanatos_overview() {
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

    // 1. Swarm Quantum Geometry Route
    let (code, val) = query_http(port, "/api/swarm/quantum-geometry");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert_eq!(val["tensor"]["theorem_155_fubini_study_verified"], true);
    assert_eq!(val["chern_number"]["theorem_156_chern_quantized"], true);
    assert_eq!(val["holonomy"]["theorem_157_unitarity_verified"], true);

    // 2. Crypto Blind MPC Route
    let (code, val) = query_http(port, "/api/crypto/blind-mpc");
    assert_eq!(code, 200);
    assert_eq!(val["oblivious_transfer"]["transferred_value"], 200);
    assert_eq!(val["multiplication"]["computed_product"], 3600);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Mean-Field Master Route
    let (code, val) = query_http(port, "/api/cognitive/mean-field-master");
    assert_eq!(code, 200);
    assert_eq!(val["master_state"]["theorem_161_viscosity_unique"], true);
    assert_eq!(val["lions_derivative"]["test_point_y"], 2.5);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview Route
    let (code, overview) = query_http(port, "/api/cockpit/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.3");
    assert_eq!(overview["total_theorems_enforced"], 181);
    assert_eq!(overview["codename"], "THANATOS");

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
