# Release policy

Pandora uses SemVer tags. A stable major release also receives a product
codename in its title and notes; the codename does not change the tag. For
example, the stable release is titled `Anubis v2.0.0` and uses the tag
`v2.0.0`. Prereleases use neutral SemVer suffixes such as `v2.0.0-alpha.1`,
`v2.0.0-beta.1`, and `v2.0.0-rc.1`.

Existing legacy preview tags remain immutable.

The older `v2.0.0-anubis.1`, `v2.0.0-anubis.2`, and `v2.0.0-anubis.3` tags are
archived previews from an earlier naming scheme. They remain available for
history and reproducibility, but they are not the active release line or
recommended install targets.

The canonical prerelease tags for this line are `v2.0.0-alpha.1` through
`v2.0.0-alpha.5`. Codenames apply to stable major release titles only, never to
alpha, beta, or release-candidate tags.

## v2.0.0-alpha.1

This was the first CLI-only prerelease. Its shipped scope is recorded in
[CHANGELOG.md](CHANGELOG.md).

Historical preview tags are retained as immutable compatibility references and
are not the active release line.

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

## v2.0.0-alpha.2

This was a CLI-only prerelease. It carried the provider, session,
configuration, launcher, and CI improvements listed in
[CHANGELOG.md](CHANGELOG.md). The public command name remains `pandora`, and
the npm/Bun package name is `pandora-agent`.

The release remains a prerelease and does not claim desktop, mobile, remote,
or marketplace support. Native artifacts are valid only when the tagged
release workflow has completed and its checksums, signature, SBOM, and
provenance assets are present.

## v2.0.0-alpha.3

This CLI-only prerelease added the built-in `core-source` Source Harness,
session inspection, and completion support for session subcommands.

## v2.0.0-alpha.5

This is the current CLI-only prerelease. It adds local Skill package admission
through `pandora skill install`; admitted Skills remain disabled until an
operator enables them. The existing terminal clients and governed execution
path are unchanged.

## v2.0.0-alpha.4

This is the current CLI-only prerelease. It adds the line-oriented `pandora
chat` client and full-screen `pandora tui` client. Both reuse the existing
AgentLoop, session, approval, and governed effect path.

## Version rules

- Patch releases (`v2.0.x`) fix regressions without changing the public contract.
- Minor releases (`v2.x.0`) add compatible capabilities within the current release line.
- Stable major releases change the public contract, increment the major version, and receive a codename in the release title.
- Prereleases use `-alpha.N`, `-beta.N`, or `-rc.N` suffixes.
- Release tags are immutable. Each release must publish notes, checksums, an SBOM, supported-platform results, and rollback instructions.
- A release is not considered stable until local and GitHub checks pass on Windows, macOS, and Linux.
