#![forbid(unsafe_code)]

//! Phase 33, Phase 34, & Phase 35 End-to-End Integration Tests
//!
//! Validates:
//! - Phase 33: Autonomous Untrusted Sandbox Isolation & Resource Roofline Hardening (Theorems 74–76)
//! - Phase 34: Post-Quantum Hybrid Attestation & Algebraic Sponge State Commitments (Theorems 77–79)
//! - Phase 35: Continuous Introspective Metacognition & Hallucination Entropy Gating (Theorems 80–82)
//! - Cockpit Web REST Endpoints:
//!   - `GET /api/sandbox/telemetry`
//!   - `GET /api/moira/pq`
//!   - `GET /api/agent/introspect`

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(3300);

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
            "version": "4.0.1",
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
fn test_sandbox_status_and_audit_e2e_cli() {
    let fixture = TestFixture::new("sandbox-status");

    let status_val = fixture.run_success(&["sandbox", "status", "--json"]);
    assert_eq!(status_val["command"], "sandbox status");
    assert!(status_val["platform"].is_string());
    assert_eq!(status_val["policy"]["max_memory_mb"], 512);
    assert_eq!(status_val["theorems"], serde_json::json!([74, 75, 76]));

    let audit_val = fixture.run_success(&["sandbox", "audit", "--json"]);
    assert_eq!(audit_val["command"], "sandbox audit");
    assert_eq!(audit_val["all_theorems_verified"], true);
    assert_eq!(audit_val["theorems"], serde_json::json!([74, 75, 76]));
}

#[test]
fn test_sandbox_execute_e2e_cli() {
    let fixture = TestFixture::new("sandbox-exec");

    let exec_val = fixture.run_success(&[
        "sandbox",
        "execute",
        "--workload",
        "untrusted_binary_audit",
        "--memory",
        "128",
        "--timeout",
        "250",
        "--json",
    ]);

    assert_eq!(exec_val["command"], "sandbox execute");
    assert_eq!(exec_val["report"]["workload"], "untrusted_binary_audit");
    assert!(
        exec_val["report"]["theorem_74_non_interference"]
            .as_bool()
            .unwrap()
    );
    assert!(
        exec_val["report"]["theorem_75_kill_bound_met"]
            .as_bool()
            .unwrap()
    );
    assert!(
        exec_val["report"]["theorem_76_roofline_enforced"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn test_moira_pq_sign_and_verify_e2e_cli() {
    let fixture = TestFixture::new("moira-pq");

    let sign_val = fixture.run_success(&[
        "moira",
        "pq-sign",
        "--payload",
        "e2e_checkpoint_v4",
        "--json",
    ]);

    assert_eq!(sign_val["command"], "moira pq-sign");
    assert!(sign_val["receipt"]["sponge_digest"].is_string());
    assert!(sign_val["receipt"]["classical_sig_hex"].is_string());
    assert!(sign_val["receipt"]["lattice_sig_hex"].is_string());
    assert_eq!(sign_val["theorems"], serde_json::json!([77, 78, 79]));

    let verify_val = fixture.run_success(&[
        "moira",
        "pq-verify",
        "--payload",
        "e2e_checkpoint_v4",
        "--json",
    ]);

    assert_eq!(verify_val["command"], "moira pq-verify");
    assert!(verify_val["valid"].as_bool().unwrap());
    assert!(
        verify_val["receipt"]["classical_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(verify_val["receipt"]["quantum_verified"].as_bool().unwrap());
}

#[test]
fn test_agent_introspect_and_reflect_e2e_cli() {
    let fixture = TestFixture::new("agent-introspect");

    let intro_val = fixture.run_success(&["agent", "introspect", "--steps", "6", "--json"]);
    assert_eq!(intro_val["command"], "agent introspect");
    assert_eq!(intro_val["report"]["steps_audited"], 6);
    assert!(
        intro_val["report"]["theorem_80_risk_bounded"]
            .as_bool()
            .unwrap()
    );
    assert!(
        intro_val["report"]["theorem_81_convergence_verified"]
            .as_bool()
            .unwrap()
    );
    assert!(
        intro_val["report"]["theorem_82_pac_bayes_risk"]
            .as_f64()
            .unwrap()
            <= 0.15
    );

    let reflect_val = fixture.run_success(&["agent", "reflect", "--json"]);
    assert_eq!(reflect_val["command"], "agent reflect");
    assert!(reflect_val["converged"].as_bool().unwrap());
    assert!(reflect_val["theorem_81_convergence"].as_bool().unwrap());
    assert_eq!(reflect_val["theorems"], serde_json::json!([81, 82]));
}

#[test]
fn test_cockpit_web_phase33_34_35_endpoints() {
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

    // Test GET /api/sandbox/telemetry
    {
        let mut stream =
            TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect to sandbox endpoint");
        stream
            .write_all(
                b"GET /api/sandbox/telemetry HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"theorem_74_non_interference\":true"));
        assert!(response.contains("\"theorem_75_deterministic_kill\":true"));
        assert!(response.contains("\"theorem_76_roofline\":true"));
    }

    // Test GET /api/moira/pq
    {
        let mut stream =
            TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect to moira pq endpoint");
        stream
            .write_all(
                b"GET /api/moira/pq HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"theorem_77_euf_cma\":true"));
        assert!(response.contains("\"theorem_78_hybrid_security\":true"));
        assert!(response.contains("\"theorem_79_sponge_collision_resistance\":true"));
    }

    // Test GET /api/agent/introspect
    {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .expect("connect to agent introspect endpoint");
        stream
            .write_all(
                b"GET /api/agent/introspect HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"theorem_80_entropy_bounded\":true"));
        assert!(response.contains("\"theorem_81_monotonic_convergence\":true"));
        assert!(response.contains("\"theorem_82_pac_bayes_risk\""));
    }

    shutdown_flag.store(true, Ordering::Relaxed);
    let _ = handle.join();
}
