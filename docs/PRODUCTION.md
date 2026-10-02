# Production readiness

Pandora's production boundary keeps its native architecture intact:
Parliament and the Reference Monitor remain the only authorization path, while
self-improvement can propose, evaluate, admit, activate, and roll back
candidates only through the existing evidence gates.

Production readiness is Phase 6. Prompt caching, background agents, parallel
orchestration, evaluation primitives, and memory synthesis now exist. Their
remaining operating and release work is tracked in
[the audited roadmap](ROADMAP.md).

Desktop product work is cancelled. The retained Tauri adapter stays in the tree
and stays out of every build, signing, and publication gate.

## Identity and tenant isolation

The local service supports persisted identities with viewer, operator, and
administrator roles. Every identity is bound to one principal, tenant,
workspace, and device public key. Only token digests and device public keys are
stored in identities.sqlite3; bearer credentials and private device keys are
written to separate private files.

    pandora auth enroll --principal alice --tenant team-a --workspace-id product-a --role operator
    pandora auth list
    pandora auth revoke <identity-id> --yes

Every desktop RPC carries an Ed25519 device proof over the bearer credential
digest, timestamp, nonce, HTTP method, and RPC path. The service rejects stale
proofs, wrong-device proofs, revoked identities, and replayed nonces. Runtime
reads and mutations use the authenticated identity's tenant and workspace
scope; viewers cannot run tools, operators cannot activate evolution
candidates, and administrators retain the existing governed mutation gates.
Each service process owns exactly one physical workspace. Authenticated
operators and administrators can execute only when their tenant and workspace
match that deployment scope; a mismatched scope fails before allocation or
tool execution. Run separate service processes with separate operating-system
accounts and data roots for physically isolated tenant workspaces.
Evolution lineage and activation are deployment-global by design, so they are
additionally restricted to identities in the deployment owner's tenant and
workspace. A tenant administrator outside that owner scope cannot inspect or
change global evolution state.

The service remains loopback-only. This identity layer protects local clients
and tenant boundaries; it is not a public internet listener or an operating
system sandbox.

## Encrypted secrets

Provider profiles store an environment-variable reference, never a credential.
Pandora resolves that reference from the process environment first, then from
the tenant/workspace encrypted vault. Vaults use Argon2id and
XChaCha20-Poly1305 with scope-bound authenticated data.

    set PANDORA_MASTER_KEY=<a strong passphrase>
    pandora secret set OPENAI_API_KEY --value-stdin
    pandora secret list
    pandora secret status OPENAI_API_KEY

Use the platform's credential manager or service manager to inject
PANDORA_MASTER_KEY. Do not place it in the repository, command history,
configuration, desktop UI, telemetry, or a backup beside its archive.

## Recovery

Stop the Pandora service before creating or restoring a recovery archive so
SQLite files are quiescent.

    set PANDORA_BACKUP_KEY=<a separate strong passphrase>
    pandora backup create --output pandora-recovery.json
    pandora backup inspect --input pandora-recovery.json
    pandora backup restore --input pandora-recovery.json --yes

Recovery archives use Argon2id and XChaCha20-Poly1305. The archive authenticates
every relative path, content digest, and encrypted payload. Restore rejects
path traversal and symbolic-link targets, runs SQLite integrity checks before
writing, preserves previous files under data/recovery/pre-restore-*, and rolls
back files written before an error. The encrypted archive includes credentials
and device keys so it must still be access-controlled. Losing the backup
passphrase makes the archive unrecoverable.

Operations logs, crash reports, staged application updates, and earlier
recovery archives are excluded. Evolution rollback material is included so
recovery does not break Pandora's candidate lineage.

## Provider lifecycle evidence

Logical memory compaction and local archive creation do not expire cloud
backups, remove snapshots, or destroy encryption keys. Perform those actions in
the provider control plane, independently verify the terminal provider state,
retain the provider audit response, and hash that response before creating the
version 1 lifecycle manifest documented in [the CLI guide](CLI.md).

    pandora backup lifecycle preview --input lifecycle-evidence.json
    pandora backup lifecycle record --input lifecycle-evidence.json --yes
    pandora backup lifecycle inspect --id <evidence-id>
    pandora backup lifecycle list --storage-provider aws_s3 --limit 64

Preview must succeed before record. Recording persists an append-only,
digest-bound operator attestation in `data/storage-lifecycle.sqlite3`. An exact
retry is idempotent; a changed manifest cannot reuse an evidence ID. Database
triggers reject receipt updates and deletes. The ledger is part of normal state
backup, but it is evidence of an external action, not the action itself.

The runtime does not call a cloud deletion API, rotate a provider key, verify a
provider response, or promise secure erasure. Do not record key destruction
until recovery requirements have been reviewed: destroying the only usable key
can make retained archives permanently unrecoverable. Provider retention,
replication, legal hold, object versioning, soft-delete, and cryptographic
destruction semantics remain the operator's responsibility.

## Adversarial input and package trust evidence

Tool and adapter output remains untrusted even when transport, identity, and
signature checks succeed. The runtime records a typed origin and applies the
shared content guard before provider context and durable handoffs. Quarantined
content retains only a digest, byte count, policy version, and safe reason; a
persisted or forwarded envelope is revalidated instead of trusted by shape.

Publisher trust changes and package admission outcomes append to the package
transparency ledger. Operators can inspect the SHA-256 event chain with
`pandora package transparency list` and `inspect`; those commands are read-only
and do not grant runtime authority. Production CI also compiles and runs bounded
fuzz smoke campaigns against the path, manifest, MCP RPC, handoff, approval,
and persisted effect-receipt parsers. See
[adversarial resilience](ADVERSARIAL_RESILIENCE.md) for the exact boundary and
replay commands.

### Signed Skill and Provider distribution

Remote package discovery, download, admission, enablement or Provider selection,
and effect authorization are distinct boundaries. Registry and pinned-commit
GitHub downloads accept only Official manifests signed by an active
publisher-scoped Ed25519 root, verify the canonical manifest and artifact SHA-256
digests, enforce the running-runtime requirement and supported kind, and retain
the exact source revision. A successful download writes only the inert cache in
`packages.sqlite3` and cannot admit or enable anything.

Operators should inspect and offline-verify the exact cached record before a
dry-run and confirmed `package admit-cached`. Required dependencies are exact
SemVer identities and must already be admitted. Gene and Harness artifacts enter
the package store disabled; Skill bundles enter the managed Skill root disabled;
Provider JSON enters the profile catalog inactive. Enabling a package or Skill,
selecting a Provider, approving an effect, and issuing a one-shot permit remain
their existing separate decisions.

A Provider artifact is strict JSON containing only `id`, `name`, `protocol`,
`base_url`, `default_model`, and `api_key_env`. It contains a credential reference,
never a credential value. Its ID must equal the leaf of the signed package ID. An
active Provider cannot be replaced by admission. An operator must select the
profile explicitly after admission, and normal secret resolution still happens
inside the scoped encrypted-vault/environment boundary.

Exact replay of identical cached evidence is idempotent. An identity collision,
manifest or signature substitution, hash mismatch, downgrade through the normal
admission path, untrusted publisher, revoked key, unsupported kind, missing exact
dependency, malformed bundle, or traversal attempt fails closed. Trust-root
revocation marks matching cache records revoked, removes distribution bindings,
suspends admitted managed Skills, and quarantines admitted Provider profiles.
The cache and its SHA-256 event chain are included in normal recovery state; an
offline cache is usable only while all retained trust and integrity checks still
pass.

## Telemetry and crash records

Operational telemetry is local-only under data/operations/telemetry.jsonl. It
uses a closed event/status schema and contains no prompts, outputs,
credentials, arbitrary error text, or hidden reasoning. The current file
rotates at 4 MiB and one previous file is retained.

Crash records are local-only under data/operations/crashes. They contain the
component, version, time, and a one-way digest of the source location. Panic
payloads are deliberately omitted. Pandora retains at most twenty CLI crash
records. No telemetry or crash record is uploaded automatically.

## Updates and channels

Updates remain explicit and never resolve an ambiguous latest tag:

    pandora update --release v2.0.0-beta.8 --channel beta --dry-run
    pandora update --release v2.0.0-rc.1 --channel release-candidate --dry-run
    pandora update --release v2.0.0 --channel stable --dry-run
    pandora update --rollback

Stable channels accept plain SemVer releases, release-candidate accepts
`-rc.<n>` tags, and beta accepts the remaining prerelease tags. All channels
use the same release workflow and evidence contract; the validated source scope
controls which artifact classes are admissible. The downloaded binary must
match the release checksum manifest before
staging. The checksum manifest is itself authenticated with a keyless Cosign
signature, and both installers verify that signature by default before trusting
any checksum inside it. Release assets include that keyless Cosign signature for
the checksum
manifest, an SPDX SBOM, and GitHub build provenance.
The publish job also generates release-evidence.json, tying every
checksum-verified artifact to its signature, SBOM, and provenance subjects.
The smoke-install job installs the published binary on each supported runner
with signature verification enforced, so a manifest-signing regression fails
the release instead of shipping. Local artifacts can additionally require a
detached Ed25519 signature.

## Source-bound release scope

The tracked `release-scope.json` file is the release policy bound to the exact
tagged source commit. The workflow validates it before any build or publication
job and exposes only the validated `scope`, `channel`, and
`desktop_required` values to later jobs. It never infers a broader scope from a
release tag.

- `CLI-only` is the only active scope, valid for alpha and beta tags. It
  requires native CLI artifacts and the full evidence set. CLI-only evidence
  must not contain desktop artifacts and does not claim desktop parity.
- `full` is retired because desktop work is cancelled. Re-enabling it is an
  explicit source change and is not a supported path.

The checked-in policy currently selects `cli-only` for the next prerelease line.
Changing the file is an explicit source change, not a workflow override. Because
`full` is retired, release-candidate and stable stay fail-closed and cannot be
published from this repository.

The `Agent artifact pipeline` workflow validates every tracked SDK package,
runs a deterministic evaluation gate, proves the scheduled canary stops before
activation, and checks the shared release identity. A manual promotion intent
may run only from `main` and only through the protected
`promotion-beta`, `promotion-release-candidate`, or `promotion-stable`
environment. Repository administrators must configure each environment with
required human reviewers and restrict it to `main`. Enable self-review
prevention when a second eligible release reviewer exists. A single-reviewer
repository must leave that option disabled or every promotion deadlocks; the
evolution contract still independently forbids a scorecard evaluator from
approving its own promotion. The request supplies the exact commit, a
repository-relative tracked SDK `.artifact` or `.wasm` path with its sibling
manifest and SHA-256 digest, the rollout evidence digest, a unique
`promotion_id` record key, channel, and channel-valid SemVer tag. Validation
writes a canonical evidence manifest binding the
package-validation report, deterministic evaluation report, pipeline boundary
record, commit, and channel. The validation job recomputes the submitted
artifact and evidence digests before environment review; after review, the
promotion job downloads the exact run evidence and verifies both digests again
before creating a tag. `promotion_id` is a uniqueness key, not a
substitute for GitHub's protected-environment reviewer decision. After
environment approval, the workflow creates that one annotated tag and retains
an `approved-tag.json` evidence artifact. Existing tag annotations are checked
so the same record ID cannot authorize another tag or channel.

For promotion, first run the workflow without promotion intent and record the
manifest `digest` from the uploaded `evidence-manifest.json`. Then dispatch
promotion for the same `main` commit, supplying the repository-relative tracked
artifact path, its `sha256:<hex>` digest, the evidence-manifest digest, a unique
promotion ID, and the channel-valid tag. The workflow regenerates the stable
evidence bundle and fails before reviewer approval if either digest differs.

This promotion job grants only tag-creation authority. It does not publish a
release, admit a package, or activate an artifact. The tag-driven release
workflow remains the sole publication path, and the existing evolution
activation command remains the sole artifact activation path. GitHub's
protected-environment deployment record is the authoritative reviewer audit;
the retained JSON records the exact request binding and requester.

## Release boundary

All release channels use the source-bound `cli-only` scope selected by
`release-scope.json`. A `cli-only` alpha, beta, release-candidate, or stable
tag publishes the verified native CLI, launcher,
installers, and evidence set without desktop artifacts. A `full` alpha or beta
may publish unsigned native and desktop packages for testing, and GitHub marks
both as prereleases. The verification, native build, and desktop build jobs
receive no platform-signing secrets. A protected `sign-native` job handles
native CLI release approval and signing; it never signs a desktop package.
Desktop signing is not part of any supported release channel.

RC and stable no longer wait for desktop signing. The release gate requires
`scope=cli-only` and `desktop_required=false`; desktop feature parity remains
deferred, and no desktop artifact is built or published on any channel.

The published-package signature and notarization smoke checks are not part of
the CLI-only workflow. Desktop smoke is intentionally skipped on every channel;
the release provides CLI artifact, checksum, and native signing evidence only.
The provider-neutral request boundary is documented in
[external desktop signing](EXTERNAL_DESKTOP_SIGNING.md); no provider adapter or
signing evidence is configured.

Publication also waits at the protected `release-publication` environment, which
accepts only `v*` tags and requires a human reviewer. RC and stable source
verification validates the four strict native accessibility manifests against the
tag's exact commit before compilation.

A production release also requires:

- locked formatting, compilation, Clippy, Rust, Python, TypeScript, desktop,
  integration, and adversarial tests;
- CodeQL and dependency-audit success;
- clean-machine install, update, rollback, setup, doctor, and uninstall smoke
  tests on every advertised platform;
- accepted exact-commit NVDA, VoiceOver, and Orca evidence for every advertised
  desktop platform;
- checksums, signature certificate, SBOM, provenance, release notes, recovery
  instructions, and security reporting instructions;
- verification that no signing key, provider secret, master key, device key, or
  recovery passphrase entered source control or build logs.

Native RC/stable secrets and the currently blocked desktop-signing requirement
are documented in [the release policy](../RELEASES.md). Values belong only in
GitHub environment-scoped encrypted secrets and the corresponding platform
account; no future desktop signing secret may be added to an ordinary build
job.

After stable publication, a separate job runs only after every published CLI
and desktop lifecycle matrix succeeds. It records the exact compatible stable
predecessor. The first `v2.0.0` release remains explicitly
`pending_first_patch`; only a legitimate `v2.0.1` rollback drill can close the
stable-to-stable evidence gate.

### Linux desktop dependency patch

Tauri's current Linux webview stack still resolves the archived GTK3 bindings
and `glib` 0.18.5. The repository carries the reviewed upstream fix for RustSec
RUSTSEC-2024-0429 as an exact local source override with bound crates.io
provenance and a validated source digest. Dependency audit therefore evaluates
the patched source rather than accepting an advisory waiver. Remove the local
override as soon as Tauri's supported Linux stack moves to `glib` 0.20 or newer.
