//! What a backend may claim after trying, and what it must say when it could not.

use crate::profile::RequestedControl;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Version of the report wire format shared with the helper.
///
/// Bumped to 2 when the wire stopped carrying a *conclusion* and started
/// carrying *observations*. Version 1 let the helper state which controls
/// held; version 2 lets it state only what its probes observed, and the parent
/// derives the controls from that. The old shape is refused rather than
/// reinterpreted, so a version-1 helper cannot be mistaken for a version-2 one.
pub const CONFINEMENT_OUTCOME_VERSION: u16 = 2;

/// The only three answers a backend may give.
///
/// This type is the **parent's** answer, derived by
/// [`crate::derive_outcome`] from what the helper observed. It deliberately
/// does not implement [`serde::Deserialize`]: there is no `Applied` variant
/// that is reachable without a [`VerifiedControl`], no way to construct a
/// verified control other than [`VerifiedControl::verified`], and — because
/// nothing off the wire can build one — no path from bytes on a pipe to "this
/// control holds".
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ConfinementOutcome {
    /// Every requested control was proven. `verified` is non-empty and a
    /// superset of what the profile asked for.
    Applied {
        verified: BTreeSet<VerifiedControl>,
        /// Controls the profile asked for that this backend could not prove.
        unverified: BTreeSet<RequestedControl>,
    },
    /// The backend exists but could not prove a requested control. This is the
    /// honest answer on a platform without a working backend, and on Windows
    /// until a denied-operation test proves filesystem or network confinement.
    Unavailable { reason: UnavailableReason },
    /// The helper refused to proceed. Distinct from `Unavailable`: a refusal is
    /// a decision, and the parent must not retry it as though a backend might
    /// appear later.
    Refused { reason: RefusalReason },
}

impl ConfinementOutcome {
    /// The controls this outcome licenses the caller to claim. Empty unless the
    /// outcome is `Applied`, so an `Unavailable` or `Refused` outcome can never
    /// be laundered into evidence.
    pub fn verified(&self) -> BTreeSet<VerifiedControl> {
        match self {
            Self::Applied { verified, .. } => verified.clone(),
            Self::Unavailable { .. } | Self::Refused { .. } => BTreeSet::new(),
        }
    }

    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Applied { .. })
    }

    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Applied { .. } => "applied",
            Self::Unavailable { .. } => "unavailable",
            Self::Refused { .. } => "refused",
        }
    }

    /// A profile is only satisfied when every requested control was verified and
    /// nothing was left unverified. Partial coverage is not satisfaction.
    pub fn satisfies(&self, requested: &BTreeSet<RequestedControl>) -> bool {
        match self {
            Self::Applied {
                verified,
                unverified,
            } => {
                unverified.is_empty()
                    && requested
                        .iter()
                        .all(|control| verified.iter().any(|entry| entry.control() == *control))
            }
            Self::Unavailable { .. } | Self::Refused { .. } => false,
        }
    }
}

/// What the helper puts on the wire.
///
/// This is deliberately *not* a [`ConfinementOutcome`]. It carries no controls
/// and no verdict about any control, because a helper that is allowed to state
/// which confinement holds is a helper that can state confinement it never
/// applied. The helper reports what its probes observed in
/// [`crate::Proofs`] and [`crate::Mechanisms`]; the parent derives the
/// controls, in the parent, from those observations and the profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportedOutcome {
    /// The helper applied what it could and ran the self-tests inside the
    /// confined process. It does not say which controls hold; the observations
    /// do, and only the parent turns them into controls.
    SelfTested,
    /// No backend could be applied on this host. A statement about the platform,
    /// not about this exchange.
    Unavailable { reason: UnavailableReason },
    /// The helper refused to proceed. A decision, not an absence.
    Refused { reason: RefusalReason },
}

impl ReportedOutcome {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::SelfTested => "self_tested",
            Self::Unavailable { .. } => "unavailable",
            Self::Refused { .. } => "refused",
        }
    }
}

/// A control a backend proved by behaviour, plus what it proved it with.
///
/// The proof is retained rather than discarded so a receipt can state *how* a
/// control was established, and so a reviewer can tell a verified control from
/// an asserted one.
///
/// This type is **not** [`serde::Deserialize`]'d, and must never become so. It
/// is the load-bearing half of the crate's evidence claim: a `VerifiedControl`
/// can only be built by [`VerifiedControl::verified`], in the parent, from a
/// [`crate::Proofs`] observation the parent has read itself. Dropping the
/// `Deserialize` derive is what makes that a type-checked fact instead of a
/// convention, so `tests/fabrication.rs` asserts the absence at compile time.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct VerifiedControl {
    control: RequestedControl,
    mechanism: String,
    proof: ProofKind,
}

impl VerifiedControl {
    /// The only constructor. Callers must name the mechanism and the proof they
    /// actually performed.
    ///
    /// `mechanism` is `&'static str` so the name can only ever come from a
    /// closed set the parent recognises, never from a string that arrived over
    /// the wire. The only caller is [`crate::derive_outcome`], which supplies a
    /// mechanism it matched against that closed set *and* whose control the
    /// observations the parent read actually support.
    pub fn verified(control: RequestedControl, mechanism: &'static str, proof: ProofKind) -> Self {
        Self {
            control,
            mechanism: mechanism.to_owned(),
            proof,
        }
    }

    pub const fn control(&self) -> RequestedControl {
        self.control
    }

    pub fn mechanism(&self) -> &str {
        &self.mechanism
    }

    pub const fn proof(&self) -> ProofKind {
        self.proof
    }
}

/// How a control was proven.
///
/// `Unavailable` is a member of this enum rather than an absent field on
/// purpose: a caller must state that no proof exists, which makes "I did not
/// check" distinguishable from "I checked and it did not hold".
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofKind {
    /// A denied operation was attempted and refused. The only proof that counts
    /// for filesystem and network confinement.
    DeniedOperation,
    /// A real user or group identity was created and confirmed. Windows
    /// restricted-token and AppContainer confinement cannot be proven by
    /// absence, so Windows reports `Unavailable` until this exists.
    ConfirmedIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// No backend has been implemented for this platform yet.
    NoBackendOnPlatform,
    /// A backend exists but the kernel or OS refused to grant it.
    BackendRefused,
    /// The backend is present but this profile asked for something it cannot
    /// express.
    UnsupportedRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// The helper was asked to run with no confinement and the profile does not
    /// permit it.
    UnconfinedNotPermitted,
    /// The helper could not read its report channel.
    ReportChannelUnusable,
    /// Verification was required and not received, or received unparseable.
    VerificationMissing,
    /// The self-test proved the confinement was not in force after it was
    /// applied, which means the backend did not do what it claimed.
    SelfTestDisproved,
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBackendOnPlatform => {
                formatter.write_str("no confinement backend on this platform")
            }
            Self::BackendRefused => formatter.write_str("the backend refused to grant confinement"),
            Self::UnsupportedRequest => {
                formatter.write_str("the backend cannot express this request")
            }
        }
    }
}

impl fmt::Display for RefusalReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnconfinedNotPermitted => {
                formatter.write_str("unconfined execution is not permitted by the profile")
            }
            Self::ReportChannelUnusable => formatter.write_str("the report channel was unusable"),
            Self::VerificationMissing => {
                formatter.write_str("confinement was not verified before executing")
            }
            Self::SelfTestDisproved => {
                formatter.write_str("the self-test showed confinement was not in force")
            }
        }
    }
}
