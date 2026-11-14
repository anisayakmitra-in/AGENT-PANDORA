//! Phase 25 CLI Adversarial Integration Tests: Dual-Gating & Canary Promotion.
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

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(8800);

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
            "pandora-cli-adv-canary-{}-{timestamp}-{sequence}",
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
            "command failed: {:?}\nstderr: {}\nstdout: {}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
        serde_json::from_str(&stdout).expect("stdout should be valid JSON")
    }

    fn run_failure(&self, args: &[&str]) -> (i32, String) {
        let output = self.command(args).output().expect("command should execute");
        assert!(
            !output.status.success(),
            "command should have failed: {:?}\nstdout: {}",
            args,
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let combined = format!("{stderr}\n{stdout}");
        (output.status.code().unwrap_or(-1), combined)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_cli_adv_rejection_node_deficiency_n_le_3f() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "adv_byz_n_def",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id");

    // Unanimous 3 of 3 approvals on f=1 cluster (N <= 3f violation)
    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--approvals",
        "3",
        "--byz-f",
        "1",
        "--total-nodes",
        "3",
        "--json",
    ]);
    assert_ne!(code, 0);
    assert!(
        output.contains("Byzantine consensus quorum not met"),
        "must reject N <= 3f; got: {output}"
    );
}

#[test]
fn test_cli_adv_rejection_zero_approvals() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "adv_byz_zero",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id");

    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--approvals",
        "0",
        "--byz-f",
        "1",
        "--total-nodes",
        "4",
        "--json",
    ]);
    assert_ne!(code, 0);
    assert!(output.contains("Byzantine consensus quorum not met"));
}

#[test]
fn test_cli_adv_rejection_sample_size_nine() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "adv_sample_nine",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id");

    // 9 passes, 0 failures (100% pass rate, but n=9 < 10)
    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--passes",
        "9",
        "--failures",
        "0",
        "--min-samples",
        "10",
        "--approvals",
        "4",
        "--byz-f",
        "1",
        "--total-nodes",
        "4",
        "--json",
    ]);
    assert_ne!(code, 0);
    assert!(output.contains("Insufficient evaluation sample size: 9 < minimum 10"));
}

#[test]
fn test_cli_adv_rejection_baseline_one() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "adv_baseline_one",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id");

    // Impossible baseline 1.0
    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--passes",
        "100",
        "--failures",
        "0",
        "--baseline",
        "1.0",
        "--approvals",
        "4",
        "--byz-f",
        "1",
        "--total-nodes",
        "4",
        "--json",
    ]);
    assert_ne!(code, 0);
    assert!(output.contains("Bayesian LCB95 gate failed"));
}

#[test]
fn test_cli_adv_rejection_nonexistent_id() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        "canary_does_not_exist_xyz",
        "--json",
    ]);
    assert_ne!(code, 0);
    assert!(output.contains("Canary worktree 'canary_does_not_exist_xyz' not found"));
}

#[test]
fn test_cli_adv_rejection_dirty_git_worktree() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    // Initialize git repository in fixture.workspace
    let run_git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&fixture.workspace)
            .status()
            .expect("git failed");
        assert!(status.success());
    };

    run_git(&["init"]);
    run_git(&["config", "user.name", "CLI Tester"]);
    run_git(&["config", "user.email", "tester@example.com"]);
    fs::write(fixture.workspace.join("base.txt"), "base commit content\n").unwrap();
    run_git(&["add", "base.txt"]);
    run_git(&["commit", "-m", "init base"]);
    run_git(&["branch", "-M", "main"]);

    // Stage canary
    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "cli_dirty_task",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id");

    // Inject untracked dirty file into workspace
    fs::write(
        fixture.workspace.join("dirty_untracked.rs"),
        "// dirty content",
    )
    .unwrap();

    // Attempt auto-merge promotion
    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--auto-merge",
        "--target",
        "main",
        "--approvals",
        "4",
        "--byz-f",
        "1",
        "--total-nodes",
        "4",
        "--passes",
        "50",
        "--failures",
        "0",
        "--baseline",
        "0.85",
        "--json",
    ]);

    assert_ne!(code, 0);
    assert!(
        output.contains("Cannot promote: repository worktree is dirty"),
        "Must reject dirty git worktree; got: {output}"
    );
}
