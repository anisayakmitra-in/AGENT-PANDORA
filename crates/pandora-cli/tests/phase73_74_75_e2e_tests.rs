#![forbid(unsafe_code)]

//! Phase 73, 74, 75 End-to-End Integration Tests (v4.1.5, Codename: THANATOS)
//!
//! Validates:
//! - Phase 73: Higher-Order Topological Insulators & Corner Localization (Theorems 191–193)
//! - Phase 74: Dynamic Proactive Secret Sharing (DPSS) & Asynchronous Redistribution (Theorems 194–196)
//! - Phase 75: Nonlinear Koopman Operator Neural Dynamics & Spectral Decomposition (Theorems 197–199)
//! - Cockpit Web REST endpoints & Centennial Verification Suite (Theorems 1–199)

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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(730);

struct TestEnv {
    root: PathBuf,
}

impl TestEnv {
    fn new(label: &str) -> Self {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("pandora_p73_74_75_{label}_{counter}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");

        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.5",
            "active_model": "thanatos-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Phase 73-75 test fixture."
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
fn test_cli_phase73_hoti_quadrupole_and_corners() {
    let env = TestEnv::new("p73_swarm");

    // 1. hoti-quadrupole-eval
    let res = env.run_success(&[
        "swarm",
        "hoti-quadrupole-eval",
        "--intracell",
        "0.5",
        "--intercell",
        "1.5",
    ]);
    assert!(res["quadrupole"]["is_topological_phase"].as_bool().unwrap());
    assert_eq!(
        res["quadrupole"]["quantized_quadrupole_qxy"]
            .as_f64()
            .unwrap(),
        0.5
    );
    assert!(
        res["quadrupole"]["theorem_191_quadrupole_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. corner-state-audit
    let res2 = env.run_success(&[
        "swarm",
        "corner-state-audit",
        "--grid-size",
        "8",
        "--disorder",
        "0.2",
    ]);
    assert_eq!(
        res2["corner_state_audit"]["num_zero_modes"]
            .as_u64()
            .unwrap(),
        4
    );
    assert!(
        res2["corner_state_audit"]["corner_localization_ratio"]
            .as_f64()
            .unwrap()
            >= 0.80
    );
    assert!(
        res2["corner_state_audit"]["theorem_192_corner_modes_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["corner_state_audit"]["theorem_193_chiral_robustness_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cli_phase74_dpss_redistribution_and_refresh() {
    let env = TestEnv::new("p74_crypto");

    // 1. dpss-redistribute
    let res = env.run_success(&[
        "crypto",
        "dpss-redistribute",
        "--secret",
        "133742",
        "--old-nodes",
        "4",
        "--old-threshold",
        "2",
        "--new-nodes",
        "7",
        "--new-threshold",
        "3",
    ]);
    assert!(
        res["redistribution"]["secret_invariant_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        res["redistribution"]["reconstructed_secret"]
            .as_u64()
            .unwrap(),
        133742
    );
    assert!(
        res["redistribution"]["theorem_194_bivariate_reshare_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. proactive-refresh
    let res2 = env.run_success(&[
        "crypto",
        "proactive-refresh",
        "--secret",
        "133742",
        "--epochs",
        "5",
        "--nodes",
        "5",
        "--threshold",
        "2",
    ]);
    assert!(
        res2["epoch_rotation"]["mobile_adversary_immunity_certified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        res2["epoch_rotation"]["secret_preservation_fidelity"]
            .as_f64()
            .unwrap(),
        1.0
    );
    assert!(
        res2["epoch_rotation"]["theorem_195_epoch_security_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["epoch_rotation"]["theorem_196_async_reconstruction_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cli_phase75_koopman_decomposition_and_stability() {
    let env = TestEnv::new("p75_cognitive");

    // 1. koopman-decompose
    let res = env.run_success(&[
        "cognitive",
        "koopman-decompose",
        "--dimension",
        "4",
        "--snapshots",
        "100",
        "--dictionary-size",
        "16",
    ]);
    assert_eq!(
        res["decomposition"]["linearity_defect"].as_f64().unwrap(),
        0.0
    );
    assert!(
        res["decomposition"]["theorem_197_linear_lift_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["decomposition"]["theorem_198_edmd_convergence_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. spectral-stability-audit
    let res2 = env.run_success(&[
        "cognitive",
        "spectral-stability-audit",
        "--modes",
        "6",
        "--damping",
        "0.15",
    ]);
    assert!(
        res2["spectral_stability"]["is_globally_stable"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res2["spectral_stability"]["max_real_eigenvalue"]
            .as_f64()
            .unwrap()
            <= 0.0
    );
    assert!(
        res2["spectral_stability"]["theorem_199_spectral_stability_verified"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_cockpit_web_phase73_74_75_routes_and_centennial_suite() {
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

    // 1. Swarm HOTI Topology Route
    let (code, val) = query_http(port, "/api/swarm/hoti-topology");
    assert_eq!(code, 200);
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);
    assert!(
        val["quadrupole"]["theorem_191_quadrupole_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["corner_states"]["theorem_192_corner_modes_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["corner_states"]["theorem_193_chiral_robustness_verified"]
            .as_bool()
            .unwrap()
    );

    // 2. Crypto Proactive Reshare Route
    let (code, val) = query_http(port, "/api/crypto/proactive-reshare");
    assert_eq!(code, 200);
    assert!(
        val["redistribution"]["theorem_194_bivariate_reshare_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["epoch_rotations"]["theorem_195_epoch_security_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["epoch_rotations"]["theorem_196_async_reconstruction_verified"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(val["theorems_enforced"].as_array().unwrap().len(), 3);

    // 3. Cognitive Koopman Spectrum Route
    let (code, val) = query_http(port, "/api/cognitive/koopman-spectrum");
    assert_eq!(code, 200);
    assert!(
        val["koopman_decomposition"]["theorem_197_linear_lift_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["koopman_decomposition"]["theorem_198_edmd_convergence_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["spectral_stability"]["theorem_199_spectral_stability_verified"]
            .as_bool()
            .unwrap()
    );
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
