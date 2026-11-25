#![forbid(unsafe_code)]

//! Phase 36, Phase 37, & Phase 38 End-to-End Integration Tests
//!
//! Validates:
//! - Phase 36: Homomorphic Multi-Party Computation & Zero-Leakage Secret Sharing (Theorems 83–85)
//! - Phase 37: Differential Privacy & Continual Certified Unlearning (Theorems 86–88)
//! - Phase 38: Distributed Speculative Multi-Draft Verification & Tree Decoding (Theorems 89–91)
//! - Cockpit Web REST Endpoints:
//!   - `GET /api/swarm/mpc`
//!   - `GET /api/memory/privacy`
//!   - `GET /api/code/speculative`

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(3600);

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
            "version": "4.0.2",
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
fn test_swarm_secret_share_and_mpc_eval_e2e_cli() {
    let fixture = TestFixture::new("swarm-mpc");

    let share_val = fixture.run_success(&[
        "swarm",
        "secret-share",
        "--secret",
        "9999",
        "--threshold",
        "3",
        "--shares",
        "5",
        "--json",
    ]);

    assert_eq!(share_val["command"], "swarm secret-share");
    assert_eq!(share_val["recovered_secret"], 9999);
    assert_eq!(share_val["threshold"], 3);
    assert_eq!(share_val["shares_count"], 5);
    assert!(share_val["theorem_83_privacy"].as_bool().unwrap());
    assert!(share_val["theorem_84_bft"].as_bool().unwrap());
    assert_eq!(share_val["theorems"], serde_json::json!([83, 84]));

    let eval_val =
        fixture.run_success(&["swarm", "mpc-eval", "--x", "111", "--y", "222", "--json"]);

    assert_eq!(eval_val["command"], "swarm mpc-eval");
    assert_eq!(eval_val["result"], eval_val["expected"]);
    assert!(eval_val["theorem_85_constant_round"].as_bool().unwrap());
    assert_eq!(eval_val["theorems"], serde_json::json!([85]));
}

#[test]
fn test_memory_scrub_e2e_cli() {
    let fixture = TestFixture::new("memory-scrub");

    let scrub_val = fixture.run_success(&[
        "memory",
        "scrub",
        "--target",
        "sensitive_trace_99",
        "--samples",
        "4",
        "--epsilon",
        "1.5",
        "--json",
    ]);

    assert_eq!(scrub_val["command"], "memory scrub");
    assert_eq!(scrub_val["target"], "sensitive_trace_99");
    assert_eq!(scrub_val["samples"], 4);
    assert!(
        scrub_val["receipt"]["theorem_86_rdp_composed"]
            .as_bool()
            .unwrap()
    );
    assert!(
        scrub_val["receipt"]["theorem_87_certified_unlearning"]
            .as_bool()
            .unwrap()
    );
    assert!(
        scrub_val["receipt"]["theorem_88_retention_guaranteed"]
            .as_bool()
            .unwrap()
    );
    assert!(
        scrub_val["receipt"]["memory_retention_fidelity"]
            .as_f64()
            .unwrap()
            >= 0.85
    );
    assert_eq!(scrub_val["theorems"], serde_json::json!([86, 87, 88]));
}

#[test]
fn test_code_speculate_e2e_cli() {
    let fixture = TestFixture::new("code-speculate");

    let spec_val = fixture.run_success(&[
        "code",
        "speculate",
        "--prompt",
        "optimize_kernel",
        "--depth",
        "4",
        "--branching",
        "2",
        "--json",
    ]);

    assert_eq!(spec_val["command"], "code speculate");
    assert_eq!(spec_val["prompt"], "optimize_kernel");
    assert!(spec_val["report"]["tokens_accepted"].as_u64().unwrap() >= 4);
    assert!(spec_val["report"]["speedup_factor"].as_f64().unwrap() >= 2.0);
    assert!(
        spec_val["report"]["theorem_89_distributional_equivalence"]
            .as_bool()
            .unwrap()
    );
    assert!(
        spec_val["report"]["theorem_90_speedup_bound_met"]
            .as_bool()
            .unwrap()
    );
    assert!(
        spec_val["report"]["theorem_91_latency_amortized"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(spec_val["theorems"], serde_json::json!([89, 90, 91]));
}

#[test]
fn test_cockpit_web_phase36_37_38_endpoints() {
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

    // Test GET /api/swarm/mpc
    {
        let mut stream =
            TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect to swarm mpc endpoint");
        stream
            .write_all(
                b"GET /api/swarm/mpc HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"theorem_83_information_theoretic_privacy\":true"));
        assert!(response.contains("\"theorem_84_byzantine_fault_tolerance\":true"));
        assert!(response.contains("\"theorem_85_constant_round_multiplication\":true"));
    }

    // Test GET /api/memory/privacy
    {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .expect("connect to memory privacy endpoint");
        stream
            .write_all(
                b"GET /api/memory/privacy HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"theorem_86_rdp_composed\":true"));
        assert!(response.contains("\"theorem_87_certified_unlearning\":true"));
        assert!(response.contains("\"theorem_88_retention_guaranteed\":true"));
    }

    // Test GET /api/code/speculative
    {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .expect("connect to code speculative endpoint");
        stream
            .write_all(
                b"GET /api/code/speculative HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"theorem_89_distributional_equivalence\":true"));
        assert!(response.contains("\"theorem_90_speedup_bound_met\":true"));
        assert!(response.contains("\"theorem_91_latency_amortized\":true"));
    }

    shutdown_flag.store(true, Ordering::Relaxed);
    let _ = handle.join();
}
