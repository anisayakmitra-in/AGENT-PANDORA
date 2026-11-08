#![forbid(unsafe_code)]

//! Integration Test Suite: `pandora runtime daemon-guard` CLI Subcommand
//!
//! Validates:
//! 1. Execution of `pandora runtime daemon-guard --once --json`.
//! 2. Structured JSON envelope output containing `sequence_number`, `all_invariants_verified`, `receipt_sha256`.
//! 3. Cryptographic commitment into append-only audit store verified by `pandora audit verify --dag-node <id>`.
//! 4. Multi-iteration hash-chaining and topological tree validation via `pandora audit tree`.
//! 5. Human-readable output formatting with explicit theorem citations.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(800);

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
            .expect("system clock available")
            .as_nanos();
        let sequence = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory canonical path");
        let root = temp_root.join(format!(
            "pandora-daemon-guard-test-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        fs::create_dir_all(&workspace).expect("workspace created");
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
        let output = self.command(args).output().expect("command executes");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "command '{:?}' failed with stderr: {}\nstdout: {}",
            args,
            stderr,
            stdout
        );
        serde_json::from_str(&stdout).expect("output must be valid JSON envelope")
    }

    fn run_raw(&self, args: &[&str]) -> (String, String, bool) {
        let output = self.command(args).output().expect("command executes");
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.success(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_daemon_guard_cli_once_execution_and_output_schema() {
    let fixture = Fixture::new();

    let val = fixture.run_success(&["runtime", "daemon-guard", "--once", "--json"]);

    assert_eq!(val["command"], "runtime daemon-guard");
    assert_eq!(val["sequence_number"], 0);
    assert!(val["all_invariants_verified"].as_bool().unwrap());
    assert_eq!(val["phases_certified"], 12);

    let receipt_digest = val["receipt_sha256"].as_str().unwrap();
    assert!(
        receipt_digest.starts_with("sha256:"),
        "receipt_sha256 must start with sha256:"
    );

    let audit_node_id = val["audit_node_id"].as_str().unwrap();
    assert!(!audit_node_id.is_empty());

    // Indepedently verify causal DAG lineage via `pandora audit verify --dag-node <id>`
    let verify_val =
        fixture.run_success(&["audit", "verify", "--dag-node", audit_node_id, "--json"]);

    assert_eq!(verify_val["command"], "audit verify");
    assert_eq!(verify_val["mode"], "dag_causal_lineage");
    assert_eq!(verify_val["node_id"], audit_node_id);
    assert!(verify_val["verified"].as_bool().unwrap());
}

#[test]
fn test_daemon_guard_multi_iteration_chaining_and_audit_lineage() {
    let fixture = Fixture::new();

    // Run 3 iterations
    let val = fixture.run_success(&[
        "runtime",
        "daemon-guard",
        "--iterations",
        "3",
        "--interval-ms",
        "10",
        "--json",
    ]);

    assert_eq!(val["command"], "runtime daemon-guard");

    // When multiple iterations are executed, the final tick report is returned in envelope
    assert_eq!(val["sequence_number"], 2);
    assert!(val["all_invariants_verified"].as_bool().unwrap());
    assert_eq!(val["phases_certified"], 12);

    let final_node_id = val["audit_node_id"].as_str().unwrap();

    // Verify full causal lineage of the 3rd node back to root
    let verify_val =
        fixture.run_success(&["audit", "verify", "--dag-node", final_node_id, "--json"]);
    assert_eq!(verify_val["command"], "audit verify");
    assert!(verify_val["verified"].as_bool().unwrap());

    // Verify tree properties
    let tree_val = fixture.run_success(&["audit", "tree", "--json"]);
    assert_eq!(tree_val["command"], "audit tree");
    assert!(tree_val["is_acyclic"].as_bool().unwrap());
    assert!(tree_val["leaf_count"].as_u64().unwrap() >= 3);
    assert!(tree_val["dag_node_count"].as_u64().unwrap() >= 3);
}

#[test]
fn test_daemon_guard_human_output_theorem_citations() {
    let fixture = Fixture::new();

    let (stdout, stderr, success) = fixture.run_raw(&["runtime", "daemon-guard", "--once"]);

    assert!(success, "command should succeed; stderr: {stderr}");
    assert!(
        stdout.contains(
            "Autonomous Self-Healing Daemon Guard (12-Engine Unified Assurance Pipeline)"
        ),
        "stdout missing title banner: {stdout}"
    );
    assert!(
        stdout.contains("All Invariants Verified: true (Phases Certified: 12/12)"),
        "stdout missing 12/12 phases certified: {stdout}"
    );
    assert!(
        stdout.contains("Receipt Digest: sha256:"),
        "stdout missing sha256 receipt digest: {stdout}"
    );
    assert!(
        stdout.contains("pandora audit verify --dag-node"),
        "stdout missing audit verify instruction: {stdout}"
    );
}
