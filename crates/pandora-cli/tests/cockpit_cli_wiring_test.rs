//! Integration tests for Cockpit CLI surface and Multi-Agent Slash Command routing.
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

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(800);

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
            "pandora-cockpit-test-{}-{timestamp}-{sequence}",
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_runtime_cockpit_status_json() {
    let fixture = Fixture::new();
    let res = fixture.run_success(&["runtime", "cockpit", "status", "--panels", "12", "--json"]);

    assert_eq!(res["command"], "cockpit status");
    assert_eq!(res["panel_count"], 12);
    assert_eq!(res["healthy_panels"], 12);
    assert!(res["snapshot_digest"].as_str().is_some());
}

#[test]
fn test_runtime_cockpit_status_nine_panels_json() {
    let fixture = Fixture::new();
    let res = fixture.run_success(&["runtime", "cockpit", "status", "--panels", "9", "--json"]);

    assert_eq!(res["command"], "cockpit status");
    assert_eq!(res["panel_count"], 9);
    assert_eq!(res["healthy_panels"], 9);
}

#[test]
fn test_runtime_cockpit_launch_dry_run() {
    let fixture = Fixture::new();
    let res = fixture.run_success(&[
        "runtime",
        "cockpit",
        "launch",
        "--panels",
        "12",
        "--workspace",
        "TEST-WORKSPACE",
        "--dry-run",
        "--json",
    ]);

    assert_eq!(res["command"], "cockpit launch");
    assert_eq!(res["status"], "launched");
    assert_eq!(res["workspace"], "TEST-WORKSPACE");
    assert_eq!(res["panel_count"], 12);
    assert!(res["commands"].as_array().unwrap().len() >= 12);
}

#[test]
fn test_slash_collab_dispatch_via_cli() {
    let fixture = Fixture::new();
    let res = fixture.run_success(&[
        "/collab",
        "agy",
        "claude",
        "codex",
        "--",
        "audit",
        "governance",
        "--json",
    ]);

    assert_eq!(res["command"], "cockpit dispatch");
    assert_eq!(res["status"], "dispatched");
    let agents = res["spawned_agents"].as_array().expect("agents array");
    assert_eq!(agents.len(), 3);
}

#[test]
fn test_slash_single_agent_dispatch() {
    let fixture = Fixture::new();
    let res = fixture.run_success(&["/agy", "check", "system", "integrity", "--json"]);

    assert_eq!(res["command"], "cockpit dispatch");
    assert_eq!(res["status"], "dispatched");
    assert_eq!(res["spawned_agents"].as_array().unwrap().len(), 1);
}

#[test]
fn test_cockpit_panes_listing() {
    let fixture = Fixture::new();
    let res = fixture.run_success(&["runtime", "cockpit", "panes", "--json"]);

    assert_eq!(res["command"], "cockpit panes");
    assert_eq!(res["status"], "ok");
}
