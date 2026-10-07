//! Report-integrity tests over a real pipe.
//!
//! Each case builds the byte stream a helper would emit, writes it to one end
//! of a real OS pipe, and reads it with [`pandora_sandbox::read_one_frame`] on
//! the other. The reader is therefore exercised against genuine `read`
//! semantics — short reads and EOF-on-close — rather than against an in-memory
//! cursor that might behave differently.
//!
//! # Why a socket pair rather than a child process or a raw pipe
//!
//! The first version spawned the test binary itself as the writer child. That
//! cannot work: libtest writes its own preamble and per-test results to
//! **stdout**, which is the same pipe carrying the payload, so the parent read
//! `running ` as the eight-byte length prefix and every case failed on the Unix
//! runners.
//!
//! A raw `pipe(2)` would avoid that, but it needs an `extern "C"` declaration,
//! and this file must stay free of `unsafe` so the crate's boundary holds.
//! `UnixStream::pair` gives the properties the reader actually depends on: a
//! real OS stream, genuine short reads, and EOF when the write end is closed.
//! The one property it does not exercise is `CLOEXEC` on a pipe write end,
//! which belongs to the helper and is covered by the backend work.
//!
//! Every refusal case has a negative control:
//! [`a_single_well_formed_frame_is_accepted_over_a_pipe`] asserts a well-formed
//! single frame *is* accepted. A reader that refused everything would fail that
//! test, so the refusals below cannot pass vacuously.
//!
//! # Why the writer is not a child process
//!
//! The first version of this file spawned the test binary itself as the writer
//! child. That cannot work: libtest writes its own preamble and per-test results
//! to **stdout**, which is the same pipe carrying the payload, so the parent read
//! `running ` as the eight-byte length prefix and every case failed on the
//! Unix runners. The writer is now a raw file descriptor created by `pipe(2)`,
//! so the payload is the only content in the pipe.
//!
//! Every refusal case has a negative control:
//! [`a_single_well_formed_frame_is_accepted_over_a_pipe`] asserts a well-formed
//! single frame *is* accepted. A reader that refused everything would fail that
//! test, so the refusals below cannot pass vacuously.

#![forbid(unsafe_code)]
#![cfg(unix)]

use pandora_sandbox::report::{Observation, Proofs, VerificationReport, encode};
use pandora_sandbox::{ConfinementOutcome, MAX_REPORT_BYTES, RefusalReason, read_one_frame};
use std::io::Write as _;

/// Feeds `bytes` to the reader through a real OS stream.
///
/// The writer is dropped before the reader is awaited, which closes the write
/// end and produces the EOF that `read_one_frame` requires.
fn read_over_pipe(bytes: &[u8]) -> Result<VerificationReport, RefusalReason> {
    use std::os::unix::net::UnixStream;

    let (mut writer, reader) = UnixStream::pair().expect("a connected socket pair is available");
    let handle = std::thread::spawn(move || read_one_frame(reader));
    let _ = writer.write_all(bytes);
    // Dropping the writer closes its half, which is what yields EOF.
    drop(writer);

    handle.join().expect("the reader thread does not panic")
}

fn a_report() -> VerificationReport {
    VerificationReport::new(
        ConfinementOutcome::Applied {
            verified: Default::default(),
            unverified: Default::default(),
        },
        Some("probe-identity".to_owned()),
    )
    .with_proofs(Proofs {
        outside_write: Observation::Denied,
        canary_read: Observation::Denied,
        inet_socket: Observation::Denied,
        inside_write: Observation::Allowed,
    })
}

/// A single well-formed frame is accepted. This is the negative control for
/// every refusal below: without it, a reader that refused all input would look
/// correct.
#[test]
fn a_single_well_formed_frame_is_accepted_over_a_pipe() {
    let report = read_over_pipe(&encode(&a_report()).expect("the report encodes"))
        .expect("a valid frame is accepted");
    assert_eq!(report.restricted_identity(), Some("probe-identity"));
    assert!(
        report.proofs().all_four_as_designed(),
        "the decoded proofs must survive the pipe"
    );
}

#[test]
fn a_duplicate_frame_is_refused() {
    let one = encode(&a_report()).expect("the report encodes");
    let mut doubled = one.clone();
    doubled.extend_from_slice(&one);
    assert_eq!(
        read_over_pipe(&doubled).unwrap_err(),
        RefusalReason::SelfTestDisproved,
        "a second frame must be refused, not read as the first"
    );
}

#[test]
fn trailing_bytes_after_a_complete_frame_are_refused() {
    let mut bytes = encode(&a_report()).expect("the report encodes");
    bytes.extend_from_slice(b"extra");
    assert_eq!(
        read_over_pipe(&bytes).unwrap_err(),
        RefusalReason::SelfTestDisproved
    );
}

#[test]
fn a_truncated_body_is_refused() {
    let bytes = encode(&a_report()).expect("the report encodes");
    assert_eq!(
        read_over_pipe(&bytes[..bytes.len() - 5]).unwrap_err(),
        RefusalReason::VerificationMissing
    );
}

#[test]
fn a_helper_that_dies_before_reporting_is_refused() {
    assert_eq!(
        read_over_pipe(&[]).unwrap_err(),
        RefusalReason::VerificationMissing,
        "an absent frame is Refused, never Unavailable"
    );
}

#[test]
fn an_oversized_declared_length_is_refused() {
    let bytes = format!("{:08x}", MAX_REPORT_BYTES + 1).into_bytes();
    assert_eq!(
        read_over_pipe(&bytes).unwrap_err(),
        RefusalReason::SelfTestDisproved
    );
}

#[test]
fn a_zero_length_frame_is_refused() {
    assert_eq!(
        read_over_pipe(b"00000000").unwrap_err(),
        RefusalReason::SelfTestDisproved
    );
}

#[test]
fn a_non_numeric_length_is_refused() {
    assert_eq!(
        read_over_pipe(b"zzzzzzzz{}").unwrap_err(),
        RefusalReason::SelfTestDisproved
    );
}

#[test]
fn a_future_version_is_refused() {
    let json = r#"{"version":9999,"outcome":{"kind":"refused","reason":"verification_missing"},"restricted_identity":null,"proofs":{"outside_write":"denied","canary_read":"denied","inet_socket":"denied","inside_write":"allowed"},"mechanisms":{"filesystem":"x","network":"y","denied_socket_families":[],"rlimits":[]}}"#;
    let mut bytes = format!("{:08x}", json.len()).into_bytes();
    bytes.extend_from_slice(json.as_bytes());
    assert_eq!(
        read_over_pipe(&bytes).unwrap_err(),
        RefusalReason::SelfTestDisproved
    );
}

#[test]
fn an_unknown_field_is_refused_rather_than_ignored() {
    // deny_unknown_fields means a report carrying an unexpected claim is
    // refused outright instead of being accepted with that claim dropped.
    let json = r#"{"version":1,"outcome":{"kind":"refused","reason":"verification_missing"},"restricted_identity":null,"proofs":{},"mechanisms":{"filesystem":"","network":"","denied_socket_families":[],"rlimits":[]},"smuggled":"control"}"#;
    let mut bytes = format!("{:08x}", json.len()).into_bytes();
    bytes.extend_from_slice(json.as_bytes());
    assert_eq!(
        read_over_pipe(&bytes).unwrap_err(),
        RefusalReason::SelfTestDisproved
    );
}

/// An `Applied` outcome that verified nothing must not satisfy a profile that
/// asked for controls. This is the `accept_verified` contract, checked over the
/// pipe rather than in isolation.
#[test]
fn an_applied_outcome_with_no_controls_does_not_satisfy_a_profile() {
    let profile =
        pandora_sandbox::SandboxProfile::new(vec![std::path::PathBuf::from("/workspace")])
            .expect("a valid profile");
    let decoded = read_over_pipe(&encode(&a_report()).expect("the report encodes"))
        .expect("the frame decodes");
    let accepted = pandora_sandbox::report::accept_verified(Some(&decoded), &profile);
    assert!(
        !accepted.is_applied(),
        "an outcome that verified nothing must not pass as Applied for a profile that asked for controls"
    );
}
