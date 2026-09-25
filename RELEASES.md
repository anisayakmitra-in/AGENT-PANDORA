# Release policy

Pandora ships as a CLI. Desktop product work is cancelled, so the retained
Tauri adapter has no build, signing, publication, or support path, and this
document treats every desktop reference as historical context.

Pandora uses plain SemVer tags and release titles. Stable and prerelease
versions use the same tag format; prereleases use neutral suffixes such as
`v2.0.0-alpha.1`, `v2.0.0-beta.1`, and `v2.0.0-rc.1`.

Existing legacy preview tags remain immutable.

The older `v2.0.0-anubis.1`, `v2.0.0-anubis.2`, and `v2.0.0-anubis.3` tags are
archived previews from an earlier naming scheme. They remain available for
history and reproducibility, but they are not the active release line or
recommended install targets.

The canonical published prerelease tags for this line are `v2.0.0-alpha.1`
through `v2.0.0-alpha.6`, followed by `v2.0.0-beta.1`, `v2.0.0-beta.2`,
`v2.0.0-beta.3`, `v2.0.0-beta.4`, `v2.0.0-beta.5`, `v2.0.0-beta.6`, and
`v2.0.0-beta.7`. The next planned identity is `v2.0.0-beta.8`.

## Source-bound release scope

Every new tag is evaluated against the exact tracked `release-scope.json` at its
source commit. This is a release contract, not a mutable workflow input:

- `CLI-only` is the only active scope, covering alpha and beta tags. It
  publishes the native CLI, launcher, installers, checksums, signature, SBOM,
  provenance, notes, and CLI lifecycle evidence. Its release evidence must not
  contain desktop artifacts, and it does not claim desktop parity.
- `full` is retired. It would require desktop build, publication, smoke, and
  (for stable) rollback evidence, and desktop work is cancelled. Re-enabling it
  means editing `release-scope.json` in source, which is not a supported path.

The workflow skips desktop build, desktop artifact download and attestation,
desktop smoke, and desktop rollback closure when the validated scope is
`cli-only`. A skipped desktop job is never treated as successful full-scope
evidence. Release-candidate and stable cannot use `cli-only`, so with `full`
retired both channels stay fail-closed and cannot be published from this
repository. The external desktop-signing boundary in
[external desktop signing](docs/EXTERNAL_DESKTOP_SIGNING.md) is non-enabling
and has no provider adapter, trust root, or credentials.

The checked-in policy selects `cli-only` for the next prerelease line.

## v2.0.0-beta.8

This is the next coherent prerelease identity. It updates the coupled Rust,
npm, desktop, installer, documentation, and SDK runtime surfaces to
`2.0.0-beta.8` while retaining the source-bound `cli-only` release scope. The
identity is prepared for review and is not a public release until its exact tag
workflow completes; RC and stable remain fail-closed pending isolated desktop
signing.

## v2.0.0-beta.7

This beta groups bounded self-healing feedback, composition provenance,
verified memory synthesis, persisted graph snapshots, holdout evaluation, and
evidence-only evolution proposal intake. It is a CLI-first prerelease with a
tested Tauri desktop source, not a stable distribution claim.
The release workflow publishes release-evidence.json, linking checksums,
signatures, SBOM, provenance subjects, and platform artifacts.

## v2.0.0-beta.6

This beta adopts MIT licensing for Pandora-owned material, adds the native
Gemini provider adapter, hardens process-tree cleanup for verification and
MCP operations, adds durable evolution records and read-only evolution
inspection, and documents the governed Security Domain workflow. It is a
CLI-only prerelease; no desktop, remote-execution, or marketplace support is
claimed.

## v2.0.0-beta.5

This beta adds the read-only Data Domain Harness and completes the bounded
Security Domain workflow surface with a standard assessment entry point,
deep-scan evidence, and changed-code evidence. It remains a CLI-only
prerelease; these workflows do not execute scanners, modify code, or claim
security certification.

## v2.0.0-beta.4

This beta consolidates the read-only Security and Debugging Domain Harnesses.
It is a CLI-only prerelease with bounded evidence workflows, cross-platform
release verification, and no desktop or remote-execution support claim.

## v2.0.0-beta.3

This beta packages the governed Security Domain Harness, typed TypeScript CLI
client, and the current local MCP and runtime authority improvements. It is a
CLI-only prerelease; the built-in security workflows provide bounded evidence
and do not claim complete vulnerability scanning or automated remediation.

## v2.0.0-beta.2

This beta advances the CLI release line with durable context assembly caching,
bounded graph intelligence, deterministic golden-set evaluation, and the local
Fleet control plane. It also exposes those capabilities through stable JSON
commands for graph projections, evaluation, and Fleet operations. Runtime
authority remains unchanged: these projections and leases cannot issue effect
permits or bypass the ReferenceMonitor.

## v2.0.0-beta.1

This is the current CLI-only beta. It consolidates the governed local runtime,
authenticated loopback service, durable jobs, isolated subagents, local MCP,
signed package admission, bounded research evolution, release lifecycle tests,
and stable JSON automation contract recorded in [CHANGELOG.md](CHANGELOG.md).
Every effect remains bound to Parliament policy, a scoped execution profile,
the ReferenceMonitor, and a one-shot permit.

## v2.0.0-alpha.6

This was the final alpha prerelease. Enabled Skills contribute bounded
reference guidance to agent context. Disabled and suspended Skills are omitted,
oversized guidance fails closed, and Skill text cannot authorize effects,
change policy, satisfy approvals, or execute scripts.

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

This was a previous CLI-only prerelease. It added local Skill package admission
through `pandora skill install`; admitted Skills remain disabled until an
operator enables them. The existing terminal clients and governed execution
path are unchanged.

## v2.0.0-alpha.4

This was a previous CLI-only prerelease. It added the line-oriented `pandora
chat` client and full-screen `pandora tui` client. Both reuse the existing
AgentLoop, session, approval, and governed effect path.

## Version rules

- Patch releases (`v2.0.x`) fix regressions without changing the public contract.
- Minor releases (`v2.x.0`) add compatible capabilities within the current release line.
- Stable major releases change the public contract and increment the major version.
- Prereleases use `-alpha.N`, `-beta.N`, or `-rc.N` suffixes.
- Release tags are immutable. Each release must publish notes, checksums, a signature, an SBOM, a release evidence index, supported-platform results, and rollback instructions.
- A release is not considered stable until local and GitHub checks pass on Windows, macOS, and Linux.
- Release-candidate and stable publication must pass the protected
  `release-publication` environment. Only `v*` tags may deploy through it, and a
  configured human reviewer must approve the publication job.
- Native CLI signing is isolated in a protected job that downloads immutable
  unsigned build output. Verification, native build, and desktop build jobs do
  not receive platform-signing secrets.
- A future release candidate must have the same Windows Authenticode and Apple
  code-signing controls as stable. `full` alpha and beta packages may remain
  unsigned and must not be described as release candidates; `cli-only` packages
  contain native artifacts only.
- RC and stable publication currently fail closed before public release because
  isolated Tauri desktop signing is not yet configured. Desktop feature parity
  remains deferred. A `cli-only` source does not build or publish desktop assets;
  a `full` source keeps desktop artifact generation a publication blocker until
  the isolated signing path is available.
- The published-package signature and notarization smoke checks are not reachable
  in the current workflow while RC and stable remain blocked. `cli-only` alpha
  and beta releases intentionally skip desktop smoke, while a future `full`
  alpha or beta release can exercise only unsigned desktop lifecycle; neither
  path currently provides platform-signature or notarization evidence. The
  provider-neutral request and response requirements are documented in
  [the external signing boundary](docs/EXTERNAL_DESKTOP_SIGNING.md); no
  provider adapter or signing evidence is configured.
- Stable requires accepted exact-commit native NVDA, VoiceOver, and Orca evidence
  for every advertised desktop platform.

## Stable release credentials

Alpha and beta tags use their source-bound scope and are marked as prereleases.
A `cli-only` release publishes native artifacts only; a `full` release may
publish unsigned native and desktop packages. A future release-candidate tag
enters the protected native signing job only when
`PANDORA_RELEASE_CANDIDATE_APPROVED` is exactly `1`; a plain SemVer stable tag
likewise requires `PANDORA_STABLE_RELEASE_APPROVED` to be exactly `1`. Native
signing consumes only these environment-scoped secrets:

- PANDORA_WINDOWS_CERTIFICATE_BASE64 and
  PANDORA_WINDOWS_CERTIFICATE_PASSWORD;
- PANDORA_APPLE_CERTIFICATE_BASE64 and
  PANDORA_APPLE_CERTIFICATE_PASSWORD;
- APPLE_SIGNING_IDENTITY and APPLE_TEAM_ID.

The protected job copies immutable unsigned Windows or macOS output to its
final native-release path before signing, verifies that exact copied path, and
uploads the same path without executing the downloaded binary. The unsigned
build job never imports or receives the certificate. Apple notarization account
credentials are not exposed to the native signer.

RC and stable remain blocked until a separately reviewed remote/HSM-backed or
otherwise isolated Tauri desktop signing design is implemented and receives its
own protected credentials. Certificate values and account credentials must never
be committed, printed, placed in an artifact, or copied into recovery archives.

Published-package smoke jobs still re-download checksum-bound artifacts. The
CLI lifecycle runs for both scopes, while desktop smoke runs only for `full`
scope. Windows Authenticode, Apple signature, notarization, and Gatekeeper
checks are not reachable in the current workflow while RC and stable remain
blocked. Current evidence therefore covers native lifecycle, and any permitted
unsigned desktop lifecycle, but not platform-signature or notarization evidence.

## Stable rollback closure

The stable release index never claims rollback closure before published
lifecycle jobs run. After every stable publication, the release workflow waits
for all CLI and desktop install/update/backup/restore/rollback/uninstall jobs and
then emits `stable-rollback-evidence.json` bound to the tag, commit, predecessor,
and workflow run.

The first stable release in a compatible line has no older stable artifact. Its
evidence therefore records `pending_first_patch`, not success. For the 2.0 line,
the stable-to-stable gate can close only when `v2.0.1` legitimately rolls back
to the published `v2.0.0` artifacts. A patch tag without a compatible stable
predecessor fails closed.

All channels continue to require the signed checksum manifest, SPDX SBOM,
GitHub provenance, release notes, cross-platform tests, and published
install/update/rollback smoke tests described in
[production readiness](docs/PRODUCTION.md).
