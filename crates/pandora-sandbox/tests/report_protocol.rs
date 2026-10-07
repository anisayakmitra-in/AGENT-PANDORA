//! Report-integrity tests over a real pipe.
//!
//! These do not stub the channel. Each case starts a child that writes the byte
//! stream a helper would emit to its stdout, and the parent reads that stdout
//! with [`pandora_sandbox::read_one_frame`]. The child's stdout is a genuine OS
//! pipe, so the reader is exercised against real `read` semantics including
//! short reads and EOF-after-close, rather than against an in-memory cursor
//! that might behave differently.
//!
//! Every refusal case has a negative control:
//! [`a_single_well_formed_frame_is_accepted_over_a_pipe`] asserts a well-formed
//! single frame *is* accepted. A reader that refused everything would fail that
//! test, so the refusals below cannot be passing vacuously.
//!
//! These tests are Unix-only because they rely on `exec`-style streaming. The
//! CI runners for Linux and macOS both run them.

#![forbid(unsafe_code)]
#![cfg(unix)]

use pandora_sandbox::report::{Observation, Proofs, VerificationReport, encode};
use pandora_sandbox::{ConfinementOutcome, MAX_REPORT_BYTES, RefusalReason, read_one_frame};
use std::io::Write as _;
use std::process::{Command, Stdio};

/// Environment variable carrying the byte stream for the writer child.
const PAYLOAD: &str = "PANDORA_PIPE_TEST_PAYLOAD";

/// A report whose four self-tests are all as designed, used as the valid frame.
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

/// Feeds `bytes` to the reader through a real pipe.
///
/// The child writes the payload to stdout and exits, which closes the write end
/// and produces the EOF the reader requires.
fn read_over_pipe(bytes: &[u8]) -> Result<VerificationReport, RefusalReason> {
    let payload = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();

    let mut child = Command::new(std::env::current_exe().expect("the test binary path is known"))
        .arg("--ignored")
        .arg("--exact")
        .arg("pipe_writer_child")
        .env(PAYLOAD, &payload)
        .env("PANDORA_PIPE_WRITER", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the writer child spawns");

    // `read_one_frame` consumes the reader, so take ownership and let it drop
    // at the end of the scope. The read either consumes the whole frame or stops
    // early; either way the handle closes when it is dropped, which is what lets
    // the child observe a closed pipe and exit.
    let stdout = child.stdout.take().expect("stdout is piped");
    let outcome = read_one_frame(stdout);
    let _ = child.wait();
    outcome
}

/// The child half. Selected by the environment variable so it never runs during
/// a normal test pass.
#[test]
#[ignore = "runs only as the pipe writer child"]
fn pipe_writer_child() {
    if std::env::var("PANDORA_PIPE_WRITER").is_err() {
        return;
    }
    let payload = std::env::var(PAYLOAD).expect("the payload is set by the parent");
    let mut stdout = std::io::stdout();
    // An empty payload writes nothing and exits, which is exactly the
    // "helper died before reporting" case.
    for pair in payload.as_bytes().chunks(2) {
        if pair.len() == 2 {
            let text = std::str::from_utf8(pair).expect("the payload is hex");
            let byte = u8::from_str_radix(text, 16).expect("the payload is hex");
            stdout.write_all(&[byte]).expect("stdout accepts the byte");
        }
    }
    stdout.flush().expect("stdout flushes");
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

/// A report whose outcome claims Applied but whose outcome carries no verified
/// controls must still be refused when the profile demanded some. That is the
/// `accept_report` contract, checked over the pipe rather than in isolation.
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
