//! Platform detection and honest backend availability.
//!
//! STEP 1 implements no confinement backend. Every platform therefore probes as
//! `Unavailable`, including Windows, which the approved plan requires be
//! reported `Unavailable` for filesystem and network until a denied-operation
//! test proves otherwise.
//!
//! The reason this module exists rather than being folded into the helper is
//! that availability has to be decided *before* anything is spawned. A parent
//! that cannot learn whether confinement is available must refuse to run,
//! rather than spawn and hope.

use crate::outcome::{ProofKind, UnavailableReason, VerifiedControl};
use crate::profile::{RequestedControl, SandboxProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Which backend family this build knows how to attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformFamily {
    /// macOS Seatbelt.
    Seatbelt,
    /// Linux Landlock LSM.
    Landlock,
    /// Linux seccomp-bpf.
    Seccomp,
    /// Windows restricted token and AppContainer.
    Windows,
    /// Nothing is implemented for this target.
    Unsupported,
}

impl PlatformFamily {
    /// The backend family this build would use, if any.
    ///
    /// Compiled per target so an unlisted platform cannot accidentally inherit
    /// another one's backend by falling through a default.
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::Seatbelt
        }
        #[cfg(target_os = "linux")]
        {
            Self::Landlock
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Self::Unsupported
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Seatbelt => "seatbelt",
            Self::Landlock => "landlock",
            Self::Seccomp => "seccomp_bpf",
            Self::Windows => "windows_restricted_token",
            Self::Unsupported => "unsupported",
        }
    }
}

/// The result of asking whether a control can be proven on this host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Availability {
    family: PlatformFamily,
    provable: BTreeSet<RequestedControl>,
    reason: UnavailableReason,
}

impl Availability {
    /// Availability for the given platform family.
    ///
    /// STEP 1 ships no backend, so the provable set is empty on every platform.
    /// Windows is explicitly empty rather than "unknown": the approved plan
    /// requires filesystem and network be reported Unavailable there unless a
    /// denied-operation test proves them, and no such test exists yet.
    pub const fn probe(family: PlatformFamily) -> Self {
        Self {
            family,
            provable: BTreeSet::new(),
            reason: match family {
                PlatformFamily::Unsupported => UnavailableReason::NoBackendOnPlatform,
                // Every known family reports the same reason in STEP 1: the
                // backend exists on this platform but has not been implemented
                // here. STEP 2 replaces this with a real probe.
                _ => UnavailableReason::BackendRefused,
            },
        }
    }

    pub const fn family(&self) -> PlatformFamily {
        self.family
    }

    pub const fn reason(&self) -> UnavailableReason {
        self.reason
    }

    /// Controls this host could actually prove today. Empty in STEP 1.
    pub fn provable(&self) -> &BTreeSet<RequestedControl> {
        &self.provable
    }

    /// Whether every control this profile asks for could be proven.
    pub fn covers(&self, profile: &SandboxProfile) -> bool {
        profile
            .requested_controls()
            .iter()
            .all(|control| self.provable.contains(control))
    }

    /// A `VerifiedControl` for the given control, or the reason it cannot exist.
    ///
    /// Returning `None` is the important half: there is no path from "no probe"
    /// to "verified", so a control cannot be claimed without a proof.
    pub fn verify(&self, control: RequestedControl, proof: ProofKind) -> Option<VerifiedControl> {
        self.provable
            .contains(&control)
            .then(|| VerifiedControl::verified(control, self.family.as_str(), proof))
    }
}
