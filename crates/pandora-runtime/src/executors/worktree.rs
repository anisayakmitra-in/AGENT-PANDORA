use crate::ConsumedPermit;
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, EffectTarget, Operation, ReceiptId, ResourceScope,
    Timestamp,
};
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorktreeError {
    InvalidRepository,
    InvalidManagedRoot,
    InvalidDestination,
    DestinationOutsideManagedRoot,
    DestinationExists,
    DirtyWorktree,
    CommitMismatch,
    InvalidCommit,
    PermissionDenied,
    GitUnavailable,
    GitFailed,
}

impl WorktreeError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRepository => "invalid_repository",
            Self::InvalidManagedRoot => "invalid_managed_root",
            Self::InvalidDestination => "invalid_destination",
            Self::DestinationOutsideManagedRoot => "destination_outside_managed_root",
            Self::DestinationExists => "destination_exists",
            Self::DirtyWorktree => "dirty_worktree",
            Self::CommitMismatch => "commit_mismatch",
            Self::InvalidCommit => "invalid_commit",
            Self::PermissionDenied => "permission_denied",
            Self::GitUnavailable => "git_unavailable",
            Self::GitFailed => "git_failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeCommand {
    operation: WorktreeOperation,
    repository: PathBuf,
    destination: PathBuf,
    commit: String,
    spec: String,
}

impl WorktreeCommand {
    pub fn create(
        repository: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        commit: impl Into<String>,
    ) -> Result<Self, WorktreeError> {
        let repository =
            canonical_directory(repository.as_ref(), WorktreeError::InvalidRepository)?;
        let destination = destination.as_ref().to_path_buf();
        if !destination.is_absolute()
            || destination.file_name().is_none()
            || destination.to_str().is_none()
        {
            return Err(WorktreeError::InvalidDestination);
        }
        let commit = commit.into();
        if !is_exact_commit(&commit) {
            return Err(WorktreeError::InvalidCommit);
        }
        Ok(Self::new(
            WorktreeOperation::Create,
            repository,
            destination,
            commit,
        ))
    }

    pub fn remove(
        repository: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        commit: impl Into<String>,
    ) -> Result<Self, WorktreeError> {
        let repository =
            canonical_directory(repository.as_ref(), WorktreeError::InvalidRepository)?;
        let destination = destination.as_ref().to_path_buf();
        if !destination.is_absolute()
            || destination.file_name().is_none()
            || destination.to_str().is_none()
        {
            return Err(WorktreeError::InvalidDestination);
        }
        let commit = commit.into();
        if !is_exact_commit(&commit) {
            return Err(WorktreeError::InvalidCommit);
        }
        Ok(Self::new(
            WorktreeOperation::Remove,
            repository,
            destination,
            commit,
        ))
    }

    pub fn spec(&self) -> &str {
        &self.spec
    }

    fn new(
        operation: WorktreeOperation,
        repository: PathBuf,
        destination: PathBuf,
        commit: String,
    ) -> Self {
        let spec = serde_json::to_string(&CommandSpec {
            operation: operation.as_str(),
            repository: repository
                .to_str()
                .expect("validated repository paths are Unicode"),
            destination: destination
                .to_str()
                .expect("validated destination paths are Unicode"),
            commit: &commit,
        })
        .expect("worktree command fields are serializable");
        Self {
            operation,
            repository,
            destination,
            commit,
            spec,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorktreeOperation {
    Create,
    Remove,
}

impl WorktreeOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "git_worktree_create",
            Self::Remove => "git_worktree_remove",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorktreeChange {
    Created { path: PathBuf, commit: String },
    Removed { path: PathBuf, commit: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeResult {
    result: Result<WorktreeChange, WorktreeError>,
    receipt: EffectReceipt,
}

impl WorktreeResult {
    pub fn result(&self) -> Result<&WorktreeChange, &WorktreeError> {
        self.result.as_ref()
    }

    pub fn into_result(self) -> Result<WorktreeChange, WorktreeError> {
        self.result
    }

    pub fn receipt(&self) -> &EffectReceipt {
        &self.receipt
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitWorktreeExecutor {
    repository: PathBuf,
    managed_root: PathBuf,
}

impl GitWorktreeExecutor {
    pub fn new(
        repository: impl AsRef<Path>,
        managed_root: impl AsRef<Path>,
    ) -> Result<Self, WorktreeError> {
        let repository =
            canonical_directory(repository.as_ref(), WorktreeError::InvalidRepository)?;
        if !is_git_worktree(&repository)? {
            return Err(WorktreeError::InvalidRepository);
        }
        let managed_root =
            canonical_directory(managed_root.as_ref(), WorktreeError::InvalidManagedRoot)?;
        Ok(Self {
            repository,
            managed_root,
        })
    }

    pub fn execute(
        &self,
        permit: &ConsumedPermit,
        command: &WorktreeCommand,
        now: Timestamp,
    ) -> WorktreeResult {
        let result = match command.operation {
            WorktreeOperation::Create => self.create(permit, command),
            WorktreeOperation::Remove => self.remove(permit, command),
        };
        let outcome = match &result {
            Ok(_) => EffectOutcome::Succeeded,
            Err(error) => EffectOutcome::Failed {
                code: error.code().to_owned(),
            },
        };
        WorktreeResult {
            result,
            receipt: receipt_for(permit, now, outcome),
        }
    }

    fn create(
        &self,
        permit: &ConsumedPermit,
        command: &WorktreeCommand,
    ) -> Result<WorktreeChange, WorktreeError> {
        if !request_matches(permit, command, &self.managed_root) {
            return Err(WorktreeError::PermissionDenied);
        }
        if command.repository != self.repository {
            return Err(WorktreeError::InvalidRepository);
        }
        if command.destination.exists() {
            return Err(WorktreeError::DestinationExists);
        }
        let parent = command
            .destination
            .parent()
            .ok_or(WorktreeError::InvalidDestination)?;
        let parent = canonical_directory(parent, WorktreeError::InvalidDestination)?;
        if parent != self.managed_root {
            return Err(WorktreeError::DestinationOutsideManagedRoot);
        }
        let status = Command::new("git")
            .args(["worktree", "add", "--detach"])
            .arg(&command.destination)
            .arg(&command.commit)
            .current_dir(&self.repository)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| WorktreeError::GitUnavailable)?;
        if !status.success() {
            return Err(WorktreeError::GitFailed);
        }
        let actual = current_commit(&command.destination)?;
        if actual != command.commit.to_ascii_lowercase() {
            let _ = remove_created_worktree(&self.repository, &command.destination);
            return Err(WorktreeError::GitFailed);
        }
        Ok(WorktreeChange::Created {
            path: command.destination.clone(),
            commit: command.commit.clone(),
        })
    }

    fn remove(
        &self,
        permit: &ConsumedPermit,
        command: &WorktreeCommand,
    ) -> Result<WorktreeChange, WorktreeError> {
        if !request_matches(permit, command, &self.managed_root) {
            return Err(WorktreeError::PermissionDenied);
        }
        if command.repository != self.repository {
            return Err(WorktreeError::InvalidRepository);
        }
        let destination =
            canonical_directory(&command.destination, WorktreeError::InvalidDestination)?;
        if destination.parent() != Some(self.managed_root.as_path()) {
            return Err(WorktreeError::DestinationOutsideManagedRoot);
        }
        if current_commit(&destination)? != command.commit.to_ascii_lowercase() {
            return Err(WorktreeError::CommitMismatch);
        }
        if !is_clean_worktree(&destination)? {
            return Err(WorktreeError::DirtyWorktree);
        }
        let status = Command::new("git")
            .args(["worktree", "remove", "--"])
            .arg(&command.destination)
            .current_dir(&self.repository)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| WorktreeError::GitUnavailable)?;
        if !status.success() {
            return Err(WorktreeError::GitFailed);
        }
        Ok(WorktreeChange::Removed {
            path: command.destination.clone(),
            commit: command.commit.clone(),
        })
    }
}

#[derive(Serialize)]
struct CommandSpec<'a> {
    operation: &'a str,
    repository: &'a str,
    destination: &'a str,
    commit: &'a str,
}

fn canonical_directory(path: &Path, error: WorktreeError) -> Result<PathBuf, WorktreeError> {
    if path.to_str().is_none() {
        return Err(error);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| error.clone())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(error);
    }
    let canonical = fs::canonicalize(path).map_err(|_| error.clone())?;
    if canonical.to_str().is_none() {
        return Err(error);
    }
    Ok(canonical)
}

fn is_exact_commit(commit: &str) -> bool {
    matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_git_worktree(repository: &Path) -> Result<bool, WorktreeError> {
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repository)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| WorktreeError::GitUnavailable)?;
    if output.stdout.len() > 16 {
        return Err(WorktreeError::GitFailed);
    }
    let value = std::str::from_utf8(&output.stdout).map_err(|_| WorktreeError::GitFailed)?;
    Ok(output.status.success() && value.trim() == "true")
}

fn request_matches(permit: &ConsumedPermit, command: &WorktreeCommand, root: &Path) -> bool {
    let request = permit.request();
    let authorized_root = match request.resource_scope() {
        ResourceScope::Path { root } => {
            canonical_directory(Path::new(root), WorktreeError::PermissionDenied).ok()
        }
        _ => None,
    };
    request.capability() == Capability::ProcessExecute
        && request.operation() == Operation::Execute
        && authorized_root.as_deref() == Some(root)
        && matches!(request.target(), EffectTarget::Process { program } if program == command.spec())
}

fn current_commit(worktree: &Path) -> Result<String, WorktreeError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(worktree)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| WorktreeError::GitUnavailable)?;
    if !output.status.success() || output.stdout.len() > 128 {
        return Err(WorktreeError::GitFailed);
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_ascii_lowercase())
        .map_err(|_| WorktreeError::GitFailed)
}

fn is_clean_worktree(worktree: &Path) -> Result<bool, WorktreeError> {
    let mut child = Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=normal"])
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| WorktreeError::GitUnavailable)?;
    let mut stdout = child.stdout.take().ok_or(WorktreeError::GitFailed)?;
    let mut byte = [0_u8; 1];
    let changed = stdout
        .read(&mut byte)
        .map_err(|_| WorktreeError::GitFailed)?
        != 0;
    if changed {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|_| WorktreeError::GitFailed)?;
    if changed {
        Ok(false)
    } else if status.success() {
        Ok(true)
    } else {
        Err(WorktreeError::GitFailed)
    }
}

fn remove_created_worktree(repository: &Path, destination: &Path) -> Result<(), WorktreeError> {
    let status = Command::new("git")
        .args(["worktree", "remove", "--force", "--"])
        .arg(destination)
        .current_dir(repository)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| WorktreeError::GitUnavailable)?;
    if status.success() {
        Ok(())
    } else {
        Err(WorktreeError::GitFailed)
    }
}

fn receipt_for(permit: &ConsumedPermit, now: Timestamp, outcome: EffectOutcome) -> EffectReceipt {
    let receipt_id = ReceiptId::new(format!(
        "receipt-worktree-{}",
        NEXT_RECEIPT_ID.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated receipt ID is valid");
    EffectReceipt::new(
        receipt_id,
        permit.permit().permit_id().clone(),
        permit.permit().request_digest().clone(),
        now,
        outcome,
    )
}
