#![forbid(unsafe_code)]

//! Phase 85, 86, 87 End-to-End Integration Tests (v4.1.9, Codename: THANATOS)
//!
//! Validates:
//! - Phase 85: Categorical Memory & Kan Extension Alignment (Theorems 227–229)
//! - Phase 86: Game-Theoretic Mechanism Design & Core Equivalence (Theorems 230–232)
//! - Phase 87: Conformal Risk Control & Finite-Sample PAC Bounds (Theorems 233–235)
//! - Cockpit Web REST endpoints & Centennial Verification Suite (Theorems 1–235)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(850);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p85_86_87_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.9",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 85-87 test fixture."
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
fn test_cli_phase85_categorical_memory() {
    let env = TestEnv::new("p85_cat_mem");

    // 1. kan-extension
    let res = env.run_success(&[
        "swarm",
        "kan-extension",
        "--source-dim",
        "16",
        "--target-dim",
        "16",
    ]);
    assert!(
        res["kan_report"]["natural_transformation_commutes"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["kan_report"]["theorem_227_kan_extension_aligned"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 1);

    // 2. adjoint-sync
    let res2 = env.run_success(&[
        "swarm",
        "adjoint-sync",
        "--episodic",
        "32",
        "--semantic",
        "16",
        "--depth",
        "4",
    ]);
    assert!(
        res2["adjoint_report"]["adjunction_bijection_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["monoidal_report"]["maclane_coherence_certified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 2);
}

#[test]
fn test_cli_phase86_game_theoretic_core() {
    let env = TestEnv::new("p86_game_core");

    // 1. vcg-auction-audit
    let res = env.run_success(&[
        "crypto",
        "vcg-auction-audit",
        "--valuations",
        "14.0,10.5,7.0,3.5",
    ]);
    assert!(
        res["vcg_report"]["truthfulness_dominant_strategy_holds"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 1);

    // 2. shapley-core-eval
    let res2 = env.run_success(&["crypto", "shapley-core-eval", "--agents", "4"]);
    assert!(
        res2["attribution_report"]["axiomatic_efficiency_satisfied"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["stability_report"]["core_is_non_empty"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 2);
}

#[test]
fn test_cli_phase87_conformal_risk() {
    let env = TestEnv::new("p87_conformal");

    // 1. conformal-coverage
    let res = env.run_success(&[
        "cognitive",
        "conformal-coverage",
        "--samples",
        "100",
        "--alpha",
        "0.10",
        "--density-ratio",
        "2.5",
    ]);
    assert!(
        res["conformal_report"]["theorem_233_coverage_guaranteed"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["shift_report"]["target_coverage_valid"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 2);

    // 2. pac-risk-audit
    let res2 = env.run_success(&["cognitive", "pac-risk-audit", "--tau", "64", "--kl", "0.50"]);
    assert!(
        res2["pac_bayes_report"]["martingale_risk_certified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["pac_bayes_report"]["theorem_235_pac_bayes_bound_holds"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 1);
}

#[test]
fn test_cockpit_web_phase85_86_87_routes_and_centennial_suite() {
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

    // 1. Swarm Categorical Memory Route
    let (code, val) = query_http(port, "/api/swarm/categorical-memory");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert!(
        val["kan_extension"]["theorem_227_kan_extension_aligned"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["adjoint_sync"]["theorem_228_adjoint_sync_holds"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["monoidal_coherence"]["theorem_229_monoidal_coherence_valid"]
            .as_bool()
            .unwrap()
    );

    // 2. Crypto Game Theoretic Core Route
    let (code, val) = query_http(port, "/api/crypto/game-theoretic-core");
    assert_eq!(code, 200);
    assert!(
        val["vcg"]["theorem_230_vcg_truthfulness_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["attribution"]["theorem_231_shapley_axioms_satisfied"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["stability"]["theorem_232_core_stability_certified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Conformal Risk Route
    let (code, val) = query_http(port, "/api/cognitive/conformal-risk");
    assert_eq!(code, 200);
    assert!(
        val["conformal_coverage"]["theorem_233_coverage_guaranteed"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["covariate_shift"]["theorem_234_shift_correction_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["pac_bayes_risk"]["theorem_235_pac_bayes_bound_holds"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview & Centennial Suite (235 Theorems)
    let (code, val) = query_http(port, "/api/cockpit/overview");
    assert_eq!(code, 200);
    assert_eq!(val["architecture_version"], "4.1.9");
    assert_eq!(val["codename"], "THANATOS");
    assert_eq!(val["total_theorems_enforced"], 235);
    assert!(
        val["formal_verification"]["all_proofs_valid"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["formal_verification"]["total_centennial_theorems"], 235);

    // Shutdown
    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
