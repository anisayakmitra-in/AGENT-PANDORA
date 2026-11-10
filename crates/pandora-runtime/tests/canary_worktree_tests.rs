//! Test suite for Autonomous Git Worktree Patch Isolation & Canary Staging.

#![forbid(unsafe_code)]

use pandora_runtime::autopilot::{AutopilotCoordinator, AutopilotGoal};
use pandora_runtime::canary_worktree::{
    CanaryStagePipeline, GitWorktreeManager, IsolatedWorktreeGuard, WorktreeStatus,
};
use pandora_runtime::multi_agent_collab::ExternalAgentKind;
use pandora_runtime::multi_agent_consensus::{AgentPatchProposal, ProposalScoreVector};
use std::fs;
use std::path::PathBuf;

fn create_temp_test_dir(test_name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "pandora_canary_test_{test_name}_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create temp test directory");
    dir
}

#[test]
fn test_isolated_worktree_creation_and_cleanup() {
    let temp_root = create_temp_test_dir("create_clean");
    let cert =
        "PANDORA_PATCH_CERT_V1:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    let worktree =
        GitWorktreeManager::create_worktree(&temp_root, "pandora/canary/test-task", cert)
            .expect("worktree creation must succeed");

    assert!(worktree.path.exists(), "worktree path must exist on disk");
    assert_eq!(worktree.branch_name, "pandora/canary/test-task");
    assert_eq!(worktree.status, WorktreeStatus::Created);

    GitWorktreeManager::remove_worktree(&worktree).expect("worktree removal must succeed");
    assert!(
        !worktree.path.exists(),
        "worktree path must be removed after cleanup"
    );

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_raii_guard_automatic_cleanup_on_drop() {
    let temp_root = create_temp_test_dir("raii_drop");
    let cert =
        "PANDORA_PATCH_CERT_V1:1111222233334444555566667777888899990000aaaabbbbccccddddeeeeffff";

    let worktree_path = {
        let worktree =
            GitWorktreeManager::create_worktree(&temp_root, "pandora/canary/raii-task", cert)
                .expect("worktree creation must succeed");
        let path = worktree.path.clone();
        assert!(path.exists());
        let _guard = IsolatedWorktreeGuard::new(worktree);
        // _guard falls out of scope here without calling persist()
        path
    };

    assert!(
        !worktree_path.exists(),
        "RAII guard must automatically clean up the isolated worktree directory on drop"
    );

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_raii_guard_persist_promotes_worktree() {
    let temp_root = create_temp_test_dir("raii_persist");
    let cert =
        "PANDORA_PATCH_CERT_V1:9999888877776666555544443333222211110000aaaabbbbccccddddeeeeffff";

    let worktree_path = {
        let worktree =
            GitWorktreeManager::create_worktree(&temp_root, "pandora/canary/persist-task", cert)
                .expect("worktree creation must succeed");
        let path = worktree.path.clone();
        let mut guard = IsolatedWorktreeGuard::new(worktree);
        guard.persist(); // Mark as promoted / preserved
        path
    };

    assert!(
        worktree_path.exists(),
        "Worktree directory must be preserved on drop when guard.persist() was called"
    );

    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_canary_stage_pipeline_full_attestation() {
    let temp_root = create_temp_test_dir("pipeline_attest");
    let cert =
        "PANDORA_PATCH_CERT_V1:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    let mut worktree =
        GitWorktreeManager::create_worktree(&temp_root, "pandora/canary/attest-branch", cert)
            .expect("worktree creation must succeed");

    let patch = AgentPatchProposal::new(
        "patch-001",
        ExternalAgentKind::ClaudeCode,
        "Implement Memory Safe Vector Dispatch",
        "Replaces raw index with checked slice access",
        "+++ b/crates/dispatch.rs\n+// Memory safe dispatch implementation\npub fn dispatch() {}\n",
        ProposalScoreVector::new(0.95, 0.98, 0.94, 0.99),
        42,
    );

    let pipeline = CanaryStagePipeline::default();
    let receipt = pipeline
        .stage_candidate_patch(&mut worktree, &patch)
        .expect("staging must succeed for clean patch");

    assert_eq!(receipt.branch_name, "pandora/canary/attest-branch");
    assert_eq!(receipt.patch_certificate, cert);
    assert_eq!(receipt.passed_checks, receipt.total_checks);
    assert_eq!(worktree.status, WorktreeStatus::Attested);
    assert!(!receipt.signature_hex.is_empty());
    assert!(receipt.attestation_digest.starts_with("sha256:"));

    let _ = GitWorktreeManager::remove_worktree(&worktree);
    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_canary_sandbox_invariant_forbid_unsafe() {
    let temp_root = create_temp_test_dir("unsafe_rejection");
    let cert =
        "PANDORA_PATCH_CERT_V1:deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    let mut worktree =
        GitWorktreeManager::create_worktree(&temp_root, "pandora/canary/unsafe-branch", cert)
            .expect("worktree creation must succeed");

    let unsafe_patch = AgentPatchProposal::new(
        "patch-unsafe",
        ExternalAgentKind::Devin,
        "Unsafe pointer manipulation",
        "Bypasses borrow checker for raw throughput",
        "+++ b/crates/fast.rs\n+unsafe fn bypass() { let ptr = 0x1234 as *mut u8; }\n",
        ProposalScoreVector::new(0.80, 0.85, 0.70, 0.90),
        15,
    );

    let pipeline = CanaryStagePipeline::default();
    let err = pipeline
        .stage_candidate_patch(&mut worktree, &unsafe_patch)
        .expect_err("unsafe code patch must be rejected by sandbox verification");

    let err_str = err.to_string();
    assert!(
        err_str.contains("Memory Safety Invariant Check"),
        "error must explicitly report memory safety failure, got: {err_str}"
    );

    let _ = GitWorktreeManager::remove_worktree(&worktree);
    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_canary_sandbox_containment_path_escape() {
    let temp_root = create_temp_test_dir("escape_rejection");
    let cert =
        "PANDORA_PATCH_CERT_V1:cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe";

    let mut worktree =
        GitWorktreeManager::create_worktree(&temp_root, "pandora/canary/escape-branch", cert)
            .expect("worktree creation must succeed");

    let escape_patch = AgentPatchProposal::new(
        "patch-escape",
        ExternalAgentKind::Hermes,
        "Path traversal test",
        "Attempts to escape worktree root",
        "+++ b/../../outside_root.rs\n+pub fn escape() {}\n",
        ProposalScoreVector::new(0.80, 0.80, 0.80, 0.80),
        20,
    );

    let pipeline = CanaryStagePipeline::default();
    let err = pipeline
        .stage_candidate_patch(&mut worktree, &escape_patch)
        .expect_err("path escape attempt must be rejected");

    let err_str = err.to_string();
    assert!(
        err_str.contains("Worktree Containment Check"),
        "error must explicitly report containment violation, got: {err_str}"
    );

    let _ = GitWorktreeManager::remove_worktree(&worktree);
    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_autopilot_coordinator_with_canary_staging() {
    let temp_root = create_temp_test_dir("autopilot_canary");

    let mut goal = AutopilotGoal::new(
        "task-canary-01",
        "Refactor vector cache and verify invariants",
    );
    goal.workspace_root = temp_root.clone();
    goal.dry_run = false; // Trigger active canary worktree creation & staging!

    let coordinator = AutopilotCoordinator::default();
    let result = coordinator
        .execute_session(&goal)
        .expect("autopilot execution must succeed");

    assert!(
        result
            .patch_certificate
            .starts_with("PANDORA_PATCH_CERT_V1:")
    );
    assert!(
        result.canary_staging_receipt.is_some(),
        "canary staging receipt must be present when dry_run=false"
    );

    let receipt = result.canary_staging_receipt.unwrap();
    assert_eq!(receipt.branch_name, "pandora/canary/task-canary-01");
    assert_eq!(receipt.patch_certificate, result.patch_certificate);
    assert!(receipt.passed_checks > 0);

    let _ = fs::remove_dir_all(&temp_root);
}
