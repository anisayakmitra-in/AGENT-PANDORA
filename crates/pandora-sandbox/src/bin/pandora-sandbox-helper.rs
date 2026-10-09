//! Applies confinement to itself, proves it, reports, then becomes the target.
//!
//! The parent spawns this instead of the target so that confinement can be
//! applied to *this* process before `exec` replaces the image. Confining after
//! the target starts would be a race the target could win.
//!
//! The order is not negotiable. Confine, then self-test **in the real confined
//! process**, then write exactly one report frame, then close the write end, then
//! `exec`. Reporting before confining would let a target that never ran read a
//! report claiming it did.
//!
//! On Linux the backend is Landlock plus seccomp-bpf plus rlimits, and it is
//! reached through [`pandora_sandbox::confine`] — this binary holds no
//! confinement code of its own. Everywhere else there is no backend, so the
//! helper reports `Unavailable` and exits non-zero without executing anything.
//!
//! # The one `unsafe`
//!
//! Wrapping the parent's report descriptor is the only unsafe in the workspace,
//! and it lives in [`write_report`] and nowhere else. `deny` rather than
//! `forbid` at the crate root, so that any *new* unsafe has to carry an explicit
//! `#[allow]` with a SAFETY note rather than arriving unnoticed.
#![deny(unsafe_code)]

use pandora_sandbox::report::{Mechanisms, Proofs};
use pandora_sandbox::{
    Availability, ConfinementRequest, HelperReport, PlatformFamily, ReportedOutcome,
    SandboxProfile, confine, encode_report,
};
#[cfg(unix)]
use std::io::Write as _;
use std::path::PathBuf;

/// Exit code used when the helper refuses. Distinct from a target's own exit
/// codes so a caller can tell "the sandbox stopped this" from "the program
/// failed".
const REFUSED_EXIT: i32 = 78;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let Some(report_handle) = arguments.next() else {
        eprintln!("pandora-sandbox-helper: a report handle is required");
        std::process::exit(REFUSED_EXIT);
    };
    let program = arguments.next();
    let program_arguments: Vec<String> = arguments.collect();

    let Some(program) = program else {
        eprintln!("pandora-sandbox-helper: a target program is required");
        std::process::exit(REFUSED_EXIT);
    };

    let profile = match load_profile() {
        Ok(profile) => profile,
        Err(message) => {
            eprintln!("pandora-sandbox-helper: {message}");
            std::process::exit(REFUSED_EXIT);
        }
    };

    let availability = Availability::probe(PlatformFamily::current());
    // A pre-spawn check only. The host may have Landlock and still fail to
    // enforce a ruleset, and only `confine` finds that out.
    if !availability.covers(&profile) && !profile.allow_unsandboxed() {
        let reported = ReportedOutcome::Unavailable {
            reason: availability.reason(),
        };
        report(
            &report_handle,
            &reported,
            &Proofs::none(),
            &Mechanisms::none(),
        );
        std::process::exit(REFUSED_EXIT);
    }

    // Created before confinement, because afterwards this process must not be
    // able to write there — which is the whole point of the write probe.
    let probe_dir = match create_probe_dir() {
        Ok(probe_dir) => probe_dir,
        Err(message) => {
            eprintln!("pandora-sandbox-helper: {message}");
            std::process::exit(REFUSED_EXIT);
        }
    };

    let attempt = confine(ConfinementRequest {
        profile,
        probe_dir,
        program_dir: program_directory(&program),
    });
    let (reported, proofs, mechanisms) = (attempt.reported, attempt.proofs, attempt.mechanisms);

    report(&report_handle, &reported, &proofs, &mechanisms);

    match reported {
        // Only a helper that got as far as running its self-tests may exec. Every
        // other answer stops here, which is the whole point of the helper: it
        // never falls back to unconfined. Note that the helper cannot decide this
        // for itself — the parent derives the controls from the observations
        // below — so this branch is only the helper declining to run something it
        // could not confine, not a claim that it did confine it.
        ReportedOutcome::SelfTested => {}
        _ => {
            eprintln!(
                "pandora-sandbox-helper: refusing to exec {program} unconfined ({})",
                reported.kind()
            );
            std::process::exit(REFUSED_EXIT);
        }
    }

    exec(&program, &program_arguments);
}

/// Creates the directory the outside-write and canary-read probes target.
///
/// It must be outside every writable root, and it must be populated before
/// confinement is applied. Afterwards nobody may write here, and the canary may
/// not be read back, which is what makes both probes tests of confinement
/// rather than of the directory's existence.
fn create_probe_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("pandora-sandbox-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create the probe directory: {error}"))?;
    std::fs::write(dir.join("canary"), b"pandora-sandbox-canary")
        .map_err(|error| format!("could not write the probe canary: {error}"))?;
    Ok(dir)
}

/// The directory holding the target program, which Landlock needs a rule for.
fn program_directory(program: &str) -> Option<PathBuf> {
    std::path::Path::new(program).parent().map(PathBuf::from)
}

fn load_profile() -> Result<SandboxProfile, String> {
    let raw = std::env::var("PANDORA_SANDBOX_PROFILE_JSON")
        .map_err(|_| "PANDORA_SANDBOX_PROFILE_JSON is not set".to_owned())?;
    serde_json::from_str(&raw).map_err(|error| format!("the sandbox profile is invalid: {error}"))
}

/// Writes the single framed report to the descriptor the parent supplied.
fn report(handle: &str, reported: &ReportedOutcome, proofs: &Proofs, mechanisms: &Mechanisms) {
    let encoded = match encode_report(&HelperReport::new(
        reported.clone(),
        None,
        *proofs,
        mechanisms.clone(),
    )) {
        Ok(encoded) => encoded,
        Err(error) => {
            eprintln!("pandora-sandbox-helper: {error}");
            std::process::exit(REFUSED_EXIT);
        }
    };
    if write_report(handle, &encoded).is_err() {
        eprintln!("pandora-sandbox-helper: the report channel was unusable");
        std::process::exit(REFUSED_EXIT);
    }
}

/// Writes exactly one report frame to the descriptor the parent supplied, then
/// closes the write end.
///
/// This is the **only** `unsafe` in the workspace, and the only place a raw
/// descriptor is turned into an owned value. It is one function rather than two
/// because writing and closing have to be inseparable: a frame that is written
/// and not closed never produces the EOF the parent is waiting for, and a
/// descriptor that is closed and not written looks identical to a helper that
/// died. One function, one ownership transfer, one close.
#[cfg(unix)]
#[allow(unsafe_code)]
fn write_report(handle: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::fd::FromRawFd as _;

    let raw = handle
        .parse::<i32>()
        .map_err(|_| std::io::Error::other("the report descriptor is not a number"))?;
    // SAFETY: the parent creates the pipe, marks the write end inheritable, and
    // passes it to this process as `raw`, keeping its own descriptor for the same
    // open file description. Closing ours cannot close theirs, so the parent still
    // controls when the channel is really shut.
    //
    // The frame is written exactly once, from the single call site in [`report`],
    // and the `File` is dropped at the end of this function, which is the
    // deliberate close. Nothing else here takes ownership of a raw descriptor.
    let mut file = unsafe { std::fs::File::from_raw_fd(raw) };
    let written = file.write_all(bytes).and_then(|()| file.flush());
    // Closing explicitly rather than relying on `exec`: EOF at the parent is what
    // confirms nothing else was written, so it has to happen deterministically and
    // before the target starts.
    drop(file);
    written
}

/// Windows has no `exec`, so there is no ordering property to preserve, and no
/// backend that could honestly fill a frame.
///
/// Writing nothing is therefore the right answer for a platform with no backend:
/// the parent sees an absent frame, which is a refusal, rather than a frame
/// claiming confinement it never applied.
#[cfg(not(unix))]
fn write_report(handle: &str, bytes: &[u8]) -> std::io::Result<()> {
    let _ = (handle, bytes);
    Err(std::io::Error::other(
        "no backend writes a report frame on this platform",
    ))
}

/// Replaces this process with the target, keeping the confinement applied here.
///
/// Confinement is applied before this point, so the exec'd program inherits it.
/// On Windows there is no `exec`, so the target is spawned and this process
/// exits with its status; the confined identity is passed through the handle
/// rather than inherited from a replaced image.
fn exec(program: &str, arguments: &[String]) -> ! {
    #[cfg(windows)]
    {
        let status = std::process::Command::new(program).args(arguments).status();
        match status {
            Ok(status) => std::process::exit(status.code().unwrap_or(REFUSED_EXIT)),
            Err(error) => {
                eprintln!("pandora-sandbox-helper: could not run {program}: {error}");
                std::process::exit(REFUSED_EXIT);
            }
        }
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::process::CommandExt as _;

        let error = std::process::Command::new(program).args(arguments).exec();
        eprintln!("pandora-sandbox-helper: could not exec {program}: {error}");
        std::process::exit(REFUSED_EXIT);
    }
}
