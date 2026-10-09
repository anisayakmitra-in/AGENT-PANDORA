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
use crate::outcome::{
    CONFINEMENT_OUTCOME_VERSION, ConfinementOutcome, ProofKind, RefusalReason, ReportedOutcome,
    VerifiedControl,
};
use crate::profile::RequestedControl;
use crate::protocol::{MAX_REPORT_BYTES, REPORT_LENGTH_HEX};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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
/// Note what is *not* here: there is no `verified` list, anywhere, at any depth.
/// The report carries a [`ReportedOutcome`] (which has no controls in it), the
/// [`Proofs`] the helper observed, and the [`Mechanisms`] it named. Deserializing
/// this struct therefore cannot conjure a control — there is no field to conjure
/// one into. The controls are built in the parent, by [`derive_outcome`], from
/// observations the parent has read itself.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReport {
    version: u16,
    reported: ReportedOutcome,
    restricted_identity: Option<String>,
    #[serde(default)]
    proofs: Proofs,
    #[serde(default)]
    mechanisms: Mechanisms,
}

impl VerificationReport {
    pub fn new(
        reported: ReportedOutcome,
        restricted_identity: Option<String>,
        proofs: Proofs,
        mechanisms: Mechanisms,
    ) -> Self {
        Self {
            version: CONFINEMENT_OUTCOME_VERSION,
            reported,
            restricted_identity,
            proofs,
            mechanisms,
        }
    }

    /// The report a helper sends when no backend could be applied. Used by
    /// tests and by the helper's own unavailability path.
    pub fn unavailable(reason: crate::outcome::UnavailableReason) -> Self {
        Self::new(
            ReportedOutcome::Unavailable { reason },
            None,
            Proofs::none(),
            Mechanisms::none(),
        )
    }

    /// The report a helper sends when it refuses. No observations, because a
    /// refusal is a decision rather than a measurement.
    pub fn refused(reason: RefusalReason) -> Self {
        Self::new(
            ReportedOutcome::Refused { reason },
            None,
            Proofs::none(),
            Mechanisms::none(),
        )
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub const fn reported(&self) -> &ReportedOutcome {
        &self.reported
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

/// The mechanism names this parent recognises.
///
/// A closed set, deliberately. The mechanism string on the wire is a claim
/// about *how*, and a claim about how is not what grants a control — the
/// observations are. But accepting an arbitrary string would let a helper put
/// attacker-chosen text into a receipt that later reads as if the parent had
/// blessed it. Matching against a list the parent compiled in means a receipt
/// can only ever name a mechanism this code knows about, and a backend that
/// names something unrecognised is treated as having named no mechanism at all,
/// which leaves its controls unproven.
const RECOGNISED_FILESYSTEM_MECHANISMS: &[(&str, &str)] = &[
    ("landlock", "landlock"),
    ("seatbelt_sandbox_exec", "seatbelt_sandbox_exec"),
];

const RECOGNISED_NETWORK_MECHANISMS: &[(&str, &str)] = &[
    ("seccomp_bpf", "seccomp_bpf"),
    ("seatbelt_deny_network", "seatbelt_deny_network"),
];

/// The canonical mechanism name for a reported one, or `None` if this parent
/// does not recognise it.
fn recognised(reported: &str, table: &[(&str, &'static str)]) -> Option<&'static str> {
    table
        .iter()
        .find(|(candidate, _)| *candidate == reported)
        .map(|(_, canonical)| *canonical)
}

/// Whether the helper named exactly the address families a `DenyAll` profile
/// needs denied.
///
/// `AF_UNIX` staying reachable is a recorded residual risk, not something this
/// function claims otherwise: `NetworkDenied` means "the internet is
/// unreachable", and a helper that denied only one of the two internet families
/// has not earned that.
fn denies_both_internet_families(mechanisms: &Mechanisms) -> bool {
    let families: BTreeSet<&str> = mechanisms
        .denied_socket_families
        .iter()
        .map(String::as_str)
        .collect();
    families.contains("AF_INET") && families.contains("AF_INET6")
}

/// The mechanism to record for `control`, or `None` when the observations do
/// not support it.
///
/// Two independent gates, both of which must pass:
///
/// 1. The observation the control depends on must actually show the denial, and
///    the mechanism name must be one this parent recognises.
/// 2. For the network control, the helper must have named both internet address
///    families as denied.
fn mechanism_for(
    control: RequestedControl,
    proofs: &Proofs,
    mechanisms: &Mechanisms,
) -> Option<&'static str> {
    match control {
        RequestedControl::FilesystemWriteRestricted
        | RequestedControl::FilesystemReadRestricted => {
            // Both filesystem controls rest on the pair of denials the profile's
            // workspace-write rule is meant to produce; neither is claimed if the
            // outside write was allowed through.
            if !proofs.outside_write.proves_denial() || !proofs.canary_read.proves_denial() {
                return None;
            }
            recognised(
                mechanisms.filesystem.as_str(),
                RECOGNISED_FILESYSTEM_MECHANISMS,
            )
        }
        RequestedControl::NetworkDenied => {
            if !proofs.inet_socket.proves_denial() || !denies_both_internet_families(mechanisms) {
                return None;
            }
            recognised(mechanisms.network.as_str(), RECOGNISED_NETWORK_MECHANISMS)
        }
    }
}

/// Turns what the helper reported into the parent's [`ConfinementOutcome`].
///
/// This is the only place a [`VerifiedControl`] is built, and it builds one only
/// where an observation the parent read supports it. The helper's role ends at
/// reporting; the judgement is made here.
///
/// The rule, in full:
///
/// * `Unavailable` and `Refused` pass through unchanged. Neither licenses
///   anything: [`ConfinementOutcome::verified`] is empty for both.
/// * `SelfTested` requires the workspace-write observation to have *succeeded*.
///   A policy that denied everything would satisfy the other three probes, so
///   without this a deny-everything sandbox would look like a working one.
///   A denial there is a refusal to have applied what was asked; an inconclusive
///   probe is a refusal for want of evidence.
/// * Each requested control is verified only if its observation shows the denial
///   and the mechanism is recognised. Everything else lands in `unverified`.
pub fn derive_outcome(
    reported: &ReportedOutcome,
    proofs: &Proofs,
    mechanisms: &Mechanisms,
    profile: &crate::SandboxProfile,
) -> ConfinementOutcome {
    match reported {
        ReportedOutcome::Unavailable { reason } => {
            ConfinementOutcome::Unavailable { reason: *reason }
        }
        ReportedOutcome::Refused { reason } => ConfinementOutcome::Refused { reason: *reason },
        ReportedOutcome::SelfTested => {
            // The workspace write must have worked. A policy that denied
            // everything would satisfy the other three probes, so without this
            // gate they would prove nothing at all. Denying it is a different
            // failure from never running it, and the two deserve different
            // answers: the first means the backend did not do what it claimed,
            // the second means there is no evidence either way.
            match proofs.inside_write {
                Observation::Denied => {
                    return ConfinementOutcome::Refused {
                        reason: RefusalReason::SelfTestDisproved,
                    };
                }
                Observation::Allowed => {}
                Observation::Inconclusive => {
                    return ConfinementOutcome::Refused {
                        reason: RefusalReason::VerificationMissing,
                    };
                }
            }
            let mut verified = BTreeSet::new();
            let mut unverified = BTreeSet::new();
            for control in profile.requested_controls() {
                match mechanism_for(control, proofs, mechanisms) {
                    // `ProofKind` is fixed at `DeniedOperation` because the only
                    // thing the parent can attest to is that a denied operation
                    // was observed. A backend that proved confinement some other
                    // way has still not proven it *to the parent*.
                    Some(mechanism) => {
                        verified.insert(VerifiedControl::verified(
                            control,
                            mechanism,
                            ProofKind::DeniedOperation,
                        ));
                    }
                    None => {
                        unverified.insert(control);
                    }
                }
            }
            ConfinementOutcome::Applied {
                verified,
                unverified,
            }
        }
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

/// The fail-closed rule, in one place.
///
/// An outcome that does not satisfy every requested control becomes a refusal
/// rather than a downgrade: the parent does not read `Unavailable` as permission
/// to proceed, and it does not read a partial `Applied` as permission to proceed
/// with less. The one exception is an explicit operator decision recorded in the
/// profile, and even then the *derived* outcome is returned unchanged so the
/// receipt and the containment evidence can both show that confinement was
/// skipped on purpose.
///
/// A refusal that is already a refusal keeps its own reason. Collapsing
/// `SelfTestDisproved` into the generic `VerificationMissing` would throw away
/// the only useful thing in it, which is *why* the helper stopped.
pub fn gate(derived: ConfinementOutcome, profile: &crate::SandboxProfile) -> ConfinementOutcome {
    if matches!(derived, ConfinementOutcome::Refused { .. }) {
        return derived;
    }
    if !derived.satisfies(&profile.requested_controls()) && !profile.allow_unsandboxed() {
        return ConfinementOutcome::Refused {
            reason: RefusalReason::VerificationMissing,
        };
    }
    derived
}

/// Accepts a verification report, re-deriving every control from its
/// observations.
///
/// The reported outcome is *not* trusted as a conclusion. [`derive_outcome`]
/// rebuilds the control set from the [`Proofs`] and [`Mechanisms`] the parent
/// read, and [`gate`] applies the one accept rule. Both are shared with
/// [`crate::accept_report`], so there is one decision rather than one per call
/// site.
pub fn accept_verified(
    report: Option<&VerificationReport>,
    profile: &crate::SandboxProfile,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn applied_report() -> VerificationReport {
        VerificationReport::new(
            ReportedOutcome::SelfTested,
            None,
            Proofs::none(),
            Mechanisms::none(),
        )
    }

    fn frame_for(report: &VerificationReport) -> Vec<u8> {
        encode(report).expect("the report encodes")
    }

    #[test]
    fn a_single_frame_round_trips() {
        let report = applied_report();
        let decoded = read_one_frame(frame_for(&report).as_slice()).expect("the frame decodes");
        assert_eq!(decoded.reported(), report.reported());
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
        let json = r#"{"version":99,"reported":{"kind":"refused","reason":"verification_missing"},"restricted_identity":null,"proofs":{"outside_write":"denied","canary_read":"denied","inet_socket":"denied","inside_write":"allowed"},"mechanisms":{"filesystem":"landlock","network":"seccomp_bpf","denied_socket_families":[],"rlimits":[]}}"#;
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

    fn profile() -> crate::SandboxProfile {
        crate::SandboxProfile::new(vec![std::path::PathBuf::from("/workspace")])
            .expect("the test profile is valid")
    }

    /// The observations a working Linux backend produces. Deliberately named
    /// after nothing: these are inputs, not claims.
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
    fn a_helper_that_observed_everything_yields_every_requested_control() {
        let outcome = derive_outcome(
            &ReportedOutcome::SelfTested,
            &all_denied(),
            &linux_mechanisms(),
            &profile(),
        );

        assert!(outcome.satisfies(&profile().requested_controls()));
        assert_eq!(outcome.verified().len(), 3);
    }

    #[test]
    fn an_unrecognised_mechanism_leaves_its_control_unverified() {
        let mechanisms = Mechanisms {
            filesystem: "a-mechanism-this-parent-has-never-heard-of".to_owned(),
            ..linux_mechanisms()
        };

        let outcome = derive_outcome(
            &ReportedOutcome::SelfTested,
            &all_denied(),
            &mechanisms,
            &profile(),
        );

        assert!(!outcome.satisfies(&profile().requested_controls()));
        assert!(
            outcome
                .verified()
                .iter()
                .all(|control| control.control() != RequestedControl::FilesystemWriteRestricted),
            "an unrecognised mechanism must not put a filesystem control in the receipt"
        );
    }

    #[test]
    fn a_probe_that_saw_no_denial_verifies_nothing() {
        let proofs = Proofs::none();

        let outcome = derive_outcome(
            &ReportedOutcome::SelfTested,
            &proofs,
            &linux_mechanisms(),
            &profile(),
        );

        assert!(outcome.verified().is_empty());
        assert!(!outcome.satisfies(&profile().requested_controls()));
    }

    #[test]
    fn a_deny_everything_helper_is_refused_rather_than_believed() {
        let deny_all = Proofs {
            inside_write: Observation::Denied,
            ..all_denied()
        };

        let outcome = derive_outcome(
            &ReportedOutcome::SelfTested,
            &deny_all,
            &linux_mechanisms(),
            &profile(),
        );

        assert_eq!(
            outcome,
            ConfinementOutcome::Refused {
                reason: RefusalReason::SelfTestDisproved
            },
            "a policy that denies the workspace too proves the other three denials mean nothing"
        );
    }

    #[test]
    fn denying_one_internet_family_is_not_network_denied() {
        let mechanisms = Mechanisms {
            denied_socket_families: vec!["AF_INET".to_owned()],
            ..linux_mechanisms()
        };

        let outcome = derive_outcome(
            &ReportedOutcome::SelfTested,
            &all_denied(),
            &mechanisms,
            &profile(),
        );

        assert!(
            outcome
                .verified()
                .iter()
                .all(|control| control.control() != RequestedControl::NetworkDenied),
            "denying AF_INET alone must not be recorded as network denied"
        );
    }

    #[test]
    fn unavailable_and_refused_pass_through_without_a_control() {
        let unavailable = derive_outcome(
            &ReportedOutcome::Unavailable {
                reason: crate::outcome::UnavailableReason::NoBackendOnPlatform,
            },
            &all_denied(),
            &linux_mechanisms(),
            &profile(),
        );
        assert!(unavailable.verified().is_empty());
        assert_eq!(unavailable.kind(), "unavailable");

        let refused = derive_outcome(
            &ReportedOutcome::Refused {
                reason: RefusalReason::UnconfinedNotPermitted,
            },
            &all_denied(),
            &linux_mechanisms(),
            &profile(),
        );
        assert!(refused.verified().is_empty());
        assert_eq!(refused.kind(), "refused");
    }
}
