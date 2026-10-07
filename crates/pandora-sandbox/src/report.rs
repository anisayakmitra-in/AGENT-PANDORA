//! One-frame report integrity.
//!
//! The protocol has one job: make it impossible for a helper, a target, or a
//! hostile writer to put more than one claim in front of the parent, and make
//! every malformed thing the parent sees resolve to [`RefusalReason`] rather
//! than to a weaker answer.
//!
//! The rules, in full:
//!
//! * The frame is a fixed-width hex length prefix followed by that many bytes of
//!   JSON, and nothing else.
//! * The helper writes **exactly one** frame and then **explicitly closes** the
//!   write end. It does not rely on a destructor or on `exec` to close it.
//! * The parent reads **exactly one** frame, then requires EOF with **zero**
//!   further bytes.
//! * Malformed, truncated, duplicate, oversized, missing, and any trailing
//!   bytes all become [`RefusalReason::SelfTestDisproved`] or a protocol
//!   refusal. None of them is ever `Unavailable`: `Unavailable` means "no
//!   backend here", which is a statement about the platform, whereas a bad frame
//!   is a statement about this exchange.
//! * A helper that dies before reporting is Refused, not Unavailable.
//!
//! # What EOF proves, per platform
//!
//! On Linux and macOS the write end is close-on-exec. The helper closes it
//! explicitly before `exec`, and the kernel closes it again at `exec` if it were
//! somehow still open. The `exec`'d target therefore never holds the write end,
//! so EOF is reached deterministically once the helper has written and closed.
//! EOF here means "the helper wrote one frame and nothing else could have".
//!
//! Windows has no `exec`. The helper spawns the target as a child and then
//! exits, so EOF arrives when the helper exits rather than when the target
//! starts, and the guarantee is weaker by construction. Windows therefore
//! stays `Unavailable` in STEP 2 rather than pretending to a property it does
//! not have. The requirement for a future Windows backend is recorded in the
//! ADR: the pipe handle must be non-inheritable and passed only to the helper,
//! which closes it before creating the target process.

use crate::error::SandboxProtocolError;
use crate::outcome::{CONFINEMENT_OUTCOME_VERSION, ConfinementOutcome, RefusalReason};
use crate::protocol::{MAX_REPORT_BYTES, REPORT_LENGTH_HEX};
use serde::{Deserialize, Serialize};
use std::io::Read;

/// What a self-test observed, from inside the confined process.
///
/// This is deliberately *evidence about behaviour*, not a claim of a control.
/// A control only appears in the outcome when a backend has run the matching
/// probe and seen a denial. These strings exist so a receipt can say how each
/// control was established, and so a reviewer can tell a measured denial from an
/// assumed one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
    /// The operation was attempted and refused.
    Denied,
    /// The operation was attempted and succeeded.
    Allowed,
    /// The probe could not run at all. Never counts as a denial.
    #[default]
    Inconclusive,
}

impl Observation {
    /// Only a real refusal proves anything. `Allowed` and `Inconclusive` both
    /// fail to prove a control, and conflating them would let a broken probe
    /// masquerade as a working sandbox.
    pub const fn proves_denial(self) -> bool {
        matches!(self, Self::Denied)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Allowed => "allowed",
            Self::Inconclusive => "inconclusive",
        }
    }
}

impl std::fmt::Display for Observation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The four required self-tests.
///
/// The workspace-write probe is the control that makes the rest meaningful: a
/// policy that denied everything would satisfy the first three.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proofs {
    /// A write attempted outside every allowed path.
    pub outside_write: Observation,
    /// A read attempted of a canary file outside every allowed path.
    pub canary_read: Observation,
    /// `socket(AF_INET)`.
    pub inet_socket: Observation,
    /// A write attempted inside the workspace, which must still work.
    pub inside_write: Observation,
}

impl Proofs {
    /// Proofs as nothing was observed. Used when a backend never ran.
    pub const fn none() -> Self {
        Self {
            outside_write: Observation::Inconclusive,
            canary_read: Observation::Inconclusive,
            inet_socket: Observation::Inconclusive,
            inside_write: Observation::Inconclusive,
        }
    }

    /// True only when all three denials were observed *and* the workspace write
    /// still succeeded. A deny-everything policy fails this, by design.
    pub const fn all_four_as_designed(&self) -> bool {
        self.outside_write.proves_denial()
            && self.canary_read.proves_denial()
            && self.inet_socket.proves_denial()
            && matches!(self.inside_write, Observation::Allowed)
    }
}

/// Which mechanisms actually applied, named rather than assumed.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mechanisms {
    /// The filesystem mechanism, e.g. `landlock_abi_4` or `seatbelt_sandbox_exec`.
    pub filesystem: String,
    /// The network mechanism, e.g. `seccomp_bpf` or `seatbelt_deny_network`.
    pub network: String,
    /// Exactly which address families the network rule denies. A blanket
    /// "network denied" is never claimed while this list is short.
    pub denied_socket_families: Vec<String>,
    /// rlimits applied, by name.
    pub rlimits: Vec<String>,
}

impl Mechanisms {
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether the claim of network denial is bounded by an explicit family
    /// list. A backend that denies only AF_INET and AF_INET6 must not present
    /// that as "network denied", because AF_UNIX remains available.
    pub fn network_claim_is_bounded(&self) -> bool {
        !self.denied_socket_families.is_empty()
    }
}

/// The full helper report.
///
/// Note what is *not* here: there is no `verified` list. The outcome carries
/// controls, and controls are only constructed by
/// [`crate::outcome::VerifiedControl::verified`] inside a backend that has run
/// the matching probe. Deserializing this struct can therefore never conjure a
/// control; the parent re-validates the outcome through
/// [`crate::accept_report`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReport {
    version: u16,
    outcome: ConfinementOutcome,
    restricted_identity: Option<String>,
    #[serde(default)]
    proofs: Proofs,
    #[serde(default)]
    mechanisms: Mechanisms,
}

impl VerificationReport {
    pub fn new(outcome: ConfinementOutcome, restricted_identity: Option<String>) -> Self {
        Self {
            version: CONFINEMENT_OUTCOME_VERSION,
            outcome,
            restricted_identity,
            proofs: Proofs::none(),
            mechanisms: Mechanisms::none(),
        }
    }

    pub fn with_proofs(mut self, proofs: Proofs) -> Self {
        self.proofs = proofs;
        self
    }

    pub fn with_mechanisms(mut self, mechanisms: Mechanisms) -> Self {
        self.mechanisms = mechanisms;
        self
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub const fn outcome(&self) -> &ConfinementOutcome {
        &self.outcome
    }

    pub fn restricted_identity(&self) -> Option<&str> {
        self.restricted_identity.as_deref()
    }

    pub const fn proofs(&self) -> &Proofs {
        &self.proofs
    }

    pub const fn mechanisms(&self) -> &Mechanisms {
        &self.mechanisms
    }
}

/// Encode the report into its single frame.
pub fn encode(report: &VerificationReport) -> Result<Vec<u8>, SandboxProtocolError> {
    let json = serde_json::to_vec(report).map_err(|_| SandboxProtocolError::Unserializable)?;
    if json.len() > MAX_REPORT_BYTES {
        return Err(SandboxProtocolError::ReportTooLarge {
            len: json.len(),
            limit: MAX_REPORT_BYTES,
        });
    }
    let mut framed = Vec::with_capacity(REPORT_LENGTH_HEX + json.len());
    framed.extend_from_slice(
        format!("{:0width$x}", json.len(), width = REPORT_LENGTH_HEX).as_bytes(),
    );
    framed.extend_from_slice(&json);
    Ok(framed)
}

/// Read exactly one frame from `reader`, then require EOF with zero further
/// bytes.
///
/// Every failure below is a refusal. None of them is `Unavailable`, because none
/// of them says anything about whether the platform has a backend.
pub fn read_one_frame<R: Read>(mut reader: R) -> Result<VerificationReport, RefusalReason> {
    let mut prefix = [0u8; REPORT_LENGTH_HEX];
    // A helper that dies before reporting leaves zero bytes here. That is a
    // refusal, not an empty-but-valid report.
    if let Err(_error) = reader.read_exact(&mut prefix) {
        return Err(RefusalReason::VerificationMissing);
    }
    let text = std::str::from_utf8(&prefix).map_err(|_| RefusalReason::SelfTestDisproved)?;
    let declared = usize::from_str_radix(text, 16).map_err(|_| RefusalReason::SelfTestDisproved)?;

    if declared == 0 {
        return Err(RefusalReason::SelfTestDisproved);
    }
    if declared > MAX_REPORT_BYTES {
        return Err(RefusalReason::SelfTestDisproved);
    }

    let mut body = vec![0u8; declared];
    // A short body means the helper died mid-write. Reading it as a truncated
    // success is the failure this branch exists to prevent.
    if reader.read_exact(&mut body).is_err() {
        return Err(RefusalReason::VerificationMissing);
    }

    // The invariant that makes "exactly one frame" checkable: after the declared
    // payload, the stream must be at EOF. One extra byte is a second claim and
    // is refused outright.
    let mut trailing = [0u8; 1];
    match reader.read(&mut trailing) {
        Ok(0) => {}
        Ok(_) => return Err(RefusalReason::SelfTestDisproved),
        Err(_error) => return Err(RefusalReason::VerificationMissing),
    }

    let report: VerificationReport =
        serde_json::from_slice(&body).map_err(|_| RefusalReason::SelfTestDisproved)?;
    if report.version() != CONFINEMENT_OUTCOME_VERSION {
        return Err(RefusalReason::SelfTestDisproved);
    }
    Ok(report)
}

/// Accepts a verification report, re-validating every field.
///
/// This is the same decision [`crate::accept_report`] makes, expressed over
/// [`VerificationReport`]. The STEP 1 entry point keeps its own signature so the
/// already-merged contract tests are unaffected; both funnel into one rule:
///
/// * no report is Refused;
/// * an outcome that does not satisfy every requested control is Refused, unless
///   the profile carries an explicit `allow_unsandboxed` decision.
///
/// Deserializing a [`VerificationReport`] cannot manufacture a control, because
/// the report type has no `verified` field. The controls live in the outcome,
/// and only a backend that has run a matching probe builds them.
pub fn accept_verified(
    report: Option<&VerificationReport>,
    profile: &crate::SandboxProfile,
) -> ConfinementOutcome {
    let Some(report) = report else {
        return ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing,
        };
    };
    let outcome = report.outcome();
    if !outcome.satisfies(&profile.requested_controls()) && !profile.allow_unsandboxed() {
        return ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing,
        };
    }
    outcome.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied_report() -> VerificationReport {
        VerificationReport::new(
            ConfinementOutcome::Applied {
                verified: Default::default(),
                unverified: Default::default(),
            },
            None,
        )
    }

    fn frame_for(report: &VerificationReport) -> Vec<u8> {
        encode(report).expect("the report encodes")
    }

    #[test]
    fn a_single_frame_round_trips() {
        let report = applied_report();
        let decoded = read_one_frame(frame_for(&report).as_slice()).expect("the frame decodes");
        assert_eq!(decoded.outcome(), report.outcome());
    }

    #[test]
    fn a_second_frame_is_refused() {
        let one = frame_for(&applied_report());
        let mut doubled = one.clone();
        doubled.extend_from_slice(&one);
        assert_eq!(
            read_one_frame(doubled.as_slice()).unwrap_err(),
            RefusalReason::SelfTestDisproved,
            "a duplicate frame must be refused, not read as the first one"
        );
    }

    #[test]
    fn a_trailing_byte_is_refused() {
        let mut bytes = frame_for(&applied_report());
        bytes.push(b'x');
        assert_eq!(
            read_one_frame(bytes.as_slice()).unwrap_err(),
            RefusalReason::SelfTestDisproved
        );
    }

    #[test]
    fn a_truncated_body_is_refused() {
        let bytes = frame_for(&applied_report());
        let truncated = &bytes[..bytes.len() - 4];
        assert_eq!(
            read_one_frame(truncated).unwrap_err(),
            RefusalReason::VerificationMissing
        );
    }

    #[test]
    fn an_absent_frame_is_refused() {
        assert_eq!(
            read_one_frame(&[][..]).unwrap_err(),
            RefusalReason::VerificationMissing,
            "a helper that dies before reporting is Refused, never Unavailable"
        );
    }

    #[test]
    fn an_oversized_declared_length_is_refused() {
        let bytes = format!("{:08x}", MAX_REPORT_BYTES + 1).into_bytes();
        assert_eq!(
            read_one_frame(bytes.as_slice()).unwrap_err(),
            RefusalReason::SelfTestDisproved
        );
    }

    #[test]
    fn a_zero_length_frame_is_refused() {
        assert_eq!(
            read_one_frame(b"00000000".as_slice()).unwrap_err(),
            RefusalReason::SelfTestDisproved
        );
    }

    #[test]
    fn a_non_numeric_length_is_refused() {
        assert_eq!(
            read_one_frame(b"zzzzzzzz{}".as_slice()).unwrap_err(),
            RefusalReason::SelfTestDisproved
        );
    }

    #[test]
    fn a_future_version_is_refused() {
        let json = r#"{"version":99,"outcome":{"kind":"refused","reason":"verification_missing"},"restricted_identity":null,"proofs":{},"mechanisms":{"filesystem":"","network":"","denied_socket_families":[],"rlimits":[]}}"#;
        let mut bytes = format!("{:08x}", json.len()).into_bytes();
        bytes.extend_from_slice(json.as_bytes());
        assert_eq!(
            read_one_frame(bytes.as_slice()).unwrap_err(),
            RefusalReason::SelfTestDisproved
        );
    }

    #[test]
    fn an_inconclusive_probe_never_proves_a_denial() {
        assert!(!Observation::Inconclusive.proves_denial());
        assert!(!Observation::Allowed.proves_denial());
        assert!(Observation::Denied.proves_denial());
    }

    #[test]
    fn a_deny_everything_policy_fails_the_four_proofs() {
        let deny_all = Proofs {
            outside_write: Observation::Denied,
            canary_read: Observation::Denied,
            inet_socket: Observation::Denied,
            inside_write: Observation::Denied,
        };
        assert!(
            !deny_all.all_four_as_designed(),
            "denying the workspace write must fail, or the first three denials prove nothing"
        );
    }

    #[test]
    fn the_designed_outcomes_satisfy_all_four() {
        let proofs = Proofs {
            outside_write: Observation::Denied,
            canary_read: Observation::Denied,
            inet_socket: Observation::Denied,
            inside_write: Observation::Allowed,
        };
        assert!(proofs.all_four_as_designed());
    }
}
