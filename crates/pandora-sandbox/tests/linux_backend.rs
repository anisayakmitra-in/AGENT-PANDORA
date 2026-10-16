//! End-to-end tests for the Linux backend, run against the real helper binary
//! over a real pipe.
//!
//! These are the only tests in the crate that exercise confinement as a
//! behaviour rather than as a type. They spawn
//! `env!("CARGO_BIN_EXE_pandora-sandbox-helper")` — the same binary the executors
//! will spawn — and read exactly one frame back.
//!
//! # The report channel
//!
//! A **dedicated** pipe, created by the parent, whose write end is the only thing
//! the helper is given. The parent clears close-on-exec on that write end so the
//! child inherits it, passes the descriptor number in `argv`, and re-arms the flag
//! the instant the spawn returns so nothing spawned later can inherit it. The
//! helper wraps the number, writes its frame, re-arms the flag, and closes the
//! descriptor before `exec`.
//!
//! The target therefore keeps its own stdin, stdout and stderr untouched, holds no
//! part of the report channel, and the parent's read reaches EOF deterministically
//! once the helper has written. Reading to EOF and requiring the byte count to
//! match the declared frame length exactly is what proves one frame arrived and
//! nothing else did.
//!
//! # The gates fail, not skip
//!
//! Every test here is `#[cfg(target_os = "linux")]`, and each asserts the
//! behaviour the runner's kernel should provide. A runner without Landlock or
//! seccomp fails these tests rather than passing them vacuously — a confinement
//! claim that is silently skipped on the platform it is meant to cover is the
//! failure this crate exists to prevent. There are no `#[ignore]`s and no skip
//! branches.
//!
//! # One descriptor number is a process-wide resource
//!
//! A descriptor with close-on-exec cleared is inherited by *anything* spawned
//! while it is open, including a spawn from another test's thread. The window is
//! held under a mutex, so no other real-helper run in this binary can overlap it,
//! and the read has a timeout so that a leak which still occurs becomes a refusal
//! rather than an indefinite hang.

#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

use pandora_sandbox::report::{Mechanisms, Observation, Proofs};
use pandora_sandbox::{
    ConfinementOutcome, HelperReport, RefusalReason, SandboxProfile, VerificationReport,
    accept_verified,
};
use rustix::io::{FdFlags, fcntl_setfd};
use std::io::{PipeReader, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

const HELPER: &str = env!("CARGO_BIN_EXE_pandora-sandbox-helper");

/// How long the parent waits for one frame before it stops waiting.
///
/// Confinement, four probes and an `exec` all happen inside this window on a CI
/// runner. It is generous on purpose: a timeout that a healthy helper could trip
/// would turn a working sandbox into a refusal and hide a real defect.
const REPORT_TIMEOUT: Duration = Duration::from_secs(60);

/// Serialises the region in which a report write end is open with close-on-exec
/// cleared, so two real-helper runs in this binary cannot both hold one.
static REPORT_CHANNEL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A workspace root and the profile JSON that asks for it to be confined.
fn fixtures(allow_unsandboxed: bool) -> (PathBuf, String) {
    let base = std::env::temp_dir().join(format!("pandora-linux-test-{}", std::process::id()));
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("the workspace can be created");
    let profile = serde_json::json!({
        "version": 1,
        "filesystem": { "workspace_only": [workspace.display().to_string()] },
        "network": "deny_all",
        "allow_unsandboxed": allow_unsandboxed,
    })
    .to_string();
    (workspace, profile)
}

/// Everything the helper said, plus the target's own output.
struct Report {
    frame: Option<HelperReport>,
    stdout: String,
    stderr: String,
}

/// Runs the real helper over a dedicated pipe and reads that pipe to EOF.
///
/// `args` is given the descriptor number the target will inherit, so a test can
/// construct arguments that point at the report descriptor directly.
fn run_helper(
    profile: &str,
    env: &[(&str, &str)],
    target: &str,
    args: impl FnOnce(i32) -> Vec<String>,
) -> Result<Report, String> {
    // The cleared-close-on-exec window starts here and is held until the spawn
    // returns and the flag is re-armed, so no other real-helper run can leak into
    // it.
    let _channel = REPORT_CHANNEL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let (reader, writer) = std::io::pipe().map_err(|error| error.to_string())?;
    let report_fd = writer.as_raw_fd();
    // Clear close-on-exec on the write end only, so the child inherits exactly
    // this descriptor and nothing else new.
    fcntl_setfd(&writer, FdFlags::empty())
        .map_err(|error| format!("could not clear CLOEXEC on the write end: {error}"))?;

    let mut command = Command::new(HELPER);
    command
        .arg(report_fd.to_string())
        .arg(target)
        .args(args(report_fd))
        // Cleared *before* anything is set: clearing afterwards would wipe the
        // profile and the helper would exit before it ever wrote a frame.
        .env_clear()
        .env("PANDORA_SANDBOX_PROFILE_JSON", profile)
        // The target's own streams. The report channel is not one of them.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }

    let mut child = command.spawn().map_err(|error| error.to_string())?;
    // Re-armed the instant the spawn returns, so nothing spawned later inherits
    // the write end. This is the mitigation for the inherited-descriptor race.
    fcntl_setfd(&writer, FdFlags::CLOEXEC)
        .map_err(|error| format!("could not restore CLOEXEC on the write end: {error}"))?;
    // The parent's copy goes now. EOF on the read end then depends only on the
    // helper closing its own.
    drop(writer);

    let bytes = read_with_timeout(reader, REPORT_TIMEOUT)?;
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    let mut stderr = String::new();
    if let Some(mut sink) = child.stderr.take() {
        let _ = sink.read_to_string(&mut stderr);
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        stderr = format!("{stderr}(exit {status:?})");
    }

    let frame = decode_one_frame(&bytes)?;
    Ok(Report {
        frame,
        stdout,
        stderr,
    })
}

/// Reads a report pipe to EOF, or gives up.
///
/// A leaked write end — one inherited by something else spawned while the flag
/// was cleared — keeps the read from ever reaching EOF, and there is no way to
/// detect that other than by waiting. So a wait that outlasts its budget becomes
/// a refusal rather than an indefinite block.
fn read_with_timeout(mut reader: PipeReader, timeout: Duration) -> Result<Vec<u8>, String> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = reader.read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(read);
    });

    match receiver.recv_timeout(timeout) {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => Err(format!("the report pipe failed: {error}")),
        Err(RecvTimeoutError::Timeout) => {
            Err("the report pipe never reached EOF within the timeout".to_owned())
        }
        Err(RecvTimeoutError::Disconnected) => {
            Err("the report reader ended without producing anything".to_owned())
        }
    }
}

/// The length the frame declares for itself, or zero if there is no readable
/// prefix.
fn expected_length(bytes: &[u8]) -> usize {
    if bytes.len() < pandora_sandbox::REPORT_LENGTH_HEX {
        return 0;
    }
    let prefix = match std::str::from_utf8(&bytes[..pandora_sandbox::REPORT_LENGTH_HEX]) {
        Ok(text) => text,
        Err(_) => return 0,
    };
    match usize::from_str_radix(prefix, 16) {
        Ok(declared) => pandora_sandbox::REPORT_LENGTH_HEX + declared,
        Err(_) => 0,
    }
}

/// Decodes exactly one frame, requiring EOF with zero trailing bytes.
///
/// This is the property the pipe is read to prove, so it is checked here rather
/// than assumed: one extra byte is a second claim and must be an error.
fn decode_one_frame(bytes: &[u8]) -> Result<Option<HelperReport>, String> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let declared = expected_length(bytes);
    if declared == 0 || bytes.len() != declared {
        return Err(format!(
            "expected exactly one frame of {declared} bytes, got {}",
            bytes.len()
        ));
    }
    pandora_sandbox::decode_report(&bytes[..declared])
        .map(Some)
        .map_err(|error| error.to_string())
}

/// The parent's decision for a report that came back over the pipe.
fn accepted(report: &HelperReport, workspace: &Path) -> ConfinementOutcome {
    accept_verified(
        Some(&VerificationReport::new(
            report.reported().clone(),
            None,
            *report.proofs(),
            report.mechanisms().clone(),
        )),
        &SandboxProfile::new(vec![workspace.to_path_buf()]).expect("the profile is valid"),
    )
}

fn profile_requested(
    workspace: &Path,
) -> std::collections::BTreeSet<pandora_sandbox::RequestedControl> {
    SandboxProfile::new(vec![workspace.to_path_buf()])
        .expect("the profile is valid")
        .requested_controls()
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// The kernel on this runner must be able to give the backend what it requires.
///
/// This is the gate the "no silent skip" rule hangs on: if it fails, every other
/// test here fails with it, because none of them can prove anything on a host
/// with no confinement.
#[test]
fn the_running_kernel_supports_the_required_landlock_abi() {
    let abi = pandora_sandbox::linux::detected_abi().expect("this runner's kernel has no Landlock");
    assert!(
        pandora_sandbox::linux::abi_version(abi) >= pandora_sandbox::linux::REQUIRED_ABI,
        "this runner's Landlock ABI is {abi:?}, below the required {}",
        pandora_sandbox::linux::REQUIRED_ABI
    );
}

// ---------------------------------------------------------------------------
// The main claim
// ---------------------------------------------------------------------------

#[test]
fn a_confined_helper_verifies_every_requested_control() {
    let (workspace, profile) = fixtures(false);

    let report = run_helper(&profile, &[], "/bin/true", |_| Vec::new()).expect("the helper ran");
    let frame = report.frame.expect("the helper wrote a frame");
    let proofs = frame.proofs();
    let mechanisms = frame.mechanisms();

    assert_eq!(
        proofs.outside_write,
        Observation::Denied,
        "a write outside every allowed path must be denied; stderr={}; proofs={:?}",
        report.stderr,
        proofs
    );
    assert_eq!(
        proofs.canary_read,
        Observation::Denied,
        "a read of a canary outside every allowed path must be denied"
    );
    assert_eq!(
        proofs.inet_socket,
        Observation::Denied,
        "socket(AF_INET) must be denied"
    );
    assert_eq!(
        proofs.inside_write,
        Observation::Allowed,
        "a write inside the workspace must still succeed, or the other three \
         denials prove nothing"
    );

    assert_eq!(mechanisms.filesystem, "landlock");
    assert_eq!(mechanisms.network, "seccomp_bpf");
    assert!(
        mechanisms
            .denied_socket_families
            .contains(&"AF_INET".to_owned())
            && mechanisms
                .denied_socket_families
                .contains(&"AF_INET6".to_owned()),
        "both internet families must be named, because AF_UNIX is not denied"
    );
    assert!(
        mechanisms.rlimits.contains(&"RLIMIT_CORE".to_owned()),
        "core dumps must be disabled, and the report must say so"
    );

    let outcome = accepted(&frame, &workspace);
    assert!(
        outcome.satisfies(&profile_requested(&workspace)),
        "the parent must accept an honest confined helper, or the refusals \
         elsewhere would be vacuous; stderr: {}",
        report.stderr
    );
    assert_eq!(outcome.verified().len(), 3);
}

// ---------------------------------------------------------------------------
// Negative controls
// ---------------------------------------------------------------------------

/// With confinement explicitly disabled by the operator, the same probes must
/// show the writes, reads and sockets all succeeding.
///
/// This is what stops `outside_write: Denied` from being a missing directory
/// rather than a working sandbox. Without it, every denial above is
/// unfalsifiable.
#[test]
fn with_confinement_disabled_the_same_probes_succeed() {
    let (workspace, profile) = fixtures(true);

    let report = run_helper(&profile, &[], "/bin/true", |_| Vec::new()).expect("the helper ran");
    let frame = report.frame.expect("the helper wrote a frame");
    let proofs = frame.proofs();

    assert_eq!(
        proofs.outside_write,
        Observation::Allowed,
        "with confinement disabled the outside write must succeed, or the probe is \
         not testing confinement"
    );
    assert_eq!(
        proofs.canary_read,
        Observation::Allowed,
        "with confinement disabled the canary must be readable"
    );
    assert_eq!(
        proofs.inet_socket,
        Observation::Allowed,
        "with confinement disabled the socket must open"
    );
    assert_eq!(
        proofs.inside_write,
        Observation::Allowed,
        "the workspace write is allowed either way"
    );
    assert!(
        frame.mechanisms().filesystem.is_empty(),
        "no mechanism may be named when none was applied"
    );
    let _ = workspace;
}

/// A helper that cannot report must leave the parent with no frame at all, which
/// is a refusal rather than a weaker answer.
///
/// The report descriptor is deliberately unparseable, so the helper exits before
/// it writes anything. That is the same end state as a helper killed before it
/// reports, and it is deterministic rather than a race with the write.
///
/// Spawned directly rather than through [`run_helper`], because this is the one
/// case where the parent must *not* hand over a usable descriptor.
#[test]
fn a_helper_that_cannot_report_yields_no_frame() {
    let (workspace, profile) = fixtures(false);

    let mut command = Command::new(HELPER);
    command
        // Not a descriptor. The helper cannot wrap it, so it writes nothing.
        .arg("-1")
        .arg("/bin/true")
        .env("PANDORA_SANDBOX_PROFILE_JSON", &profile)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().expect("the helper starts");
    let mut pipe = child.stdout.take().expect("stdout is piped");
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).expect("the pipe reads to EOF");
    drop(pipe);
    let mut stderr = String::new();
    if let Some(mut sink) = child.stderr.take() {
        let _ = sink.read_to_string(&mut stderr);
    }
    let _ = child.wait();

    assert!(
        bytes.is_empty(),
        "a helper whose report channel is unusable must write nothing; got {} \
         bytes, stderr: {stderr}",
        bytes.len()
    );

    // And with no frame at all, the parent's rule is a refusal — never
    // `Unavailable`, and never an empty success.
    let outcome = accept_verified(
        None,
        &SandboxProfile::new(vec![workspace.clone()]).expect("the profile is valid"),
    );
    assert_eq!(
        outcome,
        ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        },
        "an absent frame must be Refused, never Unavailable"
    );
    assert!(outcome.verified().is_empty(), "and it must license nothing");
}

/// A helper whose report pipe never reaches EOF must be refused, not waited on.
///
/// This is the timeout doing its job: the parent holds its own copy of the write
/// end and never releases it, so no EOF can arrive, and the parent must give up
/// rather than block indefinitely. It exercises `read_with_timeout` directly, so
/// it is deterministic and needs no helper at all.
#[test]
fn a_report_pipe_that_never_ends_is_refused() {
    let (reader, writer) = std::io::pipe().expect("a pipe is available");

    let started = std::time::Instant::now();
    let error = read_with_timeout(reader, Duration::from_millis(150))
        .expect_err("a held write end can never produce EOF");
    // Holding the writer in scope until after the read has given up is what makes
    // the EOF impossible.
    drop(writer);

    assert!(
        error.contains("never reached EOF"),
        "a hang must be reported as a timeout, not as a read failure: {error}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "the parent must have actually waited for the timeout"
    );

    let profile =
        SandboxProfile::new(vec![PathBuf::from("/workspace")]).expect("the profile is valid");
    let outcome = accept_verified(None, &profile);
    assert_eq!(
        outcome,
        ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        },
        "and a parent that gives up must refuse"
    );
}

/// A kernel that cannot meet the required ABI must be reported as unsupported,
/// never as confined.
///
/// The floor is forced through `TEST_ABI_FLOOR`, because there is no other way to
/// make a modern kernel forget how to do Landlock. **This is a simulation of an
/// old kernel, not a real one.** What it tests is that the `UnsupportedRequest`
/// branch exists and reaches the parent as a refusal; the real old-kernel path is
/// not exercised here.
#[test]
fn a_forced_unsupported_abi_is_refused_rather_than_verified() {
    let (workspace, profile) = fixtures(false);

    // A floor above the highest revision this build knows about, so every kernel
    // is below it and the backend must refuse rather than apply something weaker.
    let report = run_helper(
        &profile,
        &[(pandora_sandbox::linux::TEST_ABI_FLOOR, "99")],
        "/bin/true",
        |_| Vec::new(),
    )
    .expect("the helper ran");
    let frame = report.frame.expect("the helper wrote a frame");

    assert_eq!(
        frame.reported().kind(),
        "unavailable",
        "a backend that cannot get the ABI it requires must report Unavailable"
    );
    assert!(
        !frame.proofs().all_four_as_designed(),
        "and it must not claim an observation it never made"
    );

    let outcome = accepted(&frame, &workspace);
    assert!(
        !outcome.is_applied(),
        "a simulated old kernel must not be accepted as confined"
    );
    assert!(outcome.verified().is_empty(), "and it must license nothing");
}

// ---------------------------------------------------------------------------
// The dedicated report channel
// ---------------------------------------------------------------------------

/// A target that prints to stdout after `exec` must reach the parent through its
/// own stdout, and the report must be unaffected.
///
/// This is the positive half of the channel design and the reason for the
/// dedicated pipe: because the report descriptor is *not* fd 1, the target keeps a
/// working stdout. `echo`'s bytes land in the parent's stdout capture, and the
/// report pipe still holds exactly one frame and nothing else.
#[test]
fn a_target_that_prints_to_stdout_keeps_its_own_stdout() {
    let (workspace, profile) = fixtures(false);

    let report = run_helper(&profile, &[], "/bin/sh", |_| {
        vec!["-c".to_owned(), "echo this-is-target-stdout".to_owned()]
    })
    .expect("the helper ran");

    assert!(
        report.stdout.contains("this-is-target-stdout"),
        "the target's stdout must still be connected; stderr: {}",
        report.stderr
    );
    let frame = report.frame.expect("the helper wrote a frame");
    assert_eq!(
        frame.proofs().outside_write,
        Observation::Denied,
        "confinement must have been applied before the exec; stderr={}; frame_reported_kind={}",
        report.stderr,
        frame.reported().kind()
    );
    let _ = workspace;
}

/// A target that tries to write to the inherited report descriptor must fail.
///
/// This is the close-on-exec half of the channel design. The helper inherited the
/// write end with the flag cleared, then re-armed it before `exec`, so the target
/// holds no part of the report channel. Pointing `echo` at the descriptor number
/// makes the shell report a bad descriptor rather than leaking bytes into the
/// report pipe.
#[test]
fn a_target_cannot_write_to_the_report_descriptor_it_inherited() {
    let (workspace, profile) = fixtures(false);

    let report = run_helper(&profile, &[], "/bin/sh", |report_fd| {
        vec![
            "-c".to_owned(),
            format!("echo this-is-target-stdout >&{report_fd}"),
        ]
    })
    .expect("the helper ran");

    // `run_helper` already refuses a report pipe that held anything other than
    // exactly one frame, so reaching here with a frame at all proves the target's
    // write never entered the report channel.
    let frame = report.frame.expect("the helper wrote a frame");
    assert!(
        !report.stdout.contains("this-is-target-stdout"),
        "the target must not have been able to write to the report descriptor"
    );
    assert_eq!(
        frame.proofs().outside_write,
        Observation::Denied,
        "confinement must have been applied before the exec; stderr={}; frame_reported_kind={}",
        report.stderr,
        frame.reported().kind()
    );
    let _ = workspace;
}

/// A frame that is followed by anything at all must be refused, not read as the
/// first frame with slack.
///
/// Built by hand rather than by a helper, because the property being checked is
/// the parent's reading rule.
#[test]
fn a_frame_with_trailing_bytes_is_refused() {
    let mut bytes = pandora_sandbox::encode(&VerificationReport::new(
        pandora_sandbox::ReportedOutcome::SelfTested,
        None,
        Proofs::none(),
        Mechanisms::none(),
    ))
    .expect("the frame encodes");
    bytes.extend_from_slice(b"one extra byte");

    let error = decode_one_frame(&bytes).expect_err("trailing bytes must be refused");

    assert!(
        error.contains("exactly one frame"),
        "the message must name what was expected: {error}"
    );
}

// ---------------------------------------------------------------------------
// The probes, against processes that are not confined
// ---------------------------------------------------------------------------

/// The negative control at the probe level: on a process that was never
/// confined, every probe must report `Allowed`.
///
/// This is what stops a broken probe from impersonating a working sandbox. It
/// runs in the test process itself, which is unconfined, so it costs nothing and
/// tests exactly the property the end-to-end denials rely on.
#[test]
fn the_probes_report_allowed_on_an_unconfined_process() {
    let base = std::env::temp_dir().join(format!("pandora-probes-{}", std::process::id()));
    std::fs::create_dir_all(&base).expect("the directory can be created");
    std::fs::write(base.join("canary"), b"canary").expect("the canary can be written");

    let request = pandora_sandbox::ConfinementRequest {
        profile: SandboxProfile::new(vec![base.clone()]).expect("the profile is valid"),
        probe_dir: base.clone(),
        program_dir: None,
    };
    let proofs = pandora_sandbox::linux::probe(&request);

    assert_eq!(
        proofs,
        Proofs {
            outside_write: Observation::Allowed,
            canary_read: Observation::Allowed,
            inet_socket: Observation::Allowed,
            inside_write: Observation::Allowed,
        },
        "an unconfined process must show every probe succeeding, or the probes \
         cannot tell confinement from its absence"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// A probe that cannot run is `Inconclusive`, never `Denied`.
///
/// A missing canary is the case that matters: a canary that is not there and a
/// canary that is hidden both fail to read, and conflating them would let a
/// broken probe pass as a working sandbox.
#[test]
fn a_probe_that_cannot_run_is_inconclusive_not_denied() {
    let base = std::env::temp_dir().join(format!("pandora-inconclusive-{}", std::process::id()));
    std::fs::create_dir_all(&base).expect("the directory can be created");
    // Deliberately no canary: the read probe must fail, and `Inconclusive` is the
    // only honest thing to record.

    let request = pandora_sandbox::ConfinementRequest {
        profile: SandboxProfile::new(vec![base.clone()]).expect("the profile is valid"),
        probe_dir: base.clone(),
        program_dir: None,
    };
    let proofs = pandora_sandbox::linux::probe(&request);

    assert_eq!(
        proofs.canary_read,
        Observation::Inconclusive,
        "a probe that could not run must not be recorded as a denial"
    );
    assert!(
        !proofs.all_four_as_designed(),
        "and an inconclusive probe must never satisfy the profile"
    );
    let _ = std::fs::remove_dir_all(&base);
}
