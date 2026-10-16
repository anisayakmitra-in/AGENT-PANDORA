# ADR: Sandbox report integrity and confinement backends

Status: accepted, in progress

This records the confinement protocol and the state of the backends. It is
written before the backends work rather than after, so the requirements a
backend must satisfy are reviewable before any backend claims to meet them.

## Context

STEP 1 added the `pandora-sandbox` contract crate. It defines what a control
*is*, how a control may be claimed, and the fail-closed rule for accepting a
report. It ships no backend, so every platform probed `Unavailable` and the
parent refused to run. That was deliberate: the plumbing is testable before
anything can be claimed.

This record covers two things. The report protocol, which is implemented and
tested here. And the backend requirements, which are specified here and
implemented in later pull requests.

## The helper protocol

The parent spawns a helper instead of the target, so confinement can be applied
to *this* process before `exec` replaces the image. Confining after the target
started would be a race the target could win.

The helper:

1. applies confinement,
2. runs the self-tests **from inside the confined process** and records what it
   observed,
3. writes exactly one report to the parent over a pipe, carrying those
   observations,
4. explicitly closes the write end,
5. `exec`s the target, which inherits the confinement.

The parent never infers confinement from the existence of a capability, and it
never takes the helper's word for which controls hold. The helper reports what it
observed; the parent rebuilds the control set itself. A control can only enter an
outcome through `VerifiedControl::verified`, which the parent calls from
`derive_outcome` and only where an observation supports it, so there is no path
from "the helper said so" to "the control holds".

### The wire carries observations, not conclusions

The report format has no field in which a control can arrive. `VerifiedControl`
and `ConfinementOutcome` are `Serialize`-only; the deserializable types are
`ReportedOutcome` (`SelfTested` / `Unavailable` / `Refused`), `Proofs`,
`Mechanisms`, and the two report structs wrapping them. A frame carrying a
`verified` list is refused outright rather than downgraded, because there is
nowhere to put one.

This is version **2** of the report wire format. Version 1 let the helper state
which controls held, and a helper that stated them could state confinement it had
never applied: the parent cloned the deserialized outcome and checked only that
the claimed set covered the profile. Version 1 frames are refused, not
reinterpreted.

The parent's derivation rules, in full:

- `Unavailable` and `Refused` pass through unchanged, and license nothing.
- `SelfTested` requires the workspace-write observation to have been `Allowed`. A
  policy that denied everything satisfies the other three probes, so this is what
  makes them mean something. A denial here is `SelfTestDisproved`; an
  inconclusive probe is `VerificationMissing`.
- Each requested control is verified only if its observation shows the denial
  **and** the mechanism is one this parent recognises. Everything else lands in
  `unverified`, which never satisfies a profile.
- A network control additionally requires the helper to have named both
  `AF_INET` and `AF_INET6` as denied. Denying one is not network denial.

**Mechanism names come from a closed set compiled into the parent.** A backend
that names something unrecognised is treated as having named nothing, which
leaves its controls unproven. This is not the security boundary — the observation
is — it is what stops attacker-chosen text from landing in a receipt that later
reads as though the parent had blessed it.

**What the parent still has to trust, stated plainly.** The parent trusts the
helper's *observations*; it does not trust the helper's *conclusions*, and it
cannot re-run the probes itself. A helper that lies about what it observed would
still be believed. That is irreducible in this design rather than a defect to be
fixed later: the helper is this repository's own binary, the code it confines has
not started yet, and the `exec`'d target never holds the write end of the report
channel. Confinement cannot defend against a helper that has already been
subverted, because by then it is not the thing under attack.

### One frame, then EOF

The frame is a fixed-width hex length prefix followed by that many bytes of
JSON, with a hard maximum size. The parent reads **exactly one** frame and then
requires EOF with **zero** further bytes.

Fixed-width rather than a newline delimiter, so a helper that dies mid-write
leaves the parent reading a short buffer and refusing, instead of waiting
forever. EOF is not the mechanism of detection; it is the confirmation that
nothing else was sent.

Refused, in every case: malformed, truncated, duplicate, oversized, missing,
and any trailing bytes. None of these is `Unavailable`. `Unavailable` means "no
backend on this platform", which is a statement about the platform. A bad frame
is a statement about this exchange, and conflating them would let a helper
failure be reported as a platform limitation.

A helper that dies before reporting is `Refused`, not `Unavailable`.

### What EOF proves, per platform

**Linux and macOS.** The write end is close-on-exec. The helper closes it
explicitly, and the kernel closes it again at `exec` if it were somehow still
open. The `exec`'d target therefore never holds the write end. EOF is reached
deterministically once the helper has written and closed, and it proves that no
second write was possible.

**Windows.** There is no `exec`. The helper spawns the target as a child and
then exits, so EOF arrives when the helper exits rather than when the target
starts. The guarantee is weaker by construction, and a target that inherits the
handle could in principle write a second frame. This is one reason Windows stays
`Unavailable` rather than being given a backend on the strength of a
denied-operation test it cannot pass.

### The Windows requirement, for later

Recorded now, implemented when a Windows backend exists:

- the pipe handle must be **non-inheritable**, so nothing but the helper can
  receive it;
- it must be passed only to the helper, not through the target's handle
  inheritance;
- the helper must **close it before creating the target process**.

With those three properties the Windows case would reach the same guarantee as
Unix, because the target could never hold the write end. Until they hold,
Windows confinement would be a real user identity rather than an absence of
permission, and it could not be proven by a denied-operation test.

## Backends

Both backends must prove four things from inside the confined process:

| probe | expected |
|---|---|
| write outside every allowed path | denied |
| read a canary file outside every allowed path | denied |
| `socket(AF_INET)` | denied |
| write inside the workspace | **allowed** |

The fourth is the control for the other three. A policy that denied everything
would satisfy the first three, so a workspace write that still succeeds is what
makes the denials mean something. `Proofs::all_four_as_designed` fails a
deny-everything result deliberately.

A probe that cannot run records `Inconclusive`, which never proves a control. A
broken probe and a working sandbox must not look alike.

### Linux: Landlock, seccomp-bpf, rlimits

Detect the Landlock ABI at runtime. Apply a filesystem ruleset with read-only
paths for what a process needs to start, read-write for the workspace roots
only, and everything else denied. Set `no_new_privs`, which both Landlock and
seccomp require.

Network is seccomp-bpf denying `socket()` for `AF_INET` and `AF_INET6`.
**`AF_UNIX` remains available, so the report must not claim blanket "network
denied".** It names the denied families explicitly, and a reviewer can see that
the claim is bounded. Landlock network rules need ABI 4; if detected they may
be used, and the report states which mechanism actually did the work rather than
the stricter-sounding one.

Rlimits cover address space, process count and CPU time, and the report names
only those actually applied.

If the kernel lacks Landlock or seccomp, the outcome is `Unavailable` with the
reason. There is no silent fallback to a weaker mode: a weaker mode reported as
the stronger one is the failure this crate exists to prevent.

### The Linux backend

Implemented in `src/linux.rs`, behind `#[cfg(target_os = "linux")]`, and reached
by the helper only through `pandora_sandbox::confine`. The library half still
holds no confinement code: applying a mechanism is a side effect on the caller,
and the only caller that may accept that is the code about to become the target.

**Order.** `PR_SET_NO_NEW_PRIVS` first, then Landlock, then seccomp, with rlimits
alongside. `no_new_privs` is set *and read back*, because a kernel that accepted
the call without applying it would leave every later mechanism weaker than
reported, and Landlock's own protection assumes the flag is on. The ruleset is
built with `CompatLevel::HardRequirement`, `restrict_self` must return
`RulesetStatus::FullyEnforced`, and the running ABI must meet `REQUIRED_ABI` — a
kernel below it is `UnsupportedRequest`, never a partially confined run.

**Filesystem.** Every access right is *handled*, which is what makes the default
a denial: nothing outside a granted path is reachable. `AccessFs::from_all` on
the workspace roots; `AccessFs::from_read` — which includes `Execute` — on the
system directories and on the target program's directory. Paths are
canonicalised before a rule is attached, because a rule on a symlink inode
grants nothing: `/bin` is a symlink to `usr/bin` on every mainstream
distribution. An `exec` after confinement is a path lookup like any other, so
without the program-directory rule the `exec` that carries confinement to the
target would itself be denied.

**What the probes mean.** `outside_write`, `canary_read` and `inet_socket` count
as denials only when the error is `EACCES` or `EPERM`. Anything else — a missing
path, a name too long, a directory that is absent — records `Inconclusive`,
because a probe that did not run must never look like a confinement. The fourth
probe, `inside_write`, is the control on the other three: a policy that denied
everything satisfies those, so a workspace write that still succeeds is what
makes them mean something.

**`RLIMIT_NPROC` is not applied.** On Linux the process-count limit is a
per-*user* limit, not a per-process one, and lowering it from inside a sandboxed
child counts against the user rather than the child. There is no safe
per-process process-count limit to apply, so the honest answer is to apply
nothing and claim nothing.

### macOS: deny-default Seatbelt

Generate a deny-default Seatbelt profile from the `SandboxProfile`. Allow only
the read paths needed to start, writes to the workspace only, and
`deny network*` as a category rather than by omission.

**Why `sandbox-exec` on the helper's second stage, not the sandbox API
directly.** Two mechanisms exist. `sandbox_init`/`sandbox_apply` confines the
calling process; `sandbox-exec` takes the same profile text and confines a child
it then execs.

The choice is forced by ordering, not convenience. The helper must confine
itself *before* becoming the target. `sandbox_apply` can only confine the
calling process, which cannot then `exec` into an unconfined image, so the
confinement would not survive. `sandbox-exec` confines the process it execs, so
the helper re-execs *itself* through it and the target inherits confinement that
was already in force.

`sandbox-exec` is deprecated by Apple and warns when used. It remains
supported on current macOS releases and is the only one of the two mechanisms
that survives `exec`, which is what this requirement needs. If it is ever
removed, the alternative is a small platform shim over `sandbox_apply` that
confines a forked child before it execs, which keeps the same ordering property.

### Windows

No backend. The probe returns `Unavailable`. See the requirement above.

## Known residual risks

**Linux, `AF_UNIX` remains available.** Denying only the internet families
leaves local sockets working, so a confined process can still talk to local
services over a Unix socket. The report says so. Denying `AF_UNIX` too is a
follow-up, and it will break legitimate local IPC.

**Linux, credential files under `$HOME` stay readable.** The backend grants
`AccessFs::from_read` on the system directories, and nothing yet narrows that
per profile, so a host's credential files stay readable by a confined process.
Narrowing it needs the read paths plumbed through `SandboxProfile` and the
executors, which is later work. Recorded here rather than described as confined.

**Linux, the ABI floor is a simulated old kernel in tests.** There is no way to
make a modern kernel forget how to do Landlock, so the "kernel below the
required ABI" branch is exercised by forcing the floor through
`PANDORA_SANDBOX_TEST_ABI_FLOOR`. That tests the refusal path, not a real old
kernel. The real old-kernel behaviour is covered only by
`detected_abi()` returning `None` and by `HardRequirement` refusing to build a
ruleset it cannot enforce.

**Linux, `/proc` and `/sys` reads.** Landlock grants read-only access to what a
process needs to start. If `/proc` is reachable, some kernel interfaces leak
information about the host. This is a narrower grant than most programs expect
and may need per-executor tuning.

**macOS, `sandbox-exec` is deprecated.** It warns on use and may be removed.
The replacement has to preserve confinement across `exec`, which is the property
that forced this choice.

**Neither backend confines the parent.** The helper is confined, then execs. If
the parent spawns something else, that is not covered. Wiring the executors is
later work and must go through the permit path unchanged.

**`HOME` readable for worktree git hooks.** The worktree executor clears the
environment of every git child and allows only `PATH`, `HOME` and
`USERPROFILE`. Until the filesystem sandbox covers the git path, `HOME` stays
readable, so a repository that plants a hook in the user's hooks directory is
not prevented from running it by environment clearing alone. The sandbox does not
close this: the value has to be present for git to work at all. Removing the
exposure means not inheriting `HOME` for git, which is a separate change to
`worktree.rs` and needs its own review.

**Proofs are single-shot.** Each self-test runs once, at startup, in the
confined process. A mechanism that is later relaxed inside the target is not
detected. Continuous confinement monitoring is out of scope.

**The parent's trust boundary is the helper's honesty about its own probes.** The
parent rebuilds every control from the observations the helper reports and trusts
no conclusion it sends, but it cannot independently observe the denials. A
subverted helper could report denials that never happened. See "The wire carries
observations, not conclusions" above for why that is the honest limit here.

## Confinement is not authority

Confinement is defence in depth *underneath* the permit path. It never grants
authority. A sandbox narrows what an already-permitted effect can reach; it
cannot permit an effect that Parliament refused, and a sandbox failure is always
a refusal to run rather than a reason to run unconfined. `--allow-unsandboxed`
is an explicit operator decision and must be recorded in both the receipt and
the containment evidence, so a deliberate skip is visible rather than silent.