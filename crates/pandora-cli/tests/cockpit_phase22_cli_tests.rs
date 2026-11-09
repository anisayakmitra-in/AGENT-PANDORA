//! Phase 22 Integration Tests: Interactive Cockpit TUI, Swarm Consensus & Pexer Multiplexer IPC
//!
//! # Safety Invariant
//! Strictly `#![forbid(unsafe_code)]` enforced.

#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(900);

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
            "pandora-phase22-test-{}-{timestamp}-{sequence}",
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_cockpit_status_9_and_12_panels() {
    let fixture = Fixture::new();

    let json9 = fixture.run_success(&["cockpit", "status", "--panels", "9", "--json"]);
    assert_eq!(json9["layout"], "nine_panel");
    assert_eq!(json9["panels"].as_array().unwrap().len(), 9);

    let json12 = fixture.run_success(&["cockpit", "status", "--panels", "12", "--json"]);
    assert_eq!(json12["layout"], "twelve_panel");
    assert_eq!(json12["panels"].as_array().unwrap().len(), 12);
}

#[test]
fn test_cockpit_tui_headless_snapshot() {
    let fixture = Fixture::new();

    // Since test execution runs in headless/pipe mode (not interactive TTY),
    // cockpit tui automatically falls back to headless snapshot mode
    let json = fixture.run_success(&["cockpit", "tui", "--panels", "9", "--json"]);
    assert_eq!(json["status"], "headless_snapshot");
    assert_eq!(json["layout"], "NinePanel");
    assert_eq!(json["panel_count"], 9);

    let text = fixture.run_stdout_text(&["cockpit", "tui", "--panels", "9"]);
    assert!(text.contains("MISSION-CONTROL COCKPIT"));
}

#[test]
fn test_cockpit_consensus_byzantine_resolution() {
    let fixture = Fixture::new();

    let json = fixture.run_success(&[
        "cockpit",
        "consensus",
        "--threshold",
        "0.60",
        "--divergence",
        "0.35",
        "--json",
    ]);

    assert_eq!(json["total_proposals"], 4);
    assert_eq!(json["honest_quorum_size"], 3);
    assert_eq!(json["byzantine_filtered_count"], 1);
    assert!(json["winning_proposal_id"].is_string());
    assert!(json["consensus_hash"].is_string());

    let text = fixture.run_stdout_text(&["cockpit", "consensus"]);
    assert!(text.contains("Theorem 43"));
    assert!(text.contains("Byzantine filtered"));
}

#[test]
fn test_cockpit_pexer_launch_dry_run() {
    let fixture = Fixture::new();

    let json = fixture.run_success(&["cockpit", "launch", "--dry-run", "--panels", "9", "--json"]);
    assert_eq!(json["status"], "launched");
    assert_eq!(json["dry_run"], true);
    assert_eq!(json["panel_count"], 9);

    let text = fixture.run_stdout_text(&["cockpit", "launch", "--dry-run", "--panels", "12"]);
    assert!(text.contains("Dry-run verified"));
}

#[test]
fn test_cockpit_send_command_to_agent() {
    let fixture = Fixture::new();

    let json = fixture.run_success(&[
        "cockpit",
        "send",
        "--pane-id",
        "1",
        "--keys",
        "cargo test",
        "--json",
    ]);

    assert_eq!(json["status"], "keys_dispatched");
    assert_eq!(json["pane_id"], "1");
    assert_eq!(json["keys"], "cargo test");
    assert!(json["ipc_command"].is_string());
}

#[test]
fn test_slash_consensus_direct_execution() {
    let fixture = Fixture::new();

    let json = fixture.run_success(&["/consensus", "--threshold", "0.50", "--json"]);
    assert_eq!(json["total_proposals"], 4);
    assert_eq!(json["honest_quorum_size"], 3);
    assert!(json["winning_proposal_id"].is_string());
}
