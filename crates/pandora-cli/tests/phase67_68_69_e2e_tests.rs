#![forbid(unsafe_code)]

//! Phase 67, 68, 69 End-to-End Integration Tests (v4.1.3, Codename: THANATOS)
//!
//! Validates:
//! - Phase 67: Topological Defect Entanglement & Majorana Zero-Mode Braiding in 1D Kitaev Superconducting Swarm Wires (Theorems 173–175)
//! - Phase 68: Multi-Party Verifiable Delay Functions (VDF) with Wesolowski Proofs (Theorems 176–178)
//! - Phase 69: Non-Equilibrium Path-Integral Swarm Thermodynamics & Jarzynski Equality (Theorems 179–181)
//! - Cockpit Web REST endpoints & Formal Verification Suite (Theorems 1–181)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(500);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p67_68_69_{label}_{counter}"));
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
            "system_prompt": "You are AGENT-PANDORA Phase 67-69 test fixture."
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
fn test_phase67_swarm_majorana_braiding_cli() {
    let env = TestEnv::new("p67");

    // 1. Majorana Braid
    let braid_res = env.run_success(&[
        "swarm",
        "majorana-braid",
        "--parity",
        "1",
        "--adiabatic-time",
        "10.0",
    ]);
    assert!(
        braid_res["wire_state"]["theorem_173_quantized"]
            .as_bool()
            .unwrap()
    );
    assert!(
        braid_res["braid_report"]["theorem_174_braid_unitary_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        braid_res["braid_report"]["unitary_fidelity"]
            .as_f64()
            .unwrap()
            > 0.99
    );

    // 2. Parity Protection Audit
    let audit_res = env.run_success(&[
        "swarm",
        "parity-audit",
        "--wire-length",
        "15.0",
        "--coherence",
        "1.5",
    ]);
    assert_eq!(
        audit_res["parity_audit"]["theorem_175_quasiparticle_immune"],
        true
    );
    assert!(
        audit_res["parity_audit"]["protection_factor"]
            .as_f64()
            .unwrap()
            > 1e4
    );
}

#[test]
fn test_phase68_crypto_verifiable_delay_cli() {
    let env = TestEnv::new("p68");

    // 1. VDF Evaluation
    let eval_res = env.run_success(&["crypto", "vdf-evaluate", "--seed", "7", "--steps", "10"]);
    assert_eq!(eval_res["delay_steps_t"], 10);
    assert_eq!(eval_res["theorem_176_sequential_satisfied"], true);
    assert!(
        eval_res["beacon_randomness"]
            .as_str()
            .unwrap()
            .starts_with("vdf_beacon_0x")
    );

    // 2. VDF Verification
    let verify_res = env.run_success(&["crypto", "vdf-verify", "--seed", "7", "--steps", "10"]);
    assert_eq!(verify_res["theorem_176_time_lock_valid"], true);
    assert_eq!(verify_res["theorem_177_soundness_verified"], true);
    assert_eq!(verify_res["theorem_178_beacon_unbiased"], true);
    assert_eq!(verify_res["verification_passed"], true);
}

#[test]
fn test_phase69_cognitive_path_integral_thermo_cli() {
    let env = TestEnv::new("p69");

    // 1. Jarzynski Evaluation
    let eval_res = env.run_success(&[
        "cognitive",
        "jarzynski-eval",
        "--delta-f",
        "1.5",
        "--speed",
        "0.5",
        "--samples",
        "50",
    ]);
    assert_eq!(eval_res["sample_count"], 50);
    assert_eq!(eval_res["theorem_180_equality_satisfied"], true);
    assert!(eval_res["jarzynski_average"].as_f64().unwrap() > 0.0);

    // 2. Free Energy Audit
    let audit_res = env.run_success(&[
        "cognitive",
        "free-energy-audit",
        "--work",
        "2.2",
        "--delta-f",
        "1.8",
    ]);
    assert_eq!(audit_res["second_law_satisfied"], true);
    assert_eq!(audit_res["theorem_181_crooks_verified"], true);
    assert!(audit_res["dissipated_work_mean"].as_f64().unwrap() >= 0.0);
}

#[test]
fn test_cockpit_web_v413_routes_and_thanatos_overview() {
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

    // 1. Swarm Majorana Braiding Route
    let (code, val) = query_http(port, "/api/swarm/majorana-braiding");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert_eq!(val["wire_state"]["theorem_173_quantized"], true);
    assert_eq!(
        val["braid_report"]["theorem_174_braid_unitary_verified"],
        true
    );
    assert_eq!(
        val["parity_audit"]["theorem_175_quasiparticle_immune"],
        true
    );

    // 2. Crypto Verifiable Delay Route
    let (code, val) = query_http(port, "/api/crypto/verifiable-delay");
    assert_eq!(code, 200);
    assert_eq!(val["evaluation"]["theorem_176_sequential_satisfied"], true);
    assert_eq!(val["verification"]["verification_passed"], true);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Path Integral Thermo Route
    let (code, val) = query_http(port, "/api/cognitive/path-integral-thermo");
    assert_eq!(code, 200);
    assert_eq!(val["jarzynski"]["theorem_180_equality_satisfied"], true);
    assert_eq!(val["audit"]["second_law_satisfied"], true);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview Route
    let (code, overview) = query_http(port, "/api/cockpit/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.7");
    assert_eq!(overview["total_theorems_enforced"], 217);
    assert_eq!(overview["codename"], "THANATOS");

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
