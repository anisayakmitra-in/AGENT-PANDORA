# Release map

Pandora uses SemVer tags for tooling and release codenames for product milestones. A codename never replaces the version number.

## Anubis — `v2.0.0`

Anubis is the first production release line. The current workspace is still `2.0.0-alpha.1`; no Anubis tag exists yet.

Planned sequence:

- `v2.0.0-anubis.1`: first public preview.
- `v2.0.0-rc.1`: release candidate after cross-platform install and upgrade checks.
- `v2.0.0`: stable Anubis release.

Anubis adds the CLI-first foundation:

- Parliament, Shadow Council, Domain Harness, Gene, and governed execution contracts.
- One-shot effect permits, receipts, policy decisions, and workspace-scoped executors.
- A coding Domain Harness with bounded read, search, patch, verify, and review operations.
- Provider manifests, bounded model requests, OpenAI-compatible HTTP transport, tool-call validation, and structured-output repair.
- Windows, macOS, and Linux CI, repository validation, dependency auditing, and release-build checks.

Anubis is complete only when the stable CLI can be installed, configured, run, resumed, diagnosed, updated, and removed on a clean machine without Rust.

## Version rules

- Patch releases (`v2.0.x`) fix regressions without changing the Anubis contract.
- Minor releases (`v2.x.0`) add compatible capabilities within the current release line.
- Major releases change the public contract and receive a new codename.
- Release tags are immutable. Each release must publish notes, checksums, an SBOM, supported-platform results, and rollback instructions.
- A named release is not considered stable until local and GitHub checks pass on Windows, macOS, and Linux.
