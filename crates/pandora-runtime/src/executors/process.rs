use super::filesystem::WorkspaceRoot;
use crate::ConsumedPermit;
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, EffectTarget, Operation, ReceiptId, ResourceScope,
    Timestamp,
};
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessError {
    UnsupportedProgram,
    UnsupportedArguments,
    InvalidOptions,
    PermissionDenied,
    CwdOutsideWorkspace,
    SpawnFailed,
    Io,
    OutputLimitExceeded,
    TimedOut,
    Cancelled,
}

impl ProcessError {
    fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedProgram => "unsupported_program",
            Self::UnsupportedArguments => "unsupported_arguments",
            Self::InvalidOptions => "invalid_options",
            Self::PermissionDenied => "permission_denied",
            Self::CwdOutsideWorkspace => "cwd_outside_workspace",
            Self::SpawnFailed => "spawn_failed",
            Self::Io => "process_io",
            Self::OutputLimitExceeded => "output_limit_exceeded",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationCommand {
    workspace: WorkspaceRoot,
    kind: VerificationKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VerificationKind {
    Check,
    Test,
    Format,
    Lint,
    Build,
    Status,
    Diff,
    Log,
    Refs,
}

impl VerificationCommand {
    pub fn cargo_check_locked(workspace: WorkspaceRoot) -> Self {
        Self {
            workspace,
            kind: VerificationKind::Check,
        }
    }

    pub fn cargo_test_locked(workspace: WorkspaceRoot) -> Self {
        Self {
            workspace,
            kind: VerificationKind::Test,
        }
    }

    pub fn cargo_fmt_check(workspace: WorkspaceRoot) -> Self {
        Self {
            workspace,
            kind: VerificationKind::Format,
        }
    }

    pub fn cargo_clippy_check(workspace: WorkspaceRoot) -> Self {
        Self {
            workspace,
            kind: VerificationKind::Lint,
        }
    }

    pub fn cargo_build_locked(workspace: WorkspaceRoot) -> Self {
        Self {
            workspace,
            kind: VerificationKind::Build,
        }
    }

    pub fn from_argv(
        program: &str,
        arguments: &[String],
        workspace: WorkspaceRoot,
    ) -> Result<Self, ProcessError> {
        if program == "git" {
            return match arguments {
                [command, short] if command == "status" && short == "--short" => Ok(Self {
                    workspace,
                    kind: VerificationKind::Status,
                }),
                [command, no_ext, unified]
                    if command == "diff"
                        && no_ext == "--no-ext-diff"
                        && unified == "--unified=3" =>
                {
                    Ok(Self {
                        workspace,
                        kind: VerificationKind::Diff,
                    })
                }
                [no_pager, command, oneline, decorate, count_flag, count]
                    if no_pager == "--no-pager"
                        && command == "log"
                        && oneline == "--oneline"
                        && decorate == "--decorate"
                        && count_flag == "-n"
                        && count == "20" =>
                {
                    Ok(Self {
                        workspace,
                        kind: VerificationKind::Log,
                    })
                }
                [command, format, sort, count, heads, remotes, tags]
                    if command == "for-each-ref"
                        && format == "--format=%(refname:short)"
                        && sort == "--sort=-committerdate"
                        && count == "--count=50"
                        && heads == "refs/heads"
                        && remotes == "refs/remotes"
                        && tags == "refs/tags" =>
                {
                    Ok(Self {
                        workspace,
                        kind: VerificationKind::Refs,
                    })
                }
                _ => Err(ProcessError::UnsupportedArguments),
            };
        }
        if program != "cargo" {
            return Err(ProcessError::UnsupportedProgram);
        }
        let kind = match arguments {
            [command, locked] if command == "check" && locked == "--locked" => {
                VerificationKind::Check
            }
            [command, locked] if command == "test" && locked == "--locked" => {
                VerificationKind::Test
            }
            [command, all, separator, check]
                if command == "fmt"
                    && all == "--all"
                    && separator == "--"
                    && check == "--check" =>
            {
                VerificationKind::Format
            }
            [
                command,
                workspace,
                all_targets,
                locked,
                separator,
                flag,
                level,
            ] if command == "clippy"
                && workspace == "--workspace"
                && all_targets == "--all-targets"
                && locked == "--locked"
                && separator == "--"
                && flag == "-D"
                && level == "warnings" =>
            {
                VerificationKind::Lint
            }
            [command, locked] if command == "build" && locked == "--locked" => {
                VerificationKind::Build
            }
            _ => return Err(ProcessError::UnsupportedArguments),
        };
        Ok(Self { workspace, kind })
    }

    pub(crate) fn from_spec(spec: &str, workspace: WorkspaceRoot) -> Result<Self, ProcessError> {
        match spec {
            "cargo check --locked" => Ok(Self::cargo_check_locked(workspace)),
            "cargo test --locked" => Ok(Self::cargo_test_locked(workspace)),
            "cargo fmt --all -- --check" => Ok(Self::cargo_fmt_check(workspace)),
            "cargo clippy --workspace --all-targets --locked -- -D warnings" => {
                Ok(Self::cargo_clippy_check(workspace))
            }
            "cargo build --locked" => Ok(Self::cargo_build_locked(workspace)),
            "git status --short" => Ok(Self {
                workspace,
                kind: VerificationKind::Status,
            }),
            "git diff --no-ext-diff --unified=3" => Ok(Self {
                workspace,
                kind: VerificationKind::Diff,
            }),
            "git --no-pager log --oneline --decorate -n 20" => Ok(Self {
                workspace,
                kind: VerificationKind::Log,
            }),
            "git for-each-ref --format=%(refname:short) --sort=-committerdate --count=50 refs/heads refs/remotes refs/tags" => {
                Ok(Self {
                    workspace,
                    kind: VerificationKind::Refs,
                })
            }
            _ => Err(ProcessError::UnsupportedArguments),
        }
    }

    pub fn spec(&self) -> &'static str {
        match self.kind {
            VerificationKind::Check => "cargo check --locked",
            VerificationKind::Test => "cargo test --locked",
            VerificationKind::Format => "cargo fmt --all -- --check",
            VerificationKind::Lint => {
                "cargo clippy --workspace --all-targets --locked -- -D warnings"
            }
            VerificationKind::Build => "cargo build --locked",
            VerificationKind::Status => "git status --short",
            VerificationKind::Diff => "git diff --no-ext-diff --unified=3",
            VerificationKind::Log => "git --no-pager log --oneline --decorate -n 20",
            VerificationKind::Refs => {
                "git for-each-ref --format=%(refname:short) --sort=-committerdate --count=50 refs/heads refs/remotes refs/tags"
            }
        }
    }

    fn arguments(&self) -> &'static [&'static str] {
        match self.kind {
            VerificationKind::Check => &["check", "--locked"],
            VerificationKind::Test => &["test", "--locked"],
            VerificationKind::Format => &["fmt", "--all", "--", "--check"],
            VerificationKind::Lint => &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            VerificationKind::Build => &["build", "--locked"],
            VerificationKind::Status => &["status", "--short"],
            VerificationKind::Diff => &["diff", "--no-ext-diff", "--unified=3"],
            VerificationKind::Log => &["--no-pager", "log", "--oneline", "--decorate", "-n", "20"],
            VerificationKind::Refs => &[
                "for-each-ref",
                "--format=%(refname:short)",
                "--sort=-committerdate",
                "--count=50",
                "refs/heads",
                "refs/remotes",
                "refs/tags",
            ],
        }
    }

    fn program(&self) -> &'static str {
        match self.kind {
            VerificationKind::Status
            | VerificationKind::Diff
            | VerificationKind::Log
            | VerificationKind::Refs => "git",
            _ => "cargo",
        }
    }

    fn workspace(&self) -> &WorkspaceRoot {
        &self.workspace
    }
}

#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct VerificationOptions {
    timeout: Duration,
    max_output_bytes: usize,
    cancellation: CancellationToken,
}

impl VerificationOptions {
    pub fn new(
        timeout: Duration,
        max_output_bytes: usize,
        cancellation: CancellationToken,
    ) -> Result<Self, ProcessError> {
        if timeout.is_zero() || max_output_bytes == 0 || max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(ProcessError::InvalidOptions);
        }
        Ok(Self {
            timeout,
            max_output_bytes,
            cancellation,
        })
    }
}

impl Default for VerificationOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_OUTPUT_BYTES,
            cancellation: CancellationToken::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessOutput {
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ProcessOutput {
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

pub struct ProcessResult {
    result: Result<ProcessOutput, ProcessError>,
    receipt: EffectReceipt,
}

impl ProcessResult {
    pub fn result(&self) -> Result<&ProcessOutput, &ProcessError> {
        self.result.as_ref()
    }

    pub fn into_result(self) -> Result<ProcessOutput, ProcessError> {
        self.result
    }

    pub fn receipt(&self) -> &EffectReceipt {
        &self.receipt
    }
}

pub struct ProcessExecutor {
    workspace: WorkspaceRoot,
}

impl ProcessExecutor {
    pub fn new(workspace: WorkspaceRoot) -> Self {
        Self { workspace }
    }

    pub fn run_verification(
        &self,
        permit: &ConsumedPermit,
        command: &VerificationCommand,
        options: &VerificationOptions,
        now: Timestamp,
    ) -> ProcessResult {
        let result = if !request_matches(permit, command) {
            Err(ProcessError::PermissionDenied)
        } else if self.workspace.root() != command.workspace().root() {
            Err(ProcessError::CwdOutsideWorkspace)
        } else if options.cancellation.is_cancelled() {
            Err(ProcessError::Cancelled)
        } else if options.timeout.is_zero() || options.max_output_bytes == 0 {
            Err(ProcessError::InvalidOptions)
        } else {
            run_child(command, options)
        };
        let outcome = match &result {
            Ok(output) if output.exit_code() == Some(0) => EffectOutcome::Succeeded,
            Ok(_) => EffectOutcome::Failed {
                code: "verification_failed".to_owned(),
            },
            Err(error) => EffectOutcome::Failed {
                code: error.code().to_owned(),
            },
        };
        ProcessResult {
            result,
            receipt: receipt_for(permit, now, outcome),
        }
    }
}

fn request_matches(permit: &ConsumedPermit, command: &VerificationCommand) -> bool {
    let request = permit.request();
    request.capability() == Capability::ProcessExecute
        && request.operation() == Operation::Execute
        && matches!(
            request.resource_scope(),
            ResourceScope::Workspace { .. } | ResourceScope::Path { .. }
        )
        && matches!(request.target(), EffectTarget::Process { program } if program == command.spec())
}

fn run_child(
    command: &VerificationCommand,
    options: &VerificationOptions,
) -> Result<ProcessOutput, ProcessError> {
    let mut process = Command::new(command.program());
    configure_process_group(&mut process);
    configure_runtime_environment(&mut process);
    let mut child = process
        .args(command.arguments())
        .current_dir(command.workspace().root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| ProcessError::SpawnFailed)?;
    let stdout = child.stdout.take().ok_or(ProcessError::Io)?;
    let stderr = child.stderr.take().ok_or(ProcessError::Io)?;
    let stdout_rx = spawn_reader(stdout, options.max_output_bytes);
    let stderr_rx = spawn_reader(stderr, options.max_output_bytes);
    let mut stdout_output = None;
    let mut stderr_output = None;
    let started = Instant::now();
    loop {
        if options.cancellation.is_cancelled() {
            stop_child(&mut child);
            return Err(ProcessError::Cancelled);
        }
        if started.elapsed() >= options.timeout {
            stop_child(&mut child);
            return Err(ProcessError::TimedOut);
        }
        if let Err(error) = poll_reader(&stdout_rx, &mut stdout_output) {
            stop_child(&mut child);
            return Err(error);
        }
        if let Err(error) = poll_reader(&stderr_rx, &mut stderr_output) {
            stop_child(&mut child);
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = receive_output(stdout_output, &stdout_rx)?;
                let stderr = receive_output(stderr_output, &stderr_rx)?;
                return Ok(ProcessOutput {
                    exit_code: status.code(),
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {}
            Err(_) => {
                stop_child(&mut child);
                return Err(ProcessError::Io);
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(crate) fn configure_runtime_environment(process: &mut Command) {
    process.env_clear();
    process.env("CARGO_TERM_COLOR", "never");

    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        process.env("SystemRoot", &system_root);
        process.env("WINDIR", system_root);
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> mpsc::Receiver<Result<Vec<u8>, ProcessError>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = read_bounded_output(&mut reader, limit);
        let _ = sender.send(result);
    });
    receiver
}

fn poll_reader(
    receiver: &mpsc::Receiver<Result<Vec<u8>, ProcessError>>,
    output: &mut Option<Vec<u8>>,
) -> Result<(), ProcessError> {
    match receiver.try_recv() {
        Ok(Ok(bytes)) => {
            *output = Some(bytes);
            Ok(())
        }
        Ok(Err(error)) => Err(error),
        Err(mpsc::TryRecvError::Empty) => Ok(()),
        Err(mpsc::TryRecvError::Disconnected) => Err(ProcessError::Io),
    }
}

fn receive_output(
    output: Option<Vec<u8>>,
    receiver: &mpsc::Receiver<Result<Vec<u8>, ProcessError>>,
) -> Result<Vec<u8>, ProcessError> {
    match output {
        Some(bytes) => Ok(bytes),
        None => receiver.recv().map_err(|_| ProcessError::Io)?,
    }
}

fn stop_child(child: &mut Child) {
    terminate_process_tree(child);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
pub(crate) fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(windows)]
pub(crate) fn configure_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(0x0000_0200);
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn configure_process_group(_: &mut Command) {}

#[cfg(unix)]
pub(crate) fn terminate_process_tree(child: &Child) {
    let process_group = format!("-{}", child.id());
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &process_group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
pub(crate) fn terminate_process_tree(child: &Child) {
    let process_id = child.id().to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", &process_id, "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn terminate_process_tree(_: &Child) {}

fn read_bounded_output(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, ProcessError> {
    let limit = limit.checked_add(1).ok_or(ProcessError::InvalidOptions)?;
    let mut output = Vec::new();
    reader
        .take(limit as u64)
        .read_to_end(&mut output)
        .map_err(|_| ProcessError::Io)?;
    if output.len() > limit - 1 {
        return Err(ProcessError::OutputLimitExceeded);
    }
    Ok(output)
}

fn receipt_for(permit: &ConsumedPermit, now: Timestamp, outcome: EffectOutcome) -> EffectReceipt {
    let receipt_id = ReceiptId::new(format!(
        "receipt-process-{}",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Parliament, ReferenceMonitor};
    use pandora_types::{
        ExecutionId, GeneId, OperationRequest, PolicyContext, PrincipalId, SessionId,
    };

    #[test]
    fn accepts_only_locked_cargo_check() {
        let workspace = Workspace::new();
        let command = VerificationCommand::cargo_check_locked(workspace.root.clone());

        assert_eq!(command.spec(), "cargo check --locked");
    }

    #[test]
    fn accepts_only_locked_cargo_test() {
        let workspace = Workspace::new();
        let arguments = vec!["test".to_owned(), "--locked".to_owned()];
        let command = VerificationCommand::from_argv("cargo", &arguments, workspace.root.clone())
            .expect("the fixed test command should be accepted");

        assert_eq!(command.spec(), "cargo test --locked");
    }

    #[test]
    fn accepts_only_locked_cargo_format_check() {
        let workspace = Workspace::new();
        let arguments = vec![
            "fmt".to_owned(),
            "--all".to_owned(),
            "--".to_owned(),
            "--check".to_owned(),
        ];
        let command = VerificationCommand::from_argv("cargo", &arguments, workspace.root.clone())
            .expect("the fixed format command should be accepted");

        assert_eq!(command.spec(), "cargo fmt --all -- --check");
    }

    #[test]
    fn accepts_only_locked_cargo_clippy_check() {
        let workspace = Workspace::new();
        let arguments = vec![
            "clippy".to_owned(),
            "--workspace".to_owned(),
            "--all-targets".to_owned(),
            "--locked".to_owned(),
            "--".to_owned(),
            "-D".to_owned(),
            "warnings".to_owned(),
        ];
        let command = VerificationCommand::from_argv("cargo", &arguments, workspace.root.clone())
            .expect("the fixed lint command should be accepted");

        assert_eq!(
            command.spec(),
            "cargo clippy --workspace --all-targets --locked -- -D warnings"
        );
    }

    #[test]
    fn accepts_only_locked_cargo_build() {
        let workspace = Workspace::new();
        let arguments = vec!["build".to_owned(), "--locked".to_owned()];
        let command = VerificationCommand::from_argv("cargo", &arguments, workspace.root.clone())
            .expect("the fixed build command should be accepted");

        assert_eq!(command.spec(), "cargo build --locked");
    }

    #[test]
    fn accepts_only_read_only_git_status() {
        let workspace = Workspace::new();
        let arguments = vec!["status".to_owned(), "--short".to_owned()];
        let command = VerificationCommand::from_argv("git", &arguments, workspace.root.clone())
            .expect("the fixed git status command should be accepted");

        assert_eq!(command.spec(), "git status --short");
    }

    #[test]
    fn accepts_only_read_only_git_diff() {
        let workspace = Workspace::new();
        let arguments = vec![
            "diff".to_owned(),
            "--no-ext-diff".to_owned(),
            "--unified=3".to_owned(),
        ];
        let command = VerificationCommand::from_argv("git", &arguments, workspace.root.clone())
            .expect("the fixed git diff command should be accepted");

        assert_eq!(command.spec(), "git diff --no-ext-diff --unified=3");
    }

    #[test]
    fn rejects_other_git_commands() {
        let workspace = Workspace::new();
        let arguments = vec!["diff".to_owned()];

        assert_eq!(
            VerificationCommand::from_argv("git", &arguments, workspace.root.clone()),
            Err(ProcessError::UnsupportedArguments)
        );
    }

    #[test]
    fn rejects_git_diff_options_outside_the_allowlist() {
        let workspace = Workspace::new();
        let arguments = vec!["diff".to_owned(), "--no-ext-diff".to_owned()];

        assert_eq!(
            VerificationCommand::from_argv("git", &arguments, workspace.root.clone()),
            Err(ProcessError::UnsupportedArguments)
        );
    }

    #[test]
    fn accepts_only_read_only_git_log() {
        let workspace = Workspace::new();
        let arguments = vec![
            "--no-pager".to_owned(),
            "log".to_owned(),
            "--oneline".to_owned(),
            "--decorate".to_owned(),
            "-n".to_owned(),
            "20".to_owned(),
        ];
        let command = VerificationCommand::from_argv("git", &arguments, workspace.root.clone())
            .expect("the fixed git log command should be accepted");

        assert_eq!(
            command.spec(),
            "git --no-pager log --oneline --decorate -n 20"
        );
    }

    #[test]
    fn rejects_git_log_options_outside_the_allowlist() {
        let workspace = Workspace::new();
        let arguments = vec!["--no-pager".to_owned(), "log".to_owned()];

        assert_eq!(
            VerificationCommand::from_argv("git", &arguments, workspace.root.clone()),
            Err(ProcessError::UnsupportedArguments)
        );
    }

    #[test]
    fn accepts_only_read_only_git_refs() {
        let workspace = Workspace::new();
        let arguments = vec![
            "for-each-ref".to_owned(),
            "--format=%(refname:short)".to_owned(),
            "--sort=-committerdate".to_owned(),
            "--count=50".to_owned(),
            "refs/heads".to_owned(),
            "refs/remotes".to_owned(),
            "refs/tags".to_owned(),
        ];
        let command = VerificationCommand::from_argv("git", &arguments, workspace.root.clone())
            .expect("the fixed git refs command should be accepted");

        assert_eq!(
            command.spec(),
            "git for-each-ref --format=%(refname:short) --sort=-committerdate --count=50 refs/heads refs/remotes refs/tags"
        );
    }

    #[cfg(windows)]
    #[test]
    fn preserves_only_windows_system_runtime_environment() {
        let mut process = Command::new("git");
        configure_runtime_environment(&mut process);

        let variables = process
            .get_envs()
            .filter_map(|(name, value)| value.map(|value| (name, value)))
            .collect::<Vec<_>>();
        let expected_system_root = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        assert!(variables.iter().any(|(name, value)| {
            *name == "SystemRoot" && *value == expected_system_root.as_os_str()
        }));
        assert!(variables.iter().any(|(name, _)| *name == "WINDIR"));
        assert!(!variables.iter().any(|(name, _)| *name == "PATH"));
        assert!(!variables.iter().any(|(name, _)| *name == "USERPROFILE"));
    }

    #[test]
    fn rejects_git_refs_options_outside_the_allowlist() {
        let workspace = Workspace::new();
        let arguments = vec!["for-each-ref".to_owned(), "--format=%(refname)".to_owned()];

        assert_eq!(
            VerificationCommand::from_argv("git", &arguments, workspace.root.clone()),
            Err(ProcessError::UnsupportedArguments)
        );
    }

    #[test]
    fn rejects_extra_test_arguments() {
        let workspace = Workspace::new();
        let arguments = vec![
            "test".to_owned(),
            "--locked".to_owned(),
            "--nocapture".to_owned(),
        ];

        assert_eq!(
            VerificationCommand::from_argv("cargo", &arguments, workspace.root.clone()),
            Err(ProcessError::UnsupportedArguments)
        );
    }

    #[test]
    fn rejects_shell_and_unknown_programs() {
        let workspace = Workspace::new();
        let shell_args = vec!["-c".to_owned(), "echo unsafe".to_owned()];
        let unknown_args = Vec::new();

        assert_eq!(
            VerificationCommand::from_argv("sh", &shell_args, workspace.root.clone()),
            Err(ProcessError::UnsupportedProgram)
        );
        assert_eq!(
            VerificationCommand::from_argv("unknown", &unknown_args, workspace.root.clone()),
            Err(ProcessError::UnsupportedProgram)
        );
    }

    #[test]
    fn rejects_shell_metacharacters_in_arguments() {
        let workspace = Workspace::new();
        let args = vec!["check".to_owned(), "--locked".to_owned(), "&&".to_owned()];

        assert_eq!(
            VerificationCommand::from_argv("cargo", &args, workspace.root.clone()),
            Err(ProcessError::UnsupportedArguments)
        );
    }

    #[test]
    fn rejects_a_command_outside_the_executor_workspace() {
        let executor_workspace = Workspace::new();
        let command_workspace = Workspace::new();
        let command = VerificationCommand::cargo_check_locked(command_workspace.root.clone());
        let executor = ProcessExecutor::new(executor_workspace.root.clone());
        let permit = permit_for("cargo check --locked", Capability::ProcessExecute);

        let result = executor.run_verification(
            &permit,
            &command,
            &VerificationOptions::default(),
            Timestamp::from_unix_seconds(10),
        );

        assert_eq!(result.result(), Err(&ProcessError::CwdOutsideWorkspace));
    }

    #[test]
    fn rejects_a_filesystem_permit_for_process_execution() {
        let workspace = Workspace::new();
        let command = VerificationCommand::cargo_check_locked(workspace.root.clone());
        let executor = ProcessExecutor::new(workspace.root.clone());
        let permit = permit_for("cargo check --locked", Capability::FilesystemRead);

        let result = executor.run_verification(
            &permit,
            &command,
            &VerificationOptions::default(),
            Timestamp::from_unix_seconds(10),
        );

        assert_eq!(result.result(), Err(&ProcessError::PermissionDenied));
    }

    #[test]
    fn bounded_output_rejects_more_than_the_configured_limit() {
        let mut output = std::io::Cursor::new(b"12345".to_vec());

        assert_eq!(
            read_bounded_output(&mut output, 4),
            Err(ProcessError::OutputLimitExceeded)
        );
    }

    struct Workspace {
        root: WorkspaceRoot,
        path: std::path::PathBuf,
    }

    impl Workspace {
        fn new() -> Self {
            let path = crate::test_support::new_temp_dir("pandora-process-test").unwrap();
            let root = WorkspaceRoot::new(&path).unwrap();
            Self { root, path }
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn permit_for(spec: &str, capability: Capability) -> ConsumedPermit {
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            crate::test_support::execution_profile("process"),
            GeneId::new("verification").unwrap(),
            None,
            capability,
            Operation::Execute,
            EffectTarget::process(spec),
            ResourceScope::workspace("workspace-1"),
        )
        .unwrap();
        let context = PolicyContext::new(1, [capability], []);
        let monitor = ReferenceMonitor::new_with_policy(context.clone(), 60);
        let decision = Parliament::new(1).decide(&request, &context);
        let permit = monitor
            .authorize(request.clone(), decision, Timestamp::from_unix_seconds(10))
            .unwrap();
        monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(10))
            .unwrap()
    }
}
