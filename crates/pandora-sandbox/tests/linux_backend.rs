//! End-to-end tests for the Linux backend, run against the real helper binary
//! over a real pipe.
//!
//! These are the only tests in the crate that exercise confinement as a
//! behaviour rather than as a type. They spawn
//! `env!("CARGO_BIN_EXE_pandora-sandbox-helper")` â€” the same binary the executors
//! will spawn â€” and read exactly one frame back.
//!
//! # The report channel
//!
//! The helper writes its frame to the descriptor the parent names and then
//! **closes** it, so the tests pass `1` and give the child a piped stdout. That is
//! a real kernel pipe whose write end only the child holds, and the close is what
//! makes the EOF on the parent's side deterministic: by the time the `exec`'d
//! target starts, its standard output is already shut, so it cannot append
//! anything to the report. Reading the pipe to EOF and requiring the byte count
//! to match the declared frame length exactly is therefore a real check that
//! exactly one frame arrived and nothing else did.
//!
//! # The gates fail, not skip
//!
//! Every test here is `#[cfg(target_os = "linux")]`, and each asserts the
//! behaviour the runner's kernel should provide. A runner without Landlock or
//! seccomp fails these tests rather than passing them vacuously â€” a confinement
//! claim that is silently skipped on the platform it is meant to cover is the
//! failure this crate exists to prevent. There are no `#[ignore]`s and no skip
//! branches.

#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]

use pandora_sandbox::report::{Observation, Proofs};
use pandora_sandbox::{
    ConfinementOutcome, HelperReport, RefusalReason, SandboxProfile, VerificationReport,
    accept_verified,
};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HELPER: &str = env!("CARGO_BIN_EXE_pandora-sandbox-helper");

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

/// Everything the helper said, plus what it wrote to standard error.
struct Report {
    frame: Option<HelperReport>,
    stderr: String,
}

/// Runs the helper over a real pipe and reads the pipe to EOF.
///
/// `report_handle` is the descriptor number handed to the helper. Passing `"1"`
/// alongside `Stdio::piped()` gives the helper a genuine kernel pipe as its
/// report channel; passing `"-1"` gives it one it cannot parse, which is the
/// deterministic way to make a helper that never reports.
fn run_helper(
    profile: &str,
    env: &[(&str, &str)],
    target: &str,
    report_handle: &str,
) -> Result<Report, String> {
    let mut command = Command::new(HELPER);
    command
        .arg(report_handle)
        .arg(target)
        .env("PANDORA_SANDBOX_PROFILE_JSON", profile)
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }

    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let mut pipe = child.stdout.take().expect("stdout is piped");
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    drop(pipe);

    let mut stderr = String::new();
    if let Some(mut sink) = child.stderr.take() {
        let _ = sink.read_to_string(&mut stderr);
    }
    let status = child.wait().map_err(|error| error.to_string())?;

    let frame = decode_one_frame(&bytes)?;
    Ok(Report {
        frame,
        stderr: if status.success() {
            stderr
        } else {
            format!("{stderr}(exit {status:?})")
        },
    })
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
/// This is the property the pipe is being read to prove, so it is checked here
/// rather than assumed: one extra byte is a second claim and must be an error.
fn decode_one_frame(bytes: &[u8]) -> Result<Option<HelperReport>, String> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let declared = expected_length(bytes);
    if declared == 0 || bytes.len() != declared {
        return Err(format!(
            "expected exactly one frame of {} bytes, got {}",
            declared,
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
    let abi = pandora_sandbox::linux::detected_abi()
        .expect("this runner's kernel has no Landlock, so no test here can pass");
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

    let report = run_helper(&profile, &[], "/bin/true", "1")
        .expect("the helper ran")
        .frame
        .expect("the helper wrote a frame");
    let proofs = report.proofs();
    let mechanisms = report.mechanisms();

    assert_eq!(
        proofs.outside_write,
        Observation::Denied,
        "a write outside every allowed path must be denied"
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

    let outcome = accepted(&report, &workspace);
    assert!(
        outcome.satisfies(&profile_requested(&workspace)),
        "the parent must accept an honest confined helper, or the refusals \
         elsewhere would be vacuous"
    );
    assert_eq!(outcome.verified().len(), 3);
}

fn profile_requested(
    workspace: &Path,
) -> std::collections::BTreeSet<pandora_sandbox::RequestedControl> {
    SandboxProfile::new(vec![workspace.to_path_buf()])
        .expect("the profile is valid")
        .requested_controls()
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

    let report = run_helper(&profile, &[], "/bin/true", "1")
        .expect("the helper ran")
        .frame
        .expect("the helper wrote a frame");
    let proofs = report.proofs();

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
        report.mechanisms().filesystem.is_empty(),
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
#[test]
fn a_helper_that_cannot_report_yields_no_frame() {
    let (workspace, profile) = fixtures(false);

    let report = run_helper(&profile, &[], "/bin/true", "-1").expect("the helper ran");
    assert!(
        report.frame.is_none(),
        "a helper whose report channel is unusable must write nothing; stderr: {}",
        report.stderr
    );

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
        "1",
    )
    .expect("the helper ran")
    .frame
    .expect("the helper wrote a frame");

    assert_eq!(
        report.reported().kind(),
        "unavailable",
        "a backend that cannot get the ABI it requires must report Unavailable"
    );
    assert!(
        !report.proofs().all_four_as_designed(),
        "and it must not claim an observation it never made"
    );

    let outcome = accepted(&report, &workspace);
    assert!(
        !outcome.is_applied(),
        "a simulated old kernel must not be accepted as confined"
    );
    assert!(outcome.verified().is_empty(), "and it must license nothing");
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
