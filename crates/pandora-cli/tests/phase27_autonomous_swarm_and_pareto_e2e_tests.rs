//! Phase 27 E2E Integration Suite:
//! Autonomous Multi-Agent Swarm Coding Pipeline, Token-Level Grammar AST Patching,
//! PAC-Bayes Risk Verification, Schwartz-Zippel ZK Attestation,
//! Hierarchical KV-Cache Offloading, and Inference Pareto Frontier Optimization
//! (Theorems 33, 35, 41, 48, 50, 56, 57).
//!
//! # Invariants
//! - Strictly `#![forbid(unsafe_code)]` workspace-wide.
//! - Deterministic polynomial verifications and pushdown grammar parsing.
//! - Safe ephemeral sockets and isolated fixture sandboxes.

#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(2700);

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
fn test_autonomous_swarm_pipeline_run() {
    let fixture = TestFixture::new("swarm-pipeline");
    let res = fixture.run_success(&[
        "swarm",
        "pipeline",
        "run",
        "--task",
        "task-e2e-heal-01",
        "--max-rounds",
        "3",
        "--auto-heal",
        "--json",
    ]);

    assert_eq!(res["pipeline"], "autonomous_swarm_sdlc");
    assert_eq!(res["task_id"], "task-e2e-heal-01");
    assert_eq!(res["certified"], true);
    assert!(res["total_rounds"].as_u64().unwrap() >= 1);
    assert!(!res["merkle_receipt_sha256"].as_str().unwrap().is_empty());

    let theorems = res["theorems_verified"].as_array().expect("theorems list");
    let theorem_nums: Vec<u64> = theorems.iter().filter_map(|t| t.as_u64()).collect();
    assert!(theorem_nums.contains(&33)); // PAC-Bayes risk bound
    assert!(theorem_nums.contains(&35)); // Spectral Cheeger bound
    assert!(theorem_nums.contains(&41)); // Merkle DAG
    assert!(theorem_nums.contains(&48)); // Schwartz-Zippel ZK attestation
    assert!(theorem_nums.contains(&50)); // Grammar AST patching
}

#[test]
fn test_autonomous_swarm_pipeline_status() {
    let fixture = TestFixture::new("swarm-status");
    let res = fixture.run_success(&["swarm", "pipeline", "status", "--json"]);

    assert_eq!(res["status"], "ready");
    assert_eq!(res["ast_confinement"], "active");
    assert_eq!(res["zk_attestation"], "schwartz_zippel_goldilocks");
    assert_eq!(res["roles"].as_array().unwrap().len(), 4);
}

#[test]
fn test_inference_pareto_frontier_cli() {
    let fixture = TestFixture::new("inference-pareto");
    let res = fixture.run_success(&["runtime", "inference", "--pareto", "--json"]);

    assert_eq!(res["engine"], "InferenceParetoAnalyzer");
    assert_eq!(res["theorem"], 57);
    assert_eq!(res["theorem_57_pareto_optimality_verified"], true);
    assert!(res["pareto_optimal_points_count"].as_u64().unwrap() >= 1);
    assert!(res["recommended_configuration"].is_string());
}

#[test]
fn test_hierarchical_kv_cache_cli() {
    let fixture = TestFixture::new("kv-cache-hierarchical");
    let res = fixture.run_success(&["runtime", "kv-cache", "--hierarchical", "--json"]);

    assert_eq!(res["mode"], "hierarchical_3_tier");
    assert_eq!(res["theorem"], 56);
    assert_eq!(res["theorem_56_latency_bound_satisfied"], true);
    assert!(res["l0_vram_blocks"].as_u64().unwrap() > 0);
    assert!(res["total_blocks"].as_u64().unwrap() > 0);
}

#[test]
fn test_cockpit_web_benchmark_and_swarm_endpoints() {
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

    // 1. Test GET /api/benchmark
    let mut stream_bench = std::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .expect("connected to benchmark endpoint");
    stream_bench
        .write_all(b"GET /api/benchmark HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .expect("write request");
    let mut response_bench = String::new();
    stream_bench
        .read_to_string(&mut response_bench)
        .expect("read benchmark response");
    assert!(
        response_bench.contains("200 OK"),
        "benchmark endpoint response: {response_bench}"
    );
    assert!(response_bench.contains("theorem_57_pareto_optimality_verified"));

    // 2. Test GET /api/swarm/pipeline
    let mut stream_swarm = std::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .expect("connected to swarm endpoint");
    stream_swarm
        .write_all(
            b"GET /api/swarm/pipeline HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .expect("write request");
    let mut response_swarm = String::new();
    stream_swarm
        .read_to_string(&mut response_swarm)
        .expect("read swarm response");
    assert!(
        response_swarm.contains("200 OK"),
        "swarm endpoint response: {response_swarm}"
    );
    assert!(response_swarm.contains("task-autonomous-heal-01"));

    shutdown_flag.store(true, Ordering::Relaxed);
    let _ = handle.join();
}
