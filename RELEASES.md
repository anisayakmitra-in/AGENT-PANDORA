# Release map

Pandora uses SemVer tags for tooling and release codenames for product milestones. A codename never replaces the version number.

## Anubis — `v2.0.0`

Anubis is Phase 1 and the first CLI-first release line. The current public
preview is `v2.0.0-anubis.2`, published from `main`.

Anubis adds the CLI-first foundation:

- Parliament, Shadow Council, Domain Harness, Gene, and governed execution contracts.
- One-shot effect permits, receipts, policy decisions, and workspace-scoped executors.
- A coding Domain Harness with bounded read, search, patch, verify, and review operations.
- Provider manifests, bounded model requests, OpenAI-compatible HTTP transport, tool-call validation, and structured-output repair.
- Windows, macOS, and Linux CI, repository validation, dependency auditing, and release-build checks.

The preview is a prerelease. It documents the current CLI foundation and does
not claim stable desktop support. Native release artifacts are valid only when
the tagged release workflow has completed and its checksums, signature, SBOM,
and provenance assets are present.

## Naming

Anubis owns the `v2.0.x` release line. Patch releases keep the phase name:
`Anubis v2.0.1` and `Anubis v2.0.2` are fixes to the same contract, not new
phases.

## Version rules

- Patch releases (`v2.0.x`) fix regressions without changing the Anubis contract.
- Minor releases (`v2.x.0`) add compatible capabilities within the current release line.
- Major releases change the public contract and receive a new codename.
- Release tags are immutable. Each release must publish notes, checksums, an SBOM, supported-platform results, and rollback instructions.
- A named release is not considered stable until local and GitHub checks pass on Windows, macOS, and Linux.
