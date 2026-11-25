#![forbid(unsafe_code)]

//! Phase 31 & Phase 32 End-to-End Integration Tests
//!
//! Validates:
//! - Phase 31: Grammar-Constrained Token-Level BNF Synthesis & Pushdown Automata (Theorems 68–70)
//! - Phase 32: Decentralized Cross-Cluster Swarm CRDT & Epidemic Gossip Synchronization (Theorems 71–73)
//! - Cockpit Web REST Endpoints: `/api/swarm/crdt` and `/api/code/synthesize/grammar`

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(3100);

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
        let config = root.join("pandora.json");

        fs::create_dir_all(&workspace).expect("create workspace dir");
        fs::create_dir_all(&data).expect("create data dir");

        let default_config = serde_json::json!({
            "version": "4.0.0",
            "active_model": "test-mock-llm",
            "max_tool_iterations": 10,
            "cost_limit_dollars": 5.0,
            "timeout_seconds": 30,
            "system_prompt": "You are AGENT-PANDORA test fixture."
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

#[test]
fn test_code_synthesize_e2e_cli() {
    let fixture = TestFixture::new("code-synth");

    let val = fixture.run_success(&["code", "synthesize", "--kind", "json", "--json"]);
    assert_eq!(val["command"], "code synthesize");
    assert_eq!(val["grammar"], "json");
    assert!(val["report"]["is_valid_terminal"].as_bool().unwrap());
    assert!(
        val["report"]["theorem_68_zero_parse_error"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["report"]["theorem_69_invariance_preserved"]
            .as_bool()
            .unwrap()
    );
    assert!(
        val["report"]["theorem_70_roofline_compliant"]
            .as_bool()
            .unwrap()
    );

    // SQL synthesis
    let sql_val = fixture.run_success(&["code", "synthesize", "--kind", "sql", "--json"]);
    assert_eq!(sql_val["grammar"], "sql");
    assert!(sql_val["report"]["is_valid_terminal"].as_bool().unwrap());
}

#[test]
fn test_code_grammar_mask_and_validate_e2e_cli() {
    let fixture = TestFixture::new("code-mask");

    let mask_val = fixture.run_success(&["code", "grammar", "mask", "--kind", "json", "--json"]);
    assert_eq!(mask_val["grammar"], "json");
    assert!(
        mask_val["mask"]["admissible_tokens_count"]
            .as_u64()
            .unwrap()
            > 0
    );

    // Validate admissible
    let val = fixture.run_success(&[
        "code", "grammar", "validate", "--kind", "json", "--token", "{", "--json",
    ]);
    assert!(val["admissible"].as_bool().unwrap());

    // Validate inadmissible
    let inv_val = fixture.run_success(&[
        "code", "grammar", "validate", "--kind", "json", "--token", ":", "--json",
    ]);
    assert!(!inv_val["admissible"].as_bool().unwrap());
}

#[test]
fn test_swarm_crdt_status_and_merge_e2e_cli() {
    let fixture = TestFixture::new("swarm-crdt");

    let status_val = fixture.run_success(&["swarm", "crdt", "status", "--json"]);
    assert_eq!(status_val["crdt_pn_counter_value"], 12);
    assert!(status_val["theorem_71_sec"].as_bool().unwrap());

    let merge_val = fixture.run_success(&["swarm", "crdt", "merge", "--json"]);
    assert_eq!(merge_val["merged_nodes"], 2);
    assert!(merge_val["theorem_71_sec_verified"].as_bool().unwrap());
    assert!(merge_val["theorem_73_causality"].as_bool().unwrap());
}

#[test]
fn test_swarm_gossip_simulate_e2e_cli() {
    let fixture = TestFixture::new("swarm-gossip");

    let sim_val = fixture.run_success(&[
        "swarm", "gossip", "simulate", "--nodes", "8", "--fanout", "2", "--json",
    ]);
    assert_eq!(sim_val["nodes"], 8);
    assert!(sim_val["converged"].as_bool().unwrap());
    assert!(sim_val["theorem_72_dissemination_bound"].as_bool().unwrap());
}

#[test]
fn test_cockpit_web_crdt_and_grammar_endpoints() {
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

    // Test GET /api/swarm/crdt
    {
        let mut stream =
            TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect to crdt endpoint");
        stream
            .write_all(
                b"GET /api/swarm/crdt HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"theorem_71_sec\":true"));
        assert!(response.contains("\"theorem_72_dissemination\":true"));
    }

    // Test GET /api/code/synthesize/grammar
    {
        let mut stream =
            TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect to grammar endpoint");
        stream.write_all(b"GET /api/code/synthesize/grammar HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"supported_grammars\""));
        assert!(response.contains("\"theorems\":[68,69,70]"));
    }

    shutdown_flag.store(true, Ordering::Relaxed);
    let _ = handle.join();
}
