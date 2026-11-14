//! Phase 25 CLI Integration Tests: Autonomous Canary Auto-Merge & Promotion
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

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(2500);

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
            "pandora-phase25-canary-test-{}-{timestamp}-{sequence}",
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

    fn new_git() -> Self {
        let fixture = Self::new();
        // Initialize real git repository
        let init = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&fixture.workspace)
            .output()
            .expect("git init must succeed");
        assert!(init.status.success());

        for (k, v) in [
            ("user.name", "ANISAYAK MITRA"),
            (
                "user.email",
                "266799942+anisayakmitra-in@users.noreply.github.com",
            ),
        ] {
            let _ = Command::new("git")
                .args(["config", k, v])
                .current_dir(&fixture.workspace)
                .output();
        }

        fs::write(fixture.workspace.join(".gitignore"), ".pandora/\ntarget/\n").unwrap();
        fs::write(
            fixture.workspace.join("README.md"),
            "# AGENT-PANDORA Real Workspace\n",
        )
        .unwrap();

        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(&fixture.workspace)
            .output();
        let commit = Command::new("git")
            .args(["commit", "-m", "initial seed commit"])
            .env("GIT_AUTHOR_NAME", "ANISAYAK MITRA")
            .env(
                "GIT_AUTHOR_EMAIL",
                "266799942+anisayakmitra-in@users.noreply.github.com",
            )
            .env("GIT_COMMITTER_NAME", "ANISAYAK MITRA")
            .env(
                "GIT_COMMITTER_EMAIL",
                "266799942+anisayakmitra-in@users.noreply.github.com",
            )
            .current_dir(&fixture.workspace)
            .output()
            .unwrap();
        assert!(commit.status.success());

        fixture
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

    fn run_plain(&self, args: &[&str]) -> (bool, String) {
        let output = self.command(args).output().expect("command should execute");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.success(), format!("{stdout}\n{stderr}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_cli_canary_promote_missing_id_error() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    let (code, output) = fixture.run_failure(&["canary", "promote", "--dir", ws_str]);
    assert_ne!(code, 0);
    assert!(output.contains("canary promote requires '--id <canary_id>'"));
}

#[test]
fn test_cli_canary_promote_byzantine_rejection() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    // 1. Stage canary
    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "byz_test_task",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id required");

    // 2. Reject promotion when approvals < 2f + 1
    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--approvals",
        "2",
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
fn test_cli_canary_promote_bayesian_rejection() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    // 1. Stage canary
    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "bayes_test_task",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id required");

    // 2. Reject promotion when Bayesian LCB95 <= baseline
    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--passes",
        "10",
        "--failures",
        "10",
        "--baseline",
        "0.90",
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
fn test_cli_canary_promote_insufficient_sample_size() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "sample_size_task",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id required");

    // Total passes + failures = 5 < min 10
    let (code, output) = fixture.run_failure(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--passes",
        "5",
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
    assert!(output.contains("Insufficient evaluation sample size"));
}

#[test]
fn test_cli_canary_promote_auto_merge_success() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    // 1. Stage canary
    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "auto_merge_success_task",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id required");

    // 2. Promote canary with --auto-merge
    let prom_res = fixture.run_success(&[
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
        "100",
        "--failures",
        "0",
        "--baseline",
        "0.85",
        "--json",
    ]);

    assert_eq!(prom_res["status"], "promoted");
    assert_eq!(prom_res["canary_id"], canary_id);
    assert_eq!(prom_res["target_branch"], "main");
    assert_eq!(prom_res["auto_merged"], true);
    assert_eq!(prom_res["byzantine_quorum_met"], true);
    assert_eq!(prom_res["byzantine_approvals"], 4);
    assert!(prom_res["bayesian_lcb95"].as_f64().unwrap() > 0.85);
    assert_eq!(
        prom_res["author"],
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );
    assert_eq!(
        prom_res["tag_name"],
        format!("pandora/canary/promoted/{canary_id}")
    );
    assert!(!prom_res["signature_hex"].as_str().unwrap().is_empty());
}

#[test]
fn test_cli_canary_promote_plain_text_and_author_invariant() {
    let fixture = Fixture::new();
    let ws_str = fixture.workspace.to_str().unwrap();

    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "plain_text_task",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id required");

    let (ok, output) = fixture.run_plain(&[
        "canary",
        "promote",
        "--dir",
        ws_str,
        "--id",
        canary_id,
        "--target",
        "release",
        "--approvals",
        "4",
        "--byz-f",
        "1",
        "--total-nodes",
        "4",
        "--passes",
        "75",
        "--failures",
        "0",
        "--baseline",
        "0.80",
    ]);

    assert!(ok, "promote command should succeed; output: {output}");
    assert!(output.contains("promoted successfully!"));
    assert!(output.contains("Target Branch:       release"));
    assert!(output.contains("Byzantine Quorum:    PASS"));
    assert!(output.contains("Bayesian LCB95:      PASS"));
    assert!(output.contains(
        "Author Identity:     ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    ));
    assert!(output.contains(&format!(
        "Annotated Tag:       pandora/canary/promoted/{canary_id}"
    )));
}

#[test]
fn test_cli_canary_promote_auto_merge_real_git() {
    let fixture = Fixture::new_git();
    let ws_str = fixture.workspace.to_str().unwrap();

    // 1. Stage canary
    let stage_res = fixture.run_success(&[
        "canary",
        "stage",
        "--dir",
        ws_str,
        "--task",
        "real_git_task",
        "--json",
    ]);
    let canary_id = stage_res["canary_id"].as_str().expect("canary_id required");

    // 2. Promote canary with --auto-merge in real git repo
    let prom_res = fixture.run_success(&[
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
        "100",
        "--failures",
        "0",
        "--baseline",
        "0.85",
        "--json",
    ]);

    assert_eq!(prom_res["status"], "promoted");
    assert_eq!(prom_res["auto_merged"], true);
    assert_eq!(
        prom_res["author"],
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );
    let tag_name = prom_res["tag_name"].as_str().unwrap();

    // Verify real git tag exists via git CLI directly
    let tag_out = Command::new("git")
        .args(["cat-file", "-t", tag_name])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&tag_out.stdout).trim(), "tag");

    // Verify commit author via git CLI
    let author_out = Command::new("git")
        .args(["log", "-1", "--format=%an <%ae>"])
        .current_dir(&fixture.workspace)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&author_out.stdout).trim(),
        "ANISAYAK MITRA <266799942+anisayakmitra-in@users.noreply.github.com>"
    );
}
