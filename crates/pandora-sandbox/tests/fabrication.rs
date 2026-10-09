//! The criterion, as an executable assertion.
//!
//! PR #17's review criterion 1 is "`VerifiedControl` is constructible only
//! through `VerifiedControl::verified`; no `Deserialize` derive may produce
//! one". A doc comment asserting that is worth nothing, so this file holds it in
//! two independent ways.
//!
//! # The derive guard
//!
//! [`no_deserialize_derive`] reads this crate's own source at compile time and
//! fails if `Deserialize` is ever added back to the derive list of
//! `VerifiedControl` or `ConfinementOutcome`.
//!
//! It is a source guard rather than a type-level assertion, and that is a
//! deliberate trade. Rust has no negative trait bounds, and the usual workaround
//! — two overlapping blanket impls plus a marker — does not apply here: the
//! coherence checker rejects the overlap outright for a concrete local type
//! (`error[E0119]`), and the autoref-specialisation variant compiles but always
//! resolves to the fallback, so it cannot tell the two cases apart. Both were
//! tried and both were rejected. A grep-based guard that always compiles is worth
//! more than a negative bound that does not build.
//!
//! # The wire guard
//!
//! The real invariant is that nothing arriving on the pipe can become a
//! control. That is what the runtime tests below check, by sending the exact
//! frames a hostile writer would send: each one is refused outright, because the
//! report format has nowhere to put a control.

#![forbid(unsafe_code)]

use pandora_sandbox::report::{Mechanisms, Observation, Proofs};
use pandora_sandbox::{
    ConfinementOutcome, RefusalReason, ReportedOutcome, SandboxProfile, VerificationReport,
    accept_verified, decode_report, encode_report, read_one_frame,
};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Derive guard
// ---------------------------------------------------------------------------

/// The derive list attached to `struct_name` in `source`.
fn derives_for<'a>(source: &'a str, struct_name: &str) -> &'a str {
    let declaration = source
        .find(struct_name)
        .unwrap_or_else(|| panic!("{struct_name} is no longer in outcome.rs"));
    let attributes = &source[..declaration];
    let start = attributes
        .rfind("#[derive(")
        .unwrap_or_else(|| panic!("{struct_name} no longer derives anything"));
    let end = start
        + attributes[start..]
            .find(")]")
            .unwrap_or_else(|| panic!("{struct_name} has an unterminated derive list"))
        + 2;
    &source[start..end]
}

#[test]
fn no_type_a_control_lives_in_derives_deserialize() {
    let source = include_str!("../src/outcome.rs");

    for struct_name in ["pub struct VerifiedControl", "pub enum ConfinementOutcome"] {
        let derives = derives_for(source, struct_name);
        assert!(
            !derives.contains("Deserialize"),
            "{struct_name} must not be Deserialize: {derives}. A control that can be built \
             from bytes on a pipe can be fabricated, and the whole evidence chain depends \
             on there being no way to do that."
        );
    }
}

#[test]
fn the_wire_types_are_still_deserializable() {
    // The guard above is only meaningful if the report types still *can* be
    // read, so this pins the other half of the change: the wire moved from
    // conclusions to observations, it did not become unreadable.
    let source = include_str!("../src/outcome.rs");
    let reported = derives_for(source, "pub enum ReportedOutcome");
    assert!(
        reported.contains("Deserialize"),
        "ReportedOutcome is the wire type and must remain readable: {reported}"
    );

    let protocol = include_str!("../src/protocol.rs");
    assert!(
        derives_for(protocol, "pub struct HelperReport").contains("Deserialize"),
        "HelperReport is the wire type and must remain readable"
    );
}

// ---------------------------------------------------------------------------
// Wire guard
// ---------------------------------------------------------------------------

fn profile() -> SandboxProfile {
    SandboxProfile::new(vec![PathBuf::from("/workspace")]).expect("a valid profile")
}

fn frame_of(json: &str) -> Vec<u8> {
    let mut bytes = format!("{:08x}", json.len()).into_bytes();
    bytes.extend_from_slice(json.as_bytes());
    bytes
}

/// The frame that worked against the pre-amendment protocol: a full `Applied`
/// outcome carrying three invented controls, alongside observations that
/// contradict every one of them.
///
/// It must be refused outright, not merely downgraded, because the wire format
/// has nowhere to put a control.
#[test]
fn a_frame_carrying_a_verified_control_list_is_refused() {
    let json = format!(
        r#"{{"version":{},"reported":{{"kind":"applied","verified":[{{"control":"filesystem_write_restricted","mechanism":"i-made-this-up","proof":"denied_operation"}},{{"control":"filesystem_read_restricted","mechanism":"i-made-this-up","proof":"denied_operation"}},{{"control":"network_denied","mechanism":"i-made-this-up","proof":"denied_operation"}}],"unverified":[]}},"restricted_identity":null}}"#,
        pandora_sandbox::CONFINEMENT_OUTCOME_VERSION
    );

    assert_eq!(
        read_one_frame(frame_of(&json).as_slice()).unwrap_err(),
        RefusalReason::SelfTestDisproved,
        "there is no field in which a control may arrive, so this frame cannot parse"
    );
}

/// The pre-amendment field name is refused rather than reinterpreted.
#[test]
fn the_pre_amendment_outcome_field_is_refused() {
    let json = format!(
        r#"{{"version":{},"outcome":{{"kind":"applied","verified":[],"unverified":[]}},"restricted_identity":null}}"#,
        pandora_sandbox::CONFINEMENT_OUTCOME_VERSION
    );

    assert_eq!(
        read_one_frame(frame_of(&json).as_slice()).unwrap_err(),
        RefusalReason::SelfTestDisproved
    );
}

/// A version-1 frame — the shape this amendment removed — is refused rather than
/// silently read as a version-2 frame with fewer claims.
#[test]
fn a_version_one_frame_is_refused() {
    let json = r#"{"version":1,"outcome":{"kind":"applied","verified":[],"unverified":[]},"restricted_identity":null}"#;

    assert_eq!(
        decode_report(frame_of(json).as_slice()).unwrap_err(),
        pandora_sandbox::SandboxProtocolError::Unparsable,
        "the old shape must not be readable as the new one"
    );
}

/// The honest frame, for contrast. It parses, and it derives every control —
/// because the observations support them, not because the frame asked.
#[test]
fn an_honest_frame_is_accepted_and_derives_every_control() {
    let frame = encode_report(&pandora_sandbox::HelperReport::new(
        ReportedOutcome::SelfTested,
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
            rlimits: Vec::new(),
        },
    ))
    .expect("the report encodes");

    let decoded = decode_report(frame.as_slice()).expect("the report decodes");
    let accepted = accept_verified(
        Some(&VerificationReport::new(
            ReportedOutcome::SelfTested,
            None,
            *decoded.proofs(),
            decoded.mechanisms().clone(),
        )),
        &profile(),
    );

    assert_eq!(accepted.verified().len(), 3);
    assert!(accepted.satisfies(&profile().requested_controls()));
}

/// A frame that claims `self_tested` while every probe observed nothing derives
/// nothing, so a helper cannot report the *act* of testing and leave the parent
/// to assume the *result*.
#[test]
fn a_frame_that_only_claims_to_have_tested_derives_nothing() {
    let report = VerificationReport::new(
        ReportedOutcome::SelfTested,
        None,
        Proofs::none(),
        Mechanisms::none(),
    );

    let accepted = accept_verified(Some(&report), &profile());

    assert!(
        accepted.verified().is_empty(),
        "a claim of having tested is not a result"
    );
    assert_eq!(
        accepted,
        ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing
        }
    );
}

/// A helper that denies everything — including the workspace write — must not
/// pass as a working sandbox, because the other three denials prove nothing on
/// their own.
#[test]
fn a_deny_everything_frame_is_refused() {
    let report = VerificationReport::new(
        ReportedOutcome::SelfTested,
        None,
        Proofs {
            outside_write: Observation::Denied,
            canary_read: Observation::Denied,
            inet_socket: Observation::Denied,
            inside_write: Observation::Denied,
        },
        Mechanisms {
            filesystem: "landlock".to_owned(),
            network: "seccomp_bpf".to_owned(),
            denied_socket_families: vec!["AF_INET".to_owned(), "AF_INET6".to_owned()],
            rlimits: Vec::new(),
        },
    );

    let accepted = accept_verified(Some(&report), &profile());

    assert_eq!(
        accepted,
        ConfinementOutcome::Refused {
            reason: RefusalReason::SelfTestDisproved
        }
    );
}
