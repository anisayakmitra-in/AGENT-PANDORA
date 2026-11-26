#![forbid(unsafe_code)]

//! Phase 70, 71, 72 End-to-End Integration Tests (v4.1.4, Codename: THANATOS)
//!
//! Validates:
//! - Phase 70: Non-Hermitian PT-Symmetric Swarm Topology & Exceptional Points (Theorems 182–184)
//! - Phase 71: Multi-Prover Interactive Proofs (MIP*) & Entangled Non-Local Games (Theorems 185–187)
//! - Phase 72: Stochastic Port-Hamiltonian Neural Systems & Energy-Casimir Consensus (Theorems 188–190)
//! - Cockpit Web REST endpoints & Formal Verification Suite (Theorems 1–190)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(600);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p70_71_72_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.4",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 70-72 test fixture."
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
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");

    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("write request");

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read response");
    let resp = String::from_utf8_lossy(&buf);

    let parts: Vec<&str> = resp.split("\r\n\r\n").collect();
    let header_line = parts[0].lines().next().unwrap_or("");
    let status_code: u16 = header_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);

    let body = parts.get(1).unwrap_or(&"");
    let val: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::json!({}));
    (status_code, val)
}

#[test]
fn test_cli_phase70_non_hermitian_pt_topology() {
    let env = TestEnv::new("p70_swarm");

    // 1. pt-spectrum-eval
    let res = env.run_success(&[
        "swarm",
        "pt-spectrum-eval",
        "--coupling",
        "2.0",
        "--gamma",
        "0.8",
    ]);
    assert!(res["pt_spectrum"]["is_pt_unbroken"].as_bool().unwrap());
    assert!(
        res["pt_spectrum"]["theorem_182_pseudo_hermitian_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. exceptional-audit
    let res2 = env.run_success(&[
        "swarm",
        "exceptional-audit",
        "--coupling",
        "2.0",
        "--gamma",
        "2.0",
    ]);
    assert!(
        res2["exceptional_point"]["is_at_exceptional_point"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["exceptional_point"]["theorem_183_chiral_transport_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["skin_effect"]["theorem_184_skin_effect_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cli_phase71_quantum_mip_chsh() {
    let env = TestEnv::new("p71_crypto");

    // 1. chsh-evaluate
    let res = env.run_success(&["crypto", "chsh-evaluate", "--rounds", "1000"]);
    assert!(res["quantum_win_rate"].as_f64().unwrap() > 0.75);
    assert!(
        res["theorem_185_quantum_advantage_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. mip-verify
    let res2 = env.run_success(&["crypto", "mip-verify", "--provers", "3", "--rounds", "32"]);
    assert!(
        res2["relativistic_mip"]["theorem_186_relativistic_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["rigidity"]["theorem_187_rigidity_certified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cli_phase72_port_hamiltonian_cognitive() {
    let env = TestEnv::new("p72_cognitive");

    // 1. hamiltonian-eval
    let res = env.run_success(&["cognitive", "hamiltonian-eval"]);
    assert!(res["is_skew_symmetric"].as_bool().unwrap());
    assert!(res["is_dissipative"].as_bool().unwrap());
    assert!(res["theorem_188_passivity_verified"].as_bool().unwrap());

    // 2. casimir-audit
    let res2 = env.run_success(&[
        "cognitive",
        "casimir-audit",
        "--agents",
        "6",
        "--delay-ms",
        "15.0",
    ]);
    assert!(
        res2["casimir_invariants"]["theorem_189_casimir_conserved"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["consensus"]["theorem_190_consensus_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cockpit_web_phase70_71_72_routes_and_centennial_suite() {
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

    // 1. Swarm PT Topology Route
    let (code, val) = query_http(port, "/api/swarm/pt-topology");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert!(
        val["spectrum"]["theorem_182_pseudo_hermitian_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["exceptional_point"]["theorem_183_chiral_transport_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["skin_effect"]["theorem_184_skin_effect_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. Crypto Quantum MIP Route
    let (code, val) = query_http(port, "/api/crypto/quantum-mip");
    assert_eq!(code, 200);
    assert!(
        val["chsh_game"]["theorem_185_quantum_advantage_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["relativistic_mip"]["theorem_186_relativistic_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["rigidity"]["theorem_187_rigidity_certified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Port-Hamiltonian Route
    let (code, val) = query_http(port, "/api/cognitive/port-hamiltonian");
    assert_eq!(code, 200);
    assert!(
        val["port_hamiltonian_state"]["theorem_188_passivity_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["casimir_invariants"]["theorem_189_casimir_conserved"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["consensus"]["theorem_190_consensus_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview Route
    let (code, overview) = query_http(port, "/api/cockpit/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.4");
    assert_eq!(overview["total_theorems_enforced"], 190);
    assert_eq!(overview["codename"], "THANATOS");

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
