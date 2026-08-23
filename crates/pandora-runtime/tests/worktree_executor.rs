use pandora_runtime::{
    GitWorktreeExecutor, Parliament, ReferenceMonitor, WorktreeChange, WorktreeCommand,
    WorktreeError,
};
use pandora_types::{
    Capability, EffectOutcome, EffectTarget, ExecutionId, GeneId, Operation, OperationRequest,
    PolicyContext, PrincipalId, ResourceScope, SessionId, Timestamp,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

#[test]
fn creates_a_worktree_at_the_exact_authorized_commit() {
    let fixture = RepositoryFixture::new();
    let command = WorktreeCommand::create(
        fixture.repository(),
        fixture.destination(),
        fixture.first_commit(),
    )
    .expect("the fixture command should be valid");
    let permit = consumed_permit(
        &command,
        fixture.managed_root(),
        "coordination.worktree.create",
    );
    let executor = GitWorktreeExecutor::new(fixture.repository(), fixture.managed_root())
        .expect("the fixture roots should be valid");

    let result = executor.execute(&permit, &command, Timestamp::from_unix_seconds(20));

    assert_eq!(
        result.result(),
        Ok(&WorktreeChange::Created {
            path: fixture.destination().to_path_buf(),
            commit: fixture.first_commit().to_owned(),
        })
    );
    assert_eq!(result.receipt().outcome(), &EffectOutcome::Succeeded);
    assert_eq!(
        fs::read_to_string(fixture.destination().join("version.txt")).unwrap(),
        "one\n"
    );
    assert_eq!(
        git_output(fixture.destination(), &["rev-parse", "HEAD"]),
        fixture.first_commit()
    );
}

#[test]
fn rejects_a_destination_outside_the_managed_worktree_root() {
    let fixture = RepositoryFixture::new();
    let outside = fixture.root.join("outside-worker");
    let command = WorktreeCommand::create(fixture.repository(), &outside, fixture.first_commit())
        .expect("the absolute destination should be structurally valid");
    let permit = consumed_permit(
        &command,
        fixture.managed_root(),
        "coordination.worktree.create",
    );
    let executor = GitWorktreeExecutor::new(fixture.repository(), fixture.managed_root())
        .expect("the fixture roots should be valid");

    let result = executor.execute(&permit, &command, Timestamp::from_unix_seconds(20));

    assert_eq!(
        result.result(),
        Err(&WorktreeError::DestinationOutsideManagedRoot)
    );
    assert!(!outside.exists());
}

#[test]
fn rejects_a_consumed_permit_for_a_different_worktree_command() {
    let fixture = RepositoryFixture::new();
    let authorized = WorktreeCommand::create(
        fixture.repository(),
        fixture.destination(),
        fixture.first_commit(),
    )
    .unwrap();
    let other_destination = fixture.managed_root().join("worker-2");
    let attempted = WorktreeCommand::create(
        fixture.repository(),
        &other_destination,
        fixture.first_commit(),
    )
    .unwrap();
    let permit = consumed_permit(
        &authorized,
        fixture.managed_root(),
        "coordination.worktree.create",
    );
    let executor = GitWorktreeExecutor::new(fixture.repository(), fixture.managed_root()).unwrap();

    let result = executor.execute(&permit, &attempted, Timestamp::from_unix_seconds(20));

    assert_eq!(result.result(), Err(&WorktreeError::PermissionDenied));
    assert!(!other_destination.exists());
}

#[test]
fn preserves_an_existing_destination_when_creation_collides() {
    let fixture = RepositoryFixture::new();
    fs::create_dir(&fixture.destination).unwrap();
    fs::write(fixture.destination.join("owner.txt"), "existing\n").unwrap();
    let command = WorktreeCommand::create(
        fixture.repository(),
        fixture.destination(),
        fixture.first_commit(),
    )
    .unwrap();
    let permit = consumed_permit(
        &command,
        fixture.managed_root(),
        "coordination.worktree.create",
    );
    let executor = GitWorktreeExecutor::new(fixture.repository(), fixture.managed_root()).unwrap();

    let result = executor.execute(&permit, &command, Timestamp::from_unix_seconds(20));

    assert_eq!(result.result(), Err(&WorktreeError::DestinationExists));
    assert_eq!(
        fs::read_to_string(fixture.destination.join("owner.txt")).unwrap(),
        "existing\n"
    );
}

#[test]
fn removes_a_clean_managed_worktree_with_a_separate_permit() {
    let fixture = RepositoryFixture::new();
    let executor = GitWorktreeExecutor::new(fixture.repository(), fixture.managed_root()).unwrap();
    let create = WorktreeCommand::create(
        fixture.repository(),
        fixture.destination(),
        fixture.first_commit(),
    )
    .unwrap();
    let create_permit = consumed_permit(
        &create,
        fixture.managed_root(),
        "coordination.worktree.create",
    );
    assert!(
        executor
            .execute(&create_permit, &create, Timestamp::from_unix_seconds(20))
            .result()
            .is_ok()
    );
    let remove = WorktreeCommand::remove(
        fixture.repository(),
        fixture.destination(),
        fixture.first_commit(),
    )
    .unwrap();
    let remove_permit = consumed_permit(
        &remove,
        fixture.managed_root(),
        "coordination.worktree.remove",
    );

    let result = executor.execute(&remove_permit, &remove, Timestamp::from_unix_seconds(30));

    assert_eq!(
        result.result(),
        Ok(&WorktreeChange::Removed {
            path: fixture.destination().to_path_buf(),
            commit: fixture.first_commit().to_owned(),
        })
    );
    assert_eq!(result.receipt().outcome(), &EffectOutcome::Succeeded);
    assert!(!fixture.destination().exists());
}

#[test]
fn preserves_a_dirty_worktree_during_cleanup() {
    let fixture = RepositoryFixture::new();
    let executor = GitWorktreeExecutor::new(fixture.repository(), fixture.managed_root()).unwrap();
    let create = WorktreeCommand::create(
        fixture.repository(),
        fixture.destination(),
        fixture.first_commit(),
    )
    .unwrap();
    let create_permit = consumed_permit(
        &create,
        fixture.managed_root(),
        "coordination.worktree.create",
    );
    assert!(
        executor
            .execute(&create_permit, &create, Timestamp::from_unix_seconds(20))
            .result()
            .is_ok()
    );
    fs::write(fixture.destination().join("uncommitted.txt"), "keep me\n").unwrap();
    let remove = WorktreeCommand::remove(
        fixture.repository(),
        fixture.destination(),
        fixture.first_commit(),
    )
    .unwrap();
    let remove_permit = consumed_permit(
        &remove,
        fixture.managed_root(),
        "coordination.worktree.remove",
    );

    let result = executor.execute(&remove_permit, &remove, Timestamp::from_unix_seconds(30));

    assert_eq!(result.result(), Err(&WorktreeError::DirtyWorktree));
    assert_eq!(
        fs::read_to_string(fixture.destination().join("uncommitted.txt")).unwrap(),
        "keep me\n"
    );
}

#[test]
fn preserves_a_worktree_when_the_remove_commit_does_not_match() {
    let fixture = RepositoryFixture::new();
    let executor = GitWorktreeExecutor::new(fixture.repository(), fixture.managed_root()).unwrap();
    let create = WorktreeCommand::create(
        fixture.repository(),
        fixture.destination(),
        fixture.first_commit(),
    )
    .unwrap();
    let create_permit = consumed_permit(
        &create,
        fixture.managed_root(),
        "coordination.worktree.create",
    );
    assert!(
        executor
            .execute(&create_permit, &create, Timestamp::from_unix_seconds(20))
            .result()
            .is_ok()
    );
    let remove = WorktreeCommand::remove(
        fixture.repository(),
        fixture.destination(),
        fixture.second_commit(),
    )
    .unwrap();
    let remove_permit = consumed_permit(
        &remove,
        fixture.managed_root(),
        "coordination.worktree.remove",
    );

    let result = executor.execute(&remove_permit, &remove, Timestamp::from_unix_seconds(30));

    assert_eq!(result.result(), Err(&WorktreeError::CommitMismatch));
    assert!(fixture.destination().exists());
    assert_eq!(
        git_output(fixture.destination(), &["rev-parse", "HEAD"]),
        fixture.first_commit()
    );
}

#[test]
fn rejects_a_non_git_repository_during_executor_construction() {
    let root = unique_temp_dir("pandora-worktree-non-repository");
    let repository = root.join("repository");
    let managed_root = root.join("worktrees");
    fs::create_dir_all(&repository).unwrap();
    fs::create_dir_all(&managed_root).unwrap();

    let result = GitWorktreeExecutor::new(&repository, &managed_root);

    assert_eq!(result, Err(WorktreeError::InvalidRepository));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_a_non_unicode_destination_before_authorization_spec_creation() {
    let fixture = RepositoryFixture::new();
    let destination = non_unicode_destination(fixture.managed_root());

    let result = WorktreeCommand::create(fixture.repository(), destination, fixture.first_commit());

    assert_eq!(result, Err(WorktreeError::InvalidDestination));
}

#[test]
fn rejects_a_non_unicode_repository_before_authorization_spec_creation() {
    let root = unique_temp_dir("pandora-worktree-non-unicode-repository");
    fs::create_dir_all(&root).unwrap();
    let repository = non_unicode_destination(&root);
    fs::create_dir(&repository).unwrap();

    let result = WorktreeCommand::create(
        &repository,
        root.join("worker"),
        "0000000000000000000000000000000000000000",
    );

    assert_eq!(result, Err(WorktreeError::InvalidRepository));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_a_non_unicode_managed_root_during_executor_construction() {
    let fixture = RepositoryFixture::new();
    let managed_root = non_unicode_destination(&fixture.root);
    fs::create_dir(&managed_root).unwrap();

    let result = GitWorktreeExecutor::new(fixture.repository(), &managed_root);

    assert_eq!(result, Err(WorktreeError::InvalidManagedRoot));
}

fn consumed_permit(
    command: &WorktreeCommand,
    managed_root: &Path,
    gene_id: &str,
) -> pandora_runtime::ConsumedPermit {
    let request = OperationRequest::new(
        ExecutionId::new("execution-worktree-1").unwrap(),
        SessionId::new("session-worktree-1").unwrap(),
        PrincipalId::new("principal-worktree-1").unwrap(),
        GeneId::new(gene_id).unwrap(),
        None,
        Capability::ProcessExecute,
        Operation::Execute,
        EffectTarget::process(command.spec()),
        ResourceScope::path(managed_root.to_string_lossy()),
    )
    .unwrap();
    let policy = PolicyContext::new(1, [Capability::ProcessExecute], []);
    let monitor = ReferenceMonitor::new_with_policy(policy.clone(), 60);
    let decision = Parliament::new(1).decide(&request, &policy);
    let permit = monitor
        .authorize(request.clone(), decision, Timestamp::from_unix_seconds(10))
        .unwrap();
    monitor
        .store()
        .consume(permit, &request, Timestamp::from_unix_seconds(10))
        .unwrap()
}

struct RepositoryFixture {
    root: PathBuf,
    repository: PathBuf,
    managed_root: PathBuf,
    destination: PathBuf,
    first_commit: String,
    second_commit: String,
}

impl RepositoryFixture {
    fn new() -> Self {
        let root = unique_temp_dir("pandora-worktree-executor");
        let repository = root.join("repository");
        let managed_root = root.join("worktrees");
        let destination = managed_root.join("worker-1");
        fs::create_dir_all(&repository).unwrap();
        fs::create_dir_all(&managed_root).unwrap();
        git(&repository, &["init"]);
        fs::write(repository.join("version.txt"), "one\n").unwrap();
        git(&repository, &["add", "version.txt"]);
        git_with_identity(&repository, &["commit", "-m", "first"]);
        let first_commit = git_output(&repository, &["rev-parse", "HEAD"]);
        fs::write(repository.join("version.txt"), "two\n").unwrap();
        git(&repository, &["add", "version.txt"]);
        git_with_identity(&repository, &["commit", "-m", "second"]);
        let second_commit = git_output(&repository, &["rev-parse", "HEAD"]);
        Self {
            root,
            repository,
            managed_root,
            destination,
            first_commit,
            second_commit,
        }
    }

    fn repository(&self) -> &Path {
        &self.repository
    }

    fn managed_root(&self) -> &Path {
        &self.managed_root
    }

    fn destination(&self) -> &Path {
        &self.destination
    }

    fn first_commit(&self) -> &str {
        &self.first_commit
    }

    fn second_commit(&self) -> &str {
        &self.second_commit
    }
}

impl Drop for RepositoryFixture {
    fn drop(&mut self) {
        if self.destination.exists() {
            let _ = Command::new("git")
                .args(["worktree", "remove", "--force", "--"])
                .arg(&self.destination)
                .current_dir(&self.repository)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{nonce}-{sequence}",
        std::process::id()
    ))
}

#[cfg(windows)]
fn non_unicode_destination(root: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    root.join(OsString::from_wide(&[0xd800]))
}

#[cfg(unix)]
fn non_unicode_destination(root: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    root.join(OsString::from_vec(vec![0xff]))
}

fn git(repository: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git should start");
    assert!(status.success(), "git {arguments:?} failed");
}

fn git_with_identity(repository: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=Pandora Tests"])
        .args(["-c", "user.email=pandora@example.invalid"])
        .args(arguments)
        .current_dir(repository)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git should start");
    assert!(status.success(), "git {arguments:?} failed");
}

fn git_output(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should start");
    assert!(output.status.success(), "git {arguments:?} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
