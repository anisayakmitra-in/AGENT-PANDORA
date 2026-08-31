# Pandora Meta Harness starter

This copyable starter is a deterministic, composition-only Meta Harness. It
coordinates the built-in `coding-domain@0.1.0` and
`research-domain@0.1.0` profiles with a maximum of four handoffs. It owns no
Genes and has no effect authority.

Generate a fresh exact version with the local CLI:

```text
pandora package scaffold meta-harness --output my-meta --id example/my-meta --version 1.0.0 --domains coding-domain@0.1.0,research-domain@0.1.0 --max-handoffs 4
```

The generator does not read credentials, contact a network, admit a package,
or change an active binding. The TUI command `/meta-starter` repeats this
guidance without writing files.

## Validate and inspect

From this directory:

```text
pandora package validate --manifest pandora.package.json --artifact meta-harness.artifact
pandora package admit --manifest pandora.package.json --artifact meta-harness.artifact
pandora package enable example/meta-starter 1.0.0 --dry-run
pandora package enable example/meta-starter 1.0.0 --yes
pandora package inspect example/meta-starter 1.0.0
pandora package disable example/meta-starter 1.0.0 --dry-run
pandora package disable example/meta-starter 1.0.0 --yes
```

Admission is disabled by default. Exact required custom Domains must already
be admitted and enabled before the Meta profile can be enabled. Built-in
Domains are resolved by their compiled exact versions.

For rollback evidence, scaffold and admit `2.0.0` with the same package ID,
enable `1.0.0`, enable `2.0.0`, and then run:

```text
pandora package rollback example/meta-starter --dry-run
pandora package rollback example/meta-starter --yes
```

Inspection reports composition, exact dependencies, unverified trust,
activation generation, active version, and rollback target. See
[`ARCHITECTURE.md`](ARCHITECTURE.md) for the authority boundary.

## Failure fixtures

The CLI and runtime tests exercise unknown, disabled, duplicate, self-cyclic,
wrong-kind, incompatible, and over-limit compositions. Each fails before any
effect execution. Orchestration tests also reject an undeclared Domain and a
plan above this starter's four-handoff ceiling.
