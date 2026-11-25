#![forbid(unsafe_code)]

//! Phase 40, Phase 41, & Phase 42 End-to-End Integration Tests
//!
//! Validates:
//! - Phase 40: Distributed Multi-Tenant Mesh Orchestration & Cross-Cluster Atomic Commitments (Theorems 92–94)
//! - Phase 41: Hyper-Dimensional Vector VFS & Holographic Cognitive Associative Memory (Theorems 95–97)
//! - Phase 42: Formal Kernel Verification & Lean 4 / Coq Machine-Checked Proof Exports (Theorems 98–100)
//! - Cockpit Web REST Endpoints:
//!   - `GET /api/swarm/mesh`
//!   - `GET /api/memory/hyper`
//!   - `GET /api/audit/formal`
//!   - `GET /api/olympus/overview` (Total 100 Centennial Theorems)

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(4000);

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
            "pandora-phase40-{prefix}-{}-{timestamp}-{seq}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.0.5",
            "active_model": "test-hyperion-llm",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA Centennial test fixture."
        });
        fs::write(
            &config,
            serde_json::to_string_pretty(&default_config).expect("serialize default config"),
        )
        .expect("write default config");

        Self {
            root,
            config,
            data,
            workspace,
        }
    }

    fn run_success(&self, args: &[&str]) -> Value {
        let bin = env!("CARGO_BIN_EXE_pandora");
        let mut cmd = Command::new(bin);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.arg("--config")
            .arg(&self.config)
            .arg("--data-dir")
            .arg(&self.data)
            .arg("--workspace")
            .arg(&self.workspace);

        let output = cmd.output().expect("execute command");
        assert!(
            output.status.success(),
            "command failed: {:?}\nstatus: {:?}\nstderr: {}\nstdout: {}",
            args,
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout).expect("parse json response")
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn query_http(port: u16, path: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to cockpit server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("set write timeout");

    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("send http request");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read http response");
    let resp_str = String::from_utf8_lossy(&response);

    let parts: Vec<&str> = resp_str.split("\r\n\r\n").collect();
    assert!(
        parts.len() >= 2,
        "malformed http response: no header delimiter"
    );

    let header_line = parts[0]
        .lines()
        .next()
        .expect("first line of response header");
    let status_code: u16 = header_line
        .split_whitespace()
        .nth(1)
        .expect("status code in header line")
        .parse()
        .expect("parse u16 status code");

    let body_json: Value =
        serde_json::from_str(parts[1]).expect("parse response body as valid json");
    (status_code, body_json)
}

#[test]
fn test_phase40_swarm_mesh_and_cross_commit_cli() {
    let fixture = TestFixture::new("p40-mesh");

    // 1. Swarm Mesh status
    let res = fixture.run_success(&["swarm", "mesh", "--json"]);
    assert_eq!(res["total_nodes"], 8);
    assert_eq!(res["theorem_92_isolated"], true);
    assert_eq!(res["theorem_93_bft_atomic"], true);
    assert_eq!(res["theorem_94_hop_bound_met"], true);

    // 2. Swarm Cross-Cluster Commit
    let res = fixture.run_success(&[
        "swarm",
        "cross-commit",
        "--tx-id",
        "tx-olympus-01",
        "--shards",
        "shard-0,shard-1,shard-2",
        "--json",
    ]);
    assert_eq!(res["committed"], true);
    assert_eq!(res["shards_approved"], 3);
    assert_eq!(res["theorem_92_non_interference_verified"], true);
    assert_eq!(res["theorem_93_atomic_consensus_verified"], true);
    assert_eq!(res["theorem_94_routing_bounded"], true);
}

#[test]
fn test_phase41_hyper_vfs_and_holographic_recall_cli() {
    let fixture = TestFixture::new("p41-hyper");

    // 1. Hyper-dimensional projection
    let res = fixture.run_success(&[
        "memory",
        "hyper-project",
        "--concept",
        "quantum_supremacy",
        "--dim",
        "1024",
        "--json",
    ]);
    assert_eq!(res["concept"], "quantum_supremacy");
    assert_eq!(res["dimension"], 1024);
    assert_eq!(res["theorem_95_jl_fidelity"], true);

    // 2. Holographic Associative Recall
    let res = fixture.run_success(&[
        "memory",
        "holographic-recall",
        "--key",
        "agent_goal",
        "--val",
        "reach_superintelligence",
        "--json",
    ]);
    assert_eq!(res["key"], "agent_goal");
    assert_eq!(res["value"], "reach_superintelligence");
    assert!(res["recall_cosine_similarity"].as_f64().unwrap() > 0.0);
    assert_eq!(res["audit"]["theorem_96_associative_recall_verified"], true);
    assert_eq!(
        res["audit"]["theorem_97_orthogonal_stability_verified"],
        true
    );
}

#[test]
fn test_phase42_formal_verification_cli() {
    let fixture = TestFixture::new("p42-formal");

    // 1. Lean 4 formal export and check
    let res = fixture.run_success(&["audit", "formal-verify", "--dialect", "lean4", "--json"]);
    assert_eq!(res["dialect"], "lean4");
    assert_eq!(res["all_proofs_valid"], true);
    assert_eq!(res["total_centennial_theorems"], 145);
    assert_eq!(res["theorem_98_soundness"], true);
    assert_eq!(res["theorem_99_decidable"], true);
    assert_eq!(res["theorem_100_centennial_complete"], true);

    // 2. Coq formal export and check
    let res = fixture.run_success(&["audit", "formal-verify", "--dialect", "coq", "--json"]);
    assert_eq!(res["dialect"], "coq");
    assert_eq!(res["all_proofs_valid"], true);
    assert_eq!(res["total_centennial_theorems"], 145);
}

#[test]
fn test_cockpit_web_phase40_41_42_rest_endpoints() {
    use pandora_runtime::cockpit::{CockpitController, CockpitLayout};
    use pandora_runtime::cockpit_web::CockpitWebServer;
    use std::sync::{Arc, Mutex};

    let controller = Arc::new(Mutex::new(CockpitController::new(
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

    // 1. Swarm Mesh endpoint
    let (code, mesh) = query_http(port, "/api/swarm/mesh");
    assert_eq!(code, 200);
    assert_eq!(mesh["theorem_92_isolated"], true);
    assert_eq!(mesh["theorem_93_bft_atomic"], true);
    assert_eq!(mesh["theorem_94_hop_bound_met"], true);

    // 2. Memory Hyper endpoint
    let (code, hyper) = query_http(port, "/api/memory/hyper");
    assert_eq!(code, 200);
    assert_eq!(hyper["theorem_95_jl_fidelity_preserved"], true);
    assert_eq!(hyper["theorem_96_associative_recall_verified"], true);
    assert_eq!(hyper["theorem_97_orthogonal_stability_verified"], true);

    // 3. Audit Formal endpoint
    let (code, formal) = query_http(port, "/api/audit/formal");
    assert_eq!(code, 200);
    assert_eq!(formal["all_proofs_valid"], true);
    assert_eq!(formal["total_centennial_theorems"], 145);
    assert_eq!(formal["theorem_100_centennial_complete"], true);

    // 4. Olympus Centennial Overview
    let (code, overview) = query_http(port, "/api/olympus/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["total_theorems_enforced"], 145);
    assert_eq!(overview["architecture_version"], "4.0.10");
    assert_eq!(overview["codename"], "THANATOS");
    assert_eq!(overview["formal_verification"]["all_proofs_valid"], true);

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
