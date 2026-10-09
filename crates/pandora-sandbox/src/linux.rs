//! The Linux backend: Landlock, seccomp-bpf, and rlimits.
//!
//! Three mechanisms, applied in this order, each of which matters for the
//! ordering:
//!
//! 1. `PR_SET_NO_NEW_PRIVS`, before everything else. Landlock and an
//!    unprivileged seccomp filter both require it, and it is the flag that stops
//!    a `setuid` binary from regaining what confinement removed.
//! 2. Landlock, which restricts filesystem access by path.
//! 3. seccomp-bpf, which denies `socket()` for the two internet address
//!    families.
//!
//! Rlimits are applied alongside, and only what was actually set is reported.
//!
//! # What this backend does not do
//!
//! It does not grant anything. [`crate::derive_outcome`] rebuilds the control set
//! from [`probe`]'s observations, and this module's only job is to apply a real
//! mechanism and then tell the truth about what it saw.
//!
//! # Confinement that cannot be applied is a refusal
//!
//! Every failure below maps to [`UnavailableReason`], never to a partial
//! success. The Landlock ruleset is built with
//! [`CompatLevel::HardRequirement`], so a kernel that cannot enforce the rights
//! asked for returns an error rather than silently enforcing fewer of them, and
//! `restrict_self` must report [`RulesetStatus::FullyEnforced`] with
//! `no_new_privs` set. Anything else is a kernel that did not do what was asked,
//! and the answer is that confinement is unavailable.

use crate::ConfinementRequest;
use crate::outcome::UnavailableReason;
use crate::report::{Mechanisms, Observation, Proofs};
use landlock::{
    ABI, Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
    RulesetCreatedAttr, RulesetError, RulesetStatus,
};
use rustix::process::{self, Resource, Rlimit};
use rustix::thread::{no_new_privs, set_no_new_privs};
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule, TargetArch,
};
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::io;
use std::path::{Path, PathBuf};

/// The minimum Landlock ABI this backend will run on.
///
/// ABI 1 is the first revision that carries filesystem access rights at all, so
/// a kernel below it has no Landlock to speak of. The backend reports
/// [`UnavailableReason::UnsupportedRequest`] there rather than running with a
/// weaker filesystem story and calling it confined.
pub const REQUIRED_ABI: u8 = 1;

/// Environment variable read only by tests, to force a floor the running kernel
/// cannot meet.
///
/// This is the only way to exercise the "the kernel cannot give us what we
/// require" branch on a runner whose kernel *can*: there is otherwise no way to
/// make a modern kernel forget how to do Landlock. It is a simulation of an old
/// kernel, not a real one, and the PR body says so.
pub const TEST_ABI_FLOOR: &str = "PANDORA_SANDBOX_TEST_ABI_FLOOR";

/// The ceiling on open file descriptors. Only ever lowered, never raised.
const MAX_OPEN_FILES: u64 = 1024;

/// `EPERM` on every Linux architecture, from `include/uapi/asm-generic/errno-base.h`.
const EPERM: u32 = 1;

/// `socket()`'s first argument, which is the address family.
const SOCKET_DOMAIN_ARG: u8 = 0;

/// `AF_INET`, from `include/uapi/asm-generic/socket.h`. Stable across every
/// architecture seccompiler supports, which is why it is written here rather
/// than taken from `libc` and pinned to one.
const AF_INET: u64 = 2;

/// `AF_INET6`, same source.
const AF_INET6: u64 = 10;

/// Directory trees granted read-only so a confined process can start.
///
/// `AccessFs::from_read` includes `Execute`, so these also carry the permission
/// to run a binary out of them. Paths that do not exist are skipped rather than
/// failing the ruleset: a container without `/sbin` is not a confinement
/// failure.
const SYSTEM_READ_ONLY_PATHS: &[&str] = &["/usr", "/etc", "/lib", "/lib64", "/bin", "/sbin"];

/// Denies `socket()` for the two internet address families, leaving everything
/// else alone.
///
/// On success the caller is confined and must not undo it. Every error is a
/// reason confinement is unavailable; none is a partial success.
pub fn apply(request: &ConfinementRequest) -> Result<Mechanisms, UnavailableReason> {
    let floor = abi_floor();
    let detected = detected_abi().ok_or(UnavailableReason::BackendRefused)?;
    let detected = abi_version(detected);
    if detected < floor {
        // The kernel has Landlock, but not the revision this backend requires.
        // Running anyway would be a silent downgrade.
        return Err(UnavailableReason::UnsupportedRequest);
    }
    if floor < REQUIRED_ABI {
        return Err(UnavailableReason::UnsupportedRequest);
    }

    apply_no_new_privs()?;
    apply_landlock(request)?;
    let rlimits = apply_rlimits()?;
    apply_seccomp()?;

    Ok(Mechanisms {
        filesystem: "landlock".to_owned(),
        network: "seccomp_bpf".to_owned(),
        denied_socket_families: vec!["AF_INET".to_owned(), "AF_INET6".to_owned()],
        rlimits,
    })
}

/// Runs the four self-tests from inside the confined process.
///
/// Each observation records what actually happened, not what should have
/// happened. A probe that failed for any reason other than a permission refusal
/// is [`Observation::Inconclusive`]: a missing directory is not a confinement,
/// and treating it as one would let a broken probe impersonate a working
/// sandbox.
pub fn probe(request: &ConfinementRequest) -> Proofs {
    let outside_write = observe_write(&request.probe_dir.join("outside-write"));
    let canary = observe_read(&request.probe_dir.join("canary"));
    let inet_socket = observe_inet_socket();
    let inside_write = request
        .profile
        .writable_roots()
        .first()
        .map(|root| observe_write(&root.join("inside-write")))
        .unwrap_or(Observation::Inconclusive);

    Proofs {
        outside_write,
        canary_read: canary,
        inet_socket,
        inside_write,
    }
}

// ---------------------------------------------------------------------------
// Landlock
// ---------------------------------------------------------------------------

/// The highest Landlock ABI the running kernel supports, or `None` when the
/// kernel has no Landlock at all.
///
/// Probed by asking for the full access set of each revision in turn and taking
/// the first one the kernel accepts, rather than by comparing a version number:
/// the crate's compatibility machinery exists precisely because comparing
/// revision numbers directly misses the differences between them.
pub fn detected_abi() -> Option<ABI> {
    for abi in [
        ABI::V9,
        ABI::V8,
        ABI::V7,
        ABI::V6,
        ABI::V5,
        ABI::V4,
        ABI::V3,
        ABI::V2,
        ABI::V1,
    ] {
        let ruleset = Ruleset::default().set_compatibility(CompatLevel::HardRequirement);
        if ruleset.handle_access(AccessFs::from_all(abi)).is_ok() {
            return Some(abi);
        }
    }
    None
}

/// The ABI this backend requires, overridable only by
/// [`TEST_ABI_FLOOR`].
fn abi_floor() -> u8 {
    std::env::var(TEST_ABI_FLOOR)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(REQUIRED_ABI)
}

pub fn abi_version(abi: ABI) -> u8 {
    match abi {
        ABI::Unsupported => 0,
        ABI::V1 => 1,
        ABI::V2 => 2,
        ABI::V3 => 3,
        ABI::V4 => 4,
        ABI::V5 => 5,
        ABI::V6 => 6,
        ABI::V7 => 7,
        ABI::V8 => 8,
        ABI::V9 => 9,
        // `ABI` is marked non-exhaustive, so a revision this build does not know
        // about maps to zero. Zero is below every required floor, which makes an
        // unknown kernel report "unsupported" rather than silently pass a floor it
        // has not been measured against.
        _ => 0,
    }
}

fn apply_no_new_privs() -> Result<(), UnavailableReason> {
    set_no_new_privs(true).map_err(|_| UnavailableReason::BackendRefused)?;
    // Not "set it and hope": read it back, because a kernel that accepted the
    // call without applying it would leave every later mechanism weaker than
    // reported.
    if !no_new_privs().map_err(|_| UnavailableReason::BackendRefused)? {
        return Err(UnavailableReason::BackendRefused);
    }
    Ok(())
}

fn apply_landlock(request: &ConfinementRequest) -> Result<(), UnavailableReason> {
    let abi = detected_abi().ok_or(UnavailableReason::BackendRefused)?;
    // Handling every access right is what makes the default "denied": nothing
    // outside a granted path is reachable, rather than merely some things.
    let access = AccessFs::from_all(abi);
    let read = AccessFs::from_read(abi);

    let mut ruleset = Ruleset::default().set_compatibility(CompatLevel::HardRequirement);
    ruleset = ruleset.handle_access(access).map_err(ruleset_refused)?;
    let created = ruleset
        .create()
        .map_err(|_| UnavailableReason::BackendRefused)?;

    let mut rules = Vec::new();
    for path in SYSTEM_READ_ONLY_PATHS {
        if let Some(resolved) = resolved_path(path) {
            rules.push(PathBeneath::new(
                PathFd::new(&resolved).map_err(|_| UnavailableReason::BackendRefused)?,
                read,
            ));
        }
    }
    for root in request.profile.writable_roots() {
        if let Some(resolved) = resolved_path(root) {
            rules.push(PathBeneath::new(
                PathFd::new(&resolved).map_err(|_| UnavailableReason::BackendRefused)?,
                access,
            ));
        }
    }
    // The program that will be exec'd. Without this the `exec` that carries
    // confinement to the target would itself be denied.
    if let Some(dir) = &request.program_dir
        && let Some(resolved) = resolved_path(dir)
    {
        rules.push(PathBeneath::new(
            PathFd::new(&resolved).map_err(|_| UnavailableReason::BackendRefused)?,
            read,
        ));
    }

    let mut enforced = created;
    for rule in rules {
        enforced = enforced
            .add_rule(rule)
            .map_err(|_| UnavailableReason::BackendRefused)?;
    }
    let status = enforced
        .restrict_self()
        .map_err(|_| UnavailableReason::BackendRefused)?;

    // Both halves are load-bearing. `FullyEnforced` because a ruleset the kernel
    // only partly enforces is a weaker sandbox than reported, and `no_new_privs`
    // because Landlock's own protection assumes it.
    if !matches!(status.ruleset, RulesetStatus::FullyEnforced) || !status.no_new_privs {
        return Err(UnavailableReason::BackendRefused);
    }
    Ok(())
}

fn ruleset_refused(_error: RulesetError) -> UnavailableReason {
    // `HardRequirement` turns "the kernel cannot enforce this" into an error
    // here rather than into a ruleset with fewer rights than asked for.
    UnavailableReason::BackendRefused
}

/// Canonicalises a path, or `None` if it does not exist.
///
/// Canonicalising first is not tidiness: a rule attached to a symlink inode does
/// not apply to the directory it points at, and `/bin` is a symlink to `usr/bin`
/// on every mainstream distribution. A rule on the symlink would grant nothing.
fn resolved_path(path: impl AsRef<Path>) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

// ---------------------------------------------------------------------------
// rlimits
// ---------------------------------------------------------------------------

/// Applies the resource limits this backend supports, and names only the ones it
/// actually set.
fn apply_rlimits() -> Result<Vec<String>, UnavailableReason> {
    let mut applied = Vec::new();

    // No core dumps: a crash must not leave a memory image on disk for another
    // process to read credentials out of.
    process::setrlimit(
        Resource::Core,
        Rlimit {
            current: Some(0),
            maximum: Some(0),
        },
    )
    .map_err(|_| UnavailableReason::BackendRefused)?;
    applied.push("RLIMIT_CORE".to_owned());

    // Only ever lowered. Raising a soft limit needs privileges this process does
    // not have, and a limit that was already below the ceiling needs no change.
    let current = process::getrlimit(Resource::Nofile);
    if let Some(soft) = current.current
        && soft > MAX_OPEN_FILES
    {
        process::setrlimit(
            Resource::Nofile,
            Rlimit {
                current: Some(MAX_OPEN_FILES),
                // `None` leaves the hard limit alone, so this cannot fail by
                // trying to raise something only a privileged process may raise.
                maximum: None,
            },
        )
        .map_err(|_| UnavailableReason::BackendRefused)?;
        applied.push("RLIMIT_NOFILE".to_owned());
    }

    // `RLIMIT_NPROC` is deliberately absent. Limiting the number of processes a
    // uid may have is a system-wide limit on Linux, not a per-process one, and
    // lowering it from inside a sandboxed child would count against the *user*,
    // not the child. There is no safe per-process process-count limit to apply,
    // so the honest answer is not to apply one and not to claim one.

    Ok(applied)
}

// ---------------------------------------------------------------------------
// seccomp-bpf
// ---------------------------------------------------------------------------

/// Denies `socket()` for the two internet address families, leaving everything
/// else alone.
fn apply_seccomp() -> Result<Vec<String>, UnavailableReason> {
    let arch: TargetArch = std::env::consts::ARCH
        .try_into()
        .map_err(|_| UnavailableReason::UnsupportedRequest)?;

    let mut socket_rules = Vec::new();
    for family in [AF_INET, AF_INET6] {
        socket_rules.push(
            SeccompRule::new(vec![
                SeccompCondition::new(
                    SOCKET_DOMAIN_ARG,
                    SeccompCmpArgLen::Dword,
                    SeccompCmpOp::Eq,
                    family,
                )
                .map_err(|_| UnavailableReason::BackendRefused)?,
            ])
            .map_err(|_| UnavailableReason::BackendRefused)?,
        );
    }

    let mut rules = BTreeMap::new();
    rules.insert(socket_syscall(arch), socket_rules);

    // The default is Allow, because this filter exists to remove one capability,
    // not to build an allow-list. An allow-list built by hand would be a
    // footgun: it would break the target in ways that look like a sandbox bug.
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(EPERM),
        arch,
    )
    .map_err(|_| UnavailableReason::BackendRefused)?;
    let program: BpfProgram = filter
        .try_into()
        .map_err(|_| UnavailableReason::BackendRefused)?;
    seccompiler::apply_filter(&program).map_err(|_| UnavailableReason::BackendRefused)?;

    Ok(vec!["AF_INET".to_owned(), "AF_INET6".to_owned()])
}

/// The `socket` syscall number for an architecture seccompiler supports.
///
/// These are the three architectures seccompiler compiles filters for, and the
/// numbers are stable ABI: `socket` is 41 in the x86_64 table and 198 in the
/// asm-generic table that both aarch64 and riscv64 use. They are written out
/// rather than taken from `libc` so that this crate does not take a fourth
/// dependency for two constants.
fn socket_syscall(arch: TargetArch) -> i64 {
    match arch {
        TargetArch::x86_64 => 41,
        TargetArch::aarch64 => 198,
        TargetArch::riscv64 => 198,
    }
}

// ---------------------------------------------------------------------------
// The probes
// ---------------------------------------------------------------------------

/// What a filesystem probe saw.
fn observe_write(target: &Path) -> Observation {
    match std::fs::File::create(target) {
        Ok(_) => Observation::Allowed,
        Err(error) if is_denial(&error) => Observation::Denied,
        Err(_) => Observation::Inconclusive,
    }
}

fn observe_read(target: &Path) -> Observation {
    match std::fs::read(target) {
        Ok(_) => Observation::Allowed,
        Err(error) if is_denial(&error) => Observation::Denied,
        Err(_) => Observation::Inconclusive,
    }
}

/// An internet socket, attempted through the standard library rather than a raw
/// syscall: `bind` performs the `socket(AF_INET, ...)` call first, so a seccomp
/// refusal surfaces as the error from that call.
fn observe_inet_socket() -> Observation {
    match std::net::UdpSocket::bind("127.0.0.1:0") {
        Ok(_) => Observation::Allowed,
        Err(error) if is_denial(&error) => Observation::Denied,
        Err(_) => Observation::Inconclusive,
    }
}

/// Whether an error means "the kernel refused this" rather than "this could not
/// be attempted".
///
/// Only `EACCES` and `EPERM` count, which is what `PermissionDenied` covers, plus
/// a read-only filesystem. Anything else — a missing path, a name too long, a
/// directory that does not exist — is a probe that did not run, and a probe that
/// did not run must never be recorded as a denial.
fn is_denial(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}
