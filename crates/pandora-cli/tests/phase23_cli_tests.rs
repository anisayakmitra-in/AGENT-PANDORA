//! Phase 23 Integration Tests: Federated Policy & Cockpit Reputation CLI
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced workspace-wide.

#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(950);

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be available")
            .as_nanos();
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory should have a canonical path");
        let root = temp_root.join(format!(
            "pandora-phase23-test-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::create_dir_all(&data).expect("data dir should be created");
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
        let output = self.command(args).output().expect("command should execute");
        assert!(
            output.status.success(),
            "command '{:?}' failed with stderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
    }

    fn run_stdout_text(&self, args: &[&str]) -> String {
        let output = self.command(args).output().expect("command should execute");
        assert!(
            output.status.success(),
            "command '{:?}' failed with stderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn run_failure(&self, args: &[&str]) -> String {
        let output = self.command(args).output().expect("command should execute");
        assert!(
            !output.status.success(),
            "command '{:?}' should have failed",
            args
        );
        String::from_utf8_lossy(&output.stderr).to_string()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_runtime_federated_default_execution() {
    let fixture = Fixture::new();

    // Human-readable output check
    let text = fixture.run_stdout_text(&["runtime", "federated"]);
    assert!(text.contains("Phase 23 Federated Reinforcement Learning"));
    assert!(text.contains("Policy Parameters (6 Dimensions)"));

    // JSON structured output check
    let val = fixture.run_success(&["runtime", "federated", "--json"]);

    assert_eq!(val["command"], "runtime federated");
    let data = &val;
    assert_eq!(data["status"], "ok");
    assert_eq!(data["steps"], 5);
    assert_eq!(data["total_agents"], 9);
    assert_eq!(data["honest_count"], 7);
    assert_eq!(data["byzantine_count"], 2);
    assert!(data["theorem_45_convergence_verified"].as_bool().unwrap());
    assert!(data["theorem_46_quarantine_verified"].as_bool().unwrap());

    let policy = data["final_policy"].as_array().expect("policy array");
    assert_eq!(policy.len(), 6);
    for p in policy {
        let val_f64 = p.as_f64().expect("float policy value");
        assert!((0.0..=1.0).contains(&val_f64));
    }
}

#[test]
fn test_runtime_federated_custom_parameters_and_quarantine() {
    let fixture = Fixture::new();
    let val = fixture.run_success(&[
        "runtime",
        "federated",
        "--agents",
        "7",
        "--byzantine",
        "2",
        "--steps",
        "6",
        "--learning-rate",
        "0.08",
        "--momentum",
        "0.85",
        "--max-norm",
        "1.5",
        "--seed",
        "12345",
        "--json",
    ]);

    let data = &val;
    assert_eq!(data["steps"], 6);
    assert_eq!(data["total_agents"], 7);
    assert_eq!(data["honest_count"], 5);
    assert_eq!(data["byzantine_count"], 2);
    assert!((data["learning_rate"].as_f64().unwrap() - 0.08).abs() < 1e-4);
    assert!((data["momentum"].as_f64().unwrap() - 0.85).abs() < 1e-4);

    let quarantined = data["quarantined_agents"].as_array().expect("quarantined array");
    // After 6 steps of Byzantine updates, adversarial agents must be quarantined under Theorem 46
    assert!(!quarantined.is_empty(), "adversarial agents must be quarantined after 6 steps");
}

#[test]
fn test_runtime_federated_invalid_parameters() {
    let fixture = Fixture::new();

    // 1. Byzantine >= agents / 2
    let err = fixture.run_failure(&[
        "runtime",
        "federated",
        "--agents",
        "4",
        "--byzantine",
        "2",
    ]);
    assert!(err.contains("trimmed mean breakdown resilience"));

    // 2. Agents outside bounds
    let err_agents = fixture.run_failure(&[
        "runtime",
        "federated",
        "--agents",
        "2",
    ]);
    assert!(err_agents.contains("--agents must be between 3 and 9"));

    // 3. Zero steps
    let err_steps = fixture.run_failure(&[
        "runtime",
        "federated",
        "--steps",
        "0",
    ]);
    assert!(err_steps.contains("--steps must be at least 1"));
}

#[test]
fn test_cockpit_reputation_table_and_mutations() {
    let fixture = Fixture::new();

    // 0. Human-readable table output
    let text = fixture.run_stdout_text(&["cockpit", "reputation"]);
    assert!(text.contains("Multi-Agent Dynamic Reputation & Quarantine State"));
    assert!(text.contains("Total Agents Tracked: 9"));

    // 1. Default reputation inspection
    let val = fixture.run_success(&["cockpit", "reputation", "--json"]);
    assert_eq!(val["command"], "cockpit reputation");
    let data = &val;
    assert_eq!(data["status"], "ok");
    assert_eq!(data["total_agents"], 9);

    let agents = data["agents"].as_object().expect("agents map");
    assert_eq!(agents.len(), 9);
    assert!(agents.contains_key("agy"));
    assert!(agents.contains_key("claude"));

    // 2. Single agent filter
    let val_single = fixture.run_success(&["cockpit", "reputation", "--agent", "claude", "--json"]);
    let data_single = &val_single;
    assert_eq!(data_single["total_agents"], 1);

    // 3. Record honest outcome
    let val_honest = fixture.run_success(&[
        "cockpit",
        "reputation",
        "--record-honest",
        "claude",
        "--quality",
        "0.9",
        "--agent",
        "claude",
        "--json",
    ]);
    let claude_rec = &val_honest["agents"]["claude"];
    assert_eq!(claude_rec["total_evaluations"], 1);
    assert!((claude_rec["alpha"].as_f64().unwrap() - 2.9).abs() < 1e-4);

    // 4. Record Byzantine outcome
    let val_byz = fixture.run_success(&[
        "cockpit",
        "reputation",
        "--record-byzantine",
        "devin",
        "--agent",
        "devin",
        "--json",
    ]);
    let devin_rec = &val_byz["agents"]["devin"];
    assert_eq!(devin_rec["total_evaluations"], 1);
    assert!((devin_rec["beta"].as_f64().unwrap() - 3.0).abs() < 1e-4);
}

#[test]
fn test_slash_reputation_routing() {
    let fixture = Fixture::new();
    // Test direct dispatch of /reputation via cockpit slash routing
    let stdout = fixture.run_stdout_text(&["cockpit", "dispatch", "/reputation"]);
    assert!(stdout.contains("Inspecting multi-agent reputation standings"));
}
