#![forbid(unsafe_code)]

//! Phase 79, 80, 81 End-to-End Integration Tests (v4.1.7, Codename: THANATOS)
//!
//! Validates:
//! - Phase 79: Twisted Bilayer Moiré Flat Bands & Topological Superconductivity (Theorems 209–211)
//! - Phase 80: Indistinguishability Obfuscation (iO) & Punctured PRFs (Theorems 212–214)
//! - Phase 81: Information-Geometric Dual Connections & Natural Gradient (Theorems 215–217)
//! - Cockpit Web REST endpoints & Bicentennial Verification Suite (Theorems 1–217)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(790);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p79_80_81_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.7",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 79-81 test fixture."
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

    let mut response_bytes = Vec::new();
    stream
        .read_to_end(&mut response_bytes)
        .expect("read response");
    let response_str = String::from_utf8_lossy(&response_bytes);

    let parts: Vec<&str> = response_str.split("\r\n\r\n").collect();
    assert!(parts.len() >= 2, "Invalid HTTP response");
    let status_line = parts[0].lines().next().unwrap();
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();

    let json_val: serde_json::Value =
        serde_json::from_str(parts[1]).expect("parse response body as json");
    (status_code, json_val)
}

#[test]
fn test_cli_phase79_moire_flat_band_and_superfluid() {
    let env = TestEnv::new("p79_moire");

    // 1. moire-flat-band
    let res = env.run_success(&[
        "swarm",
        "moire-flat-band",
        "--angle",
        "1.08",
        "--interlayer-coupling",
        "0.57735",
    ]);
    assert!(res["flat_band"]["is_magic_regime"].as_bool().unwrap());
    assert!(res["flat_band"]["flat_band_width_mev"].as_f64().unwrap() <= 15.0);
    assert!(
        res["flat_band"]["theorem_209_velocity_vanishing"]
            .as_bool()
            .unwrap()
    );

    // 2. superfluid-weight-audit
    let res2 = env.run_success(&[
        "swarm",
        "superfluid-weight-audit",
        "--gap",
        "1.5",
        "--metric-trace",
        "0.45",
    ]);
    assert!(
        res2["superfluid"]["superfluid_weight_ds"].as_f64().unwrap()
            >= res2["superfluid"]["geometric_lower_bound"]
                .as_f64()
                .unwrap()
    );
    assert!(
        res2["superfluid"]["theorem_211_superfluid_bound_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cli_phase80_io_obfuscate_and_punctured_prf() {
    let env = TestEnv::new("p80_io");

    // 1. io-obfuscate
    let res = env.run_success(&[
        "crypto",
        "io-obfuscate",
        "--circuit-depth",
        "4",
        "--inputs",
        "4",
    ]);
    assert_eq!(res["obfuscated_circuit"]["evaluation_errors"], 0);
    assert!(
        res["obfuscated_circuit"]["theorem_212_functionality_preserved"]
            .as_bool()
            .unwrap()
    );

    // 2. io-punctured-audit
    let res2 = env.run_success(&[
        "crypto",
        "io-punctured-audit",
        "--seed",
        "424242",
        "--puncture-point",
        "5",
    ]);
    assert!(
        res2["punctured_prf"]["theorem_214_punctured_security_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["soundness"]["theorem_213_io_soundness_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cli_phase81_info_geo_dual_and_natural_gradient() {
    let env = TestEnv::new("p81_info_geo");

    // 1. info-geo-dual
    let res = env.run_success(&[
        "cognitive",
        "info-geo-dual",
        "--alpha",
        "1.0",
        "--dimension",
        "4",
    ]);
    assert!(
        res["dual_connection"]["theorem_215_dual_connection_compatible"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["pythagorean"]["theorem_217_pythagorean_theorem_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. natural-gradient-eval
    let res2 = env.run_success(&[
        "cognitive",
        "natural-gradient-eval",
        "--learning-rate",
        "0.05",
    ]);
    assert!(
        res2["natural_gradient"]["theorem_216_natural_gradient_invariant"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cockpit_web_phase79_80_81_routes_and_centennial_suite() {
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

    // 1. Swarm Moire Topology Route
    let (code, val) = query_http(port, "/api/swarm/moire-topology");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert!(
        val["flat_band"]["theorem_209_velocity_vanishing"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["valley_chern"]["theorem_210_valley_chern_quantized"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["superfluid"]["theorem_211_superfluid_bound_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. Crypto Indistinguishability Obfuscation Route
    let (code, val) = query_http(port, "/api/crypto/indistinguishability-obfuscation");
    assert_eq!(code, 200);
    assert!(
        val["circuit"]["theorem_212_functionality_preserved"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["soundness"]["theorem_213_io_soundness_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["punctured_prf"]["theorem_214_punctured_security_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Information Geometry Route
    let (code, val) = query_http(port, "/api/cognitive/information-geometry");
    assert_eq!(code, 200);
    assert!(
        val["dual_connection"]["theorem_215_dual_connection_compatible"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["natural_gradient"]["theorem_216_natural_gradient_invariant"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["pythagorean"]["theorem_217_pythagorean_theorem_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview & Centennial Suite (217 Theorems)
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
