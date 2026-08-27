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
