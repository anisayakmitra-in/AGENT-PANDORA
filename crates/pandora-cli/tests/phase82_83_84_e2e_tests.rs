#![forbid(unsafe_code)]

//! Phase 82, 83, 84 End-to-End Integration Tests (v4.1.8, Codename: THANATOS)
//!
//! Validates:
//! - Phase 82: Simplicial Complex Swarm Homology & Persistent Topology (Theorems 218â€“220)
//! - Phase 83: Linear Logic Capability Monads & Non-Interference (Theorems 221â€“223)
//! - Phase 84: Sheaf-Theoretic Context Consistency & Cohomology Obstruction (Theorems 224â€“226)
//! - Cockpit Web REST endpoints & Centennial Verification Suite (Theorems 1â€“226)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(820);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p82_83_84_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.8",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 82-84 test fixture."
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
        assert!(
            out.status.success(),
            "command failed: {:?}\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!(
                "failed to parse JSON from stdout:\nerror: {e}\nstdout: {stdout}\nstderr: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        })
    }
}

fn query_http(port: u16, path: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to cockpit server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("write request");

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read response");
    let resp = String::from_utf8_lossy(&buf);

    let mut lines = resp.split("\r\n");
    let status_line = lines.next().expect("status line");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("parse status code");

    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("");
    let json: serde_json::Value =
        serde_json::from_str(body).unwrap_or(serde_json::Value::String(body.to_string()));

    (status_code, json)
}

#[test]
fn test_cli_phase82_simplicial_homology() {
    let env = TestEnv::new("p82_simplicial");

    // 1. betti-numbers
    let res = env.run_success(&["swarm", "betti-numbers", "--nodes", "16", "--radius", "1.2"]);
    assert!(
        res["betti_report"]["theorem_218_cech_approximation"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["betti_report"]["theorem_219_betti_invariant_holds"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 2);

    // 2. spectral-cheeger-audit
    let res2 = env.run_success(&["swarm", "spectral-cheeger-audit", "--lambda2", "0.35"]);
    assert!(
        res2["spectral_report"]["theorem_220_cheeger_bound_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 1);
}

#[test]
fn test_cli_phase83_linear_capability() {
    let env = TestEnv::new("p83_linear_cap");

    // 1. linear-token-eval
    let res = env.run_success(&[
        "crypto",
        "linear-token-eval",
        "--token-id",
        "cap-tok-001",
        "--operations",
        "2",
    ]);
    assert!(
        res["linear_token"]["theorem_221_linear_non_duplication"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 1);

    // 2. non-interference-audit
    let res2 = env.run_success(&[
        "crypto",
        "non-interference-audit",
        "--high-domain",
        "enclave_high",
        "--low-domain",
        "public_low",
        "--allocated",
        "16",
    ]);
    assert!(
        res2["non_interference"]["theorem_222_non_interference_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["affine_reclamation"]["theorem_223_affine_deadlock_free"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 2);
}

#[test]
fn test_cli_phase84_sheaf_context() {
    let env = TestEnv::new("p84_sheaf_ctx");

    // 1. sheaf-gluing-check
    let res = env.run_success(&[
        "cognitive",
        "sheaf-gluing-check",
        "--covers",
        "4",
        "--agree",
        "true",
    ]);
    assert!(
        res["sheaf_gluing"]["theorem_224_sheaf_gluing_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 1);

    // 2. cohomology-obstruction
    let res2 = env.run_success(&["cognitive", "cohomology-obstruction", "--defect", "0.75"]);
    assert!(
        res2["obstruction"]["theorem_225_cohomology_obstruction_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["repair"]["theorem_226_cocycle_repair_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 2);
}

#[test]
fn test_cockpit_web_phase82_83_84_routes_and_centennial_suite() {
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

    // 1. Swarm Simplicial Homology Route
    let (code, val) = query_http(port, "/api/swarm/simplicial-homology");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert!(
        val["betti"]["theorem_218_cech_approximation"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["betti"]["theorem_219_betti_invariant_holds"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["spectral"]["theorem_220_cheeger_bound_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. Crypto Linear Capability Route
    let (code, val) = query_http(port, "/api/crypto/linear-capability");
    assert_eq!(code, 200);
    assert!(
        val["token"]["theorem_221_linear_non_duplication"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["non_interference"]["theorem_222_non_interference_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["reclamation"]["theorem_223_affine_deadlock_free"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Sheaf Context Route
    let (code, val) = query_http(port, "/api/cognitive/sheaf-context");
    assert_eq!(code, 200);
    assert!(
        val["gluing"]["theorem_224_sheaf_gluing_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["obstruction"]["theorem_225_cohomology_obstruction_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["repair"]["theorem_226_cocycle_repair_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview & Centennial Suite (226 Theorems)
    let (code, val) = query_http(port, "/api/cockpit/overview");
    assert_eq!(code, 200);
    assert_eq!(val["architecture_version"], "4.2.0");
    assert_eq!(val["codename"], "THANATOS");
    assert_eq!(val["total_theorems_enforced"], 244);
    assert!(
        val["formal_verification"]["all_proofs_valid"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["formal_verification"]["total_centennial_theorems"], 244);

    // Shutdown
    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
