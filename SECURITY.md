# Security Policy

Pandora treats tool execution, package admission, provider access, credentials,
and persisted agent state as security boundaries.

## Supported versions

| Version | Security fixes |
|---|---|
| `main` | Yes, when the change is part of the supported development line |
| Latest published release | Yes |
| Older releases | Best effort only; upgrade before reporting an issue against an old release |

The repository is in prerelease development. Do not use the local MCP preview
or unreleased binaries with untrusted native executables.

## Reporting a vulnerability

Please do not disclose an exploitable vulnerability in a public issue. Use the
GitHub repository's private security advisory workflow when it is available. If
that workflow is unavailable, open a minimal issue asking for a private contact
channel without including exploit details.

Include:

- affected commit, release, or platform;
- precise component and reachable entry point;
- security impact and prerequisites;
- a minimal reproduction that does not expose secrets or destructive payloads;
- any proposed mitigation and whether exploitation is active.

Remove API keys, access tokens, private source, personal data, and destructive
payloads from reports. If a report includes sensitive material, revoke exposed
credentials immediately and tell the maintainers what was revoked.

Maintainers will acknowledge a report when they can and will keep a reporter's
identity confidential unless disclosure is required by law or the reporter
asks to be identified. Coordinated disclosure dates are agreed with the
reporter after impact and affected versions are confirmed.

## Scope

High-value reports include permit bypasses, approval reuse, package identity or
signature confusion, path traversal, credential exposure, cross-workspace data
leaks, unsafe process execution, and tenant or provider-cache contamination.

The local MCP preview assumes an explicitly configured operator-trusted process;
it is not an OS sandbox for hostile native executables.

Reports about provider-side model behavior, third-party services, or an
operator's intentionally granted permissions belong with the relevant provider
or administrator unless Pandora creates an additional security boundary.

## Security design commitments

- No valid permit means no governed effect.
- Permits are scoped, expiring, and one-shot.
- Runtime policy remains outside model output and self-improvement loops.
- Credentials and hidden reasoning are not persisted as ordinary evidence.
- Package hashes, signatures, compatibility, and activation are separate gates.
- Security fixes should include a regression test and a release-note entry when
  they affect a supported contract.

## Testing and release gates

Before a security-sensitive change is released, maintainers should run the
focused regression suite, the workspace tests, dependency auditing, and the
platform checks listed in [the release documentation](RELEASES.md). A release
must not claim support for a platform or execution mode that has not passed
those checks.

## Reviewed dependency backports

Tauri 2.11.5 and Wry 0.55.1 currently bind the supported Linux desktop to the
final GTK3 `gtk 0.18` dependency line. That line cannot resolve the published
`glib 0.20` fix for RUSTSEC-2024-0429 while Tauri's GTK4 migration remains
unreleased.

The repository therefore vendors the exact crates.io `glib 0.18.5` source and
applies the upstream one-line mutable out-argument fix from gtk-rs/gtk-rs-core
pull request 1343. The desktop manifest uses a Cargo source override; it does
not edit the lockfile version to hide the dependency. Repository validation
binds the reviewed source digest, crates.io revision, Cargo override, and fixed
code shape. The source provenance and retirement condition are recorded in
`third_party/glib-0.18.5-patched/PANDORA-PATCH.md`.

This override must be removed as soon as the supported Tauri/Wry release moves
Linux to `glib 0.20` or newer. The version-based RustSec exception is limited to
RUSTSEC-2024-0429; new advisories remain release-blocking.
