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
//! This is the contract step. No backend is implemented, so
//! [`detect::Availability::probe`] reports nothing provable on every platform
//! and [`outcome::ConfinementOutcome`] therefore resolves to `Unavailable`.
//! That is deliberate: it is the honest answer, and it means the parent refuses
//! to run until STEP 2 supplies a backend that can prove itself.
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
