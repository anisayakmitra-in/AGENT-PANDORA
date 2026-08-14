# Release policy

Pandora uses plain SemVer tags. Existing `v2.0.0-anubis.*` tags are historical
preview identifiers and remain immutable; new releases do not use product
codenames.

## Current preview

The current public preview is `v2.0.0-anubis.3`, published from `main`.

It provides the CLI-first foundation:

- Parliament, Shadow Council, Domain Harness, Gene, and governed execution contracts.
- One-shot effect permits, receipts, policy decisions, and workspace-scoped executors.
- A coding Domain Harness with bounded read, search, patch, verify, and review operations.
- Provider manifests, bounded model requests, OpenAI-compatible HTTP transport, tool-call validation, and structured-output repair.
- Windows, macOS, and Linux CI, repository validation, dependency auditing, and release-build checks.

The preview is not a stable release and does not claim desktop support. Native
release artifacts are valid only when the tagged release workflow has
completed and its checksums, signature, SBOM, and provenance assets are
present.

## Version rules

- Patch releases (`v2.0.x`) fix regressions without changing the public contract.
- Minor releases (`v2.x.0`) add compatible capabilities within the current release line.
- Major releases change the public contract and increment the major version.
- Prereleases use `-alpha.N`, `-beta.N`, or `-rc.N` suffixes.
- Release tags are immutable. Each release must publish notes, checksums, an SBOM, supported-platform results, and rollback instructions.
- A release is not considered stable until local and GitHub checks pass on Windows, macOS, and Linux.
