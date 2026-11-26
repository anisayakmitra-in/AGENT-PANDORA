#![forbid(unsafe_code)]

//! Phase 88, 89, 90 End-to-End Integration Tests (v4.2.0, Codename: THANATOS)
//!
//! Validates:
//! - Phase 88: Monoidal Functor Action Spaces & Higher-Order Consensus (Theorems 236â€“238)
//! - Phase 89: Post-Quantum Threshold Zero-Knowledge Arguments (Theorems 239â€“241)
//! - Phase 90: Monadic Pushdown Automata & Provably Bounded Context Grammars (Theorems 242â€“244)
//! - Cockpit Web REST endpoints & Centennial Verification Suite (Theorems 1â€“244)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(880);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p88_89_90_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.2.0",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 88-90 test fixture."
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
fn test_cli_phase88_monoidal_action() {
    let env = TestEnv::new("p88_monoidal");

    // 1. monoidal-action-eval
    let res = env.run_success(&["swarm", "monoidal-action-eval", "--rank", "4"]);
    assert!(
        res["monoidal_action"]["sequential_composition_valid"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["monoidal_action"]["tensor_product_preserved"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["monoidal_action"]["theorem_236_functoriality_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 1);

    // 2. braided-consensus-audit
    let res2 = env.run_success(&[
        "swarm",
        "braided-consensus-audit",
        "--tiers",
        "3",
        "--agents",
        "6",
    ]);
    assert!(
        res2["interchange_consensus"]["deadlock_free_consensus_certified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["interchange_consensus"]["theorem_237_interchange_law_holds"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["braided_morphism"]["braiding_symmetry_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["braided_morphism"]["theorem_238_braided_invariance_certified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 2);
}

#[test]
fn test_cli_phase89_threshold_zk() {
    let env = TestEnv::new("p89_threshold_zk");

    // 1. threshold-zk-prove
    let res = env.run_success(&[
        "crypto",
        "threshold-zk-prove",
        "--threshold",
        "3",
        "--parties",
        "5",
    ]);
    assert!(
        res["threshold_sigma"]["protocol_completeness_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["threshold_sigma"]["theorem_239_completeness_and_hvzk"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 1);

    // 2. fiat-shamir-audit
    let res2 = env.run_success(&[
        "crypto",
        "fiat-shamir-audit",
        "--forks",
        "2",
        "--bits",
        "256",
    ]);
    assert!(
        res2["witness_extraction"]["witness_extracted_valid"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["witness_extraction"]["theorem_240_special_soundness_holds"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["fiat_shamir"]["simulation_soundness_certified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["fiat_shamir"]["theorem_241_non_malleable_fiat_shamir"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 2);
}

#[test]
fn test_cli_phase90_pushdown_grammar() {
    let env = TestEnv::new("p90_pushdown");

    // 1. grammar-mask-eval
    let res = env.run_success(&[
        "cognitive",
        "grammar-mask-eval",
        "--rules",
        "12",
        "--vocab-size",
        "32000",
        "--allowed",
        "48",
    ]);
    assert!(
        res["pushdown_recognizer"]["recognition_time_complexity_linear"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["pushdown_recognizer"]["theorem_242_pushdown_equivalence_holds"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["grammar_masking"]["theorem_243_masking_soundness_guaranteed"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res["theorems"].as_array().unwrap().len(), 2);

    // 2. pushdown-depth-audit
    let res2 = env.run_success(&["cognitive", "pushdown-depth-audit", "--depth", "14"]);
    assert!(
        res2["bounded_stack"]["decidability_certified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["bounded_stack"]["theorem_244_bounded_memory_decidable"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(res2["theorems"].as_array().unwrap().len(), 1);
}

#[test]
fn test_cockpit_web_phase88_89_90_routes_and_centennial_suite() {
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

    // 1. Swarm Monoidal Action Route
    let (code, val) = query_http(port, "/api/swarm/monoidal-action");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert!(
        val["action"]["theorem_236_functoriality_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["interchange"]["theorem_237_interchange_law_holds"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["braiding"]["theorem_238_braided_invariance_certified"]
            .as_bool()
            .unwrap()
    );

    // 2. Crypto Threshold ZK Route
    let (code, val) = query_http(port, "/api/crypto/threshold-zk");
    assert_eq!(code, 200);
    assert!(
        val["sigma"]["theorem_239_completeness_and_hvzk"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["extraction"]["theorem_240_special_soundness_holds"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["fiat_shamir"]["theorem_241_non_malleable_fiat_shamir"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Pushdown Grammar Route
    let (code, val) = query_http(port, "/api/cognitive/pushdown-grammar");
    assert_eq!(code, 200);
    assert!(
        val["pushdown"]["theorem_242_pushdown_equivalence_holds"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["masking"]["theorem_243_masking_soundness_guaranteed"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["bounded_stack"]["theorem_244_bounded_memory_decidable"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview & Centennial Suite (244 Theorems)
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
