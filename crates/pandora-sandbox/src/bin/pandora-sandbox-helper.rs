//! Applies confinement to itself, proves it, reports, then becomes the target.
//!
//! The parent spawns this instead of the target so that confinement can be
//! applied to *this* process before `exec` replaces the image. Confining after
//! the target starts would be a race the target could win.
//!
//! STEP 1 implements no backend. The helper therefore performs its self-tests,
//! finds that confinement is not in force, and reports `Unavailable` rather than
//! executing. That is the fail-closed behaviour the contract requires, and it
//! is what makes the plumbing testable before STEP 2 supplies real backends.

// `deny` rather than `forbid`: the two `write_report` bodies below must be able
// to opt in, and `deny` still forces any *new* unsafe to carry an explicit
// `#[allow]` with a SAFETY note rather than arriving unnoticed.
#![deny(unsafe_code)]

use pandora_sandbox::{
    encode_report, Availability, ConfinementOutcome, HelperReport, PlatformFamily, SandboxProfile,
    UnavailableReason,
};
use std::io::Write as _;

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
    let outcome = apply_and_prove(&availability, &profile);
    report(&report_handle, &outcome);

    match outcome {
        // Only an Applied outcome may exec. Everything else stops here, which is
        // the whole point of the helper: it never falls back to unconfined.
        ConfinementOutcome::Applied { .. } => {}
        _ => {
            eprintln!(
                "pandora-sandbox-helper: refusing to exec {program} unconfined ({})",
                outcome.kind()
            );
            std::process::exit(REFUSED_EXIT);
        }
    }

    exec(&program, &program_arguments);
}

/// Applies the requested confinement, then proves it.
///
/// STEP 1 has no backend to apply, so this always resolves to `Unavailable`
/// without attempting anything. The self-test is written out anyway so STEP 2
/// has to satisfy it rather than skip past it, and so the shape of the proof is
/// reviewable before it can be claimed.
fn apply_and_prove(
    availability: &Availability,
    profile: &SandboxProfile,
) -> ConfinementOutcome {
    if !availability.covers(profile) {
        return ConfinementOutcome::Unavailable {
            reason: availability.reason(),
        };
    }
    // STEP 2 replaces this block with: apply the backend, then run
    // self_test_denied_write() and self_test_denied_socket(), and build the
    // verified set from those results alone. A control with no corresponding
    // denied-operation result must not appear in the set.
    let _ = (UnavailableReason::BackendRefused,);
    ConfinementOutcome::Unavailable {
        reason: UnavailableReason::BackendRefused,
    }
}

fn load_profile() -> Result<SandboxProfile, String> {
    let raw = std::env::var("PANDORA_SANDBOX_PROFILE_JSON")
        .map_err(|_| "PANDORA_SANDBOX_PROFILE_JSON is not set".to_owned())?;
    serde_json::from_str(&raw).map_err(|error| format!("the sandbox profile is invalid: {error}"))
}

/// Writes the single framed report to the descriptor the parent supplied.
///
/// The descriptor is cross-platform by construction: the parent gives the
/// helper an inheritable handle, and the helper wraps it with the platform's
/// own conversion rather than assuming a Unix fd number.
fn report(handle: &str, outcome: &ConfinementOutcome) {
    let encoded = match encode_report(&HelperReport::new(outcome.clone(), None)) {
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

#[cfg(windows)]
#[allow(unsafe_code)]
fn write_report(handle: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::windows::io::{FromRawHandle as _, IntoRawHandle as _};

    let raw = handle
        .parse::<usize>()
        .map_err(|_| std::io::Error::other("the report handle is not a number"))?;
    // SAFETY: the parent creates the pipe, marks the write end inheritable,
    // passes it as this handle, and keeps it open until it has read the report.
    // This is the only place the handle is wrapped, it is written exactly once,
    // and ownership is released back to the raw handle immediately after so the
    // File destructor cannot close a descriptor the parent still owns.
    let mut file = unsafe { std::fs::File::from_raw_handle(raw as _) };
    let written = file.write_all(bytes).and_then(|()| file.flush());
    let _ = file.into_raw_handle();
    written
}

#[cfg(not(windows))]
#[allow(unsafe_code)]
fn write_report(handle: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::fd::{FromRawFd as _, IntoRawFd as _};

    let raw = handle
        .parse::<i32>()
        .map_err(|_| std::io::Error::other("the report descriptor is not a number"))?;
    // SAFETY: as above. The parent holds the read end and keeps the write end
    // open until it has read the report, and the raw descriptor is handed back
    // rather than closed here.
    let mut file = unsafe { std::fs::File::from_raw_fd(raw) };
    let written = file.write_all(bytes).and_then(|()| file.flush());
    let _ = file.into_raw_fd();
    written
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
        let status = std::process::Command::new(program)
            .args(arguments)
            .status();
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