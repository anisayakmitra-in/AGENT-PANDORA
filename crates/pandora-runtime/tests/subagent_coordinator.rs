use pandora_runtime::executors::WorkspaceRoot;
use pandora_runtime::{
    ExecutionController, GitWorktreeExecutor, SubagentCleanupContext, SubagentCoordinator,
    SubagentCoordinatorError, SubagentPreparation, SubagentScope, SubagentSpawnContext,
    SubagentStore, SubagentStoreError, WorktreeError,
};
use pandora_types::{
    Capability, EffectOutcome, ExecutionId, JobId, JobWorkerId, PolicyContext, PrincipalId,
    SessionId, SubagentBudgets, SubagentId, SubagentRequest, SubagentStatus, SubagentWorktreeState,
    TenantId, Timestamp, WorkspaceId,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

const SUBAGENT_PATH_DIGEST: &str =
    "8e8560771fbf15d29379685af1cd6084f36616c9e5b967f139aec279e92afdfb";
static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

#[test]
fn spawn_persists_preparation_before_authorized_worktree_effect() {
    let fixture = CoordinatorFixture::new();
    let missing_commit = "0".repeat(40);

    let result = fixture.coordinator().spawn(
        fixture.spawn_context(),
        fixture.request(missing_commit),
        fixture.now(),
    );

    assert!(matches!(
        result,
        Err(SubagentCoordinatorError::Worktree(WorktreeError::GitFailed))
    ));
    let record = fixture
        .store
        .inspect(&fixture.subagent_id, &fixture.scope)
        .unwrap();
    assert_eq!(record.status(), SubagentStatus::Failed);
    assert_eq!(record.worktree_state(), SubagentWorktreeState::Pending);
    assert!(matches!(
        record.create_receipt().map(|receipt| receipt.outcome()),
        Some(EffectOutcome::Failed { code }) if code == "git_failed"
    ));
    assert!(!fixture.destination().exists());
}

#[test]
fn preparing_reconciliation_preserves_uncertain_existing_destination() {
    let fixture = CoordinatorFixture::new();
    fixture.prepare();
    fixture.create_exact_worktree_outside_coordinator();

    let record = fixture
        .coordinator()
        .reconcile_preparing(&fixture.subagent_id, &fixture.scope, fixture.now())
        .unwrap();

    assert_eq!(record.status(), SubagentStatus::Interrupted);
    assert_eq!(record.worktree_state(), SubagentWorktreeState::Preserved);
    assert!(record.create_receipt().is_none());
    assert_eq!(
        record
            .result()
            .and_then(|result| result.get("outcome_known"))
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert!(fixture.destination().exists());
}

#[test]
fn cleanup_requires_a_fresh_permit_and_preserves_dirty_worktree() {
    let fixture = CoordinatorFixture::new();
    let completed = fixture.completed();
    fs::write(fixture.destination().join("dirty.txt"), "keep").unwrap();

    let result = fixture.coordinator().cleanup(
        &fixture.subagent_id,
        fixture.cleanup_context(),
        fixture.now(),
    );

    assert!(matches!(
        result,
        Err(SubagentCoordinatorError::Worktree(
            WorktreeError::DirtyWorktree
        ))
    ));
    let record = fixture
        .store
        .inspect(&fixture.subagent_id, &fixture.scope)
        .unwrap();
    assert_eq!(record.worktree_state(), SubagentWorktreeState::Preserved);
    let remove_receipt = record
        .remove_receipt()
        .expect("failed cleanup is receipted");
    assert!(matches!(
        remove_receipt.outcome(),
        EffectOutcome::Failed { code } if code == "dirty_worktree"
    ));
    assert_ne!(
        completed.create_receipt().unwrap().permit_id(),
        remove_receipt.permit_id(),
        "cleanup must consume a fresh permit"
    );
    assert!(fixture.destination().join("dirty.txt").exists());
}

#[test]
fn repeated_cleanup_rejection_does_not_execute_or_replace_the_receipt() {
    let fixture = CoordinatorFixture::new();
    fixture.completed();
    let dirty_path = fixture.destination().join("dirty.txt");
    fs::write(&dirty_path, "keep").unwrap();

    let first = fixture.coordinator().cleanup(
        &fixture.subagent_id,
        fixture.cleanup_context_named("first"),
        fixture.now(),
    );
    assert!(matches!(
        first,
        Err(SubagentCoordinatorError::Worktree(
            WorktreeError::DirtyWorktree
        ))
    ));
    let first_receipt_id = fixture
        .store
        .inspect(&fixture.subagent_id, &fixture.scope)
        .unwrap()
        .remove_receipt()
        .unwrap()
        .receipt_id()
        .clone();
    fs::remove_file(dirty_path).unwrap();

    let repeated = fixture.coordinator().cleanup(
        &fixture.subagent_id,
        fixture.cleanup_context_named("second"),
        Timestamp::from_unix_seconds(11),
    );

    assert!(matches!(
        repeated,
        Err(SubagentCoordinatorError::Store(
            SubagentStoreError::InvalidTransition { .. }
        ))
    ));
    let stored = fixture
        .store
        .inspect(&fixture.subagent_id, &fixture.scope)
        .unwrap();
    assert_eq!(
        stored.remove_receipt().unwrap().receipt_id(),
        &first_receipt_id
    );
    assert!(
        fixture.destination().exists(),
        "claim rejection must occur before Git can remove the worktree"
    );
}

#[test]
fn concurrent_cleanup_allows_only_one_effect_attempt() {
    let fixture = CoordinatorFixture::new();
    fixture.completed();
    fs::write(fixture.destination().join("dirty.txt"), "keep").unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let results = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for name in ["first", "second"] {
            let barrier = Arc::clone(&barrier);
            let coordinator = fixture.coordinator();
            let id = fixture.subagent_id.clone();
            let context = fixture.cleanup_context_named(name);
            handles.push(scope.spawn(move || {
                barrier.wait();
                coordinator.cleanup(&id, context, Timestamp::from_unix_seconds(12))
            }));
        }
        barrier.wait();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                Err(SubagentCoordinatorError::Worktree(
                    WorktreeError::DirtyWorktree
                ))
            ))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                Err(SubagentCoordinatorError::Store(
                    SubagentStoreError::InvalidTransition { .. }
                ))
            ))
            .count(),
        1
    );
    let stored = fixture
        .store
        .inspect(&fixture.subagent_id, &fixture.scope)
        .unwrap();
    assert_eq!(stored.worktree_state(), SubagentWorktreeState::Preserved);
    assert!(stored.remove_receipt().is_some());
    assert!(fixture.destination().join("dirty.txt").exists());
}

#[test]
fn spawn_rejects_a_parent_binding_mismatch_before_persistence() {
    let fixture = CoordinatorFixture::new();
    let request = SubagentRequest::new(
        SessionId::new("different-parent-session").unwrap(),
        fixture.parent_execution_id.clone(),
        1,
        fixture.commit.clone(),
        "inspect the repository",
        fixture.budgets(),
    )
    .unwrap();

    let result = fixture
        .coordinator()
        .spawn(fixture.spawn_context(), request, fixture.now());

    assert!(matches!(
        result,
        Err(SubagentCoordinatorError::ParentBindingMismatch)
    ));
    assert!(
        fixture
            .store
            .inspect(&fixture.subagent_id, &fixture.scope)
            .is_err()
    );
    assert!(!fixture.destination().exists());
}

#[test]
fn spawn_uses_a_deterministic_direct_child_destination() {
    let fixture = CoordinatorFixture::new();

    let record = fixture
        .coordinator()
        .spawn(
            fixture.spawn_context(),
            fixture.request(fixture.commit.clone()),
            fixture.now(),
        )
        .unwrap();

    assert_eq!(record.worktree_path(), fixture.destination());
    assert_eq!(
        record.worktree_path().parent(),
        Some(fixture.managed_root.as_path())
    );
    assert_eq!(
        record.worktree_path().file_name().unwrap(),
        SUBAGENT_PATH_DIGEST
    );
    assert_eq!(record.status(), SubagentStatus::Queued);
}

struct CoordinatorFixture {
    root: PathBuf,
    repository: PathBuf,
    managed_root: PathBuf,
    store: SubagentStore,
    controller: ExecutionController,
    executor: GitWorktreeExecutor,
    scope: SubagentScope,
    subagent_id: SubagentId,
    job_id: JobId,
    child_session_id: SessionId,
    child_execution_id: ExecutionId,
    parent_session_id: SessionId,
    parent_execution_id: ExecutionId,
    commit: String,
}

impl CoordinatorFixture {
    fn new() -> Self {
        let root = unique_temp_dir("pandora-subagent-coordinator");
        let repository = root.join("repository");
        let managed_root = root.join("worktrees");
        fs::create_dir_all(&repository).unwrap();
        fs::create_dir_all(&managed_root).unwrap();
        git(&repository, &["init"]);
        git(
            &repository,
            &["config", "user.email", "pandora@example.invalid"],
        );
        git(&repository, &["config", "user.name", "Pandora Test"]);
        fs::write(repository.join("version.txt"), "one\n").unwrap();
        git(&repository, &["add", "version.txt"]);
        git(&repository, &["commit", "-m", "initial"]);
        let commit = git_output(&repository, &["rev-parse", "HEAD"]);
        let store = SubagentStore::open(root.join("jobs.sqlite3")).unwrap();
        let controller = ExecutionController::with_policy(
            WorkspaceRoot::new(&repository).unwrap(),
            PolicyContext::new(1, [Capability::ProcessExecute], []),
        );
        let executor = GitWorktreeExecutor::new(&repository, &managed_root).unwrap();
        Self {
            root,
            repository,
            managed_root,
            store,
            controller,
            executor,
            scope: SubagentScope::new(
                PrincipalId::new("principal-subagent-1").unwrap(),
                TenantId::new("tenant-subagent-1").unwrap(),
                WorkspaceId::new("workspace-subagent-1").unwrap(),
            ),
            subagent_id: SubagentId::new("subagent-1").unwrap(),
            job_id: JobId::new("job-subagent-1").unwrap(),
            child_session_id: SessionId::new("session-child-1").unwrap(),
            child_execution_id: ExecutionId::new("execution-child-1").unwrap(),
            parent_session_id: SessionId::new("session-parent-1").unwrap(),
            parent_execution_id: ExecutionId::new("execution-parent-1").unwrap(),
            commit,
        }
    }

    fn coordinator(&self) -> SubagentCoordinator<'_> {
        SubagentCoordinator::new(&self.store, &self.controller, &self.executor)
    }

    fn destination(&self) -> PathBuf {
        self.managed_root.join(SUBAGENT_PATH_DIGEST)
    }

    fn budgets(&self) -> SubagentBudgets {
        SubagentBudgets::new(4, 8, 1_000, 300, 2, 8_192).unwrap()
    }

    fn request(&self, commit: String) -> SubagentRequest {
        SubagentRequest::new(
            self.parent_session_id.clone(),
            self.parent_execution_id.clone(),
            1,
            commit,
            "inspect the repository",
            self.budgets(),
        )
        .unwrap()
    }

    fn spawn_context(&self) -> SubagentSpawnContext {
        SubagentSpawnContext::new(
            self.subagent_id.clone(),
            self.job_id.clone(),
            self.scope.clone(),
            self.child_session_id.clone(),
            self.child_execution_id.clone(),
            self.parent_session_id.clone(),
            self.parent_execution_id.clone(),
            Some("provider-sha256:abc123".to_owned()),
            Some("harness-sha256:def456".to_owned()),
        )
    }

    fn cleanup_context(&self) -> SubagentCleanupContext {
        self.cleanup_context_named("1")
    }

    fn cleanup_context_named(&self, name: &str) -> SubagentCleanupContext {
        SubagentCleanupContext::new(
            self.scope.clone(),
            SessionId::new(format!("session-cleanup-{name}")).unwrap(),
            ExecutionId::new(format!("execution-cleanup-{name}")).unwrap(),
        )
    }

    fn prepare(&self) {
        self.store
            .prepare(SubagentPreparation::new(
                self.subagent_id.clone(),
                self.job_id.clone(),
                self.scope.clone(),
                self.child_session_id.clone(),
                self.child_execution_id.clone(),
                self.request(self.commit.clone()),
                self.executor.repository().to_path_buf(),
                self.managed_root.join(SUBAGENT_PATH_DIGEST),
                Some("provider-sha256:abc123".to_owned()),
                Some("harness-sha256:def456".to_owned()),
                self.now(),
            ))
            .unwrap();
    }

    fn create_exact_worktree_outside_coordinator(&self) {
        let destination = self.managed_root.join(SUBAGENT_PATH_DIGEST);
        let status = Command::new("git")
            .args(["worktree", "add", "--detach"])
            .arg(&destination)
            .arg(&self.commit)
            .current_dir(&self.repository)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn completed(&self) -> pandora_runtime::SubagentRecord {
        self.coordinator()
            .spawn(
                self.spawn_context(),
                self.request(self.commit.clone()),
                self.now(),
            )
            .unwrap();
        let worker = JobWorkerId::new("worker-subagent-1").unwrap();
        self.store
            .claim_next(&self.scope, &worker, Timestamp::from_unix_seconds(20))
            .unwrap()
            .unwrap();
        self.store
            .finish(
                &self.subagent_id,
                &worker,
                SubagentStatus::Completed,
                &json!({"summary": "done"}),
                Timestamp::from_unix_seconds(30),
            )
            .unwrap()
    }

    fn now(&self) -> Timestamp {
        Timestamp::from_unix_seconds(10)
    }
}

impl Drop for CoordinatorFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{timestamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    root
}

fn git(repository: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git command failed: {arguments:?}");
}

fn git_output(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
