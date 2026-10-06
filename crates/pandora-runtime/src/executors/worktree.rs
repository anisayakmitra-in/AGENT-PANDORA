use crate::{ConsumedPermit, receipt_id::allocate_effect_receipt_id};
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, EffectTarget, Operation, ResourceScope, Timestamp,
};
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

    pub fn repository(&self) -> &Path {
        &self.repository
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn commit(&self) -> &str {
        &self.commit
    }

    pub fn operation(&self) -> &'static str {
        self.operation.as_str()
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

    pub fn repository(&self) -> &Path {
        &self.repository
    }

    pub fn managed_root(&self) -> &Path {
        &self.managed_root
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
        let mut process = Command::new("git");
        configure_git_environment(&mut process);
        let status = process
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
        let mut process = Command::new("git");
        configure_git_environment(&mut process);
        let status = process
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

/// Variables forwarded to a git child. Named once so the helper and its tests
/// cannot drift apart: a test asserting against a stale copy of this list
/// would pass while the real allowlist changed.
const ALLOWED_CHILD_VARIABLES: [&str; 3] = ["PATH", "HOME", "USERPROFILE"];

/// The environment a git child is permitted to see.
///
/// Clearing the environment is the point of this function, not the allowlist.
/// The worktree executor runs `git` on behalf of a permit, and the parent
/// process may hold provider credentials, agent tokens, or CI secrets in its
/// environment that have no business reaching a repository child. Only the
/// variables git needs in order to run at all are forwarded.
///
/// PATH is required for the executable lookup of any helper git itself invokes
/// (hooks, filters, credential programs). HOME is required because git resolves
/// its global configuration through it. USERPROFILE is the Windows equivalent
/// and is forwarded when present so git does not fall back to a synthesised
/// path. On Windows, SystemRoot is mandatory for the MSYS runtime that git is
/// built on; without it git fails to start at all, so it is not optional
/// there.
fn configure_git_environment(command: &mut Command) {
    // `std::env::var_os` itself is not higher-ranked over its argument, so it is
    // wrapped rather than passed directly.
    configure_git_environment_from(command, |name: &str| std::env::var_os(name));
}

/// The allowlist rule, with the parent environment supplied by the caller.
///
/// Taking the lookup as a parameter is what lets a test verify every
/// allowlisted variable on every host. Reading `std::env` directly would make
/// the assertion conditional on what the machine happens to define, and HOME
/// is absent on a stock Windows host, so the rule would go unverified there.
fn configure_git_environment_from(
    command: &mut Command,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) {
    command.env_clear();
    for name in ALLOWED_CHILD_VARIABLES {
        if let Some(value) = lookup(name) {
            command.env(name, value);
        }
    }

    #[cfg(windows)]
    if let Some(system_root) = lookup("SystemRoot") {
        command.env("SystemRoot", &system_root);
        command.env("WINDIR", system_root);
    }
}

fn is_git_worktree(repository: &Path) -> Result<bool, WorktreeError> {
    let mut process = Command::new("git");
    configure_git_environment(&mut process);
    let output = process
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
    let mut process = Command::new("git");
    configure_git_environment(&mut process);
    let output = process
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
    let mut process = Command::new("git");
    configure_git_environment(&mut process);
    let mut child = process
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
    let mut process = Command::new("git");
    configure_git_environment(&mut process);
    let status = process
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
    let receipt_id = allocate_effect_receipt_id("worktree");
    EffectReceipt::new(
        receipt_id,
        permit.permit().permit_id().clone(),
        permit.permit().request_digest().clone(),
        now,
        outcome,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ALLOWED_CHILD_VARIABLES, configure_git_environment, configure_git_environment_from,
    };
    use std::collections::BTreeMap;
    use std::process::Command;

    /// Variables the Windows shell creates for itself when the environment
    /// block it is handed does not contain them. A cleared child still shows
    /// these, so a test that scans raw text would either false-positive on
    /// `PATHEXT` while looking for `PATH`, or pick `COMSPEC` as a canary and
    /// then fail for a reason that has nothing to do with the allowlist.
    const SHELL_SYNTHESIZED: [&str; 3] = ["COMSPEC", "PATHEXT", "PROMPT"];

    /// A parent environment that is defined everywhere, including on a stock
    /// Windows host where HOME is not set. Lets the forwarding rule be tested
    /// in full rather than only for the variables the machine happens to have.
    const SYNTHETIC_VALUES: [(&str, &str); 4] = [
        ("PATH", "/synthetic/bin"),
        ("HOME", "/synthetic/home"),
        ("USERPROFILE", "/synthetic/profile"),
        ("SystemRoot", "/synthetic/windows"),
    ];

    /// A parent environment that is defined everywhere, including on a stock
    /// Windows host where HOME is not set. Lets the forwarding rule be tested
    /// in full rather than only for the variables the machine happens to have.
    fn synthetic_parent() -> BTreeMap<String, String> {
        let mut parent: BTreeMap<String, String> = SYNTHETIC_VALUES
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        parent.insert("PANDORA_UNRELATED".to_owned(), "inherited".to_owned());
        parent
    }

    /// A command that prints its own environment, so a test can read what a
    /// child was actually given rather than inferring it from the parent's.
    ///
    /// `env` and `cmd /c set` are the only dependable environment dumps
    /// available without depending on the CLI under test. git has no such
    /// verb, which is why this probes the helper directly.
    fn environment_probe() -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args(["/c", "set"]);
            command
        }
        #[cfg(not(windows))]
        {
            Command::new("env")
        }
    }

    /// Parsed by exact name. Substring matching over raw output is unsound
    /// here: `PATH` is a substring of `PATHEXT`, which is how an earlier
    /// version of this test passed while PATH was in fact being dropped.
    fn parse_environment(text: &str) -> BTreeMap<String, String> {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    fn child_environment(clear: bool) -> BTreeMap<String, String> {
        let mut command = environment_probe();
        if clear {
            configure_git_environment(&mut command);
        }
        let output = command
            .stdin(std::process::Stdio::null())
            .output()
            .expect("environment probe should run");
        assert!(
            output.status.success(),
            "environment probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        parse_environment(&String::from_utf8(output.stdout).expect("probe emits utf-8"))
    }

    fn child_environment_from(parent: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut command = environment_probe();
        configure_git_environment_from(&mut command, lookup_from(parent));
        let output = command
            .stdin(std::process::Stdio::null())
            .output()
            .expect("environment probe should run");
        assert!(
            output.status.success(),
            "environment probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        parse_environment(&String::from_utf8(output.stdout).expect("probe emits utf-8"))
    }

    fn lookup_from(
        parent: &BTreeMap<String, String>,
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + '_ {
        move |name: &str| parent.get(name).map(std::ffi::OsString::from)
    }

    /// A variable that genuinely exists in this process and is neither on the
    /// allowlist nor synthesised by the shell, so it can stand in for a
    /// provider credential.
    ///
    /// This crate forbids unsafe code and `env::set_var` is unsafe on edition
    /// 2024, so the canary cannot be synthesised. Borrowing a real variable is
    /// stronger anyway: it is a value the environment actually set rather than
    /// one this test placed to suit itself.
    fn canary_from_parent() -> String {
        let mut borrowed: Vec<String> = std::env::vars()
            .map(|(name, _)| name)
            .filter(|name| !ALLOWED_CHILD_VARIABLES.contains(&name.as_str()))
            .filter(|name| !SHELL_SYNTHESIZED.contains(&name.as_str()))
            .filter(|name| !name.starts_with("SystemRoot") && name != "WINDIR")
            .collect();
        borrowed.sort();
        assert!(
            !borrowed.is_empty(),
            "the parent environment holds nothing outside the allowlist, so \
             this environment cannot prove the clearing works"
        );
        borrowed[0].clone()
    }

    #[test]
    fn a_parent_variable_is_invisible_to_a_cleared_child() {
        let canary = canary_from_parent();

        assert!(
            child_environment(false).contains_key(&canary),
            "precondition: the parent variable must be visible before clearing"
        );
        assert!(
            !child_environment(true).contains_key(&canary),
            "a non-allowlisted parent variable reached a cleared child"
        );
    }

    #[test]
    fn the_probe_would_notice_the_canary_if_the_environment_were_not_cleared() {
        // The negative control. Without it, the test above could still pass if
        // the child inherited nothing for an unrelated reason, such as the
        // probe failing to spawn. This proves the assertion can fail.
        let canary = canary_from_parent();

        assert!(
            child_environment(false).contains_key(&canary),
            "the probe cannot observe a variable it is expected to see, so the \
             clearing test proves nothing"
        );
    }

    #[test]
    fn the_allowlisted_variables_reach_the_cleared_child() {
        // Clearing must not take the variables git needs with it. PATH is
        // asserted because losing it breaks git for every user.
        let path = std::env::var_os("PATH").expect("PATH must be set in the parent");
        assert!(!path.is_empty());

        let environment = child_environment(true);

        assert_eq!(
            environment.get("PATH").map(String::as_str),
            Some(path.to_string_lossy().as_ref()),
            "PATH did not reach the cleared child unchanged"
        );
    }

    #[test]
    fn every_allowed_variable_is_actually_forwarded() {
        // Driven by a synthetic parent so the rule is verified in full on every
        // host. Reading the real environment would skip HOME wherever the host
        // does not define it, which is a stock Windows machine.
        let synthetic = synthetic_parent();
        let environment = child_environment_from(&synthetic);

        for name in ALLOWED_CHILD_VARIABLES {
            assert_eq!(
                environment.get(name).map(String::as_str),
                synthetic.get(name).map(String::as_str),
                "{name} is allowlisted but was not forwarded unchanged"
            );
        }
    }

    #[test]
    fn a_synthetic_parent_lets_every_allowlist_entry_be_verified() {
        // Guard on the guard: if the synthetic map stopped containing an
        // allowlisted name, the test above would compare against nothing and
        // pass for the wrong reason.
        let synthetic = synthetic_parent();
        for name in ALLOWED_CHILD_VARIABLES {
            assert!(
                synthetic.contains_key(name),
                "{name} is allowlisted but absent from the synthetic parent, so \
                 the forwarding rule cannot be verified for it"
            );
        }
    }

    #[test]
    fn the_cleared_child_sees_only_the_allowlist_and_shell_defaults() {
        // The strongest form of the claim: whatever the parent holds, the child
        // is left with the allowlist, the Windows-required SystemRoot pair, and
        // whatever the shell invents for itself.
        let environment = child_environment_from(&synthetic_parent());

        for name in environment.keys() {
            let permitted = ALLOWED_CHILD_VARIABLES.contains(&name.as_str())
                || SHELL_SYNTHESIZED.contains(&name.as_str())
                || name == "SystemRoot"
                || name == "WINDIR";
            assert!(
                permitted,
                "{name} reached a cleared child but is neither allowlisted nor \
                 a Windows system requirement"
            );
        }
    }

    #[test]
    fn the_allowlist_is_exactly_what_git_needs_and_nothing_else() {
        // Pins the allowlist rather than deriving its expectation from the
        // allowlist. An invariant of the form "whatever is allowlisted is
        // acceptable" cannot detect the allowlist being widened, which is the
        // one change that would leak a parent value to git. Widening this list
        // must be a deliberate, reviewed act, so it has to fail a test.
        assert_eq!(
            ALLOWED_CHILD_VARIABLES,
            ["PATH", "HOME", "USERPROFILE"],
            "the child environment allowlist changed. Widening it forwards a \
             parent value to git, so the change must be reviewed deliberately."
        );
    }

    #[cfg(windows)]
    #[test]
    fn system_root_is_forwarded_because_git_will_not_start_without_it() {
        // The Windows requirement is not part of ALLOWED_CHILD_VARIABLES
        // because it is mandatory rather than optional, so it needs its own
        // assertion. Without SystemRoot the MSYS runtime git is built on cannot
        // initialise and every worktree operation fails closed.
        let synthetic = synthetic_parent();
        let environment = child_environment_from(&synthetic);

        assert_eq!(
            environment.get("SystemRoot").map(String::as_str),
            Some("/synthetic/windows"),
            "SystemRoot did not reach the cleared child"
        );
        assert_eq!(
            environment.get("WINDIR").map(String::as_str),
            Some("/synthetic/windows"),
            "WINDIR did not reach the cleared child"
        );
    }

    #[test]
    fn a_synthetic_secret_does_not_reach_the_child() {
        // The whole point of the change, stated against a value whose name is
        // known to be absent from the allowlist rather than borrowed from the
        // host.
        let mut synthetic = synthetic_parent();
        synthetic.insert("PANDORA_PROVIDER_TOKEN".to_owned(), "s3cret".to_owned());
        let environment = child_environment_from(&synthetic);

        assert!(!environment.contains_key("PANDORA_PROVIDER_TOKEN"));
        assert!(!environment.values().any(|value| value.contains("s3cret")));
    }
}
