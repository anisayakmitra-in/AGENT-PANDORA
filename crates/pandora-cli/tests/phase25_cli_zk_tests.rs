//! Phase 25 CLI Integration Tests: Zero-Knowledge Execution Attestation & Merkle Rollup (Theorem 48)
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

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1925);

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
            "pandora-phase25-zk-test-{}-{timestamp}-{sequence}",
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
            "command failed: {:?}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout should be valid UTF-8");
        serde_json::from_str(&stdout).expect("stdout should be valid JSON")
    }

    fn run_failure(&self, args: &[&str]) -> (i32, String) {
        let output = self.command(args).output().expect("command should execute");
        assert!(
            !output.status.success(),
            "command should have failed: {:?}",
            args
        );
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.code().unwrap_or(-1), stderr)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn test_cli_zk_prove_and_verify_execution_attestation() {
    let fixture = Fixture::new();
    let proof_file = fixture.root.join("proof.json");

    // 1. Generate execution attestation proof
    let prove_val = fixture.run_success(&[
        "zk",
        "prove",
        "--patch-cert",
        "PANDORA_PATCH_CERT_V1:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "--holdout-count",
        "8",
        "--tests-passed",
        "8",
        "--invariants-verified",
        "true",
        "--out",
        proof_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(prove_val["mode"], "execution_attestation");
    assert_eq!(prove_val["verified"], true);
    assert_eq!(prove_val["memory_safety_verified"], true);
    assert_eq!(prove_val["holdout_test_count"], 8);
    assert_eq!(prove_val["holdout_tests_passed"], 8);
    assert!(proof_file.exists());

    // 2. Verify proof with polymorphic verifier
    let verify_val = fixture.run_success(&[
        "zk",
        "verify",
        "--proof-file",
        proof_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(verify_val["type"], "execution_attestation");
    assert_eq!(verify_val["valid"], true);
    assert_eq!(verify_val["memory_safety_verified"], true);
}

#[test]
fn test_cli_zk_rollup_batch_mock_and_inclusion() {
    let fixture = Fixture::new();
    let epoch_file = fixture.root.join("epoch.json");
    let proof_file = fixture.root.join("inclusion_proof.json");

    // 1. Rollup batch 8 decisions
    let batch_val = fixture.run_success(&[
        "zk",
        "rollup",
        "batch",
        "--epoch",
        "1",
        "--mock-count",
        "8",
        "--out",
        epoch_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(batch_val["decision_count"], 8);
    assert_eq!(batch_val["tree_height"], 3);
    assert!(batch_val["epoch_root_hex"].is_string());
    assert!(epoch_file.exists());

    // 2. Prove inclusion for leaf at index 3
    let prove_inc_val = fixture.run_success(&[
        "zk",
        "rollup",
        "prove-inclusion",
        "--epoch-file",
        epoch_file.to_str().unwrap(),
        "--index",
        "3",
        "--out",
        proof_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(prove_inc_val["leaf_index"], 3);
    assert_eq!(prove_inc_val["self_verified"], true);
    assert_eq!(prove_inc_val["proof_steps"], 3);
    assert!(proof_file.exists());

    // 3. Verify inclusion proof against root
    let verify_inc_val = fixture.run_success(&[
        "zk",
        "rollup",
        "verify-inclusion",
        "--proof-file",
        proof_file.to_str().unwrap(),
        "--json",
    ]);

    assert_eq!(verify_inc_val["valid"], true);
    assert_eq!(verify_inc_val["leaf_index"], 3);
}

#[test]
fn test_cli_zk_legacy_compatibility_preserved() {
    let fixture = Fixture::new();

    // 1. Legacy ZK benchmark
    let bench_val = fixture.run_success(&[
        "zk",
        "benchmark",
        "--domain-size",
        "8",
        "--forgeries",
        "16",
        "--json",
    ]);
    assert_eq!(bench_val["completeness_verified"], true);
    assert_eq!(bench_val["forgeries_rejected"], 16);

    // 2. Legacy ZK prove without patch flags
    let prove_val = fixture.run_success(&["zk", "prove", "--domain-size", "8", "--json"]);
    assert_eq!(prove_val["verified"], true);
    assert!(prove_val["pre_state_commitment_hex"].is_string());
}

#[test]
fn test_cli_zk_error_handling() {
    let fixture = Fixture::new();

    // Missing subcommand
    let (code, stderr) = fixture.run_failure(&["zk"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("zk requires"));

    // Unknown subcommand
    let (code2, stderr2) = fixture.run_failure(&["zk", "unknown-command"]);
    assert_ne!(code2, 0);
    assert!(stderr2.contains("unknown zk subcommand"));

    // Missing index for prove-inclusion
    let (code3, stderr3) = fixture.run_failure(&["zk", "rollup", "prove-inclusion"]);
    assert_ne!(code3, 0);
    assert!(stderr3.contains("--index"));
}
