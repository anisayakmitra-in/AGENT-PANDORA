//! OS-level confinement for governed child processes.
//!
//! # Why this crate exists
//!
//! Every other crate in the workspace carries `#![forbid(unsafe_code)]`. This is
//! the single exception, and it is the exception because applying an OS
//! confinement mechanism needs platform FFI that cannot be expressed safely.
//! The exception is bounded in three ways:
//!
//! 1. No other crate may use `unsafe`. They keep their `forbid`.
//! 2. The library half decides and describes; it never applies confinement and
//!    never claims a control. Only the helper half does that.
//! 3. A control can only enter an outcome through
//!    [`outcome::VerifiedControl::verified`], which the *parent* calls from
//!    [`report::derive_outcome`] and only where an observation the parent read
//!    itself supports it. The wire format has no field in which a control could
//!    arrive, so there is no path from "the backend says so" to "the control
//!    holds".
//!
//! # What confinement is not
//!
//! Confinement is defence in depth *underneath* the permit path. It never grants
//! authority. A sandbox narrows what an already-permitted effect can reach; it
//! cannot permit an effect that Parliament refused, and a sandbox failure is
//! always a refusal to run rather than a reason to run unconfined.
//!
//! # State in this step
//!
//! The Linux backend is implemented and tested. `Availability::probe` reports
//! what the host can attempt, [`confine`] applies it and returns the
//! observations the helpers saw, and [`derive_outcome`] turns those into
//! controls — in the parent, never in the helper. Every other platform reports
//! `Unavailable`: macOS Seatbelt and Windows restricted-token backends are later
//! steps, and neither is claimed here.
//!
//! On Windows, filesystem and network confinement are reported `Unavailable`
//! specifically. Windows confinement is a real user or group identity rather
//! than an absence of permission, so it cannot be proven by the
//! denied-operation test used elsewhere. Until a test proves it, the honest
//! answer is that it is unavailable.

// The library half of this crate needs no unsafe at all: it only decides and
// describes. Forbidding it here makes the split above machine-checked rather
// than a convention, so a future change that tries to apply confinement from
// the library fails to compile instead of quietly widening the exception.
#![forbid(unsafe_code)]

pub mod detect;
pub mod error;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod outcome;
pub mod profile;
pub mod protocol;
pub mod report;

pub use detect::{Availability, PlatformFamily};
pub use error::{SandboxProfileError, SandboxProtocolError};
pub use outcome::{
    CONFINEMENT_OUTCOME_VERSION, ConfinementOutcome, ProofKind, RefusalReason, ReportedOutcome,
    UnavailableReason, VerifiedControl,
};
pub use profile::{
    FilesystemConfinement, NetworkConfinement, RequestedControl, SANDBOX_PROFILE_VERSION,
    SandboxProfile,
};
pub use protocol::{
    HelperReport, MAX_REPORT_BYTES, REPORT_LENGTH_HEX, decode_report, encode_report,
};
pub use report::{
    Mechanisms, Observation, Proofs, VerificationReport, accept_verified, derive_outcome, encode,
    gate, read_one_frame,
};

/// Everything the backend needs, gathered by the caller before anything is
/// applied.
///
/// The probe directory is created **before** confinement, because afterwards
/// nobody may write there — which is the only thing that makes the write probe a
/// test of confinement rather than a test of the directory's existence.
pub struct ConfinementRequest {
    /// The request, so the backend can see which roots are writable.
    pub profile: SandboxProfile,
    /// A directory outside every allowed root, populated with a canary file.
    pub probe_dir: std::path::PathBuf,
    /// Directory holding the program that will be `exec`'d, if known. Landlock
    /// needs a read-and-execute rule for it, because an `exec` after confinement
    /// is a path lookup like any other.
    pub program_dir: Option<std::path::PathBuf>,
}

/// What applying the platform backend produced.
///
/// A backend reports *observations*, never controls. [`derive_outcome`] turns
/// these into the control set, in the parent, so a helper that has been subverted
/// can report what it saw but cannot decide what it proved.
pub struct ConfinementAttempt {
    /// What the helper did: ran its probes, or could not, or refused.
    pub reported: ReportedOutcome,
    /// What the probes saw, from inside the confined process.
    pub proofs: Proofs,
    /// Which mechanisms did the work. Empty when nothing was applied.
    pub mechanisms: Mechanisms,
}

/// Applies the platform backend to the calling process and runs its self-tests.
///
/// Called by the helper before it executes anything, and by nothing else. The
/// library half stays free of it: applying confinement is a side effect on the
/// caller, and the only caller that may accept that is the code that is about to
/// become the target.
///
/// On a platform with no backend this reports [`UnavailableReason`] rather than
/// running unconfined, which is what makes the parent refuse to start something
/// it cannot confine.
pub fn confine(request: ConfinementRequest) -> ConfinementAttempt {
    #[cfg(target_os = "linux")]
    {
        if request.profile.allow_unsandboxed() {
            // An explicit operator decision to run unconfined. The probes still
            // run, so the report shows that the denials and the workspace write
            // are not in force — the receipt must be able to see the gap.
            return ConfinementAttempt {
                reported: ReportedOutcome::SelfTested,
                proofs: linux::probe(&request),
                mechanisms: Mechanisms::none(),
            };
        }
        match linux::apply(&request) {
            Ok(mechanisms) => ConfinementAttempt {
                reported: ReportedOutcome::SelfTested,
                proofs: linux::probe(&request),
                mechanisms,
            },
            Err(reason) => ConfinementAttempt {
                reported: ReportedOutcome::Unavailable { reason },
                proofs: Proofs::none(),
                mechanisms: Mechanisms::none(),
            },
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = request;
        ConfinementAttempt {
            reported: ReportedOutcome::Unavailable {
                reason: UnavailableReason::BackendRefused,
            },
            proofs: Proofs::none(),
            mechanisms: Mechanisms::none(),
        }
    }
}

/// The decision the parent makes about whether to run the target at all.
///
/// The helper's report is never taken as a verdict. The controls are rebuilt
/// from the observations it carries by [`derive_outcome`], and the one fail-closed
/// rule is [`gate`]. This function exists so the STEP 1 entry point keeps its
/// signature; the decision it makes is exactly the one [`accept_verified`]
/// makes, so there is one rule rather than one per call site.
pub fn accept_report(
    report: Option<&HelperReport>,
    profile: &SandboxProfile,
) -> ConfinementOutcome {
    let Some(report) = report else {
        return ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing,
        };
    };
    let derived = derive_outcome(
        report.reported(),
        report.proofs(),
        report.mechanisms(),
        profile,
    );
    gate(derived, profile)
}
