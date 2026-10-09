//! Platform detection and honest backend availability.
//!
//! Availability is decided *before* anything is spawned. A parent that cannot
//! learn whether confinement is available must refuse to run, rather than spawn
//! and hope.
//!
//! Detection is a pre-spawn check, not the answer. A host can have a usable
//! kernel ABI and still fail to enforce a ruleset; only [`crate::confine`]
//! discovers that, and its observations — not this module — are what the parent
//! derives controls from.

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
    /// This is a pre-spawn check, not the answer. On Linux it reports what the
    /// backend can attempt; the authoritative statement comes from
    /// [`crate::confine`] and the observations its probes return, because a
    /// host can have Landlock and still fail to enforce a ruleset.
    ///
    /// Windows is explicitly empty rather than "unknown": the approved plan
    /// requires filesystem and network be reported Unavailable there unless a
    /// denied-operation test proves them, and no such test exists yet.
    pub fn probe(family: PlatformFamily) -> Self {
        Self {
            family,
            provable: provable_for(family),
            reason: match family {
                PlatformFamily::Unsupported => UnavailableReason::NoBackendOnPlatform,
                // Every other family without a backend reports this: a backend
                // exists for the platform but has not been written here.
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

    /// Controls this host could actually prove today, before any of it runs.
    pub fn provable(&self) -> &BTreeSet<RequestedControl> {
        &self.provable
    }

    /// Whether every control this profile asks for could be attempted here.
    ///
    /// This is the pre-spawn gate, and only that: a host that passes it can still
    /// fail to confine, which [`crate::confine`] discovers and reports.
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

/// What this build's backend could attempt on this host, before any of it runs.
///
/// Empty everywhere except Linux, and there only when the kernel actually has a
/// usable Landlock ABI. A host whose kernel does not is reported as covering
/// nothing, so the parent refuses rather than spawning a child it cannot confine.
fn provable_for(family: PlatformFamily) -> BTreeSet<RequestedControl> {
    let linux_can_attempt = {
        #[cfg(target_os = "linux")]
        {
            matches!(family, PlatformFamily::Landlock | PlatformFamily::Seccomp)
                && crate::linux::detected_abi()
                    .is_some_and(|abi| crate::linux::abi_version(abi) >= crate::linux::REQUIRED_ABI)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = family;
            false
        }
    };

    if linux_can_attempt {
        [
            RequestedControl::FilesystemWriteRestricted,
            RequestedControl::FilesystemReadRestricted,
            RequestedControl::NetworkDenied,
        ]
        .into_iter()
        .collect()
    } else {
        BTreeSet::new()
    }
}
