# Security policy

Pandora treats tool execution, package admission, provider access, credentials,
and persisted agent state as security boundaries.

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

## Scope

High-value reports include permit bypasses, approval reuse, package identity or
signature confusion, path traversal, credential exposure, cross-workspace data
leaks, unsafe process execution, and tenant or provider-cache contamination.

The local MCP preview assumes an explicitly configured operator-trusted process;
it is not an OS sandbox for hostile native executables.

## Security design commitments

- No valid permit means no governed effect.
- Permits are scoped, expiring, and one-shot.
- Runtime policy remains outside model output and self-improvement loops.
- Credentials and hidden reasoning are not persisted as ordinary evidence.
- Package hashes, signatures, compatibility, and activation are separate gates.
- Security fixes should include a regression test and a release-note entry when
  they affect a supported contract.
