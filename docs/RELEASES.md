# Releases

Pandora uses standard Semantic Versioning and publishes the CLI as the primary
product. Release tags are immutable and are never force-pushed or reused.

## Current release

`v2.0.0-alpha.6` is the current public prerelease. It is suitable for testing
the CLI contract, not a stable production guarantee. The release is CLI-only
and supports native Windows x64, Linux x64, and macOS Intel and Apple Silicon
artifacts.

The release includes:

- setup, doctor, provider profiles, and configuration migration;
- read-only runs, bounded agent runs, sessions, and resumable transcripts;
- the built-in `core-source` Source Harness and session inspection;
- the governed Coding Domain Harness;
- Skills, Tools, approvals, orchestration roles, and strategy discovery;
- the line-oriented `pandora chat` client and full-screen `pandora tui` client;
- local Skill package admission with disabled-by-default activation;
- enabled Skill guidance is included in agent context only when explicitly
  enabled, with bounded size and no authority to change policy or permissions;
- verified update, rollback, uninstall, shell completion, and npm/Bun launchers.

This release remains CLI-only and is a prerelease. Native artifacts are valid
only when the tagged release workflow and its checksums, signature, SBOM, and
provenance assets are present.

## Versioning

- `v2.0.0-alpha.N`, `v2.0.0-beta.N`, and `v2.0.0-rc.N` are prereleases.
- `v2.0.0` is the stable release for the v2 public contract.
- `v2.0.x` contains compatible fixes.
- `v2.x.0` contains compatible capabilities within the v2 contract.
- A public-contract break requires a new major version.

Prerelease tags are not stable releases. A stable release requires passing local
and GitHub checks, verified native artifacts for every supported platform, clean
installation and upgrade tests, checksums, a signed checksum manifest, an SPDX
SBOM, build provenance, release notes, and rollback instructions.

## Artifact policy

Every published release is built from the tagged source and publishes the native
CLI, bootstrap installers, and the npm/Bun launcher. Installers verify the
artifact checksum before replacing an existing binary. Optional signature
verification uses the release's Cosign certificate and signature.

WSL uses the Linux CLI environment and is not a separate release target. Pandora
does not claim desktop, mobile, or marketplace support without a corresponding
packaged artifact and platform validation.

## Release history

`v2.0.0-alpha.1` is the first plain-SemVer v2 CLI prerelease. Earlier codename
preview tags remain immutable historical artifacts and are not part of the
current release naming scheme.
