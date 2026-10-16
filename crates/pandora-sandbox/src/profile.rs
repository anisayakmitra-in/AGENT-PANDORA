//! Confinement policy for a governed child process.
//!
//! This module describes *what confinement is requested*. It never reports that
//! a control was applied; that claim belongs to [`crate::outcome`] and can only
//! be made after a backend has verified it.

use crate::error::SandboxProfileError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

/// Version of the profile wire format, bumped when a field changes meaning.
pub const SANDBOX_PROFILE_VERSION: u16 = 1;

/// Upper bound on allowed writable roots, so a malformed profile cannot be used
/// to hand the helper an unbounded filesystem grant.
pub const MAX_WRITABLE_ROOTS: usize = 64;

/// What the caller wants confined, before any backend has acted.
///
/// A profile is a request, never an assertion. Building one cannot fail in a way
/// that implies confinement is available; that is what
/// [`crate::detect::probe`] is for.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxProfile {
    version: u16,
    filesystem: FilesystemConfinement,
    network: NetworkConfinement,
    /// Set only by an explicit operator decision. Recorded so the receipt and
    /// the containment evidence can both show that confinement was skipped on
    /// purpose rather than silently unavailable.
    allow_unsandboxed: bool,
}

impl SandboxProfile {
    /// The profile used when nothing more specific is demanded: deny network,
    /// and confine the filesystem to the given roots.
    pub fn new(writable_roots: Vec<PathBuf>) -> Result<Self, SandboxProfileError> {
        Self::build(
            FilesystemConfinement::WorkspaceOnly(writable_roots),
            NetworkConfinement::DenyAll,
            false,
        )
    }

    /// Builds a profile with explicit filesystem and network intent.
    ///
    /// `allow_unsandboxed` is deliberately awkward to set: callers are expected
    /// to pass it from an operator flag whose receipt and evidence they have
    /// already accounted for, not to compute it.
    pub fn build(
        filesystem: FilesystemConfinement,
        network: NetworkConfinement,
        allow_unsandboxed: bool,
    ) -> Result<Self, SandboxProfileError> {
        if let FilesystemConfinement::WorkspaceOnly(roots) = &filesystem
            && roots.len() > MAX_WRITABLE_ROOTS
        {
            return Err(SandboxProfileError::TooManyWritableRoots {
                count: roots.len(),
                limit: MAX_WRITABLE_ROOTS,
            });
        }
        Ok(Self {
            version: SANDBOX_PROFILE_VERSION,
            filesystem,
            network,
            allow_unsandboxed,
        })
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub const fn filesystem(&self) -> &FilesystemConfinement {
        &self.filesystem
    }

    pub const fn network(&self) -> &NetworkConfinement {
        &self.network
    }

    pub const fn allow_unsandboxed(&self) -> bool {
        self.allow_unsandboxed
    }

    /// The roots the filesystem request may write to. Empty for `Host`, which
    /// writes anywhere by request rather than by accident.
    pub fn writable_roots(&self) -> &[PathBuf] {
        match &self.filesystem {
            FilesystemConfinement::WorkspaceOnly(roots) => roots,
            FilesystemConfinement::Host => &[],
        }
    }

    /// The controls this profile asks for. A backend reports only the subset it
    /// verified, so this is an upper bound on what may ever be claimed.
    pub fn requested_controls(&self) -> BTreeSet<RequestedControl> {
        let mut requested = BTreeSet::new();
        match &self.filesystem {
            FilesystemConfinement::WorkspaceOnly(_) => {
                requested.insert(RequestedControl::FilesystemWriteRestricted);
                requested.insert(RequestedControl::FilesystemReadRestricted);
            }
            FilesystemConfinement::Host => {
                requested.insert(RequestedControl::FilesystemWriteRestricted);
            }
        }
        match self.network {
            NetworkConfinement::DenyAll => {
                requested.insert(RequestedControl::NetworkDenied);
            }
            NetworkConfinement::Inherit => {}
        }
        requested
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum FilesystemConfinement {
    /// Confine reads and writes to the given roots, plus the minimum a process
    /// needs to start. This is the default and the only value STEP 1 models as
    /// enforceable.
    WorkspaceOnly(Vec<PathBuf>),
    /// Leave the host filesystem alone. Used for executors such as the worktree
    /// git path whose whole job is repository I/O.
    Host,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum NetworkConfinement {
    /// Deny all sockets. The process executor's default.
    DenyAll,
    /// Leave the host network stack alone. Only reachable for an MCP server
    /// that was configured with network access, and only when the profile
    /// records it explicitly.
    Inherit,
}

/// A control a profile asks for. This is deliberately distinct from
/// [`crate::outcome::VerifiedControl`]: a request is not a verification.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedControl {
    FilesystemWriteRestricted,
    FilesystemReadRestricted,
    NetworkDenied,
}

impl RequestedControl {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FilesystemWriteRestricted => "filesystem_write_restricted",
            Self::FilesystemReadRestricted => "filesystem_read_restricted",
            Self::NetworkDenied => "network_denied",
        }
    }
}

impl fmt::Display for RequestedControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
