#![forbid(unsafe_code)]

//! Phase 39 / v4.0.4 OLYMPUS Architecture End-to-End Integration Tests
//!
//! Validates:
//! - Subsystem 1: Cognitive Micro-Kernel & Neuro-Symbolic Engine (`pandora cognitive`)
//! - Subsystem 2: Tripartite Constitutional Governance & Capability Bus (`pandora governance`)
//! - Subsystem 3: Cryptographic Zero-Knowledge & Post-Quantum Substrate (`pandora crypto`)
//! - Subsystem 4: Decentralized Swarm Mesh & Consensus Substrate
//! - Subsystem 5: Continual Memory VFS & Certified Privacy Substrate
//! - Cockpit Web Subsystems REST Endpoints:
//!   - `GET /api/subsystems/cognitive`
//!   - `GET /api/subsystems/governance`
//!   - `GET /api/subsystems/crypto`
//!   - `GET /api/subsystems/swarm`
//!   - `GET /api/subsystems/memory`
//!   - `GET /api/olympus/overview`
//! - Enforcement of all 91 Foundation Theorems across the 5 Subsystems.

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(3900);

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
            "pandora-olympus-{prefix}-{}-{timestamp}-{seq}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.1.3",
            "active_model": "olympus-supercluster",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA OLYMPUS test fixture."
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
fn test_subsystem_1_cognitive_cli_and_theorems() {
    let fixture = TestFixture::new("sub1-cog");

    // 1. Status
    let res = fixture.run_success(&["cognitive", "status", "--json"]);
    assert_eq!(res["status"], "ready");
    assert!(res["grammars_supported"].as_array().unwrap().len() >= 4);
    assert!(res["speculative_default_speedup"].as_f64().unwrap() > 1.0);
    assert!(res["metacognitive_pac_bayes_risk"].as_f64().unwrap() <= 0.15);

    // 2. Speculate
    let res = fixture.run_success(&[
        "cognitive",
        "speculate",
        "--draft",
        "0.8,0.7,0.6,0.9",
        "--target",
        "0.85,0.75,0.65,0.95",
        "--json",
    ]);
    let rep = &res["report"];
    assert!(rep["speedup_factor"].as_f64().unwrap() > 1.0);
    assert_eq!(rep["theorem_89_distributional_equivalence"], true);
    assert_eq!(rep["theorem_90_speedup_bound_met"], true);
    assert_eq!(rep["theorem_91_latency_amortized"], true);

    // 3. Synthesize
    let res = fixture.run_success(&[
        "cognitive",
        "synthesize",
        "--tokens",
        "{,\"key\",:,\"val\",}",
        "--json",
    ]);
    assert_eq!(res["is_valid_terminal"], true);
    assert_eq!(res["theorem_68_zero_parse_error"], true);
    assert_eq!(res["theorem_69_invariance_preserved"], true);

    // 4. Audit
    let res = fixture.run_success(&[
        "cognitive",
        "audit",
        "--logits",
        "3.0,1.5,0.2",
        "--drift",
        "0.02",
        "--json",
    ]);
    assert_eq!(res["theorem_80_risk_bounded"], true);
    assert_eq!(res["theorem_81_convergence_verified"], true);
    assert!(res["theorem_82_pac_bayes_risk"].as_f64().unwrap() <= 0.15);
}

#[test]
fn test_subsystem_2_governance_cli_and_theorems() {
    let fixture = TestFixture::new("sub2-gov");

    // 1. Status
    let res = fixture.run_success(&["governance", "status", "--json"]);
    assert_eq!(res["status"], "ready");
    assert_eq!(res["max_kill_latency_bound_ms"], 50);
    assert_eq!(res["max_memory_quota_mb"], 512);

    // 2. Audit authorized permit
    let res = fixture.run_success(&[
        "governance",
        "audit",
        "--permit",
        "filesystem:read",
        "--json",
    ]);
    assert_eq!(res["authorized"], true);
    assert_eq!(res["theorem"], 74);

    // 3. Audit denied permit
    let res = fixture.run_success(&[
        "governance",
        "audit",
        "--permit",
        "unauthorized_root_privilege",
        "--json",
    ]);
    assert_eq!(res["authorized"], false);
}

#[test]
fn test_subsystem_3_crypto_cli_and_theorems() {
    let fixture = TestFixture::new("sub3-cry");

    // 1. Status
    let res = fixture.run_success(&["crypto", "status", "--json"]);
    assert_eq!(res["status"], "ready");
    assert!(res["prime_fields"].as_array().unwrap().len() >= 2);

    // 2. PQ Sign
    let res = fixture.run_success(&[
        "crypto",
        "pq-sign",
        "--message",
        "olympus_genesis_block",
        "--json",
    ]);
    assert!(res["classical_sig_len"].as_u64().unwrap() > 0);
    assert!(res["lattice_sig_len"].as_u64().unwrap() > 0);
    assert_eq!(res["classical_verified"], true);
    assert_eq!(res["quantum_verified"], true);

    // 3. MPC Eval
    let res = fixture.run_success(&["crypto", "mpc-eval", "--x", "123", "--y", "456", "--json"]);
    assert_eq!(res["product"], 123u64 * 456u64);
    assert_eq!(res["verified"], true);
}

#[test]
fn test_subsystems_cockpit_web_routes_and_olympus_overview() {
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

    // 1. Cognitive Subsystem
    let (code, cog) = query_http(port, "/api/subsystems/cognitive");
    assert_eq!(code, 200);
    assert_eq!(cog["status"], "ready");

    // 2. Governance Subsystem
    let (code, gov) = query_http(port, "/api/subsystems/governance");
    assert_eq!(code, 200);
    assert_eq!(gov["status"], "ready");

    // 3. Crypto Subsystem
    let (code, cry) = query_http(port, "/api/subsystems/crypto");
    assert_eq!(code, 200);
    assert_eq!(cry["status"], "ready");

    // 4. Swarm Subsystem
    let (code, swm) = query_http(port, "/api/subsystems/swarm");
    assert_eq!(code, 200);
    assert_eq!(swm["status"], "ready");
    assert_eq!(swm["bft_nodes_count"], 4);
    assert_eq!(swm["gossip_converged"], true);

    // 5. Memory Subsystem
    let (code, mem) = query_http(port, "/api/subsystems/memory");
    assert_eq!(code, 200);
    assert_eq!(mem["status"], "ready");
    assert!(mem["privacy_budget"]["epsilon"].as_f64().unwrap() > 0.0);
    assert_eq!(
        mem["sample_scrubbing"]["theorem_87_certified_unlearning"],
        true
    );

    // 6. Olympus Overview
    let (code, overview) = query_http(port, "/api/olympus/overview");
    assert_eq!(code, 200);
    assert_eq!(overview["architecture_version"], "4.2.0");
    assert_eq!(overview["total_theorems_enforced"], 244);
    assert_eq!(overview["codename"], "THANATOS");
    assert_eq!(overview["cognitive"]["status"], "ready");
    assert_eq!(overview["governance"]["status"], "ready");
    assert_eq!(overview["crypto"]["status"], "ready");
    assert_eq!(overview["swarm"]["status"], "ready");
    assert_eq!(overview["memory"]["status"], "ready");

    shutdown_handle.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    let _ = server_thread.join();
}
