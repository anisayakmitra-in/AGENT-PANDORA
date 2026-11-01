#![forbid(unsafe_code)]

use pandora_runtime::{
    WorktreeError, WorktreeIsolationManager, strip_unc,
};
use pandora_types::{FinalizeAction, WorktreeStatus};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

struct TempFixture {
    root: PathBuf,
    repo_dir: PathBuf,
}

impl TempFixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("pandora-wt-iso-{}-{nonce}-{seq}", std::process::id()));
        let repo_dir = root.join("repo");
        fs::create_dir_all(&repo_dir).expect("create repo dir");
        init_git_repo(&repo_dir);
        Self { root, repo_dir }
    }

    fn repo(&self) -> &Path {
        &self.repo_dir
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.repo_dir)
            .status();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn init_git_repo(path: &Path) {
    let mut cmd = Command::new("git");
    cmd.args(["init", "-b", "main"])
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git init succeeds");

    let _ = Command::new("git")
        .args(["config", "user.name", "Pandora Test"])
        .current_dir(path)
        .status();

    let _ = Command::new("git")
        .args(["config", "user.email", "pandora@example.com"])
        .current_dir(path)
        .status();

    let readme = path.join("README.md");
    fs::write(&readme, "# Test Repo\n").expect("write readme");

    let _ = Command::new("git")
        .args(["add", "README.md"])
        .current_dir(path)
        .status();

    let _ = Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(path)
        .status();
}

#[test]
fn test_strip_unc_utility() {
    let unc = Path::new(r"\\?\C:\repo\target");
    assert_eq!(strip_unc(unc), PathBuf::from(r"C:\repo\target"));

    let regular = Path::new(r"C:\repo\target");
    assert_eq!(strip_unc(regular), PathBuf::from(r"C:\repo\target"));
}

#[test]
fn test_worktree_isolation_manager_lifecycle() {
    let fixture = TempFixture::new();
    let mgr = WorktreeIsolationManager::new(fixture.repo()).expect("manager created");
    assert!(
        mgr.managed_root().ends_with(".pandora/worktrees")
            || mgr.managed_root().ends_with(r".pandora\worktrees")
    );

    // Provision worktree
    let task_id = "test-task-1";
    let ctx = mgr.provision_worktree(task_id, None).expect("provisioned");
    assert_eq!(ctx.task_id, task_id);
    assert_eq!(ctx.branch_name, "goal/test-task-1");
    assert_eq!(ctx.status, WorktreeStatus::Provisioned);
    assert!(ctx.worktree_path.exists());

    // List worktrees
    let worktrees = mgr.list_worktrees().expect("list worktrees");
    assert!(worktrees.iter().any(|wt| wt.task_id == task_id));

    // Evaluate gates
    let report = mgr.evaluate_gates(&ctx).expect("evaluate gates");
    assert_eq!(report.gate_results.len(), 4);
    assert!(report.all_passed);

    // Make a change in worktree
    let changed_file = ctx.worktree_path.join("feature.txt");
    fs::write(&changed_file, "New feature implementation\n").expect("write file");

    // Finalize with SquashMerge
    let merge_res = mgr
        .finalize_task(&ctx, FinalizeAction::SquashMerge)
        .expect("finalize squash merge");
    assert!(merge_res.success);
    assert!(merge_res.squash_commit.is_some());
    assert!(merge_res.worktree_cleaned);

    // Verify worktree directory is cleaned up
    assert!(!ctx.worktree_path.exists());
}

#[test]
fn test_worktree_isolation_manager_pr_generation() {
    let fixture = TempFixture::new();
    let mgr = WorktreeIsolationManager::new(fixture.repo()).expect("manager created");
    let task_id = "pr-task-2";
    let ctx = mgr.provision_worktree(task_id, None).expect("provisioned");

    let new_doc = ctx.worktree_path.join("doc.md");
    fs::write(&new_doc, "PR documentation\n").expect("write doc");

    let pr_res = mgr
        .finalize_task(&ctx, FinalizeAction::PullRequest)
        .expect("finalize PR");
    assert!(pr_res.success);
    assert!(pr_res.worktree_cleaned);
    assert!(!ctx.worktree_path.exists());
}

#[test]
fn test_worktree_isolation_manager_discard() {
    let fixture = TempFixture::new();
    let mgr = WorktreeIsolationManager::new(fixture.repo()).expect("manager created");
    let task_id = "discard-task-3";
    let ctx = mgr.provision_worktree(task_id, None).expect("provisioned");
    assert!(ctx.worktree_path.exists());

    let discard_res = mgr
        .finalize_task(&ctx, FinalizeAction::Discard)
        .expect("discard");
    assert!(discard_res.success);
    assert!(discard_res.worktree_cleaned);
    assert!(!ctx.worktree_path.exists());
}

#[test]
fn test_worktree_isolation_rejects_invalid_task_id() {
    let fixture = TempFixture::new();
    let mgr = WorktreeIsolationManager::new(fixture.repo()).expect("manager created");

    assert_eq!(
        mgr.provision_worktree("../escape", None).unwrap_err(),
        WorktreeError::InvalidDestination
    );
    assert_eq!(
        mgr.provision_worktree("", None).unwrap_err(),
        WorktreeError::InvalidDestination
    );
}

#[test]
fn test_worktree_cleanup_operations() {
    let fixture = TempFixture::new();
    let mgr = WorktreeIsolationManager::new(fixture.repo()).expect("manager created");
    let ctx = mgr
        .provision_worktree("cleanup-task-4", None)
        .expect("provisioned");
    assert!(ctx.worktree_path.exists());

    let cleaned = mgr.cleanup_worktree("cleanup-task-4", true).expect("cleanup");
    assert!(cleaned);
    assert!(!ctx.worktree_path.exists());
}
