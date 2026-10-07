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
2. proves it by attempting denied operations **from inside the confined
   process**,
3. writes exactly one verification report to the parent over a pipe,
4. explicitly closes the write end,
5. `exec`s the target, which inherits the confinement.

The parent never infers confinement from the existence of a capability. A
report that says a mechanism was applied is not evidence; only an observed
denial is. A control can only enter an outcome through
`VerifiedControl::verified`, which requires naming the proof performed, so there
is no path from "the API call succeeded" to "the control holds".

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

## Confinement is not authority

Confinement is defence in depth *underneath* the permit path. It never grants
authority. A sandbox narrows what an already-permitted effect can reach; it
cannot permit an effect that Parliament refused, and a sandbox failure is always
a refusal to run rather than a reason to run unconfined. `--allow-unsandboxed`
is an explicit operator decision and must be recorded in both the receipt and
the containment evidence, so a deliberate skip is visible rather than silent.