# Production readiness

Pandora's production boundary keeps its native architecture intact:
Parliament and the Reference Monitor remain the only authorization path, while
self-improvement can propose, evaluate, admit, activate, and roll back
candidates only through the existing evidence gates.

Production readiness is Phase 6. Higher-level agent-platform work such as
prompt caching, background agents, parallel orchestration, evaluation-driven
loops, memory consolidation, self-healing tests, agent CI/CD, and the
OpenDesign-informed frontend direction is tracked in [the roadmap](ROADMAP.md).

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
    pandora update --release v2.0.0 --channel stable --dry-run
    pandora update --rollback

Stable channels accept plain SemVer releases; beta channels accept prerelease
tags. The downloaded binary must match the release checksum manifest before
staging. Local artifacts can additionally require a detached Ed25519
signature. Release assets include a keyless Cosign signature for the checksum
manifest, an SPDX SBOM, and GitHub build provenance.

## Release boundary

Prerelease tags may publish unsigned platform packages for testing, and GitHub
marks them as prereleases. A stable tag fails closed unless the repository has
an explicit stable-release approval plus a Windows code-signing certificate
and Apple signing/notarization credentials. The release workflow signs native
Windows and macOS binaries when credentials are present, signs Windows desktop
installers, and gives Tauri the Apple identity and notarization credentials.

A production release also requires:

- locked formatting, compilation, Clippy, Rust, Python, TypeScript, desktop,
  integration, and adversarial tests;
- CodeQL and dependency-audit success;
- clean-machine install, update, rollback, setup, doctor, and uninstall smoke
  tests on every advertised platform;
- checksums, signature certificate, SBOM, provenance, release notes, recovery
  instructions, and security reporting instructions;
- verification that no signing key, provider secret, master key, device key, or
  recovery passphrase entered source control or build logs.

Required stable-release secrets are documented by name in
[the release policy](../RELEASES.md). Their values belong only in GitHub
encrypted secrets and the corresponding platform account.

### Linux desktop dependency exception

Tauri's current Linux webview stack still resolves the archived GTK3 bindings
and `glib` 0.18.5. RustSec RUSTSEC-2024-0429 affects only
`glib::VariantStrIter` string-iterator methods. Pandora and the resolved Tauri,
Wry, WebKitGTK, and GTK runtime sources do not call those methods, so the
desktop audit records that advisory as a reviewed exception while continuing
to fail on vulnerabilities and yanked crates. Remove the exception as soon as
Tauri's supported Linux stack moves to `glib` 0.20 or newer. The Linux desktop
artifact remains an explicitly monitored platform, not an implied waiver for
new advisories.
