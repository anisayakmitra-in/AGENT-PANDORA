//! The one report-integrity test that must hold on every platform.
//!
//! `tests/report_protocol.rs` is `#![cfg(unix)]`, because it exercises a
//! `UnixStream` pair. This file is deliberately not gated: the promise it tests
//! — that a helper which writes nothing leaves the parent refusing — is platform
//! independent, and on a platform with no backend it is the *only* thing the
//! parent will ever see. Windows relies on exactly this: the helper writes no
//! frame and exits non-zero, and the parent's answer must be `Refused`.
//!
//! Because it runs everywhere, it also fails everywhere. That is the point: a
//! "no frame means refusal" rule that only held on Unix would be a rule that
//! held only where it was checked.

#![forbid(unsafe_code)]

use pandora_sandbox::report::{Mechanisms, Observation, Proofs};
use pandora_sandbox::{ConfinementOutcome, RefusalReason, VerificationReport, accept_verified};
use std::path::PathBuf;

/// What a helper on a platform with no backend produces: no frame at all.
///
/// The Windows `write_report` body returns an error without writing, the helper
/// exits non-zero, and the parent's read end reaches EOF with zero bytes. That
/// is the value the parent's decision function receives, on every platform.
fn a_helper_that_writes_nothing() -> Option<VerificationReport> {
    None
}

#[test]
fn a_helper_that_writes_no_frame_is_refused_on_every_platform() {
    let profile = pandora_sandbox::SandboxProfile::new(vec![PathBuf::from("/workspace")])
        .expect("the profile is valid");

    let outcome = accept_verified(a_helper_that_writes_nothing().as_ref(), &profile);

    assert_eq!(
        outcome,
        ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        },
        "a helper that wrote nothing must be Refused — never Unavailable, and \
         never an empty success"
    );
    assert!(
        outcome.verified().is_empty(),
        "and it must license no control at all"
    );
    assert!(
        !outcome.satisfies(&profile.requested_controls()),
        "and it must never satisfy a profile that asked for controls"
    );
}

/// The same rule through the STEP 1 entry point, which shares it.
///
/// Both entry points funnel into [`pandora_sandbox::derive_outcome`] and
/// [`pandora_sandbox::gate`], so this is not a second rule being tested — it is
/// the second caller of the one rule, and it has to agree with the first.
#[test]
fn the_step_one_entry_point_also_refuses_a_silent_helper() {
    let profile = pandora_sandbox::SandboxProfile::new(vec![PathBuf::from("/workspace")])
        .expect("the profile is valid");

    let outcome = pandora_sandbox::accept_report(None, &profile);

    assert_eq!(
        outcome,
        ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        }
    );
}

/// A frame whose probes saw nothing must derive nothing, whatever platform
/// produced it. This is the case a no-backend platform reports, and it must not
/// be confused with a pass.
#[test]
fn a_frame_with_no_observations_licenses_nothing() {
    let profile = pandora_sandbox::SandboxProfile::new(vec![PathBuf::from("/workspace")])
        .expect("the profile is valid");

    let report = VerificationReport::new(
        pandora_sandbox::ReportedOutcome::SelfTested,
        None,
        Proofs::none(),
        Mechanisms::none(),
    );

    let outcome = accept_verified(Some(&report), &profile);

    assert!(
        outcome.verified().is_empty(),
        "a helper that reported no observation must not license a control"
    );
    assert!(
        !outcome.satisfies(&profile.requested_controls()),
        "and it must not satisfy a profile that asked for controls"
    );
}

/// The companion to the above: the observations that *do* license a control must
/// still be derivable. Without this, a reader that derived nothing would pass
/// every refusal in this file.
///
/// This is deliberately cross-platform: it exercises the parent's derivation,
/// which is portable code, and makes no claim about whether any host can produce
/// these observations. Producing them is `tests/linux_backend.rs`'s job, and it
/// is gated on Linux.
#[test]
fn a_frame_with_real_observations_still_derives_its_controls() {
    let profile = pandora_sandbox::SandboxProfile::new(vec![PathBuf::from("/workspace")])
        .expect("the profile is valid");

    let report = VerificationReport::new(
        pandora_sandbox::ReportedOutcome::SelfTested,
        None,
        Proofs {
            outside_write: Observation::Denied,
            canary_read: Observation::Denied,
            inet_socket: Observation::Denied,
            inside_write: Observation::Allowed,
        },
        Mechanisms {
            filesystem: "landlock".to_owned(),
            network: "seccomp_bpf".to_owned(),
            denied_socket_families: vec!["AF_INET".to_owned(), "AF_INET6".to_owned()],
            rlimits: vec!["RLIMIT_CORE".to_owned()],
        },
    );

    let outcome = accept_verified(Some(&report), &profile);

    assert!(
        outcome.satisfies(&profile.requested_controls()),
        "the parent must still accept a report whose observations support every \
         control, or every refusal above is vacuous"
    );
    assert_eq!(outcome.verified().len(), 3);
}
