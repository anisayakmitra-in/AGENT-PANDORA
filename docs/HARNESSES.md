# Harness packages

Pandora keeps package metadata separate from runtime authority.

## Shipped

`pandora-types::PackageManifest` is the package envelope. It records:

- package ID and exact version;
- package kind;
- publisher;
- SHA-256 artifact hash;
- dependencies and runtime compatibility;
- license;
- inline trust evidence.

`pandora-runtime::PackageStore` compares the declared package metadata with the embedded manifest, hashes the supplied bytes, checks required dependencies, and records the verified metadata in the local `packages.sqlite3` store. The store retains the artifact bytes so a later process can revalidate the record before using it.

The package store does not load code, enable a Harness, issue a permit, or grant runtime authority. A recorded package is metadata that passed admission, not an executable extension.

The built-in `core-source` Harness is the runtime's Source Harness. It binds the
`pandora-runtime` constitutional service and owns no Genes. It is discoverable
and inspectable, but it is not a user-runnable task target. Source Harnesses
augment one constitutional service; they do not provide a second execution
hierarchy.

The built-in `coordination-meta` Meta Harness declares the Domain Harnesses it
may coordinate and its handoff ceiling. It owns no Genes. The orchestration
engine checks those limits before registering a plan, so a plan cannot introduce
an undeclared Domain Harness through a Meta Harness.

`pandora harness inspect coordination-meta --json` exposes that composition
boundary as `meta_composition.allowed_domains` and
`meta_composition.max_handoffs`. This is metadata for routing and validation;
it is not a permission grant.

Local package admission is explicit:

```text
pandora package admit --manifest <manifest.json> --artifact <artifact>
pandora package list
pandora package inspect <id> <version>
```

The command uses one local manifest as both the declared and embedded record.
It is a local admission path, not a signature verifier or a registry client.

`pandora harness list --json` keeps built-in Harnesses under `harnesses` and
reports locally admitted package records separately under `package_records`.
Those records are discoverable metadata; they are not active Harnesses and do
not become runnable through discovery.

Meta Harnesses coordinate existing Domain Harnesses. They do not augment a
constitutional service, execute effects, install packages, or grant permits.

## Domain profiles

`DomainAgent` is a runtime profile of a Domain Harness, not a fourth Harness
kind. It gives one Domain Harness a selected role set, provider bindings, and
bounded work loop while preserving the same Genes and effect policy.

A `Swarm` is a Domain Harness composition profile for multiple workers in one
domain. It declares worker roles, handoff limits, and shared budgets. A Swarm
does not create a parallel execution hierarchy. Cross-domain work still goes
through a Meta Harness and its declared composition.

## Package kinds

The external vocabulary is closed and uses these exact values:

`gene`, `domain_harness`, `meta_harness`, `source_harness`, `package`, `provider`, and `skill`.

`gene` packages can pass the executable package-install boundary. `meta_harness`
packages can pass the separate composition-profile admission boundary described
below. Domain, Source, Provider, Skill, and generic Package records remain
recognized but rejected until their lifecycles have their own validation and
execution rules.

Custom Meta Harness packages may be admitted as composition-only metadata
profiles. Admission verifies their declared Domain members, exact artifact
hash, package identity, and required dependencies. Admission records use the
`admitted` state and never activate a Harness, execute native code, issue a
permit, or grant runtime authority.

The built-in Coding Domain Harness remains the only executable Domain Harness in
the current preview. Downloaded native code is never executed automatically.

## Ownership

The package envelope is owned by `pandora-types`. Existing Harness and Gene manifests remain implementation contracts; `PackageManifest::from_harness` is the explicit adapter between them.

K-O Palace remains a separate registry. Pandora may consume verified package metadata later, but Palace does not own activation, permissions, runtime events, or execution policy.
