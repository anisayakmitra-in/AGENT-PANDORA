//! What a backend may claim after trying, and what it must say when it could not.

use crate::profile::RequestedControl;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Version of the outcome wire format shared with the helper.
pub const CONFINEMENT_OUTCOME_VERSION: u16 = 1;

/// The only three answers a backend may give.
///
/// There is deliberately no `Applied` variant that is reachable without a
/// [`VerifiedControl`], and no way to construct a verified control other than
/// [`VerifiedControl::verified`], which takes the probe result that justified
/// it. That is what stops a backend from reporting a capability it merely
/// believes it has.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

/// A control a backend proved by behaviour, plus what it proved it with.
///
/// The proof is retained rather than discarded so a receipt can state *how* a
/// control was established, and so a reviewer can tell a verified control from
/// an asserted one.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedControl {
    control: RequestedControl,
    mechanism: String,
    proof: ProofKind,
}

impl VerifiedControl {
    /// The only constructor. Callers must name the mechanism and the proof they
    /// actually performed.
    ///
    /// The mechanism is taken as `&'static str` so a caller cannot pass a
    /// runtime-computed name, then stored as `String` because this type is
    /// deserialized from the helper's report.
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
