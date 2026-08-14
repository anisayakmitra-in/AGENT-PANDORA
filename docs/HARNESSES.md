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

`pandora-runtime::HarnessRegistry` compares the declared package metadata with the embedded manifest, hashes the supplied bytes, checks required dependencies, and records the verified metadata in an in-memory registry.

The registry does not load code, enable a Harness, issue a permit, or grant runtime authority. A recorded package is metadata that passed admission, not an executable extension.

## Package kinds

The external vocabulary is closed and uses these exact values:

`gene`, `domain_harness`, `meta_harness`, `source_harness`, `package`, `provider`, and `skill`.

Only `gene` metadata can pass the current package-install boundary. Domain, Meta, Source, Provider, Skill, and generic Package records are recognized but rejected as non-installable until their lifecycles have their own validation and execution rules.

The built-in Coding Domain Harness remains the only executable Domain Harness in Anubis. Downloaded native code is never executed automatically.

## Ownership

The package envelope is owned by `pandora-types`. Existing Harness and Gene manifests remain implementation contracts; `PackageManifest::from_harness` is the explicit adapter between them.

K-O Palace remains a separate registry. Pandora may consume verified package metadata later, but Palace does not own activation, permissions, runtime events, or execution policy.
