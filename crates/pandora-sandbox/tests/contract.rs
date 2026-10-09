//! Contract tests for the confinement policy types.
//!
//! These are the tests that keep the contract honest rather than the plumbing.
//! Each control test has a paired negative control, because the failure mode
//! that matters most here is a test that passes because it never exercised the
//! rule.

use pandora_sandbox::report::{Mechanisms, Observation, Proofs};
use pandora_sandbox::{
    Availability, ConfinementOutcome, FilesystemConfinement, HelperReport, NetworkConfinement,
    PlatformFamily, ProofKind, RefusalReason, ReportedOutcome, RequestedControl, SandboxProfile,
    UnavailableReason, VerifiedControl, accept_report, decode_report, encode_report,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn profile() -> SandboxProfile {
    SandboxProfile::new(vec![PathBuf::from("/workspace")]).expect("the test profile is valid")
}

fn requested(controls: &[RequestedControl]) -> BTreeSet<RequestedControl> {
    controls.iter().copied().collect()
}

fn applied(controls: &[RequestedControl]) -> ConfinementOutcome {
    ConfinementOutcome::Applied {
        verified: controls
            .iter()
            .map(|control| VerifiedControl::verified(*control, "test", ProofKind::DeniedOperation))
            .collect(),
        unverified: BTreeSet::new(),
    }
}

/// The observations a working Linux backend produces.
fn all_denied() -> Proofs {
    Proofs {
        outside_write: Observation::Denied,
        canary_read: Observation::Denied,
        inet_socket: Observation::Denied,
        inside_write: Observation::Allowed,
    }
}

fn linux_mechanisms() -> Mechanisms {
    Mechanisms {
        filesystem: "landlock".to_owned(),
        network: "seccomp_bpf".to_owned(),
        denied_socket_families: vec!["AF_INET".to_owned(), "AF_INET6".to_owned()],
        rlimits: Vec::new(),
    }
}

#[test]
fn a_default_profile_denies_all_network() {
    let profile = profile();

    assert_eq!(profile.network(), &NetworkConfinement::DenyAll);
    assert!(
        profile
            .requested_controls()
            .contains(&RequestedControl::NetworkDenied),
        "the process executor's default must include a network deny"
    );
}

#[test]
fn a_default_profile_confines_the_filesystem() {
    let profile = profile();

    assert!(matches!(
        profile.filesystem(),
        FilesystemConfinement::WorkspaceOnly(_)
    ));
    assert_eq!(
        profile.requested_controls(),
        requested(&[
            RequestedControl::FilesystemWriteRestricted,
            RequestedControl::FilesystemReadRestricted,
            RequestedControl::NetworkDenied,
        ])
    );
}

#[test]
fn inheriting_the_network_does_not_request_a_deny() {
    let profile = SandboxProfile::build(
        FilesystemConfinement::WorkspaceOnly(vec![PathBuf::from("/workspace")]),
        NetworkConfinement::Inherit,
        false,
    )
    .expect("the profile is valid");

    assert!(
        !profile
            .requested_controls()
            .contains(&RequestedControl::NetworkDenied),
        "an MCP server configured with network access must not request a deny"
    );
}

#[test]
fn an_unsandboxed_profile_is_off_by_default() {
    assert!(!profile().allow_unsandboxed());
    assert!(
        SandboxProfile::build(
            FilesystemConfinement::Host,
            NetworkConfinement::Inherit,
            true
        )
        .expect("the profile is valid")
        .allow_unsandboxed(),
        "the flag must be settable, or an operator could never record it"
    );
}

#[test]
fn an_excessive_root_list_is_rejected() {
    let roots = (0..65)
        .map(|index| PathBuf::from(format!("/r{index}")))
        .collect();

    let error = SandboxProfile::new(roots).expect_err("65 roots must be rejected");

    assert_eq!(
        error,
        pandora_sandbox::SandboxProfileError::TooManyWritableRoots {
            count: 65,
            limit: 64
        }
    );
}

#[test]
fn no_platform_can_prove_anything_in_this_step() {
    for family in [
        PlatformFamily::Seatbelt,
        PlatformFamily::Landlock,
        PlatformFamily::Seccomp,
        PlatformFamily::Windows,
        PlatformFamily::Unsupported,
    ] {
        let availability = Availability::probe(family);
        assert!(
            availability.provable().is_empty(),
            "{family:?} claimed provable controls without a backend"
        );
        assert!(
            !availability.covers(&profile()),
            "{family:?} claimed to cover the default profile"
        );
    }
}

#[test]
fn windows_reports_filesystem_and_network_unavailable() {
    let availability = Availability::probe(PlatformFamily::Windows);

    assert!(
        availability.provable().is_empty(),
        "Windows must not claim a filesystem or network control until a \
         denied-operation test proves it"
    );
    assert_eq!(availability.reason(), UnavailableReason::BackendRefused);
}

#[test]
fn an_unproven_control_cannot_become_a_verified_one() {
    let availability = Availability::probe(PlatformFamily::Landlock);

    for control in [
        RequestedControl::FilesystemWriteRestricted,
        RequestedControl::FilesystemReadRestricted,
        RequestedControl::NetworkDenied,
    ] {
        assert!(
            availability
                .verify(control, ProofKind::DeniedOperation)
                .is_none(),
            "{control} became verified without a probe backing it"
        );
    }
}

#[test]
fn an_applied_outcome_licenses_exactly_what_it_verified() {
    let outcome = applied(&[RequestedControl::NetworkDenied]);

    assert_eq!(outcome.verified().len(), 1);
    assert!(outcome.is_applied());
}

#[test]
fn an_unavailable_outcome_licenses_nothing() {
    let outcome = ConfinementOutcome::Unavailable {
        reason: UnavailableReason::NoBackendOnPlatform,
    };

    assert!(
        outcome.verified().is_empty(),
        "an unavailable outcome must not be laundered into evidence"
    );
    assert!(!outcome.is_applied());
}

#[test]
fn partial_coverage_is_not_satisfaction() {
    let outcome = applied(&[RequestedControl::NetworkDenied]);

    assert!(
        !outcome.satisfies(&profile().requested_controls()),
        "denying network alone must not satisfy a profile that also confines the filesystem"
    );
}

#[test]
fn a_satisfied_outcome_covers_every_requested_control() {
    let outcome = applied(
        &profile()
            .requested_controls()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
    );

    assert!(outcome.satisfies(&profile().requested_controls()));
}

#[test]
fn an_outcome_with_unverified_controls_is_never_satisfied() {
    let outcome = ConfinementOutcome::Applied {
        verified: applied(
            &profile()
                .requested_controls()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
        )
        .verified(),
        unverified: requested(&[RequestedControl::NetworkDenied]),
    };

    assert!(
        !outcome.satisfies(&profile().requested_controls()),
        "an outcome that admits something was unverified must not be accepted"
    );
}

#[test]
fn a_missing_report_is_refused() {
    let outcome = accept_report(None, &profile());

    assert_eq!(
        outcome,
        ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        }
    );
}

#[test]
fn an_unverified_report_is_refused_rather_than_downgraded() {
    let report = HelperReport::unavailable(UnavailableReason::BackendRefused);

    let outcome = accept_report(Some(&report), &profile());

    assert_eq!(
        outcome,
        ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        },
        "an unavailable outcome must not pass through as permission"
    );
}

#[test]
fn an_unsandboxed_profile_still_refuses_but_may_be_honoured_later() {
    // With the flag set the parent is permitted to accept an unsatisfied
    // outcome; STEP 4 is where that decision is recorded in the receipt and the
    // containment evidence. This test only pins that the flag changes the
    // parent's decision, not that it is honoured silently today.
    let strict = profile();
    let relaxed = SandboxProfile::build(
        FilesystemConfinement::WorkspaceOnly(vec![PathBuf::from("/workspace")]),
        NetworkConfinement::DenyAll,
        true,
    )
    .expect("the profile is valid");
    let report = HelperReport::unavailable(UnavailableReason::BackendRefused);

    assert!(
        accept_report(Some(&report), &strict).eq(&ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        })
    );
    assert!(
        accept_report(Some(&report), &relaxed).eq(&ConfinementOutcome::Unavailable {
            reason: UnavailableReason::BackendRefused
        }),
        "with the flag set the derived outcome must be returned so it can be recorded"
    );
}

#[test]
fn a_report_round_trips_through_the_pipe_format() {
    let report = HelperReport::new(
        ReportedOutcome::SelfTested,
        Some("restricted-sandbox".to_owned()),
        all_denied(),
        linux_mechanisms(),
    );

    let frame = encode_report(&report).expect("the report encodes");
    let decoded = decode_report(&frame).expect("the report decodes");

    assert_eq!(decoded, report);
    assert_eq!(decoded.restricted_identity(), Some("restricted-sandbox"));
    assert_eq!(decoded.proofs(), &all_denied());
}

#[test]
fn a_truncated_report_is_rejected_rather_than_read_as_empty() {
    let frame = encode_report(&HelperReport::refused(RefusalReason::VerificationMissing))
        .expect("the report encodes");

    let truncated = &frame[..frame.len() - 3];

    assert!(
        decode_report(truncated).is_err(),
        "a short buffer must not be mistaken for a complete report"
    );
}

#[test]
fn a_frame_shorter_than_its_length_prefix_is_rejected() {
    assert!(decode_report(b"00").is_err());
}

#[test]
fn a_non_numeric_length_prefix_is_rejected() {
    let mut frame = b"zzzzzzzz".to_vec();
    frame.extend_from_slice(b"{}");

    assert!(decode_report(&frame).is_err());
}

#[test]
fn a_declared_length_over_the_limit_is_refused_before_allocating() {
    let frame = format!("{:08x}", pandora_sandbox::MAX_REPORT_BYTES + 1).into_bytes();

    assert_eq!(
        decode_report(&frame),
        Err(pandora_sandbox::SandboxProtocolError::ReportTooLarge {
            len: pandora_sandbox::MAX_REPORT_BYTES + 1,
            limit: pandora_sandbox::MAX_REPORT_BYTES,
        })
    );
}

#[test]
fn an_empty_report_is_rejected() {
    let frame = format!("{:08x}", 0).into_bytes();

    assert_eq!(
        decode_report(&frame),
        Err(pandora_sandbox::SandboxProtocolError::EmptyReport)
    );
}

#[test]
fn a_report_from_a_future_version_is_rejected() {
    let json = r#"{"version":99,"reported":{"kind":"refused","reason":"verification_missing"},"restricted_identity":null,"proofs":{"outside_write":"denied","canary_read":"denied","inet_socket":"denied","inside_write":"allowed"},"mechanisms":{"filesystem":"landlock","network":"seccomp_bpf","denied_socket_families":["AF_INET","AF_INET6"],"rlimits":[]}}"#;
    let mut frame = format!("{:08x}", json.len()).into_bytes();
    frame.extend_from_slice(json.as_bytes());

    assert_eq!(
        decode_report(&frame),
        Err(pandora_sandbox::SandboxProtocolError::VersionMismatch {
            found: 99,
            expected: pandora_sandbox::CONFINEMENT_OUTCOME_VERSION
        })
    );
}

#[test]
fn the_helper_refuses_when_asked_for_a_control_it_cannot_prove() {
    // The negative control for the whole contract: if this ever stops holding,
    // the crate has started claiming confinement it does not have.
    let outcome = ConfinementOutcome::Unavailable {
        reason: Availability::probe(PlatformFamily::current()).reason(),
    };

    assert_eq!(outcome.kind(), "unavailable");
    assert!(outcome.verified().is_empty());
    assert!(!outcome.satisfies(&profile().requested_controls()));
}

#[test]
fn a_helper_that_observed_every_requested_control_is_accepted() {
    let report = HelperReport::new(
        ReportedOutcome::SelfTested,
        None,
        all_denied(),
        linux_mechanisms(),
    );

    let outcome = accept_report(Some(&report), &profile());

    assert!(
        outcome.satisfies(&profile().requested_controls()),
        "the parent must accept a helper whose own observations support every control"
    );
    assert_eq!(outcome.verified().len(), 3);
}

#[test]
fn a_helper_that_observed_nothing_verifies_nothing() {
    let report = HelperReport::new(
        ReportedOutcome::SelfTested,
        None,
        Proofs::none(),
        Mechanisms::none(),
    );

    let outcome = accept_report(Some(&report), &profile());

    assert_eq!(
        outcome,
        ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        }
    );
    assert!(outcome.verified().is_empty());
}

#[test]
fn leaving_the_observations_out_of_the_frame_gains_nothing() {
    // The omission must be cheaper than the observation, or a helper could
    // report "self tested" and leave the parent to guess.
    let json = format!(
        r#"{{"version":{},"reported":{{"kind":"self_tested"}},"restricted_identity":null}}"#,
        pandora_sandbox::CONFINEMENT_OUTCOME_VERSION
    );
    let mut frame = format!("{:08x}", json.len()).into_bytes();
    frame.extend_from_slice(json.as_bytes());

    let report = decode_report(&frame).expect("a report that omits them still parses");
    let outcome = accept_report(Some(&report), &profile());

    assert!(
        !outcome.satisfies(&profile().requested_controls()),
        "omitting the observations must derive zero controls, not all of them"
    );
}
