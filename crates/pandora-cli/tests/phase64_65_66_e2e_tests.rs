#![forbid(unsafe_code)]

//! Phase 64, 65, 66 End-to-End Integration Tests (v4.1.2, Codename: THANATOS)
//!
//! Validates:
//! - Phase 64: Non-Abelian Anyonic Quantum Memory & Fibonacci Braid Fault-Tolerance (Theorems 164–166)
//! - Phase 65: Post-Quantum Transparent zk-SNARKs with Lattice Polynomial Commitments (Theorems 167–169)
//! - Phase 66: Stochastic McKean-Vlasov Neural Jump-Diffusion with Non-Local Lévy Operators (Theorems 170–172)
//! - Cockpit Web REST endpoints & Formal Verification Suite (Theorems 1–172)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(400);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p64_65_66_{label}_{counter}"));
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
            "system_prompt": "You are AGENT-PANDORA Phase 64-66 test fixture."
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
fn test_phase64_swarm_topological_memory_cli() {
    let env = TestEnv::new("p64");

    // 1. Anyonic Braid Synthesis
    let braid_res = env.run_success(&[
        "swarm",
        "anyonic-braid",
        "--target",
        "1.5",
        "--max-length",
        "16",
    ]);
    assert!(
        braid_res["report"]["theorem_164_density_satisfied"]
            .as_bool()
            .unwrap()
    );
    assert!(
        braid_res["memory_block"]["logical_state_fidelity"]
            .as_f64()
            .unwrap()
            > 0.95
    );
    assert_eq!(braid_res["theorems"].as_array().unwrap().len(), 2);

    // 2. Syndrome Measurement & Fusion
    let fusion_res = env.run_success(&[
        "swarm",
        "syndrome-fusion",
        "--physical-error",
        "0.01",
        "--syndromes",
        "0,1,0,1",
    ]);
    assert_eq!(
        fusion_res["fusion_report"]["primary_outcome"],
        "VacuumIdentity"
    );
    assert_eq!(fusion_res["fusion_report"]["recovery_success"], true);
    assert!(
        fusion_res["fusion_report"]["theorem_166_recovery_probability"]
            .as_f64()
            .unwrap()
            > 0.99
    );
}

#[test]
fn test_phase65_crypto_lattice_zk_cli() {
    let env = TestEnv::new("p65");

    // 1. Lattice Prove
    let prove_res = env.run_success(&[
        "crypto",
        "lattice-prove",
        "--coeffs",
        "42,-99,1337,2048",
        "--queries",
        "500",
    ]);
    assert_eq!(prove_res["commitment"]["ring_dimension"], 64);
    assert_eq!(prove_res["proof_valid"], true);
    assert!(
        prove_res["fiat_shamir_challenge"]
            .as_str()
            .unwrap()
            .starts_with("fs_0x")
    );

    // 2. Lattice Verify
    let verify_res = env.run_success(&[
        "crypto",
        "lattice-verify",
        "--coeffs",
        "42,-99,1337,2048",
        "--queries",
        "500",
    ]);
    assert_eq!(verify_res["theorem_167_commitment_sound"], true);
    assert_eq!(verify_res["theorem_168_permutation_sound"], true);
    assert_eq!(verify_res["theorem_169_qrom_nizk_verified"], true);
    assert_eq!(verify_res["verification_passed"], true);
}

#[test]
fn test_phase66_cognitive_mckean_vlasov_jump_cli() {
    let env = TestEnv::new("p66");

    // 1. McKean-Vlasov Jump Simulation
    let sim_res = env.run_success(&[
        "cognitive",
        "jump-simulate",
        "--particles",
        "20",
        "--dim",
        "2",
        "--steps",
        "10",
        "--dt",
        "0.01",
    ]);
    assert_eq!(sim_res["particle_count"], 20);
    assert_eq!(sim_res["dimension"], 2);
    assert_eq!(sim_res["theorem_170_well_posed"], true);
    assert_eq!(sim_res["theorem_172_chaos_bound_satisfied"], true);
    assert!(sim_res["propagation_of_chaos_error"].as_f64().unwrap() > 0.0);

    // 2. Generator Audit
    let audit_res = env.run_success(&["cognitive", "generator-audit", "--points", "0.1,0.5,1.0"]);
    assert_eq!(audit_res["theorem_171_generator_conserved"], true);
    assert!(audit_res["dissipation_bound"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_cockpit_web_v412_routes_and_thanatos_overview() {
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

    // 1. Swarm Topological Quantum Memory Route
    let (code, val) = query_http(port, "/api/swarm/topological-memory");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert_eq!(val["rotation"]["theorem_164_density_satisfied"], true);
    assert!(
        val["memory_block"]["logical_state_fidelity"]
            .as_f64()
            .unwrap()
            > 0.95
    );
    assert_eq!(val["fusion"]["recovery_success"], true);

    // 2. Crypto Lattice zk-SNARK Route
    let (code, val) = query_http(port, "/api/crypto/lattice-zk");
    assert_eq!(code, 200);
    assert_eq!(val["proof"]["proof_valid"], true);
    assert_eq!(val["verification_report"]["verification_passed"], true);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive McKean-Vlasov Jump Route
    let (code, val) = query_http(port, "/api/cognitive/mckean-vlasov-jump");
    assert_eq!(code, 200);
    assert_eq!(val["simulation"]["theorem_170_well_posed"], true);
    assert_eq!(val["audit"]["theorem_171_generator_conserved"], true);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview Route
    let (code, overview) = query_http(port, "/api/cockpit/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.8");
    assert_eq!(overview["total_theorems_enforced"], 226);
    assert_eq!(overview["codename"], "THANATOS");

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
