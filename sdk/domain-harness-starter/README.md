# Pandora Domain Harness starter

This is a copyable, deterministic reference package for a declarative Domain
Harness. It owns the exact built-in `workspace.read@0.1.0` Gene and declares
one optional Auto Route hint. The artifact is metadata only; Pandora never
loads it as native code.

You can also generate a fresh starter with the installed CLI:

```text
pandora package scaffold domain-harness --output my-domain --id example/my-domain --version 1.0.0 --gene workspace.read@0.1.0 --route-hint "my domain"
```

## Build and validate

No separate compiler is required for a metadata-only profile. Copy this
directory, edit the manifest and artifact together, then update the manifest's
SHA-256 `content_hash`. Run the non-persistent validator from the package
directory:

```text
pandora package validate --manifest pandora.package.json --artifact domain-harness.artifact
```

The manifest uses strict package SemVer, exact Gene versions, and exact
compatibility with Pandora `2.0.0-beta.7`. Change the runtime requirement when
targeting another Pandora build. Unknown fields—including a `capabilities`
field—are rejected. Duplicate Gene IDs, noncanonical or duplicate route hints,
an incorrect artifact hash, and invalid identity or SemVer values fail closed.

## Admit, enable, inspect, and disable

```text
pandora package admit --manifest pandora.package.json --artifact domain-harness.artifact
pandora package enable example/domain-starter 1.0.0 --dry-run
pandora package enable example/domain-starter 1.0.0 --yes
pandora package inspect example/domain-starter 1.0.0
pandora package disable example/domain-starter 1.0.0 --dry-run
pandora package disable example/domain-starter 1.0.0 --yes
```

Admission resolves every required Gene and starts the package disabled. The
preview reports dependencies and blockers without changing state. Confirmation
changes the exact-version package binding only.

## Update and rollback

Copy the package, change both version fields to a new exact SemVer, update the
artifact hash, then validate and admit it. Enable the original version followed
by the new version. Pandora retains one exact rollback target:

```text
pandora package rollback example/domain-starter --dry-run
pandora package rollback example/domain-starter --yes
```

All commands are local. The starter needs no network request or credential.
Read [ARCHITECTURE.md](ARCHITECTURE.md) before adding Genes or route hints.
