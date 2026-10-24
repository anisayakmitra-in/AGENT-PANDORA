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
    job_object_supported: bool,
}

impl Availability {
    /// Availability for the given platform family.
    ///
    /// On Windows, reports Job Object process confinement capabilities while
    /// keeping filesystem and network provable set empty until a denied-operation
    /// test proves them.
    pub const fn probe(family: PlatformFamily) -> Self {
        let job_object_supported = matches!(family, PlatformFamily::Windows);
        Self {
            family,
            provable: BTreeSet::new(),
            reason: match family {
                PlatformFamily::Unsupported => UnavailableReason::NoBackendOnPlatform,
                _ => UnavailableReason::BackendRefused,
            },
            job_object_supported,
        }
    }

    pub const fn family(&self) -> PlatformFamily {
        self.family
    }

    pub const fn reason(&self) -> UnavailableReason {
        self.reason
    }

    /// Whether Windows Job Object process confinement is supported.
    pub const fn job_object_supported(&self) -> bool {
        self.job_object_supported
    }

    /// Alias for `job_object_supported`.
    pub const fn supports_job_object(&self) -> bool {
        self.job_object_supported
    }

    /// Whether OS-level process confinement is available on this platform.
    pub const fn has_process_confinement(&self) -> bool {
        self.job_object_supported
            || matches!(
                self.family,
                PlatformFamily::Windows | PlatformFamily::Landlock | PlatformFamily::Seatbelt
            )
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
