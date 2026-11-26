#![forbid(unsafe_code)]

//! Phase 76, 77, 78 End-to-End Integration Tests (v4.1.6, Codename: THANATOS)
//!
//! Validates:
//! - Phase 76: Anyonic Quantum Hall Interferometry & Fractional Statistics (Theorems 200–202)
//! - Phase 77: Functional Encryption for Inner Products & Quadratic Forms (Theorems 203–205)
//! - Phase 78: Neural Thermodynamic Uncertainty Relations (TUR) & Dissipation (Theorems 206–208)
//! - Cockpit Web REST endpoints & Bicentennial Verification Suite (Theorems 1–208)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(760);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p76_77_78_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.6",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 76-78 test fixture."
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
fn test_cli_phase76_anyonic_interferometry_and_thermal_hall() {
    let env = TestEnv::new("p76_swarm");

    // 1. anyon-interfere
    let res = env.run_success(&[
        "swarm",
        "anyon-interfere",
        "--flux",
        "1.0",
        "--filling-fraction",
        "0.3333333333",
        "--quasiparticles",
        "2",
    ]);
    assert!(
        res["interference"]["fractional_statistics_detected"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["interference"]["theorem_200_interferometry_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["interference"]["theorem_201_monodromy_parity_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. thermal-hall-audit
    let res2 = env.run_success(&[
        "swarm",
        "thermal-hall-audit",
        "--temperature",
        "0.05",
        "--central-charge",
        "1.0",
    ]);
    assert!(
        res2["thermal_hall"]["cft_universality_certified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        res2["thermal_hall"]["disorder_immunity_error"]
            .as_f64()
            .unwrap(),
        0.0
    );
    assert!(
        res2["thermal_hall"]["theorem_202_thermal_hall_quantized"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cli_phase77_mife_inner_product_and_security() {
    let env = TestEnv::new("p77_crypto");

    // 1. fe-inner-product
    let res = env.run_success(&[
        "crypto",
        "fe-inner-product",
        "--x-vector",
        "1,2,3,4",
        "--y-vector",
        "4,3,2,1",
    ]);
    assert_eq!(
        res["inner_product"]["inner_product_result"]
            .as_u64()
            .unwrap(),
        20
    );
    assert!(
        res["inner_product"]["decryption_matches_exact_inner_product"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["inner_product"]["theorem_203_inner_product_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. fe-security-audit
    let res2 = env.run_success(&["crypto", "fe-security-audit", "--dimension", "4"]);
    assert!(
        res2["mife_security"]["cross_party_privacy_preserved"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["mife_security"]["theorem_204_mife_quadratic_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["mife_security"]["theorem_205_adaptive_security_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cli_phase78_tur_bound_and_dissipation() {
    let env = TestEnv::new("p78_cognitive");

    // 1. tur-bound-eval
    let res = env.run_success(&[
        "cognitive",
        "tur-bound-eval",
        "--current",
        "1.0",
        "--variance",
        "0.5",
        "--entropy-rate",
        "5.0",
    ]);
    assert!(res["tur_bound"]["tur_bound_satisfied"].as_bool().unwrap());
    assert!(
        res["tur_bound"]["relative_uncertainty"].as_f64().unwrap()
            >= res["tur_bound"]["tur_lower_bound"].as_f64().unwrap()
    );
    assert!(
        res["tur_bound"]["theorem_206_tur_bound_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. dissipation-audit
    let res2 = env.run_success(&[
        "cognitive",
        "dissipation-audit",
        "--states",
        "4",
        "--temperature",
        "300.0",
    ]);
    assert!(
        res2["dissipation_audit"]["entropy_production_rate"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert!(
        res2["dissipation_audit"]["carnot_efficiency_limit"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert!(
        res2["dissipation_audit"]["theorem_207_entropy_production_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["dissipation_audit"]["theorem_208_carnot_efficiency_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cockpit_web_phase76_77_78_routes_and_centennial_suite() {
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

    // 1. Swarm Anyonic Interferometry Route
    let (code, val) = query_http(port, "/api/swarm/anyonic-interferometry");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert!(
        val["interference"]["theorem_200_interferometry_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["interference"]["theorem_201_monodromy_parity_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["thermal_hall"]["theorem_202_thermal_hall_quantized"]
            .as_bool()
            .unwrap()
    );

    // 2. Crypto Functional Encryption Route
    let (code, val) = query_http(port, "/api/crypto/multi-input-functional-encryption");
    assert_eq!(code, 200);
    assert!(
        val["inner_product"]["theorem_203_inner_product_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["mife_quadratic"]["theorem_204_mife_quadratic_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["mife_quadratic"]["theorem_205_adaptive_security_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Thermodynamic TUR Route
    let (code, val) = query_http(port, "/api/cognitive/thermodynamic-tur");
    assert_eq!(code, 200);
    assert!(
        val["tur_bound"]["theorem_206_tur_bound_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["dissipation"]["theorem_207_entropy_production_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["dissipation"]["theorem_208_carnot_efficiency_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 4. Cockpit Overview Route
    let (code, overview) = query_http(port, "/api/cockpit/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.1.6");
    assert_eq!(overview["total_theorems_enforced"], 208);
    assert_eq!(overview["codename"], "THANATOS");

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
