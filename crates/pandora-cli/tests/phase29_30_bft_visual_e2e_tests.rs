//! Phase 29 & Phase 30 E2E Integration Suite:
//! Distributed Swarm PBFT Consensus State Machine Replication (Theorems 62, 63, 64)
//! and Multi-Modal Visual Perception & Layout Verification Harness (Theorems 65, 66, 67).
//!
//! # Invariants
//! - Strictly `#![forbid(unsafe_code)]` workspace-wide.
//! - Non-blocking Byzantine agreement and deterministic view-change.
//! - Isolated fixture sandboxes and ephemeral HTTP/1.1 sockets.

#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(2900);

struct TestFixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl TestFixture {
    fn new(prefix: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock valid")
            .as_nanos();
        let seq = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temp dir canonical path");
        let root = temp_root.join(format!(
            "pandora-{prefix}-{}-{timestamp}-{seq}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("workspace dir created");
        fs::create_dir_all(&data).expect("data dir created");
        Self {
            config: root.join("config.json"),
            data,
            workspace,
            root,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace);
        cmd
    }

    fn run_success(&self, args: &[&str]) -> Value {
        let output = self.command(args).output().expect("command executed");
        assert!(
            output.status.success(),
            "command failed: {:?}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout valid UTF-8");
        serde_json::from_str(&stdout).expect("stdout valid JSON")
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_swarm_bft_simulate_e2e_cli() {
    let fixture = TestFixture::new("swarm-bft-sim");
    let res = fixture.run_success(&[
        "swarm", "bft", "simulate", "--nodes", "4", "--faults", "1", "--rounds", "3", "--json",
    ]);

    assert_eq!(res["command"], "swarm bft simulate");
    assert_eq!(res["nodes"], 4);
    assert_eq!(res["byzantine_faults_injected"], 1);
    assert_eq!(res["rounds_executed"], 3);
    assert_eq!(res["status"], "completed");
    assert!(res["theorem_62_resilience"].as_bool().unwrap());
    assert!(res["theorem_63_safety"].as_bool().unwrap());
    assert!(res["theorem_64_state_sync"].as_bool().unwrap());

    let merkle_root = res["state_merkle_root"].as_str().unwrap();
    assert!(!merkle_root.is_empty());
    assert_eq!(merkle_root.len(), 64);
}

#[test]
fn test_swarm_bft_status_e2e_cli() {
    let fixture = TestFixture::new("swarm-bft-status");
    let res = fixture.run_success(&["swarm", "bft", "status", "--json"]);

    assert_eq!(res["command"], "swarm bft status");
    assert_eq!(res["total_nodes"], 4);
    assert_eq!(res["max_byzantine_faults_tolerated"], 1);
    assert_eq!(res["quorum_threshold"], 3);
    assert_eq!(res["current_view"], 0);
    assert!(
        res["theorems"]["theorem_62_optimal_resilience"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["theorems"]["theorem_63_commit_safety"]
            .as_bool()
            .unwrap()
    );
    assert!(
        res["theorems"]["theorem_64_logarithmic_state_sync"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_harness_visual_inspect_e2e_cli() {
    let fixture = TestFixture::new("harness-visual-inspect");
    let res = fixture.run_success(&["harness", "visual", "inspect", "--json"]);

    assert_eq!(res["command"], "harness visual inspect");
    assert_eq!(res["gene"], "gene-visual-perception");

    let viewports = res["supported_viewports"].as_array().unwrap();
    assert_eq!(viewports.len(), 3);
    assert_eq!(viewports[0]["profile"], "Desktop");
    assert_eq!(viewports[1]["profile"], "Tablet");
    assert_eq!(viewports[2]["profile"], "Mobile");

    let theorems = res["theorems_enforced"].as_array().unwrap();
    assert!(theorems.iter().any(|v| v.as_u64() == Some(65)));
    assert!(theorems.iter().any(|v| v.as_u64() == Some(66)));
    assert!(theorems.iter().any(|v| v.as_u64() == Some(67)));
}

#[test]
fn test_harness_visual_contrast_e2e_cli() {
    let fixture = TestFixture::new("harness-visual-contrast");
    let res = fixture.run_success(&[
        "harness", "visual", "contrast", "--fg", "#00f0ff", "--bg", "#0a0d14", "--json",
    ]);

    assert_eq!(res["command"], "harness visual contrast");
    assert_eq!(res["fg"], "#00f0ff");
    assert_eq!(res["bg"], "#0a0d14");
    assert!(res["passes_wcag_aa"].as_bool().unwrap());
    assert_eq!(res["wcag_level"], "AAA");
    assert!(res["contrast_ratio"].as_f64().unwrap() > 10.0);
    assert!(res["theorem_66_contrast_soundness"].as_bool().unwrap());
}

#[test]
fn test_harness_visual_hash_and_layout_e2e_cli() {
    let fixture = TestFixture::new("harness-visual-ops");
    let hash_res = fixture.run_success(&[
        "harness",
        "visual",
        "hash",
        "--input",
        "dashboard_viewport_test",
        "--compare",
        "dashboard_viewport_test",
        "--json",
    ]);

    assert_eq!(hash_res["command"], "harness visual hash");
    assert_eq!(hash_res["hamming_distance"], 0);
    assert!(hash_res["passes_theorem_65"].as_bool().unwrap());

    let layout_res = fixture.run_success(&[
        "harness",
        "visual",
        "layout",
        "--viewport",
        "mobile",
        "--json",
    ]);

    assert_eq!(layout_res["command"], "harness visual layout");
    assert_eq!(layout_res["viewport"], "mobile");
    assert_eq!(layout_res["width"], 393);
    assert_eq!(layout_res["height"], 851);
    assert!(layout_res["header_contained"].as_bool().unwrap());
    assert!(layout_res["main_contained"].as_bool().unwrap());
    assert!(!layout_res["siblings_overlap"].as_bool().unwrap());
    assert!(layout_res["theorem_67_layout_soundness"].as_bool().unwrap());
}

#[test]
fn test_cockpit_web_bft_and_visual_endpoints() {
    let controller = std::sync::Arc::new(std::sync::Mutex::new(
        pandora_runtime::cockpit::CockpitController::new(
            pandora_runtime::cockpit::CockpitLayout::TwelvePanel,
        ),
    ));
    let server =
        pandora_runtime::cockpit_web::CockpitWebServer::new(0, "127.0.0.1", controller, None)
            .expect("server initialized");

    let port = server.port();
    let shutdown_flag = server.shutdown_handle();

    let handle = thread::spawn(move || {
        let _ = server.run();
    });

    thread::sleep(Duration::from_millis(150));

    // 1. Test GET /api/swarm/bft
    let mut stream_bft = std::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .expect("connected to bft endpoint");
    stream_bft
        .write_all(b"GET /api/swarm/bft HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .expect("write bft request");
    let mut response_bft = String::new();
    stream_bft
        .read_to_string(&mut response_bft)
        .expect("read bft response");
    assert!(response_bft.contains("200 OK"));
    assert!(response_bft.contains("\"max_byzantine_faults_tolerated\":1"));
    assert!(response_bft.contains("theorem_62_optimal_resilience"));

    // 2. Test GET /api/harness/visual
    let mut stream_visual = std::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .expect("connected to visual endpoint");
    stream_visual
        .write_all(
            b"GET /api/harness/visual HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .expect("write visual request");
    let mut response_visual = String::new();
    stream_visual
        .read_to_string(&mut response_visual)
        .expect("read visual response");
    assert!(response_visual.contains("200 OK"));
    assert!(response_visual.contains("gene-visual-perception"));
    assert!(response_visual.contains("Xiaomi 15"));

    shutdown_flag.store(true, Ordering::Relaxed);
    let _ = handle.join();
}
